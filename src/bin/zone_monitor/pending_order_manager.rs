// src/bin/zone_monitor/pending_order_manager.rs
// Redis-first pending order management with deal enrichment

use crate::csv_logger::CsvLogger;
use crate::db::OrderDatabase;
use crate::types::ZoneAlert;
use crate::types::{PriceUpdate, Zone};
use chrono::{DateTime, Utc};
use redis::{Client, Commands, Connection};
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

use crate::db::order_db::EnrichedDeal;
use crate::active_order_manager::EnrichedActiveTrade;
use crate::strategy_manager::StrategyManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOrder {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub order_type: String, // "BUY_LIMIT" or "SELL_LIMIT"
    pub entry_price: f64,   // The limit price
    pub lot_size: i32,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub ctrader_order_id: Option<String>, // Order ID returned by cTrader
    pub placed_at: DateTime<Utc>,
    pub status: String, // "PENDING" - status never changes
    pub zone_high: f64,
    pub zone_low: f64,
    pub zone_strength: f64,
    pub touch_count: i32,
    pub distance_when_placed: f64, // Distance in pips when order was placed
}

// New deal structure from broker API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerDeal {
    pub deal_id: u64,
    pub position_id: u64,
    pub order_id: Option<u64>,
    pub symbol_id: i32,
    pub volume: f64,
    pub original_trade_side: i32,
    pub closing_trade_side: i32,
    pub entry_price: f64,
    pub exit_price: f64,
    pub price_difference: f64,
    pub pips_profit: f64,
    pub open_time: i64,
    pub close_time: i64,
    pub duration: i64,
    pub gross_profit: f64,
    pub net_profit: f64,
    pub swap: f64,
    pub commission: f64,
    pub pnl_conversion_fee: f64,
    pub balance_after_trade: f64,
    pub balance_version: i64,
    pub quote_to_deposit_conversion_rate: f64,
    pub deal_status: i32,
}

pub struct PendingOrderManager {
    enabled: bool,
    distance_threshold_pips: f64,
    default_lot_size: i32,
    default_sl_pips: f64,
    default_tp_pips: f64,
    ctrader_api_url: String,
    http_client: reqwest::Client,
    symbol_ids: HashMap<String, i32>, // symbol -> symbol_id mapping
    allowed_timeframes: Vec<String>,  // allowed timeframes for trading
    order_database: Option<Arc<OrderDatabase>>, // InfluxDB for order lifecycle tracking
    redis_client: Option<Client>,
    min_zone_strength: f64,
    redis_connection: Option<Arc<Mutex<Connection>>>,
}

