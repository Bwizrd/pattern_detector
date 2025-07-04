// src/bin/zone_monitor/active_order_manager.rs
// Simplified: Focus only on PENDING orders and CLOSED trades (via deals API)
// Skip complex position tracking - let open positions exist until they close

use chrono::{DateTime, Utc};
use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::fs;
use tracing::{debug, error, info, warn};

// Import the pending order structures
use crate::pending_order_manager::{PendingOrdersContainer};
use crate::db::{OrderDatabase};
use crate::db::schema::{OrderEvent};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Order {
    pub orderId: u64,
    pub symbolId: u32,
    pub volume: u32,
    pub tradeSide: u32,
    pub orderType: u32,
    pub limitPrice: f64,
    pub orderStatus: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Position {
    pub positionId: u64,
    pub symbolId: u32,
    pub volume: u32,
    pub tradeSide: u32,
    pub price: f64,
    pub openTimestamp: u64,
    pub status: u32,
    pub swap: f64,
    pub commission: f64,
    pub usedMargin: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopLoss: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub takeProfit: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PositionsResponse {
    pub positions: Vec<Position>,
    pub orders: Vec<Order>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClosedTrade {
    #[serde(rename = "dealId")]
    pub deal_id: u64,
    #[serde(rename = "orderId")]
    pub order_id: u64,
    #[serde(rename = "positionId")]
    pub position_id: u64,
    #[serde(rename = "symbolId")]
    pub symbol_id: u32,
    #[serde(rename = "originalTradeSide")]
    pub original_trade_side: u32,
    #[serde(rename = "closingTradeSide")]
    pub closing_trade_side: u32,
    pub volume: u32,
    #[serde(rename = "volumeInLots")]
    pub volume_in_lots: f64,
    #[serde(rename = "entryPrice")]
    pub entry_price: f64,
    #[serde(rename = "exitPrice")]
    pub exit_price: f64,
    #[serde(rename = "priceDifference")]
    pub price_difference: f64,
    #[serde(rename = "pipsProfit")]
    pub pips_profit: f64,
    #[serde(rename = "openTime")]
    pub open_time: u64,
    #[serde(rename = "closeTime")]
    pub close_time: u64,
    pub duration: u64,
    #[serde(rename = "grossProfit")]
    pub gross_profit: f64,
    pub swap: f64,
    pub commission: f64,
    #[serde(rename = "pnlConversionFee")]
    pub pnl_conversion_fee: f64,
    #[serde(rename = "netProfit")]
    pub net_profit: f64,
    #[serde(rename = "balanceAfterTrade")]
    pub balance_after_trade: f64,
    #[serde(rename = "balanceVersion")]
    pub balance_version: u64,
    #[serde(rename = "quoteToDepositConversionRate")]
    pub quote_to_deposit_conversion_rate: f64,
    pub label: String,
    pub comment: String,
    #[serde(rename = "dealStatus")]
    pub deal_status: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DealsResponse {
    #[serde(rename = "closedTrades")]
    pub closed_trades: Vec<ClosedTrade>,
    #[serde(rename = "totalDeals")]
    pub total_deals: u32,
    #[serde(rename = "hasMore")]
    pub has_more: bool,
}

#[derive(Debug)]
pub struct ActiveOrderManager {
    api_url: String,
    http_client: reqwest::Client,
    symbol_id_mapping: HashMap<u32, String>,
    order_database: Arc<tokio::sync::RwLock<Option<Arc<OrderDatabase>>>>,
    processed_deals: Arc<tokio::sync::Mutex<std::collections::HashSet<u64>>>, // Track processed deal IDs
}

impl ActiveOrderManager {
    pub fn new() -> Self {
        let api_url = std::env::var("POSITIONS_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8000/positions".to_string());

        // Symbol ID mapping from trading engine
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

        info!("📋 Simplified Active Order Manager initialized:");
        info!("   API URL: {}", api_url);
        info!("   Symbol mappings: {}", symbol_id_mapping.len());
        info!("   Focus: PENDING orders → InfluxDB, CLOSED trades → InfluxDB");

        Self {
            api_url,
            http_client: reqwest::Client::new(),
            symbol_id_mapping,
            order_database: Arc::new(tokio::sync::RwLock::new(None)),
            processed_deals: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }
    
    /// Set the order database for InfluxDB tracking
    pub async fn set_order_database(&self, order_database: Arc<OrderDatabase>) {
        let mut db_lock = self.order_database.write().await;
        *db_lock = Some(order_database);
        info!("📋 ActiveOrderManager: Order database connected for InfluxDB tracking");
    }

    pub async fn fetch_positions(&self) -> Result<PositionsResponse, String> {
        info!("📋 Fetching positions from API: {}", self.api_url);

        match self
            .http_client
            .get(&self.api_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<PositionsResponse>().await {
                        Ok(positions_response) => {
                            debug!("✅ Successfully fetched {} positions and {} orders", 
                                  positions_response.positions.len(), 
                                  positions_response.orders.len());
                            Ok(positions_response)
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to parse positions JSON: {}", e);
                            error!("❌ {}", error_msg);
                            Err(error_msg)
                        }
                    }
                } else {
                    let error_msg = format!("API returned error status: {}", response.status());
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

    /// Fetch recent deals from broker API to get position closure data with P&L
    pub async fn fetch_recent_deals(&self) -> Result<DealsResponse, String> {
        // Use today's date for the deals endpoint
        let today = Utc::now().format("%Y-%m-%d");
        let deals_url = self.api_url.replace("/positions", &format!("/deals/{}", today));
        debug!("📋 Fetching recent deals from API: {}", deals_url);

        match self
            .http_client
            .get(&deals_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<DealsResponse>().await {
                        Ok(deals_response) => {
                            debug!("✅ Successfully fetched {} closed trades", deals_response.closed_trades.len());
                            Ok(deals_response)
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to parse deals JSON: {}", e);
                            error!("❌ {}", error_msg);
                            Err(error_msg)
                        }
                    }
                } else {
                    let error_msg = format!("Deals API returned error status: {}", response.status());
                    error!("❌ {}", error_msg);
                    Err(error_msg)
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to call deals API: {}", e);
                error!("❌ {}", error_msg);
                Err(error_msg)
            }
        }
    }

    /// Main processing method - simplified to focus on status changes only
    pub async fn process_order_updates(&self) -> Result<(), String> {
        info!("🔄 Processing order updates (PENDING status changes + CLOSED deals)...");

        // 1. Process pending order status changes (PENDING → FILLED/CANCELLED)
        if let Err(e) = self.process_pending_order_changes().await {
            error!("❌ Failed to process pending order changes: {}", e);
        }

        // 2. Process new closed deals
        if let Err(e) = self.process_closed_deals().await {
            error!("❌ Failed to process closed deals: {}", e);
        }

        Ok(())
    }

    /// Process changes in pending orders (PENDING → FILLED/CANCELLED)
    async fn process_pending_order_changes(&self) -> Result<(), String> {
        debug!("📋 Starting pending order changes processing...");
        let positions_response = self.fetch_positions().await?;
        
        debug!("📋 Fetched {} positions and {} orders from broker", 
              positions_response.positions.len(), positions_response.orders.len());
        
        // Load pending orders file
        let mut pending_container: PendingOrdersContainer =
            match fs::read_to_string("shared_pending_orders.json").await {
                Ok(content) => {
                    debug!("📋 Successfully loaded pending orders file");
                    serde_json::from_str(&content)
                        .map_err(|e| format!("Failed to parse pending orders: {}", e))?
                }
                Err(e) => {
                    debug!("📋 No pending orders file found: {}", e);
                    return Ok(()); // File doesn't exist, nothing to update
                }
            };

        debug!("📋 Loaded {} pending orders from file", pending_container.orders.len());

        let broker_pending_ids: std::collections::HashSet<String> = positions_response
            .orders
            .iter()
            .map(|order| order.orderId.to_string())
            .collect();

        let broker_position_ids: std::collections::HashSet<String> = positions_response
            .positions
            .iter()
            .map(|pos| pos.positionId.to_string())
            .collect();

        debug!("📋 Broker has {} pending order IDs, {} position IDs", 
              broker_pending_ids.len(), broker_position_ids.len());

        let mut updates_made = false;

        // Check each PENDING order for status changes
        for pending_order in pending_container.orders.values_mut() {
            if pending_order.status != "PENDING" {
                continue; // Skip non-pending orders
            }

            if let Some(ref ctrader_id) = pending_order.ctrader_order_id {
                if broker_position_ids.contains(ctrader_id) {
                    // Order became a position - mark as FILLED
                    info!("✅ Order {} (zone: {}) became position - marking FILLED", 
                          ctrader_id, pending_order.zone_id);
                    
                    pending_order.status = "FILLED".to_string();
                    updates_made = true;

                    // Write FILLED event to InfluxDB (we still track this transition)
                    self.write_filled_event_to_influx(pending_order).await;

                } else if !broker_pending_ids.contains(ctrader_id) {
                    // Order no longer exists at broker - mark as CANCELLED
                    info!("❌ Order {} (zone: {}) no longer at broker - marking CANCELLED", 
                          ctrader_id, pending_order.zone_id);
                    
                    pending_order.status = "CANCELLED".to_string();
                    updates_made = true;

                    // Write CANCELLED event to InfluxDB
                    self.write_cancelled_event_to_influx(pending_order).await;
                }
            }
        }

        // Save updates if any were made
        if updates_made {
            pending_container.last_updated = Utc::now();
            let json_content = serde_json::to_string_pretty(&pending_container)
                .map_err(|e| format!("Failed to serialize pending orders: {}", e))?;

            fs::write("shared_pending_orders.json", json_content)
                .await
                .map_err(|e| format!("Failed to write pending orders file: {}", e))?;

            info!("💾 Updated pending orders file with status changes");
        } else {
            debug!("📋 No pending order status changes detected");
        }

        Ok(())
    }

    /// Process new closed deals and write to InfluxDB
    async fn process_closed_deals(&self) -> Result<(), String> {
        debug!("📋 Starting closed deals processing...");
        let deals_response = self.fetch_recent_deals().await?;
        
        debug!("📋 Fetched {} closed trades from deals API", deals_response.closed_trades.len());
        
        if deals_response.closed_trades.is_empty() {
            debug!("📋 No closed trades found to process");
            return Ok(());
        }

        // Load pending orders to get zone enrichment data
        let pending_container: Option<PendingOrdersContainer> =
            match fs::read_to_string("shared_pending_orders.json").await {
                Ok(content) => {
                    debug!("📋 Loaded pending orders for deal enrichment");
                    serde_json::from_str(&content).ok()
                }
                Err(e) => {
                    debug!("📋 Could not load pending orders for deal enrichment: {}", e);
                    None
                }
            };

        let mut processed_deals = self.processed_deals.lock().await;
        let mut new_deals_count = 0;

        debug!("📋 Previously processed {} deals", processed_deals.len());

        for closed_trade in &deals_response.closed_trades {
            // Skip if we've already processed this deal
            if processed_deals.contains(&closed_trade.deal_id) {
                debug!("📋 Skipping already processed deal {}", closed_trade.deal_id);
                continue;
            }

            debug!("📋 Processing new deal {}", closed_trade.deal_id);

            // Try to find the original pending order for enrichment
            let enrichment_data = if let Some(ref pending_container) = pending_container {
                self.find_pending_order_for_deal(pending_container, closed_trade)
            } else {
                None
            };

            debug!("📋 Deal {} enrichment: {}", 
                  closed_trade.deal_id, 
                  if enrichment_data.is_some() { "found" } else { "not found" });

            // Write CLOSED event to InfluxDB
            self.write_closed_event_to_influx(closed_trade, enrichment_data).await;

            // Mark this deal as processed
            processed_deals.insert(closed_trade.deal_id);
            new_deals_count += 1;
        }

        if new_deals_count > 0 {
            info!("✅ Processed {} new closed deals and wrote to InfluxDB", new_deals_count);
        } else {
            debug!("📋 No new deals to process");
        }

        Ok(())
    }

    /// Find the original pending order that matches this deal for enrichment
    fn find_pending_order_for_deal<'a>(
        &self,
        pending_container: &'a PendingOrdersContainer,
        closed_trade: &ClosedTrade
    ) -> Option<&'a crate::pending_order_manager::PendingOrder> {
        let symbol_name = self.symbol_id_mapping.get(&closed_trade.symbol_id)
            .map(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        let deal_side = if closed_trade.original_trade_side == 1 { "BUY" } else { "SELL" };
        let deal_timestamp = DateTime::from_timestamp((closed_trade.open_time / 1000) as i64, 0)
            .unwrap_or(Utc::now());

        debug!("🔍 Looking for pending order match for deal {} ({} {} at {:.5})", 
               closed_trade.deal_id, symbol_name, deal_side, closed_trade.entry_price);

        // Look for matching pending order
        for pending_order in pending_container.orders.values() {
            // Match by symbol
            if pending_order.symbol != symbol_name {
                continue;
            }

            // Match by trade side
            let pending_side = if pending_order.order_type.contains("BUY") { "BUY" } else { "SELL" };
            if pending_side != deal_side {
                continue;
            }

            // Match by price proximity (within 10 pips)
            let price_diff = (pending_order.entry_price - closed_trade.entry_price).abs();
            let pip_value = self.get_pip_value(symbol_name);
            let price_diff_pips = price_diff / pip_value;
            
            if price_diff_pips > 10.0 {
                continue;
            }

            // Match by time proximity (within 7 days)
            let time_diff_hours = deal_timestamp
                .signed_duration_since(pending_order.placed_at)
                .num_hours()
                .abs();
            
            if time_diff_hours > 168 { // 7 days
                continue;
            }

            // Check for cTrader ID match if available
            if let Some(ref pending_ctrader_id) = pending_order.ctrader_order_id {
                // Try to match by order_id or position_id
                if pending_ctrader_id == &closed_trade.order_id.to_string() || 
                   pending_ctrader_id == &closed_trade.position_id.to_string() {
                    info!("🔗 Perfect match found: deal {} ↔ pending order {} (cTrader ID match)", 
                          closed_trade.deal_id, pending_order.zone_id);
                    return Some(pending_order);
                }
            }

            // Fuzzy match based on price and time
            info!("🔗 Fuzzy match found: deal {} ↔ pending order {} (price diff: {:.2} pips, time diff: {}h)", 
                  closed_trade.deal_id, pending_order.zone_id, price_diff_pips, time_diff_hours);
            return Some(pending_order);
        }

        warn!("🔍 No matching pending order found for deal {} ({} {})", 
              closed_trade.deal_id, symbol_name, deal_side);
        None
    }

    /// Write FILLED event to InfluxDB (tracking transition only)
    async fn write_filled_event_to_influx(&self, pending_order: &crate::pending_order_manager::PendingOrder) {
        let order_db_guard = self.order_database.read().await;
        if let Some(ref order_db) = *order_db_guard {
            let base_event = OrderEvent::new_pending(
                pending_order.zone_id.clone(),
                pending_order.symbol.clone(),
                pending_order.timeframe.clone(),
                pending_order.zone_type.clone(),
                pending_order.order_type.clone(),
                pending_order.entry_price,
                pending_order.lot_size,
                pending_order.stop_loss,
                pending_order.take_profit,
            );
            
            let filled_event = base_event.to_filled(
                pending_order.ctrader_order_id.clone().unwrap_or_default(),
                pending_order.entry_price,
            );
            
            let order_db_clone = Arc::clone(order_db);
            let event_clone = filled_event.clone();
            tokio::spawn(async move {
                if let Err(e) = order_db_clone.write_order_event(&event_clone).await {
                    debug!("📊 FILLED event write failed (might already exist): {}", e);
                } else {
                    info!("📊 Wrote FILLED event to InfluxDB for order {}", 
                          event_clone.ctrader_order_id.as_ref().unwrap_or(&"unknown".to_string()));
                }
            });
        }
        drop(order_db_guard);
    }

    /// Write CANCELLED event to InfluxDB
    async fn write_cancelled_event_to_influx(&self, pending_order: &crate::pending_order_manager::PendingOrder) {
        let order_db_guard = self.order_database.read().await;
        if let Some(ref order_db) = *order_db_guard {
            let base_event = OrderEvent::new_pending(
                pending_order.zone_id.clone(),
                pending_order.symbol.clone(),
                pending_order.timeframe.clone(),
                pending_order.zone_type.clone(),
                pending_order.order_type.clone(),
                pending_order.entry_price,
                pending_order.lot_size,
                pending_order.stop_loss,
                pending_order.take_profit,
            );
            
            let mut cancelled_event = base_event.to_cancelled();
            cancelled_event.ctrader_order_id = Some(pending_order.ctrader_order_id.clone().unwrap_or_default());
            cancelled_event.close_reason = Some("order_expired_or_cancelled".to_string());
            
            let order_db_clone = Arc::clone(order_db);
            let event_clone = cancelled_event.clone();
            tokio::spawn(async move {
                if let Err(e) = order_db_clone.write_order_event(&event_clone).await {
                    error!("❌ Failed to write CANCELLED event to InfluxDB: {}", e);
                } else {
                    info!("📊 Wrote CANCELLED event to InfluxDB for order {}", 
                          event_clone.ctrader_order_id.as_ref().unwrap_or(&"unknown".to_string()));
                }
            });
        }
        drop(order_db_guard);
    }

    /// Write CLOSED event to InfluxDB with full deal data and zone enrichment
    async fn write_closed_event_to_influx(
        &self, 
        closed_trade: &ClosedTrade, 
        pending_order: Option<&crate::pending_order_manager::PendingOrder>
    ) {
        let order_db_guard = self.order_database.read().await;
        if let Some(ref order_db) = *order_db_guard {
            let symbol_name = self.symbol_id_mapping.get(&closed_trade.symbol_id)
                .map(|s| s.as_str())
                .unwrap_or("UNKNOWN");

            let trade_side = if closed_trade.original_trade_side == 1 { "BUY" } else { "SELL" };

            let _close_time = DateTime::from_timestamp((closed_trade.close_time / 1000) as i64, 0)
                .unwrap_or(Utc::now());

            // Create event with zone enrichment if available
            let base_event = if let Some(pending) = pending_order {
                // Use enriched data from pending order
                OrderEvent::new_pending(
                    pending.zone_id.clone(),
                    pending.symbol.clone(),
                    pending.timeframe.clone(),
                    pending.zone_type.clone(),
                    pending.order_type.clone(),
                    pending.entry_price,
                    pending.lot_size,
                    pending.stop_loss,
                    pending.take_profit,
                )
            } else {
                // Create minimal event without zone enrichment
                OrderEvent::new_pending(
                    format!("deal_{}", closed_trade.deal_id),
                    symbol_name.to_string(),
                    "unknown".to_string(),
                    if trade_side == "BUY" { "demand_zone" } else { "supply_zone" }.to_string(),
                    trade_side.to_string(),
                    closed_trade.entry_price,
                    closed_trade.volume_in_lots as i32,
                    0.0,
                    0.0,
                )
            };
            
            // Convert to FILLED then to CLOSED
            let filled_event = base_event.to_filled(
                closed_trade.position_id.to_string(),
                closed_trade.entry_price,
            );
            
            let closed_event = filled_event.to_closed(
                closed_trade.deal_id.to_string(),
                closed_trade.exit_price,
                closed_trade.pips_profit, // Use the pre-calculated pips from API
                "position_closed".to_string(),
            );
            
            let order_db_clone = Arc::clone(order_db);
            let event_clone = closed_event.clone();
            let enriched = pending_order.is_some();
            let pips_profit = closed_trade.pips_profit; // Copy the value before moving into closure
            tokio::spawn(async move {
                if let Err(e) = order_db_clone.write_order_event(&event_clone).await {
                    error!("❌ Failed to write CLOSED event to InfluxDB: {}", e);
                } else {
                    info!("📊 Wrote CLOSED event to InfluxDB for deal {} (enriched: {}, P&L: {:.2} pips)", 
                          event_clone.ctrader_deal_id.as_ref().unwrap_or(&"unknown".to_string()),
                          enriched,
                          pips_profit);
                }
            });
        }
        drop(order_db_guard);
    }

    fn get_pip_value(&self, symbol: &str) -> f64 {
        if symbol.contains("JPY") {
            0.01
        } else {
            match symbol {
                "NAS100" => 1.0,
                "US500" => 0.1,
                _ => 0.0001,
            }
        }
    }
}