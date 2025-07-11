// src/bin/zone_monitor/active_order_manager.rs
// Focus on OPEN trades monitoring with Redis storage and zone enrichment
// Independent from PendingOrderManager - tracks live trades with real-time P&L

use chrono::{DateTime, Utc};
use redis::{Client, Commands};
use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

use std::sync::Arc;
use crate::db::order_db::{OrderDatabase, EnrichedDeal};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenPosition {
    #[serde(rename = "positionId")]
    pub position_id: u64,
    #[serde(rename = "symbolId")]
    pub symbol_id: u32,
    pub volume: u32,
    #[serde(rename = "tradeSide")]
    pub trade_side: u32,
    pub price: f64,
    #[serde(rename = "openTimestamp")]
    pub open_time: u64,
    pub status: u32,
    pub swap: f64,
    pub commission: f64,
    #[serde(rename = "usedMargin")]
    pub used_margin: u32,
    #[serde(rename = "stopLoss")]
    pub stop_loss: Option<f64>,
    #[serde(rename = "takeProfit")]
    pub take_profit: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedActiveTrade {
    // Basic position info from broker
    pub position_id: u64,
    pub symbol: String,
    pub trade_side: String, // "BUY" or "SELL"
    pub volume_lots: f64,
    pub entry_price: f64,
    pub current_price: f64,
    pub unrealized_pnl: f64,
    pub pips_profit: f64,
    pub open_time: DateTime<Utc>,
    pub duration_minutes: i64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub swap: f64,
    pub commission: f64,
    pub label: String,
    pub comment: String,

    // Enriched zone data (if available from original pending order)
    pub zone_id: Option<String>,
    pub zone_type: Option<String>, // "supply_zone" or "demand_zone"
    pub zone_strength: Option<f64>,
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub touch_count: Option<i32>,
    pub timeframe: Option<String>,
    pub distance_when_placed: Option<f64>,
    pub original_entry_price: Option<f64>, // From pending order
    pub slippage_pips: Option<f64>,

    // Real-time metrics
    pub distance_from_entry_pips: f64,
    pub risk_reward_ratio: Option<f64>, // Current R:R based on SL/TP
    pub is_profitable: bool,
    pub max_drawdown_pips: Option<f64>, // Track worst point
    pub max_profit_pips: Option<f64>,   // Track best point
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PositionsResponse {
    pub positions: Vec<OpenPosition>,
    #[serde(default)]
    pub orders: Vec<serde_json::Value>, // We don't need to parse orders in ActiveOrderManager
}

#[derive(Debug)]
pub struct ActiveOrderManager {
    api_url: String,
    http_client: reqwest::Client,
    symbol_id_mapping: HashMap<u32, String>,
    redis_client: Option<Client>,
    processed_positions: Arc<tokio::sync::Mutex<std::collections::HashSet<u64>>>,
    position_metrics: Arc<tokio::sync::Mutex<HashMap<u64, (f64, f64)>>>, // position_id -> (max_profit, max_drawdown)
    order_database: Option<Arc<OrderDatabase>>,
}

impl ActiveOrderManager {
    pub fn new(order_database: Option<Arc<OrderDatabase>>) -> Self {
        let api_url = std::env::var("CTRADER_API_BRIDGE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8000".to_string());

        // Initialize symbol ID mapping
        let mut symbol_id_mapping = HashMap::new();
        symbol_id_mapping.insert(185, "EURUSD".to_string());
        symbol_id_mapping.insert(199, "GBPUSD".to_string());
        symbol_id_mapping.insert(226, "USDJPY".to_string());
        symbol_id_mapping.insert(222, "USDCHF".to_string());
        symbol_id_mapping.insert(158, "AUDUSD".to_string());
        symbol_id_mapping.insert(221, "USDCAD".to_string());
        symbol_id_mapping.insert(211, "NZDUSD".to_string());
        symbol_id_mapping.insert(175, "EURGBP".to_string());
        symbol_id_mapping.insert(177, "EURJPY".to_string());
        symbol_id_mapping.insert(173, "EURCHF".to_string());
        symbol_id_mapping.insert(171, "EURAUD".to_string());
        symbol_id_mapping.insert(172, "EURCAD".to_string());
        symbol_id_mapping.insert(180, "EURNZD".to_string());
        symbol_id_mapping.insert(192, "GBPJPY".to_string());
        symbol_id_mapping.insert(191, "GBPCHF".to_string());
        symbol_id_mapping.insert(189, "GBPAUD".to_string());
        symbol_id_mapping.insert(190, "GBPCAD".to_string());
        symbol_id_mapping.insert(195, "GBPNZD".to_string());
        symbol_id_mapping.insert(155, "AUDJPY".to_string());
        symbol_id_mapping.insert(156, "AUDNZD".to_string());
        symbol_id_mapping.insert(153, "AUDCAD".to_string());
        symbol_id_mapping.insert(210, "NZDJPY".to_string());
        symbol_id_mapping.insert(162, "CADJPY".to_string());
        symbol_id_mapping.insert(163, "CHFJPY".to_string());
        symbol_id_mapping.insert(205, "NAS100".to_string());
        symbol_id_mapping.insert(220, "US500".to_string());

        let redis_client = match Client::open("redis://127.0.0.1:6379/") {
            Ok(client) => {
                info!("📊 Redis connected for active trade monitoring");
                Some(client)
            }
            Err(e) => {
                warn!("📊 Redis not available: {}, continuing without Redis", e);
                None
            }
        };

        info!("📋 Active Order Manager initialized:");
        info!("   API URL: {}", api_url);
        info!("   Symbol mappings: {}", symbol_id_mapping.len());
        info!("   Focus: Open trades monitoring with Redis storage");

        Self {
            api_url,
            http_client: reqwest::Client::new(),
            symbol_id_mapping,
            redis_client,
            processed_positions: Arc::new(
                tokio::sync::Mutex::new(std::collections::HashSet::new()),
            ),
            position_metrics: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            order_database,
        }
    }

    // No InfluxDB connection needed - ActiveOrderManager is Redis-only for transitory data

    /// Fetch open positions from broker API
    pub async fn fetch_open_positions(&self) -> Result<PositionsResponse, String> {
        let positions_url = format!("{}/positions", self.api_url);
        debug!("📋 Fetching open positions from API: {}", positions_url);

        match self
            .http_client
            .get(&positions_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<PositionsResponse>().await {
                        Ok(positions_response) => {
                            info!(
                                "✅ Successfully fetched {} open positions",
                                positions_response.positions.len()
                            );
                            Ok(positions_response)
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to parse positions JSON: {}", e);
                            error!("❌ {}", error_msg);
                            Err(error_msg)
                        }
                    }
                } else {
                    let error_msg =
                        format!("Positions API returned error status: {}", response.status());
                    error!("❌ {}", error_msg);
                    Err(error_msg)
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to call positions API: {}", e);
                error!("❌ {}", error_msg);
                Err(error_msg)
            }
        }
    }

    /// Store enriched active trade in Redis with 7-day TTL
    async fn store_active_trade_redis(
        &self,
        trade: &EnrichedActiveTrade,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client.get_connection()?;

            let key = format!("active_trade:{}", trade.position_id);
            let trade_json = serde_json::to_string(trade)?;
            conn.set(&key, &trade_json)?;

            // Set 7-day expiration for automatic cleanup
            conn.expire(&key, 7 * 24 * 3600)?; // 7 days in seconds

            debug!(
                "📊 Stored active trade {} in Redis (expires in 7 days)",
                trade.position_id
            );
            Ok(())
        } else {
            Err("Redis client not available".into())
        }
    }

    /// Remove active trade from Redis when position closes
    async fn remove_active_trade_redis(
        &self,
        position_id: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client.get_connection()?;

            let key = format!("active_trade:{}", position_id);
            let _: i32 = conn.del(&key)?;

            debug!("📊 Removed closed trade {} from Redis", position_id);
            Ok(())
        } else {
            Err("Redis client not available".into())
        }
    }

    /// Get pending order data for enrichment (cross-reference with PendingOrderManager Redis)
    async fn get_pending_order_for_enrichment(&self, order_id: u64) -> Option<serde_json::Value> {
        if let Some(ref client) = self.redis_client {
            if let Ok(mut conn) = client.get_connection() {
                // Get zone_id from lookup (same pattern as PendingOrderManager)
                let lookup_key = format!("order_lookup:{}", order_id);
                if let Ok(zone_id) = conn.get::<_, String>(&lookup_key) {
                    // Get order data
                    let order_key = format!("pending_order:{}", zone_id);
                    if let Ok(order_json) = conn.get::<_, String>(&order_key) {
                        if let Ok(order_data) =
                            serde_json::from_str::<serde_json::Value>(&order_json)
                        {
                            return Some(order_data);
                        }
                    }
                }
            }
        }
        None
    }

    /// Convert open position to enriched active trade
    async fn create_enriched_trade(&self, position: &OpenPosition) -> EnrichedActiveTrade {
        let symbol = self
            .symbol_id_mapping
            .get(&position.symbol_id)
            .map(|s| s.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();

        let trade_side = if position.trade_side == 1 {
            "BUY"
        } else {
            "SELL"
        };

        let open_time = DateTime::from_timestamp(position.open_time as i64 / 1000, 0)
            .unwrap_or_else(|| Utc::now());

        let now = Utc::now();
        let duration_minutes = (now - open_time).num_minutes();

        // Convert volume to lots (your API returns volume in units, not lots)
        let volume_in_lots = position.volume as f64 / 10000.0;

        // Calculate pip value for this symbol
        let pip_value = self.get_pip_value(&symbol);

        // Since we don't have current_price or pips_profit from API, we'll calculate/estimate
        let current_price = position.price; // Use entry price as current for now
        let pips_profit = 0.0; // We'd need real-time price to calculate this
        let unrealized_pnl = 0.0; // We'd need real-time price to calculate this

        // Calculate distance from entry in pips (will be 0 since current_price = entry_price)
        let distance_from_entry_pips = (current_price - position.price).abs() / pip_value;

        // Calculate R:R ratio if SL/TP are set
        let risk_reward_ratio =
            if let (Some(sl), Some(tp)) = (position.stop_loss, position.take_profit) {
                let risk_pips = (position.price - sl).abs() / pip_value;
                let reward_pips = (tp - position.price).abs() / pip_value;
                if risk_pips > 0.0 {
                    Some(reward_pips / risk_pips)
                } else {
                    None
                }
            } else {
                None
            };

        let is_profitable = pips_profit > 0.0;

        // Try to enrich using pending order data from Redis
        let mut zone_id = None;
        let mut zone_type = None;
        let mut zone_strength = None;
        let mut zone_high = None;
        let mut zone_low = None;
        let mut touch_count = None;
        let mut timeframe = None;
        let mut distance_when_placed = None;
        let mut original_entry_price = None;
        let mut slippage_pips = None;

        // New logic: use position_lookup to get ctrader_order_id, then order_lookup to get zone_id
        let mut ctrader_order_id: Option<String> = None;
        if let Some(ref client) = self.redis_client {
            if let Ok(mut conn) = client.get_connection() {
                let position_lookup_key = format!("position_lookup:{}", position.position_id);
                if let Ok(order_id_str) = conn.get::<_, String>(&position_lookup_key) {
                    ctrader_order_id = Some(order_id_str);
                }
            }
        }
        if let Some(ref order_id) = ctrader_order_id {
            if let Some(order) = self.get_pending_order_for_enrichment(order_id.parse::<u64>().unwrap()).await {
                zone_id = order.get("zone_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                zone_type = order.get("zone_type").and_then(|v| v.as_str()).map(|s| s.to_string());
                zone_strength = order.get("zone_strength").and_then(|v| v.as_f64());
                zone_high = order.get("zone_high").and_then(|v| v.as_f64());
                zone_low = order.get("zone_low").and_then(|v| v.as_f64());
                touch_count = order.get("touch_count").and_then(|v| v.as_i64()).map(|i| i as i32);
                timeframe = order.get("timeframe").and_then(|v| v.as_str()).map(|s| s.to_string());
                distance_when_placed = order.get("distance_when_placed").and_then(|v| v.as_f64());
                original_entry_price = order.get("entry_price").and_then(|v| v.as_f64());
                // Optionally calculate slippage if entry prices differ
                if let (Some(filled), Some(orig)) = (Some(position.price), original_entry_price) {
                    slippage_pips = Some(((filled - orig).abs()) / pip_value);
                }
            }
        }

        EnrichedActiveTrade {
            position_id: position.position_id,
            symbol,
            trade_side: trade_side.to_string(),
            volume_lots: volume_in_lots,
            entry_price: position.price,
            current_price,
            unrealized_pnl,
            pips_profit,
            open_time,
            duration_minutes,
            stop_loss: position.stop_loss,
            take_profit: position.take_profit,
            swap: position.swap,
            commission: position.commission,
            label: String::new(),   // Your API doesn't provide label
            comment: String::new(), // Your API doesn't provide comment

            // Enriched zone data
            zone_id,
            zone_type,
            zone_strength,
            zone_high,
            zone_low,
            touch_count,
            timeframe,
            distance_when_placed,
            original_entry_price,
            slippage_pips,

            // Real-time metrics
            distance_from_entry_pips,
            risk_reward_ratio,
            is_profitable,
            max_drawdown_pips: None, // Will be calculated and stored
            max_profit_pips: None,   // Will be calculated and stored
            last_updated: now,
        }
    }

    /// Update position metrics (max profit/drawdown tracking)
    async fn update_position_metrics(&self, trade: &mut EnrichedActiveTrade) {
        let mut metrics = self.position_metrics.lock().await;

        let (mut max_profit, mut max_drawdown) = metrics
            .get(&trade.position_id)
            .copied()
            .unwrap_or((0.0, 0.0));

        // Update max profit/drawdown
        if trade.pips_profit > max_profit {
            max_profit = trade.pips_profit;
        }
        if trade.pips_profit < max_drawdown {
            max_drawdown = trade.pips_profit;
        }

        // Store updated metrics
        metrics.insert(trade.position_id, (max_profit, max_drawdown));

        // Update trade with metrics
        trade.max_profit_pips = Some(max_profit);
        trade.max_drawdown_pips = Some(max_drawdown);
    }

    /// Main processing method - monitor open trades
    pub async fn process_order_updates(&self) -> Result<(), String> {
        debug!("🔄 Processing open trades with Redis storage...");

        // Clear all active_trade:* keys from Redis before repopulating
        if let Some(ref client) = self.redis_client {
            if let Ok(mut conn) = client.get_connection() {
                if let Ok(keys) = conn.keys::<_, Vec<String>>("active_trade:*") {
                    for key in keys {
                        let _: redis::RedisResult<()> = conn.del(&key);
                    }
                    debug!("🧹 Cleared all active_trade:* keys from Redis before repopulating");
                }
            }
        }

        let positions_response = self.fetch_open_positions().await?;

        if positions_response.positions.is_empty() {
            debug!("📋 No open positions to process");
            return Ok(());
        }

        let mut processed_positions = self.processed_positions.lock().await;
        let current_position_ids: std::collections::HashSet<u64> = positions_response
            .positions
            .iter()
            .map(|p| p.position_id)
            .collect();

        // Find positions that have closed (were tracked but not in current list)
        let closed_positions: Vec<u64> = processed_positions
            .difference(&current_position_ids)
            .copied()
            .collect();

        // Remove closed positions from Redis and tracking
        for closed_position_id in closed_positions {
            if let Err(e) = self.remove_active_trade_redis(closed_position_id).await {
                warn!(
                    "Failed to remove closed position {} from Redis: {}",
                    closed_position_id, e
                );
            }
            processed_positions.remove(&closed_position_id);

            // Also remove from metrics tracking
            let mut metrics = self.position_metrics.lock().await;
            metrics.remove(&closed_position_id);
            drop(metrics);

            info!(
                "📊 Position {} closed - removed from active tracking",
                closed_position_id
            );
        }

        // Process current open positions
        let mut new_positions = 0;
        let mut updated_positions = 0;

        for position in &positions_response.positions {
            let is_new_position = !processed_positions.contains(&position.position_id);

            // Create enriched trade
            let mut enriched_trade = self.create_enriched_trade(position).await;

            // Update position metrics
            self.update_position_metrics(&mut enriched_trade).await;

            // Store in Redis
            if let Err(e) = self.store_active_trade_redis(&enriched_trade).await {
                warn!(
                    "Failed to store active trade {} in Redis: {}",
                    position.position_id, e
                );
            }

            // Track this position
            processed_positions.insert(position.position_id);

            if is_new_position {
                new_positions += 1;
                info!(
                    "📊 New active trade: {} {} {:.2} lots @ {:.5} (P&L: {:.2} pips)",
                    enriched_trade.symbol,
                    enriched_trade.trade_side,
                    enriched_trade.volume_lots,
                    enriched_trade.entry_price,
                    enriched_trade.pips_profit
                );
            } else {
                updated_positions += 1;
                debug!(
                    "📊 Updated trade {}: P&L {:.2} pips, Duration: {} min",
                    position.position_id,
                    enriched_trade.pips_profit,
                    enriched_trade.duration_minutes
                );
            }
        }

        if new_positions > 0 || updated_positions > 0 {
            info!(
                "✅ Processed {} new, {} updated active trades",
                new_positions, updated_positions
            );
        }

        Ok(())
    }

    /// Poll all active trades for closure using the broker's positions endpoint
    pub async fn poll_active_trades_for_closes(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use chrono::Utc;
        use redis::Commands;
        use serde_json::Value;
        use std::env;

        info!("[Polling] Starting poll_active_trades_for_closes");
        // 1. Get all active trades from Redis (active_trade:*)
        let mut active_trades = Vec::new();
        if let Some(ref client) = self.redis_client {
            let mut conn = client.get_connection()?;
            if let Ok(keys) = conn.keys::<_, Vec<String>>("active_trade:*") {
                info!("[Polling] Found {} active trades in Redis", keys.len());
                for key in &keys {
                    if let Ok(trade_json) = conn.get::<_, String>(key) {
                        if let Ok(trade) = serde_json::from_str::<Value>(&trade_json) {
                            let position_id = trade.get("position_id").and_then(|v| v.as_u64());
                            info!("[Polling] Checking trade key={} position_id={:?}", key, position_id);
                            active_trades.push((key.clone(), trade));
                        } else {
                            info!("[Polling] Could not parse trade JSON for key={}", key);
                        }
                    } else {
                        info!("[Polling] Could not get trade JSON for key={}", key);
                    }
                }
            } else {
                info!("[Polling] Could not get active_trade:* keys from Redis");
            }
        } else {
            info!("[Polling] No Redis client available");
        }

        // 2. Fetch all open positions from broker
        let api_url = env::var("CTRADER_API_BRIDGE_URL").unwrap_or_else(|_| "http://localhost:8000".to_string());
        let client = reqwest::Client::new();
        let positions_url = format!("{}/positions", api_url);
        info!("[Polling] Fetching open positions from broker: {}", positions_url);
        let mut open_position_ids = std::collections::HashSet::new();
        match client.get(&positions_url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    match resp.json::<serde_json::Value>().await {
                        Ok(json) => {
                            if let Some(positions) = json.get("positions").and_then(|v| v.as_array()) {
                                for pos in positions {
                                    if let Some(pid) = pos.get("positionId").and_then(|v| v.as_u64()) {
                                        open_position_ids.insert(pid);
                                    }
                                }
                                info!("[Polling] Broker reports {} open positions", open_position_ids.len());
                            } else {
                                info!("[Polling] No positions array in broker response");
                            }
                        }
                        Err(e) => {
                            info!("[Polling] Could not parse broker positions JSON: {}", e);
                        }
                    }
                } else {
                    info!("[Polling] Broker positions endpoint returned error: {}", resp.status());
                }
            }
            Err(e) => {
                info!("[Polling] Failed to call broker positions endpoint: {}", e);
            }
        }

        for (key, trade) in &active_trades {
            let position_id = trade.get("position_id").and_then(|v| v.as_u64());
            if let Some(pid) = position_id {
                if !open_position_ids.contains(&pid) {
                    info!("[Polling] Trade key={} position_id={} is CLOSED at broker", key, pid);
                } else {
                    info!("[Polling] Trade key={} position_id={} is still OPEN at broker", key, pid);
                }
            } else {
                info!("[Polling] Trade key={} has no position_id", key);
            }
        }

        for (key, trade) in active_trades {
            if let Some(position_id) = trade.get("position_id").and_then(|v| v.as_u64()) {
                if !open_position_ids.contains(&position_id) {
                    // 3. Position is closed at broker
                    let zone_id = trade.get("zone_id").and_then(|v| v.as_str()).unwrap_or("");
                    info!("[Polling] Active trade {} (position_id={}, zone_id={}) is CLOSED and ready to be enriched", key, position_id, zone_id);
                    // Mark the corresponding pending order as CLOSED, add closed_at
                    if let Some(ref client) = self.redis_client {
                        let mut conn = client.get_connection()?;
                        // Find pending order by zone_id or ctrader_order_id if available
                        if let Some(zone_id) = trade.get("zone_id").and_then(|v| v.as_str()) {
                            let pending_key = format!("pending_order:{}", zone_id);
                            if let Ok(order_json) = conn.get::<_, String>(&pending_key) {
                                if let Ok(mut order) = serde_json::from_str::<Value>(&order_json) {
                                    order["status"] = Value::String("CLOSED".to_string());
                                    order["closed_at"] = Value::String(Utc::now().to_rfc3339());
                                    let updated_json = serde_json::to_string(&order)?;
                                    let _: () = conn.set(&pending_key, updated_json)?;
                                    info!("[Enrichment] Marked pending order {} as CLOSED in Redis", pending_key);
                                } else {
                                    info!("[Enrichment] Could not deserialize pending order for {}", pending_key);
                                }
                            } else {
                                info!("[Enrichment] Could not find pending order in Redis for {}", pending_key);
                            }
                        }
                        // 4. Enrich the closed deal and save to InfluxDB (TODO: implement actual enrichment and save logic)
                        info!("[Enrichment] Attempting to enrich closed trade: position_id={}, zone_id={}", position_id, zone_id);
                        // Fetch the original orderId from position_lookup
                        let mut ctrader_order_id: Option<String> = None;
                        if let Some(ref client) = self.redis_client {
                            if let Ok(mut conn) = client.get_connection() {
                                let position_lookup_key = format!("position_lookup:{}", position_id);
                                if let Ok(order_id_str) = conn.get::<_, String>(&position_lookup_key) {
                                    ctrader_order_id = Some(order_id_str);
                                }
                            }
                        }
                        if let Some(ref order_id) = ctrader_order_id {
                            info!("[Enrichment] Fetching final /order-details for closed trade: order_id={}", order_id);
                            let account_id = std::env::var("CTID_TRADER_ACCOUNT_ID").unwrap_or_else(|_| "37972727".to_string());
                            let api_url = std::env::var("CTRADER_API_BRIDGE_URL").unwrap_or_else(|_| "http://localhost:8000".to_string());
                            let url = format!("{}/order-details/{}/{}", api_url, account_id, order_id);
                            let client = reqwest::Client::new();
                            match client.get(&url).send().await {
                                Ok(resp) => {
                                    if resp.status().is_success() {
                                        if let Ok(resp_json) = resp.json::<serde_json::Value>().await {
                                            info!("[Enrichment] Got /order-details response for closed trade: order_id={}", order_id);
                                            let deals = resp_json.get("deals").and_then(|v| v.as_array());
                                            if let Some(deals) = deals {
                                                if !deals.is_empty() {
                                                    let deal = &deals[0];
                                                    // Extract close details
                                                    let close_price = deal.get("executionPrice").and_then(|v| v.as_f64());
                                                    let close_time = deal.get("executionTimestamp").and_then(|v| v.as_i64());
                                                    let pips_profit = deal.get("pipsProfit").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                                    let net_profit = deal.get("netProfit").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                                    // Fetch the active_trade from the trade value
                                                    match serde_json::from_value::<EnrichedActiveTrade>(trade.clone()) {
                                                        Ok(mut enriched) => {
                                                            // Update with close details
                                                            enriched.current_price = close_price.unwrap_or(enriched.current_price);
                                                            enriched.pips_profit = pips_profit;
                                                            enriched.unrealized_pnl = net_profit;
                                                            // Optionally update last_updated, etc.
                                                            enriched.last_updated = chrono::Utc::now();

                                                            let mut real_deal_id: Option<String> = None;
                                                            let mut real_order_id: Option<String> = None;
                                                            let mut real_position_id: Option<String> = None;
                                                            let mut real_gross_profit: Option<f64> = None;
                                                            let mut real_net_profit: Option<f64> = None;
                                                            let mut real_pips_profit: Option<f64> = None;
                                                            let mut real_entry_price: Option<f64> = None;
                                                            let mut real_exit_price: Option<f64> = None;
                                                            let mut real_open_time: Option<chrono::DateTime<chrono::Utc>> = None;
                                                            let mut real_close_time: Option<chrono::DateTime<chrono::Utc>> = None;
                                                            let mut real_duration_minutes: Option<i64> = None;

                                                            // --- Fetch /deals/<today> and merge extra fields ---
                                                            info!("[Enrichment] Fetching /deals/<today> for closed trade: position_id={}", enriched.position_id);
                                                            let today = chrono::Utc::now().date_naive();
                                                            let date_str = today.format("%Y-%m-%d").to_string();
                                                            let deals_url = format!("{}/deals/{}", api_url, date_str);
                                                            match client.get(&deals_url).send().await {
                                                                Ok(deals_resp) => {
                                                                    if deals_resp.status().is_success() {
                                                                        if let Ok(deals_json) = deals_resp.json::<serde_json::Value>().await {
                                                                            let closed_deals = deals_json.get("closedTrades").and_then(|v| v.as_array());
                                                                            info!("[Enrichment] /deals/<today> returned {} deals", closed_deals.map(|d| d.len()).unwrap_or(0));
                                                                            if let Some(closed_deals) = closed_deals {
                                                                                if let Some(matching_deal) = closed_deals.iter().find(|d| d.get("positionId").and_then(|v| v.as_u64()) == Some(enriched.position_id)) {
                                                                                    info!("[Enrichment] Found matching deal in /deals/<today> for position_id={}", enriched.position_id);
                                                                                    // Extract and log dealId, orderId, positionId
                                                                                    real_deal_id = matching_deal.get("dealId").and_then(|v| v.as_i64()).map(|v| v.to_string());
                                                                                    if let Some(ref v) = real_deal_id { info!("[Enrichment] Using real broker dealId from /deals/<today>: {}", v); }
                                                                                    real_order_id = matching_deal.get("orderId").and_then(|v| v.as_i64()).map(|v| v.to_string());
                                                                                    if let Some(ref v) = real_order_id { info!("[Enrichment] Using real broker orderId from /deals/<today>: {}", v); }
                                                                                    real_position_id = matching_deal.get("positionId").and_then(|v| v.as_u64()).map(|v| v.to_string());
                                                                                    if let Some(ref v) = real_position_id { info!("[Enrichment] Using real broker positionId from /deals/<today>: {}", v); }
                                                                                    // Extract and log grossProfit
                                                                                    real_gross_profit = matching_deal.get("closePositionDetail").and_then(|d| d.get("grossProfit")).and_then(|v| v.as_f64());
                                                                                    if let Some(gross_profit) = real_gross_profit { info!("[Enrichment] Merging gross_profit from /deals/<today>: {}", gross_profit); }
                                                                                    // Extract and log netProfit (profit)
                                                                                    real_net_profit = matching_deal.get("profit").and_then(|v| v.as_f64());
                                                                                    if let Some(net_profit) = real_net_profit { info!("[Enrichment] Merging net_profit from /deals/<today>: {}", net_profit); enriched.unrealized_pnl = net_profit; }
                                                                                    // Extract entry/exit price
                                                                                    real_entry_price = matching_deal.get("closePositionDetail").and_then(|d| d.get("entryPrice")).and_then(|v| v.as_f64());
                                                                                    real_exit_price = matching_deal.get("price").and_then(|v| v.as_f64());
                                                                                    if let (Some(entry), Some(exit)) = (real_entry_price, real_exit_price) {
                                                                                        let pip_value = self.get_pip_value(&enriched.symbol);
                                                                                        real_pips_profit = Some((exit - entry) / pip_value);
                                                                                        info!("[Enrichment] Calculated pips_profit from /deals/<today>: {}", real_pips_profit.unwrap());
                                                                                        enriched.pips_profit = real_pips_profit.unwrap();
                                                                                    }
                                                                                } else {
                                                                                    info!("[Enrichment] No matching deal in /deals/<today> for position_id={}", enriched.position_id);
                                                                                }
                                                                            }
                                                                        }
                                                                    } else {
                                                                        info!("[Enrichment] /deals/<today> endpoint returned error: {}", deals_resp.status());
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    info!("[Enrichment] Failed to call /deals/<today>: {}", e);
                                                                }
                                                            }
                                                            // --- END /deals/<today> ---

                                                            // --- Fallback: Extract open/close time and duration from /order-details ---
                                                            if let Some(order) = resp_json.get("order") {
                                                                if let Some(trade_data) = order.get("tradeData") {
                                                                    // open_time
                                                                    real_open_time = trade_data.get("openTimestamp").and_then(|v| v.as_i64()).and_then(|ts| chrono::NaiveDateTime::from_timestamp_millis(ts)).map(|dt| chrono::DateTime::<chrono::Utc>::from_utc(dt, chrono::Utc));
                                                                    if let Some(ref v) = real_open_time { info!("[Enrichment] Using open_time from /order-details: {}", v); }
                                                                    // close_time
                                                                    real_close_time = trade_data.get("closeTimestamp").and_then(|v| v.as_i64()).and_then(|ts| chrono::NaiveDateTime::from_timestamp_millis(ts)).map(|dt| chrono::DateTime::<chrono::Utc>::from_utc(dt, chrono::Utc));
                                                                    if let Some(ref v) = real_close_time { info!("[Enrichment] Using close_time from /order-details: {}", v); }
                                                                    // duration_minutes
                                                                    if let (Some(open), Some(close)) = (trade_data.get("openTimestamp").and_then(|v| v.as_i64()), trade_data.get("closeTimestamp").and_then(|v| v.as_i64())) {
                                                                        real_duration_minutes = Some(((close - open) as f64 / 60000.0).round() as i64);
                                                                        info!("[Enrichment] Calculated duration_minutes from /order-details: {}", real_duration_minutes.unwrap());
                                                                    }
                                                                    // fallback for order_id if not set from /deals/<today>
                                                                    if real_order_id.is_none() {
                                                                        real_order_id = order.get("orderId").and_then(|v| v.as_i64()).map(|v| v.to_string());
                                                                        if let Some(ref v) = real_order_id { info!("[Enrichment] Fallback orderId from /order-details: {}", v); }
                                                                    }
                                                                }
                                                            }
                                                            // --- END Fallback ---

                                                            // Log the full enriched closed trade object
                                                            if let Ok(json) = serde_json::to_string_pretty(&enriched) {
                                                                info!("[Enrichment] Would save CLOSED trade to InfluxDB: {}", json);
                                                            } else {
                                                                info!("[Enrichment] Could not serialize closed EnrichedActiveTrade for InfluxDB log");
                                                            }
                                                            // Save to InfluxDB here
                                                            if let Some(ref db) = self.order_database {
                                                                let deal_id = real_deal_id.clone().unwrap_or_else(|| format!("{}-{}", enriched.position_id, enriched.last_updated.timestamp()));
                                                                let order_id = real_order_id.clone();
                                                                let position_id = real_position_id.clone().unwrap_or_else(|| enriched.position_id.to_string());
                                                                let enriched_deal = EnrichedDeal {
                                                                    deal_id,
                                                                    order_id,
                                                                    position_id,
                                                                    symbol: enriched.symbol.clone(),
                                                                    symbol_id: None,
                                                                    volume: enriched.volume_lots * 10000.0, // convert back to units
                                                                    volume_in_lots: Some(enriched.volume_lots),
                                                                    trade_side: if enriched.trade_side == "BUY" { 1 } else { 2 },
                                                                    closing_trade_side: None,
                                                                    price: real_exit_price.unwrap_or(enriched.current_price),
                                                                    price_difference: None,
                                                                    pips_profit: real_pips_profit,
                                                                    execution_time: real_close_time.unwrap_or(enriched.last_updated),
                                                                    deal_type: "CLOSE".to_string(),
                                                                    profit: real_net_profit.or(Some(enriched.unrealized_pnl)),
                                                                    gross_profit: real_gross_profit,
                                                                    net_profit: real_net_profit,
                                                                    swap: Some(enriched.swap),
                                                                    commission: Some(enriched.commission),
                                                                    pnl_conversion_fee: None,
                                                                    balance_after_trade: None,
                                                                    balance_version: None,
                                                                    quote_to_deposit_conversion_rate: None,
                                                                    raw_data: None,
                                                                    label: Some(enriched.label.clone()),
                                                                    comment: Some(enriched.comment.clone()),
                                                                    deal_status: None,
                                                                    open_time: real_open_time.or(Some(enriched.open_time)),
                                                                    close_time: real_close_time.or(Some(enriched.last_updated)),
                                                                    duration_minutes: real_duration_minutes.or(Some(enriched.duration_minutes)),
                                                                    zone_id: enriched.zone_id.clone(),
                                                                    zone_type: enriched.zone_type.clone(),
                                                                    zone_strength: enriched.zone_strength,
                                                                    zone_high: enriched.zone_high,
                                                                    zone_low: enriched.zone_low,
                                                                    touch_count: enriched.touch_count,
                                                                    timeframe: enriched.timeframe.clone(),
                                                                    distance_when_placed: enriched.distance_when_placed,
                                                                    original_entry_price: real_entry_price.or(enriched.original_entry_price),
                                                                    stop_loss: enriched.stop_loss,
                                                                    take_profit: enriched.take_profit,
                                                                    slippage_pips: enriched.slippage_pips,
                                                                };
                                                                if let Ok(json) = serde_json::to_string_pretty(&enriched_deal) {
                                                                    info!("[Enrichment] Final EnrichedDeal to InfluxDB: {}", json);
                                                                }
                                                                match db.write_enriched_deal(&enriched_deal).await {
                                                                    Ok(_) => info!("[Enrichment] Successfully saved closed trade to InfluxDB: deal_id={}", enriched_deal.deal_id),
                                                                    Err(e) => error!("[Enrichment] Failed to save closed trade to InfluxDB: deal_id={}, error={}", enriched_deal.deal_id, e),
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            info!("[Enrichment] Could not deserialize trade to EnrichedActiveTrade for closed trade: {}", e);
                                                            info!("[Enrichment] Raw trade value: {:?}", trade);
                                                        }
                                                    }
                                                } else {
                                                    info!("[Enrichment] No deals found in /order-details for closed trade: order_id={}", order_id);
                                                }
                                            } else {
                                                info!("[Enrichment] No deals array in /order-details for closed trade: order_id={}", order_id);
                                            }
                                        } else {
                                            info!("[Enrichment] Could not parse /order-details JSON for closed trade: order_id={}", order_id);
                                        }
                                    } else {
                                        info!("[Enrichment] /order-details endpoint returned error for closed trade: order_id={}, status={}", order_id, resp.status());
                                    }
                                }
                                Err(e) => {
                                    info!("[Enrichment] Failed to call /order-details for closed trade: order_id={}, error={}", order_id, e);
                                }
                            }
                        } else {
                            info!("[Enrichment] No ctrader_order_id found for closed trade position_id={}", position_id);
                        }
                        // 5. Remove the entry from active_orders
                        let _: () = conn.del(&key)?;
                        info!("[Enrichment] Removed active trade {} from Redis after closure", key);
                    }
                }
            }
        }
        Ok(())
    }

    /// Get Redis statistics for monitoring
    pub async fn get_redis_stats(&self) -> (usize, usize) {
        if let Some(ref client) = self.redis_client {
            if let Ok(mut conn) = client.get_connection() {
                let active_trades: Result<Vec<String>, _> = conn.keys("active_trade:*");

                return (
                    active_trades.map(|v| v.len()).unwrap_or(0),
                    0, // No lookup keys for active trades
                );
            }
        }
        (0, 0)
    }

    /// Get pip value for a symbol
    fn get_pip_value(&self, symbol: &str) -> f64 {
        if symbol.contains("JPY") {
            0.01
        } else if symbol.ends_with("_SB") {
            0.0001
        } else {
            match symbol {
                "NAS100" => 1.0,
                "US500" => 0.1,
                _ => 0.0001,
            }
        }
    }

    /// Get all active trades from Redis (for web interface)
    pub async fn get_all_active_trades(&self) -> Vec<EnrichedActiveTrade> {
        let mut trades = Vec::new();

        if let Some(ref client) = self.redis_client {
            if let Ok(mut conn) = client.get_connection() {
                if let Ok(keys) = conn.keys::<_, Vec<String>>("active_trade:*") {
                    for key in keys {
                        if let Ok(trade_json) = conn.get::<_, String>(&key) {
                            if let Ok(trade) =
                                serde_json::from_str::<EnrichedActiveTrade>(&trade_json)
                            {
                                trades.push(trade);
                            }
                        }
                    }
                }
            }
        }

        trades.sort_by(|a, b| b.open_time.cmp(&a.open_time)); // Most recent first
        trades
    }

    /// Check if manager has any active trades
    pub async fn has_active_trades(&self) -> bool {
        if let Some(ref client) = self.redis_client {
            if let Ok(mut conn) = client.get_connection() {
                if let Ok(keys) = conn.keys::<_, Vec<String>>("active_trade:*") {
                    return !keys.is_empty();
                }
            }
        }
        false
    }
}