#[derive(Debug, Serialize)]
struct PlacePendingOrderRequest {
    symbolId: i32,   // Symbol ID (number)
    tradeSide: i32,  // 1 = BUY, 2 = SELL
    volume: f64,     // Volume
    entryPrice: f64, // The limit price
    stopLoss: Option<f64>,
    takeProfit: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PlacePendingOrderResponse {
    success: bool,
    message: String,
    #[serde(rename = "orderId")]
    order_id: Option<String>,
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
    error: Option<serde_json::Value>,
}

impl PendingOrderManager {
    pub fn new(strategies: Option<&StrategyManager>) -> Self {
        let (_redis_client, redis_connection) = match Client::open("redis://127.0.0.1:6379/") {
            Ok(client) => match client.get_connection() {
                Ok(conn) => {
                    info!("📊 Redis connected for order monitoring");
                    (Some(client), Some(Arc::new(Mutex::new(conn))))
                }
                Err(e) => {
                    error!("📊 Failed to establish Redis connection: {}", e);
                    (None, None)
                }
            },
            Err(e) => {
                error!("📊 Failed to create Redis client: {}", e);
                (None, None)
            }
        };

        let enabled = env::var("ENABLE_LIMIT_ORDERS")
            .unwrap_or_else(|_| "false".to_string())
            .trim()
            .to_lowercase()
            == "true";

        let distance_threshold_pips = env::var("LIMIT_ORDER_DISTANCE_PIPS")
            .unwrap_or_else(|_| "20.0".to_string())
            .parse::<f64>()
            .unwrap_or(20.0);

        let default_lot_size = env::var("LIMIT_ORDER_LOT_SIZE")
            .unwrap_or_else(|_| "1000".to_string())
            .parse::<i32>()
            .unwrap_or(1000);

        let default_sl_pips = env::var("TRADING_STOP_LOSS_PIPS")
            .unwrap_or_else(|_| "20.0".to_string())
            .parse::<f64>()
            .unwrap_or(20.0);

        let default_tp_pips = env::var("TRADING_TAKE_PROFIT_PIPS")
            .unwrap_or_else(|_| "42.0".to_string())
            .parse::<f64>()
            .unwrap_or(42.0);

        let ctrader_api_url = env::var("CTRADER_API_BRIDGE_URL")
            .unwrap_or_else(|_| "http://localhost:8000".to_string());

        let allowed_timeframes = env::var("TRADING_ALLOWED_TIMEFRAMES")
            .unwrap_or_else(|_| "30m,1h,4h,1d".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let min_zone_strength = env::var("TRADING_MIN_ZONE_STRENGTH")
            .unwrap_or_else(|_| "80.0".to_string())
            .parse::<f64>()
            .unwrap_or(80.0);

        if enabled {
            info!("📋 Pending Order Manager initialized:");
            info!("   Enabled: {}", enabled);
            info!("   Distance threshold: {:.1} pips", distance_threshold_pips);
            info!("   Default lot size: {}", default_lot_size);
            info!("   Default SL: {:.1} pips", default_sl_pips);
            info!("   Default TP: {:.1} pips", default_tp_pips);
            info!("   cTrader API: {}", ctrader_api_url);
            info!("   Allowed timeframes: {:?}", allowed_timeframes);
            info!("   Min zone strength: {:.1}", min_zone_strength);
        } else {
            info!("📋 Pending Order Manager disabled");
        }

        let day_trading_mode = std::env::var("DAY_TRADING_MODE_ACTIVE").unwrap_or_else(|_| "false".to_string()).to_lowercase() == "true";

        if day_trading_mode {
            info!("🚦 DAY TRADING MODE ACTIVE! Day trading strategy filtering is enabled for 5m/15m timeframes.");
            info!("   (Loaded strategies will be used for filtering. See /api/strategies or shared_strategies.json for details.)");
            // Reminder for allowed timeframes
            let allowed_timeframes = std::env::var("TRADING_ALLOWED_TIMEFRAMES").unwrap_or_else(|_| "5m,15m,30m,1h,4h,1d".to_string());
            if !allowed_timeframes.split(',').any(|tf| tf.trim() == "5m") {
                warn!("⚠️  5m is NOT included in TRADING_ALLOWED_TIMEFRAMES! Add it to allow 5m trades.");
            }
        }

        let redis_client = match Client::open("redis://127.0.0.1:6379/") {
            Ok(client) => {
                info!("📊 Redis connected for order monitoring");
                Some(client)
            }
            Err(e) => {
                warn!("📊 Redis not available: {}, continuing without Redis", e);
                None
            }
        };

        Self {
            enabled,
            distance_threshold_pips,
            default_lot_size,
            default_sl_pips,
            default_tp_pips,
            ctrader_api_url,
            http_client: reqwest::Client::new(),
            symbol_ids: Self::init_symbol_ids(),
            allowed_timeframes,
            order_database: None,
            redis_client: _redis_client,
            min_zone_strength,
            redis_connection,
        }
    }

    async fn ensure_redis_available(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match &self.redis_connection {
            Some(conn_arc) => {
                let mut conn = conn_arc
                    .lock()
                    .map_err(|e| format!("Redis lock error: {}", e))?;

                // Test with a simple SET operation instead of ping
                match conn.set::<&str, &str, ()>("redis_health_check", "ok") {
                    Ok(_) => {
                        // Clean up test key
                        let _: Result<(), _> = conn.del("redis_health_check");
                        Ok(())
                    }
                    Err(e) => {
                        error!("🚨 Redis health check failed: {}", e);
                        Err(format!("Redis unavailable: {}", e).into())
                    }
                }
            }
            None => {
                error!("🚨 No Redis connection available - BLOCKING ALL TRADING");
                Err("Redis connection not available".into())
            }
        }
    }

    // Add this method to your PendingOrderManager impl block
    pub async fn test_redis_storage(
        &self,
        order_json: &serde_json::Value,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client.get_connection()?;

            let zone_id = order_json
                .get("zone_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let ctrader_id = order_json
                .get("ctrader_order_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            // Store the pending order
            let pending_key = format!("pending_order:{}", zone_id);
            let order_data = serde_json::to_string(order_json)?;
            conn.set(&pending_key, &order_data)?;
            conn.expire(&pending_key, 5 * 24 * 3600)?;

            // Store the lookup key
            let lookup_key = format!("order_lookup:{}", ctrader_id);
            conn.set(&lookup_key, zone_id)?;
            conn.expire(&lookup_key, 5 * 24 * 3600)?;

            // Verify both keys exist
            let pending_exists: bool = conn.exists(&pending_key)?;
            let lookup_exists: bool = conn.exists(&lookup_key)?;

            Ok(serde_json::json!({
                "success": true,
                "pending_key": pending_key,
                "lookup_key": lookup_key,
                "pending_exists": pending_exists,
                "lookup_exists": lookup_exists,
                "zone_id": zone_id,
                "ctrader_order_id": ctrader_id
            }))
        } else {
            Err("Redis not available".into())
        }
    }

    // Add this method to your PendingOrderManager impl block
    pub async fn create_missing_lookup_keys(
        &self,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client.get_connection()?;
            let keys: Vec<String> = conn.keys("pending_order:*")?;
            let mut created = 0;

            for key in keys {
                let json_data: String = conn.get(&key)?;
                let order: serde_json::Value = serde_json::from_str(&json_data)?;

                if let (Some(zone_id), Some(ctrader_id)) = (
                    key.strip_prefix("pending_order:"),
                    order.get("ctrader_order_id").and_then(|v| v.as_str()),
                ) {
                    let lookup_key = format!("order_lookup:{}", ctrader_id);

                    // Check if lookup already exists
                    let exists: bool = conn.exists(&lookup_key)?;
                    if !exists {
                        conn.set(&lookup_key, zone_id)?;
                        conn.expire(&lookup_key, 5 * 24 * 3600)?;
                        created += 1;
                        info!("✅ Created lookup key: {} -> {}", ctrader_id, zone_id);
                    }
                }
            }
            Ok(created)
        } else {
            Err("Redis not available".into())
        }
    }

    // Add this method to your PendingOrderManager impl block
    pub async fn get_all_pending_orders_with_lookup_status(
        &self,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client.get_connection()?;

            let pending_keys: Vec<String> = conn.keys("pending_order:*")?;
            let mut orders_info = Vec::new();

            for key in pending_keys {
                let order_data: String = conn.get(&key)?;
                let order: serde_json::Value = serde_json::from_str(&order_data)?;

                let zone_id = key.strip_prefix("pending_order:").unwrap_or("");
                let ctrader_order_id = order
                    .get("ctrader_order_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("N/A");

                // Check if lookup key exists
                let lookup_key = format!("order_lookup:{}", ctrader_order_id);
                let lookup_exists: bool = conn.exists(&lookup_key)?;

                orders_info.push(json!({
                    "zone_id": zone_id,
                    "ctrader_order_id": ctrader_order_id,
                    "lookup_key": lookup_key,
                    "lookup_exists": lookup_exists,

                    // Basic order info
                    "symbol": order.get("symbol"),
                    "timeframe": order.get("timeframe"),
                    "zone_type": order.get("zone_type"),
                    "order_type": order.get("order_type"),
                    "status": order.get("status"),

                    // Price info
                    "entry_price": order.get("entry_price"),
                    "stop_loss": order.get("stop_loss"),
                    "take_profit": order.get("take_profit"),

                    // Zone info
                    "zone_high": order.get("zone_high"),
                    "zone_low": order.get("zone_low"),
                    "zone_strength": order.get("zone_strength"),
                    "touch_count": order.get("touch_count"),
                    "distance_when_placed": order.get("distance_when_placed"),

                    // Timing
                    "placed_at": order.get("placed_at"),
                    "lot_size": order.get("lot_size")
                }));
            }

            // Sort by placed_at descending (most recent first)
            orders_info.sort_by(|a, b| {
                let a_time = a.get("placed_at").and_then(|v| v.as_str()).unwrap_or("");
                let b_time = b.get("placed_at").and_then(|v| v.as_str()).unwrap_or("");
                b_time.cmp(a_time)
            });

            Ok(json!({
                "total_pending_orders": orders_info.len(),
                "all_have_lookups": orders_info.iter().all(|o| o.get("lookup_exists").and_then(|v| v.as_bool()).unwrap_or(false)),
                "orders": orders_info
            }))
        } else {
            Err("Redis not available".into())
        }
    }

    /// Set the order database for InfluxDB tracking
    pub fn set_order_database(&mut self, order_database: Arc<OrderDatabase>) {
        self.order_database = Some(order_database);
        info!("📋 Order database connected for InfluxDB tracking");
    }

    pub async fn store_pending_order_redis(
        &self,
        order: &PendingOrder,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.ensure_redis_available().await?;

        if let Some(conn_arc) = &self.redis_connection {
            let mut conn = conn_arc
                .lock()
                .map_err(|e| format!("Redis lock error: {}", e))?;

            let key = format!("pending_order:{}", order.zone_id);
            let order_json = serde_json::to_string(order)?;

            // Fix: specify types explicitly for Redis operations
            let _: () = conn.set(&key, &order_json)?;
            let _: () = conn.expire(&key, 5 * 24 * 3600)?;

            if let Some(ref ctrader_id) = order.ctrader_order_id {
                let lookup_key = format!("order_lookup:{}", ctrader_id);
                let _: () = conn.set(&lookup_key, &order.zone_id)?;
                let _: () = conn.expire(&lookup_key, 5 * 24 * 3600)?;
            }

            info!(
                "📊 Stored pending order {} in Redis (expires in 5 days)",
                order.zone_id
            );
            Ok(())
        } else {
            Err("Redis connection not available for storage".into())
        }
    }

    /// Updated zone check with connection reuse and fail-safe behavior
    async fn zone_has_pending_order(&self, zone_id: &str) -> bool {
        // FAIL-SAFE: If Redis check fails, assume order exists to prevent duplicates
        match self.ensure_redis_available().await {
            Ok(_) => {
                if let Some(conn_arc) = &self.redis_connection {
                    if let Ok(mut conn) = conn_arc.lock() {
                        let key = format!("pending_order:{}", zone_id);
                        match conn.exists::<_, bool>(&key) {
                            Ok(exists) => exists,
                            Err(e) => {
                                error!("🚨 Redis exists check failed for {}: {} - ASSUMING ORDER EXISTS", zone_id, e);
                                true // Fail-safe: assume order exists to prevent duplicates
                            }
                        }
                    } else {
                        error!(
                            "🚨 Failed to acquire Redis lock for {} - ASSUMING ORDER EXISTS",
                            zone_id
                        );
                        true // Fail-safe
                    }
                } else {
                    error!(
                        "🚨 No Redis connection for {} - ASSUMING ORDER EXISTS",
                        zone_id
                    );
                    true // Fail-safe
                }
            }
            Err(e) => {
                error!(
                    "🚨 Redis health check failed for {}: {} - BLOCKING TRADING",
                    zone_id, e
                );
                true // Fail-safe: block trading by pretending order exists
            }
        }
    }

    /// Get pending order for deal enrichment (by ctrader_order_id)
    pub async fn get_pending_order_for_enrichment(
        &self,
        ctrader_order_id: &str,
    ) -> Option<PendingOrder> {
        // If Redis is not available, return None (no enrichment)
        if self.ensure_redis_available().await.is_err() {
            warn!(
                "🚨 Redis unavailable for enrichment - skipping for order {}",
                ctrader_order_id
            );
            return None;
        }

        if let Some(conn_arc) = &self.redis_connection {
            if let Ok(mut conn) = conn_arc.lock() {
                // Get zone_id from lookup
                let lookup_key = format!("order_lookup:{}", ctrader_order_id);
                if let Ok(zone_id) = conn.get::<_, String>(&lookup_key) {
                    // Get order data
                    let order_key = format!("pending_order:{}", zone_id);
                    if let Ok(order_json) = conn.get::<_, String>(&order_key) {
                        if let Ok(order) = serde_json::from_str::<PendingOrder>(&order_json) {
                            return Some(order);
                        }
                    }
                }
            }
        }
        None
    }

    /// Initialize symbol ID mapping (matching Angular CURRENCIES_MAP)
    fn init_symbol_ids() -> HashMap<String, i32> {
        let mut symbol_ids = HashMap::new();

        // Major pairs
        symbol_ids.insert("EURUSD_SB".to_string(), 185);
        symbol_ids.insert("EURUSD".to_string(), 185);
        symbol_ids.insert("GBPUSD_SB".to_string(), 199);
        symbol_ids.insert("GBPUSD".to_string(), 199);
        symbol_ids.insert("USDJPY_SB".to_string(), 226);
        symbol_ids.insert("USDJPY".to_string(), 226);
        symbol_ids.insert("USDCHF_SB".to_string(), 222);
        symbol_ids.insert("USDCHF".to_string(), 222);
        symbol_ids.insert("AUDUSD_SB".to_string(), 158);
        symbol_ids.insert("AUDUSD".to_string(), 158);
        symbol_ids.insert("USDCAD_SB".to_string(), 221);
        symbol_ids.insert("USDCAD".to_string(), 221);
        symbol_ids.insert("NZDUSD_SB".to_string(), 211);
        symbol_ids.insert("NZDUSD".to_string(), 211);

        // EUR crosses
        symbol_ids.insert("EURGBP_SB".to_string(), 175);
        symbol_ids.insert("EURGBP".to_string(), 175);
        symbol_ids.insert("EURJPY_SB".to_string(), 177);
        symbol_ids.insert("EURJPY".to_string(), 177);
        symbol_ids.insert("EURCHF_SB".to_string(), 173);
        symbol_ids.insert("EURCHF".to_string(), 173);
        symbol_ids.insert("EURAUD_SB".to_string(), 171);
        symbol_ids.insert("EURAUD".to_string(), 171);
        symbol_ids.insert("EURCAD_SB".to_string(), 172);
        symbol_ids.insert("EURCAD".to_string(), 172);
        symbol_ids.insert("EURNZD_SB".to_string(), 180);
        symbol_ids.insert("EURNZD".to_string(), 180);

        // GBP crosses
        symbol_ids.insert("GBPJPY_SB".to_string(), 192);
        symbol_ids.insert("GBPJPY".to_string(), 192);
        symbol_ids.insert("GBPCHF_SB".to_string(), 191);
        symbol_ids.insert("GBPCHF".to_string(), 191);
        symbol_ids.insert("GBPAUD_SB".to_string(), 189);
        symbol_ids.insert("GBPAUD".to_string(), 189);
        symbol_ids.insert("GBPCAD_SB".to_string(), 190);
        symbol_ids.insert("GBPCAD".to_string(), 190);
        symbol_ids.insert("GBPNZD_SB".to_string(), 195);
        symbol_ids.insert("GBPNZD".to_string(), 195);

        // Other crosses
        symbol_ids.insert("AUDJPY_SB".to_string(), 155);
        symbol_ids.insert("AUDJPY".to_string(), 155);
        symbol_ids.insert("AUDNZD_SB".to_string(), 156);
        symbol_ids.insert("AUDNZD".to_string(), 156);
        symbol_ids.insert("AUDCAD_SB".to_string(), 153);
        symbol_ids.insert("AUDCAD".to_string(), 153);
        symbol_ids.insert("NZDJPY_SB".to_string(), 210);
        symbol_ids.insert("NZDJPY".to_string(), 210);
        symbol_ids.insert("CADJPY_SB".to_string(), 162);
        symbol_ids.insert("CADJPY".to_string(), 162);
        symbol_ids.insert("CHFJPY_SB".to_string(), 163);
        symbol_ids.insert("CHFJPY".to_string(), 163);

        // Indices
        symbol_ids.insert("NAS100_SB".to_string(), 205);
        symbol_ids.insert("NAS100".to_string(), 205);
        symbol_ids.insert("US500_SB".to_string(), 220);
        symbol_ids.insert("US500".to_string(), 220);

        symbol_ids
    }

    /// Main order placement method - simplified for Redis-only storage
    /// CRITICAL: Updated main trading method with Redis-first validation
    pub async fn check_and_place_orders(
        &mut self,
        price_update: &PriceUpdate,
        zones: &[Zone],
        csv_logger: &CsvLogger,
        trading_plan: Option<&crate::trading_plan::TradingPlan>,
        // Add strategies argument if not present
        strategies: Option<&StrategyManager>,
    ) {
        if !self.enabled {
            return;
        }

        // CRITICAL CHECK: Ensure Redis is available before ANY trading
        if let Err(e) = self.ensure_redis_available().await {
            error!("🚨 TRADING BLOCKED: Redis unavailable - {}", e);
            return; // Exit immediately - NO TRADING without Redis
        }

        let current_price = (price_update.bid + price_update.ask) / 2.0;
        let day_trading_mode = std::env::var("DAY_TRADING_MODE_ACTIVE").unwrap_or_else(|_| "false".to_string()).to_lowercase() == "true";

        for zone in zones {
            if !zone.is_active {
                continue;
            }

            // Zone strength filter
            if zone.strength < self.min_zone_strength {
                debug!(
                    "📋 Zone {} strength {:.1} below minimum {:.1}, skipping",
                    zone.id, zone.strength, self.min_zone_strength
                );
                continue;
            }

            let distance_pips = self.calculate_distance_to_zone(current_price, zone);

            if distance_pips <= self.distance_threshold_pips {
                // CRITICAL: Check Redis for existing orders (fail-safe behavior)
                if self.zone_has_pending_order(&zone.id).await {
                    debug!(
                        "📋 Zone {} already has pending order or Redis check failed, skipping",
                        zone.id
                    );
                    continue;
                }

                // Skip if timeframe not allowed
                if !self.allowed_timeframes.contains(&zone.timeframe) {
                    debug!(
                        "📋 Zone {} timeframe {} not allowed, skipping",
                        zone.id, zone.timeframe
                    );
                    continue;
                }

                // --- OR LOGIC: Trading Plan OR Day Trading Mode ---
                let mut allowed_by_plan = false;
                let mut allowed_by_strategy = false;
                let mut strategy_direction_match = false;
                let is_short_tf = zone.timeframe == "5m" || zone.timeframe == "15m";

                // Trading plan filter
                if let Some(plan) = trading_plan {
                    allowed_by_plan = plan.top_setups.iter().any(|s| s.symbol == price_update.symbol && s.timeframe == zone.timeframe);
                }

                // Day trading strategy filter (only for 5m/15m)
                if day_trading_mode && is_short_tf {
                    if let Some(strategies) = strategies {
                        let symbol_key = format!("{}_SB", price_update.symbol);
                        if let Some(strategy) = futures::executor::block_on(strategies.get_strategy_for_symbol(&symbol_key)) {
                            // Optional: check direction
                            let is_long = strategy.direction == "long";
                            let is_short = strategy.direction == "short";
                            let is_buy_zone = zone.zone_type.to_lowercase().contains("demand");
                            let is_sell_zone = zone.zone_type.to_lowercase().contains("supply");
                            strategy_direction_match = (is_long && is_buy_zone) || (is_short && is_sell_zone);
                            allowed_by_strategy = strategy_direction_match;
                        }
                    }
                }

                // OR logic: book if either filter passes
                if allowed_by_plan || allowed_by_strategy {
                    if allowed_by_plan && allowed_by_strategy {
                        info!("✅ Booking pending order: {} {} allowed by BOTH trading plan and day trading strategy", price_update.symbol, zone.timeframe);
                    } else if allowed_by_plan {
                        info!("✅ Booking pending order: {} {} allowed by trading plan", price_update.symbol, zone.timeframe);
                    } else if allowed_by_strategy {
                        info!("✅ Booking pending order: {} {} allowed by day trading strategy (direction match: {})", price_update.symbol, zone.timeframe, strategy_direction_match);
                    }
                } else {
                    debug!("❌ Skipping booking: {} {} not allowed by trading plan or day trading strategy", price_update.symbol, zone.timeframe);
                    continue;
                }

                // --- End OR logic ---

                // Create zone alert for CSV logging
                let zone_alert = ZoneAlert {
                    zone_id: zone.id.clone(),
                    symbol: price_update.symbol.clone(),
                    zone_type: zone.zone_type.clone(),
                    current_price,
                    zone_high: zone.high,
                    zone_low: zone.low,
                    distance_pips,
                    strength: zone.strength,
                    touch_count: zone.touch_count,
                    timeframe: zone.timeframe.clone(),
                    timestamp: chrono::Utc::now(),
                };

                // Attempt to place order
                match self
                    .place_pending_order_for_zone(price_update, zone, distance_pips, trading_plan)
                    .await
                {
                    Ok(pending_order) => {
                        // CRITICAL: Store in Redis IMMEDIATELY after broker success
                        match self.store_pending_order_redis(&pending_order).await {
                            Ok(_) => {
                                info!("✅ Order placed and stored in Redis for zone {}", zone.id);
                                csv_logger
                                    .log_booking_attempt(
                                        &zone_alert,
                                        "success",
                                        pending_order.ctrader_order_id.as_deref(),
                                        None,
                                    )
                                    .await;
                            }
                            Err(e) => {
                                error!("🚨 CRITICAL: Order placed with broker but Redis storage failed for zone {}: {}", zone.id, e);
                                csv_logger
                                    .log_booking_attempt(
                                        &zone_alert,
                                        "redis_storage_failed",
                                        pending_order.ctrader_order_id.as_deref(),
                                        Some(&format!("Redis storage failed: {}", e)),
                                    )
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "📋 Failed to place pending order for zone {}: {}",
                            zone.id, e
                        );
                        csv_logger
                            .log_booking_attempt(&zone_alert, "failed", None, Some(&e.to_string()))
                            .await;
                    }
                }
            }
        }
    }

    /// Place a pending order for a specific zone - returns PendingOrder
    async fn place_pending_order_for_zone(
        &mut self,
        price_update: &PriceUpdate,
        zone: &Zone,
        distance_pips: f64,
        trading_plan: Option<&crate::trading_plan::TradingPlan>, // <-- add trading_plan argument
    ) -> Result<PendingOrder, Box<dyn std::error::Error + Send + Sync>> {
        // Get symbol ID
        let symbol_id = match self.symbol_ids.get(&price_update.symbol) {
            Some(&id) => id,
            None => {
                return Err(format!("Unknown symbol: {}", price_update.symbol).into());
            }
        };

        // Determine zone type and order details
        let is_supply = zone.zone_type.to_lowercase().contains("supply");
        let trade_side = if is_supply { 2 } else { 1 }; // 1 = BUY, 2 = SELL
        let order_type_str = if is_supply { "SELL_LIMIT" } else { "BUY_LIMIT" };

        // Calculate entry price (proximal line)
        let entry_price = if is_supply {
            zone.low // For supply zones, sell when price reaches the low (proximal)
        } else {
            zone.high // For demand zones, buy when price reaches the high (proximal)
        };

        // Calculate stop loss and take profit
        let pip_value = self.get_pip_value(&price_update.symbol);
        let (sl_pips, tp_pips, plan_used) = if let Some(plan) = trading_plan {
            if let Some(setup) = plan.top_setups.iter().find(|s| s.symbol == price_update.symbol && s.timeframe == zone.timeframe) {
                (setup.sl, setup.tp, true)
            } else {
                (self.default_sl_pips, self.default_tp_pips, false)
            }
        } else {
            (self.default_sl_pips, self.default_tp_pips, false)
        };
        let (stop_loss, take_profit) = if is_supply {
            // SELL order: SL above entry, TP below entry
            let sl = entry_price + (sl_pips * pip_value);
            let tp = entry_price - (tp_pips * pip_value);
            (sl, tp)
        } else {
            // BUY order: SL below entry, TP above entry
            let sl = entry_price - (sl_pips * pip_value);
            let tp = entry_price + (tp_pips * pip_value);
            (sl, tp)
        };

        if plan_used {
            info!("📋 Using trading plan SL/TP for {} {}: SL={} TP={}", price_update.symbol, zone.timeframe, sl_pips, tp_pips);
        } else {
            info!("📋 Using default SL/TP for {} {}: SL={} TP={}", price_update.symbol, zone.timeframe, sl_pips, tp_pips);
        }

        // Volume calculation
        let display_volume = 0.1; // Base display volume
        let volume = self.convert_display_to_ctrader_volume(display_volume, symbol_id);

        // Validate volume
        if !self.validate_volume(symbol_id, volume) {
            let min_volume = self.get_minimum_volume(symbol_id);
            return Err(format!(
                "Volume {} is below minimum {} for symbol {}",
                volume, min_volume, symbol_id
            )
            .into());
        }

        // Round prices to correct decimal places for cTrader
        let (rounded_entry, rounded_sl, rounded_tp) =
            self.round_prices_for_symbol(&price_update.symbol, entry_price, stop_loss, take_profit);

        // Create the pending order request
        let request = PlacePendingOrderRequest {
            symbolId: symbol_id,
            tradeSide: trade_side,
            volume,
            entryPrice: rounded_entry,
            stopLoss: Some(rounded_sl),
            takeProfit: Some(rounded_tp),
        };

        info!(
            "📋 Placing {} pending order for zone {} at {:.5} (SL: {:.5}, TP: {:.5}) - Volume: {}",
            order_type_str, zone.id, entry_price, stop_loss, take_profit, volume
        );
        info!(
            "📋 Zone quality: strength={:.1}, touches={}, distance={:.1}pips",
            zone.strength, zone.touch_count, distance_pips
        );

        // Call cTrader API
        let api_url = format!("{}/placePendingOrder", self.ctrader_api_url);
        let response = self
            .http_client
            .post(&api_url)
            .json(&request)
            .send()
            .await?;

        let response_text = response.text().await?;

        // Parse API response
        let api_response: PlacePendingOrderResponse = serde_json::from_str(&response_text)
            .map_err(|e| {
                format!(
                    "Failed to parse API response: {} - Response: {}",
                    e, response_text
                )
            })?;

        // Create pending order record
        let pending_order = PendingOrder {
            zone_id: zone.id.clone(),
            symbol: price_update.symbol.clone(),
            timeframe: zone.timeframe.clone(),
            zone_type: zone.zone_type.clone(),
            order_type: order_type_str.to_string(),
            entry_price,
            lot_size: self.default_lot_size,
            stop_loss,
            take_profit,
            ctrader_order_id: api_response.order_id.clone(),
            placed_at: Utc::now(),
            status: "PENDING".to_string(),
            zone_high: zone.high,
            zone_low: zone.low,
            zone_strength: zone.strength,
            touch_count: zone.touch_count,
            distance_when_placed: distance_pips,
        };

        // Check if order placement was successful
        if api_response.success && api_response.order_id.is_some() {
            let order_id = api_response.order_id.unwrap();

            info!(
                "✅ Pending order placed successfully! Zone: {}, Order ID: {}",
                zone.id, order_id
            );

            // No InfluxDB write - only closed deals go to InfluxDB

            Ok(pending_order)
        } else {
            let error_msg = api_response
                .error_code
                .unwrap_or_else(|| api_response.message);
            warn!(
                "❌ Failed to place pending order for zone {}: {}",
                zone.id, error_msg
            );
            Err(error_msg.into())
        }
    }

    /// Deal enrichment method - polls broker and enriches with Redis data
    pub async fn enrich_and_save_deals(
        &self,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(0);
        }

        info!("📊 Starting deal enrichment process...");

        let broker_deals = self.fetch_broker_deals().await?;
        info!("📊 Fetched {} deals from broker", broker_deals.len());

        let mut enriched_count = 0;

        for deal in broker_deals {
            let mut enriched_deal = self.convert_broker_deal_to_enriched(&deal).await?;

            // Try to enrich with Redis pending order data
            if let Some(order_id) = &deal.order_id {
                let order_id_str = order_id.to_string();
                if let Some(pending_order) =
                    self.get_pending_order_for_enrichment(&order_id_str).await
                {
                    let zone_id_for_logging = pending_order.zone_id.clone();

                    // Enrich with zone data
                    enriched_deal.zone_id = Some(pending_order.zone_id);
                    enriched_deal.zone_type = Some(pending_order.zone_type);
                    enriched_deal.zone_strength = Some(pending_order.zone_strength);
                    enriched_deal.zone_high = Some(pending_order.zone_high);
                    enriched_deal.zone_low = Some(pending_order.zone_low);
                    enriched_deal.touch_count = Some(pending_order.touch_count);
                    enriched_deal.timeframe = Some(pending_order.timeframe);
                    enriched_deal.distance_when_placed = Some(pending_order.distance_when_placed);
                    enriched_deal.original_entry_price = Some(pending_order.entry_price);
                    enriched_deal.stop_loss = Some(pending_order.stop_loss);
                    enriched_deal.take_profit = Some(pending_order.take_profit);

                    // Calculate slippage
                    let pip_value = self.get_pip_value(&enriched_deal.symbol);
                    let slippage = (deal.exit_price - pending_order.entry_price).abs() / pip_value;
                    enriched_deal.slippage_pips = Some(slippage);

                    info!(
                        "📊 Enriched deal {} with zone data from {}",
                        deal.deal_id, zone_id_for_logging
                    );
                }
            }

            // Save to InfluxDB
            if let Some(ref order_db) = self.order_database {
                if let Err(e) = self
                    .save_enriched_deal_to_influx(&enriched_deal, order_db)
                    .await
                {
                    warn!("📊 Failed to save enriched deal: {}", e);
                } else {
                    enriched_count += 1;
                }
            }
        }

        info!(
            "📊 Deal enrichment completed - processed {} deals",
            enriched_count
        );
        Ok(enriched_count)
    }

    /// Fetch deals from broker API
    pub async fn fetch_broker_deals(
        &self,
    ) -> Result<Vec<BrokerDeal>, Box<dyn std::error::Error + Send + Sync>> {
        let today = chrono::Utc::now().date_naive();
        let date_str = today.format("%Y-%m-%d").to_string();

        let api_url = format!(
            "{}/deals/{}",
            self.ctrader_api_url, date_str
        );
        let response = self.http_client.get(&api_url).send().await?;

        let response_json: serde_json::Value = response.json().await?;
        let deals = response_json
            .get("closedTrades")
            .and_then(|v| v.as_array())
            .ok_or("No closedTrades array found")?;

        let broker_deals: Result<Vec<BrokerDeal>, _> = deals
            .iter()
            .map(|deal| serde_json::from_value(deal.clone()))
            .collect();

        Ok(broker_deals?)
    }

    /// Fetch deals from broker API for a specific date
    pub async fn fetch_broker_deals_for_date(
        &self,
        date: chrono::NaiveDate,
    ) -> Result<Vec<BrokerDeal>, Box<dyn std::error::Error + Send + Sync>> {
        let date_str = date.format("%Y-%m-%d").to_string();

        let api_url = format!(
            "{}/deals/{}",
            self.ctrader_api_url, date_str
        );
        let response = self.http_client.get(&api_url).send().await?;

        let response_json: serde_json::Value = response.json().await?;
        let deals = response_json
            .get("closedTrades")
            .and_then(|v| v.as_array())
            .ok_or("No closedTrades array found")?;

        let broker_deals: Result<Vec<BrokerDeal>, _> = deals
            .iter()
            .map(|deal| serde_json::from_value(deal.clone()))
            .collect();

        Ok(broker_deals?)
    }

    /// Convert broker deal to enriched deal structure
    pub async fn convert_broker_deal_to_enriched(
        &self,
        deal: &BrokerDeal,
    ) -> Result<EnrichedDeal, Box<dyn std::error::Error + Send + Sync>> {
        use chrono::{DateTime, Utc, NaiveDateTime};
        use serde_json::Value;

        let deal_id = deal.deal_id.to_string();
        let order_id = deal.order_id.map(|id| id.to_string());
        let position_id = deal.position_id.to_string();
        let symbol_id = Some(deal.symbol_id);
        let volume_in_lots = Some(deal.volume / 10000.0);
        let closing_trade_side = Some(deal.closing_trade_side);
        let price_difference = Some(deal.price_difference);
        let pips_profit = Some(deal.pips_profit);
        let gross_profit = Some(deal.gross_profit);
        let net_profit = Some(deal.net_profit);
        let swap = Some(deal.swap);
        let commission = Some(deal.commission);
        let pnl_conversion_fee = Some(deal.pnl_conversion_fee);
        let balance_after_trade = Some(deal.balance_after_trade);
        let balance_version = Some(deal.balance_version);
        let quote_to_deposit_conversion_rate = Some(deal.quote_to_deposit_conversion_rate);
        let deal_status = Some(deal.deal_status);
        let label = None;
        let comment = None;
        let raw_data = None; // Will be filled if present in /deals/{date}

        // Get symbol name from symbol_id
        let symbol = self
            .symbol_ids
            .iter()
            .find(|(_, &id)| id == deal.symbol_id)
            .map(|(symbol, _)| symbol.clone())
            .unwrap_or_else(|| format!("UNKNOWN_{}", deal.symbol_id));

        // --- NEW: Find both deals for this positionId in closedTrades and extract open/close times ---
        let today = chrono::Utc::now().date_naive();
        let date_str = today.format("%Y-%m-%d").to_string();
        let api_url = format!("{}/deals/{}", self.ctrader_api_url, date_str);
        let response = self.http_client.get(&api_url).send().await?;
        let response_json: serde_json::Value = response.json().await?;
        let closed_trades = response_json.get("closedTrades").and_then(|v| v.as_array()).ok_or("No closedTrades array found")?;
        let mut timestamps = vec![];
        for t in closed_trades {
            if t.get("positionId").and_then(|v| v.as_u64()) == Some(deal.position_id) {
                if let Some(ts) = t.get("executionTimestamp").and_then(|v| v.as_i64()) {
                    timestamps.push(ts);
                }
            }
        }
        tracing::info!("[Enrichment] All executionTimestamps for positionId {}: {:?}", deal.position_id, timestamps);
        let open_ts = timestamps.iter().min().cloned();
        let close_ts = timestamps.iter().max().cloned();
        let open_time = open_ts.and_then(|ts| NaiveDateTime::from_timestamp_millis(ts)).map(|dt| DateTime::<Utc>::from_utc(dt, Utc));
        let close_time = close_ts.and_then(|ts| NaiveDateTime::from_timestamp_millis(ts)).map(|dt| DateTime::<Utc>::from_utc(dt, Utc));
        let duration_minutes = match (open_ts, close_ts) {
            (Some(open), Some(close)) => Some(((close - open) as f64 / 60000.0).round() as i64),
            _ => None,
        };
        let execution_time = close_time.unwrap_or_else(|| Utc::now());
        let deal_type = match deal.deal_status {
            2 => "CLOSE".to_string(),
            1 => "OPEN".to_string(),
            _ => "UNKNOWN".to_string(),
        };
        Ok(EnrichedDeal {
            deal_id,
            order_id,
            position_id,
            symbol,
            symbol_id,
            volume: deal.volume,
            volume_in_lots,
            trade_side: deal.original_trade_side,
            closing_trade_side,
            price: deal.exit_price,
            price_difference,
            pips_profit,
            execution_time,
            deal_type,
            profit: Some(deal.net_profit),
            gross_profit,
            net_profit,
            swap,
            commission,
            pnl_conversion_fee,
            balance_after_trade,
            balance_version,
            quote_to_deposit_conversion_rate,
            raw_data,
            label,
            comment,
            deal_status,
            open_time,
            close_time,
            duration_minutes,
            // These will be enriched if pending order is found
            zone_id: None,
            zone_type: None,
            zone_strength: None,
            zone_high: None,
            zone_low: None,
            touch_count: None,
            timeframe: None,
            distance_when_placed: None,
            original_entry_price: None,
            stop_loss: None,
            take_profit: None,
            slippage_pips: None,
        })
    }

    /// Save enriched deal to InfluxDB
    async fn save_enriched_deal_to_influx(
        &self,
        deal: &EnrichedDeal,
        order_db: &Arc<OrderDatabase>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🚀 Starting InfluxDB save for deal {}", deal.deal_id);

        match order_db.write_enriched_deal(deal).await {
            Ok(()) => {
                info!("✅ Successfully saved deal {} to InfluxDB", deal.deal_id);
                Ok(())
            }
            Err(e) => {
                error!(
                    "❌ CRITICAL FAILURE saving deal {} to InfluxDB: {}",
                    deal.deal_id, e
                );
                error!(
                    "❌ Deal details: symbol={}, type={}, price={:.5}, zone_id={:?}",
                    deal.symbol, deal.deal_type, deal.price, deal.zone_id
                );
                Err(e.into())
            }
        }
    }
    /// Calculate distance from current price to zone proximal line
    fn calculate_distance_to_zone(&self, current_price: f64, zone: &Zone) -> f64 {
        let is_supply = zone.zone_type.to_lowercase().contains("supply");
        let proximal_line = if is_supply {
            zone.low // Supply zone: proximal = low
        } else {
            zone.high // Demand zone: proximal = high
        };

        let pip_value = self.get_pip_value(&zone.symbol);
        let distance_pips = (current_price - proximal_line).abs() / pip_value;

        distance_pips
    }

    /// Get pip value for a symbol
    pub fn get_pip_value(&self, symbol: &str) -> f64 {
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

    /// Round prices to correct decimal places for cTrader API
    fn round_prices_for_symbol(
        &self,
        symbol: &str,
        entry: f64,
        sl: f64,
        tp: f64,
    ) -> (f64, f64, f64) {
        let decimal_places = if symbol.contains("JPY") {
            3 // JPY pairs: 3 decimal places (e.g., 110.123)
        } else {
            match symbol {
                "NAS100" => 1,               // Index: 1 decimal place
                "US500" => 1,                // Index: 1 decimal place
                "GBPCHF" | "GBPCHF_SB" => 3, // GBPCHF: 3 decimal places
                "EURCHF" | "EURCHF_SB" => 3, // CHF pairs
                "USDCHF" | "USDCHF_SB" => 3, // CHF pairs
                _ => 5,                      // Most forex pairs: 5 decimal places
            }
        };

        let multiplier = 10_f64.powi(decimal_places);
        let round_price = |price: f64| (price * multiplier).round() / multiplier;

        (round_price(entry), round_price(sl), round_price(tp))
    }

    /// Convert display volume to cTrader volume (matching Angular logic)
    fn convert_display_to_ctrader_volume(&self, display_volume: f64, symbol_id: i32) -> f64 {
        if self.is_jpy_pair(symbol_id) {
            display_volume * 100.0 // JPY: 0.1 → 10
        } else if self.is_index_pair(symbol_id) {
            display_volume * 10.0 // Indices: 0.1 → 1.0
        } else {
            display_volume * 10000.0 // Forex: 0.1 → 1000
        }
    }

    /// Check if symbol is JPY pair
    fn is_jpy_pair(&self, symbol_id: i32) -> bool {
        matches!(symbol_id, 226 | 177 | 192 | 155 | 210 | 162 | 163)
        // 226=USDJPY, 177=EURJPY, 192=GBPJPY, 155=AUDJPY, 210=NZDJPY, 162=CADJPY, 163=CHFJPY
    }

    /// Check if symbol is index pair
    fn is_index_pair(&self, symbol_id: i32) -> bool {
        matches!(symbol_id, 220 | 205) // 220=US500, 205=NAS100
    }

    /// Validate volume
    fn validate_volume(&self, symbol_id: i32, volume: f64) -> bool {
        let min_volume = self.get_minimum_volume(symbol_id);
        volume >= min_volume
    }

    /// Get minimum volume for symbol
    fn get_minimum_volume(&self, symbol_id: i32) -> f64 {
        match symbol_id {
            220 => 1.0, // US500
            205 => 1.0, // NAS100
            _ => 0.01,  // Default for forex pairs
        }
    }

    /// Check if manager is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    /// Get Redis statistics with connection reuse
    pub async fn get_redis_stats(&self) -> (usize, usize) {
        if self.ensure_redis_available().await.is_err() {
            return (0, 0);
        }

        if let Some(conn_arc) = &self.redis_connection {
            if let Ok(mut conn) = conn_arc.lock() {
                let pending_orders: Result<Vec<String>, _> = conn.keys("pending_order:*");
                let lookups: Result<Vec<String>, _> = conn.keys("order_lookup:*");

                return (
                    pending_orders.map(|v| v.len()).unwrap_or(0),
                    lookups.map(|v| v.len()).unwrap_or(0),
                );
            }
        }
        (0, 0)
    }

    /// Poll all pending orders for fills using the cTrader order-details endpoint
    pub async fn poll_pending_orders_for_fills(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use chrono::Utc;
        use redis::Commands;
        use serde_json::Value;
        use std::env;

        // 1. Get all pending orders from Redis with status == "PENDING"
        let mut pending_orders = Vec::new();
        if let Some(ref client) = self.redis_client {
            let mut conn = client.get_connection()?;
            if let Ok(keys) = conn.keys::<_, Vec<String>>("pending_order:*") {
                tracing::debug!("[Polling] Found {} pending orders", keys.len());
                for key in keys {
                    if let Ok(order_json) = conn.get::<_, String>(&key) {
                        if let Ok(mut order) = serde_json::from_str::<Value>(&order_json) {
                            if order.get("status").and_then(|v| v.as_str()) == Some("PENDING") {
                                tracing::debug!("[Polling] Checking pending order: {} (ctrader_order_id={:?})", key, order.get("ctrader_order_id"));
                                pending_orders.push((key, order));
                            }
                        }
                    }
                }
            }
        }

        let account_id = env::var("CTID_TRADER_ACCOUNT_ID").unwrap_or_else(|_| "37972727".to_string());
        let api_url = env::var("CTRADER_API_BRIDGE_URL").unwrap_or_else(|_| "http://localhost:8000".to_string());
        let client = reqwest::Client::new();

        for (key, mut order) in pending_orders {
            if let Some(ctrader_order_id) = order.get("ctrader_order_id").and_then(|v| v.as_str()) {
                let url = format!("{}/order-details/{}/{}", api_url, account_id, ctrader_order_id);
                match client.get(&url).send().await {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            if let Ok(resp_json) = resp.json::<Value>().await {
                                let deals = resp_json.get("deals").and_then(|v| v.as_array());
                                if let Some(deals) = deals {
                                    tracing::debug!("[Polling] {}: found {} deals", ctrader_order_id, deals.len());
                                    if !deals.is_empty() {
                                        tracing::info!("[Polling] Pending order {} (ctrader_order_id={}) is FILLED and ready to be enriched (positionId={:?})", key, ctrader_order_id, deals[0].get("positionId"));
                                        // Mark as FILLED, add filled_at and positionId
                                        order["status"] = Value::String("FILLED".to_string());
                                        order["filled_at"] = Value::String(Utc::now().to_rfc3339());
                                        if let Some(position_id) = deals[0].get("positionId") {
                                            order["position_id"] = position_id.clone();
                                        }
                                        // Update in Redis
                                        if let Some(ref client) = self.redis_client {
                                            let mut conn = client.get_connection()?;
                                            let updated_json = serde_json::to_string(&order)?;
                                            let _: () = conn.set(&key, updated_json)?;
                                        }
                                        // Create entry in active_orders (active_trade:positionId)
                                        if let Some(position_id) = order.get("position_id").and_then(|v| v.as_u64()) {
                                            let active_key = format!("active_trade:{}", position_id);
                                            // Build enriched active trade from pending order and deal info
                                            if let Some(deal) = deals.get(0) {
                                                // Extract fields from deal and order-details
                                                let trade_data = resp_json.get("order").and_then(|o| o.get("tradeData"));
                                                let trade_side = trade_data.and_then(|td| td.get("tradeSide")).and_then(|v| v.as_i64()).unwrap_or(1);
                                                let symbol_id = trade_data.and_then(|td| td.get("symbolId")).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                                let symbol = self.symbol_ids.iter().find(|(_, &id)| id == symbol_id as i32).map(|(s, _)| s.clone()).unwrap_or_else(|| order.get("symbol").and_then(|v| v.as_str()).unwrap_or("").to_string());
                                                let volume = trade_data.and_then(|td| td.get("volume")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                                                let volume_lots = volume / 10000.0;
                                                let entry_price = deal.get("executionPrice").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                                let open_timestamp = trade_data.and_then(|td| td.get("openTimestamp")).and_then(|v| v.as_i64()).unwrap_or(0);
                                                let open_time = chrono::DateTime::<chrono::Utc>::from_utc(chrono::NaiveDateTime::from_timestamp_millis(open_timestamp).unwrap_or_else(|| chrono::NaiveDateTime::from_timestamp(0, 0)), chrono::Utc);
                                                let now = chrono::Utc::now();
                                                let duration_minutes = (now - open_time).num_minutes();
                                                let stop_loss = order.get("stop_loss").and_then(|v| v.as_f64());
                                                let take_profit = order.get("take_profit").and_then(|v| v.as_f64());
                                                let swap = deal.get("swap").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                                let commission = deal.get("commission").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                                let label = String::new();
                                                let comment = String::new();
                                                let pip_value = self.get_pip_value(&symbol);
                                                let distance_from_entry_pips = 0.0;
                                                let risk_reward_ratio = if let (Some(sl), Some(tp)) = (stop_loss, take_profit) {
                                                    let risk_pips = (entry_price - sl).abs() / pip_value;
                                                    let reward_pips = (tp - entry_price).abs() / pip_value;
                                                    if risk_pips > 0.0 { Some(reward_pips / risk_pips) } else { None }
                                                } else { None };
                                                let is_profitable = false;
                                                let max_drawdown_pips = None;
                                                let max_profit_pips = None;
                                                let last_updated = now;
                                                // Enrichment fields from pending order
                                                let zone_id = order.get("zone_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                                                let zone_type = order.get("zone_type").and_then(|v| v.as_str()).map(|s| s.to_string());
                                                let zone_strength = order.get("zone_strength").and_then(|v| v.as_f64());
                                                let zone_high = order.get("zone_high").and_then(|v| v.as_f64());
                                                let zone_low = order.get("zone_low").and_then(|v| v.as_f64());
                                                let touch_count = order.get("touch_count").and_then(|v| v.as_i64()).map(|i| i as i32);
                                                let timeframe = order.get("timeframe").and_then(|v| v.as_str()).map(|s| s.to_string());
                                                let distance_when_placed = order.get("distance_when_placed").and_then(|v| v.as_f64());
                                                let original_entry_price = order.get("entry_price").and_then(|v| v.as_f64());
                                                let slippage_pips = if let (Some(filled), Some(orig)) = (Some(entry_price), original_entry_price) {
                                                    Some(((filled - orig).abs()) / pip_value)
                                                } else { None };
                                                let enriched_trade = EnrichedActiveTrade {
                                                    position_id,
                                                    symbol,
                                                    trade_side: if trade_side == 1 { "BUY".to_string() } else { "SELL".to_string() },
                                                    volume_lots,
                                                    entry_price,
                                                    current_price: entry_price,
                                                    unrealized_pnl: 0.0,
                                                    pips_profit: 0.0,
                                                    open_time,
                                                    duration_minutes,
                                                    stop_loss,
                                                    take_profit,
                                                    swap,
                                                    commission,
                                                    label,
                                                    comment,
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
                                                    distance_from_entry_pips,
                                                    risk_reward_ratio,
                                                    is_profitable,
                                                    max_drawdown_pips,
                                                    max_profit_pips,
                                                    last_updated,
                                                };
                                                // Store in Redis
                                                if let Some(ref client) = self.redis_client {
                                                    let mut conn = client.get_connection()?;
                                                    let active_json = serde_json::to_string(&enriched_trade)?;
                                                    let _: () = conn.set(&active_key, active_json)?;
                                                    // Store reverse mapping: position_lookup:<position_id> -> <ctrader_order_id>
                                                    if let Some(ctrader_order_id) = order.get("ctrader_order_id").and_then(|v| v.as_str()) {
                                                        let position_lookup_key = format!("position_lookup:{}", position_id);
                                                        let _: () = conn.set(&position_lookup_key, ctrader_order_id)?;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            tracing::warn!("Order details endpoint returned error for {}: {}", ctrader_order_id, resp.status());
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to call order details endpoint for {}: {}", ctrader_order_id, e);
                    }
                }
            }
        }
        Ok(())
    }
}
