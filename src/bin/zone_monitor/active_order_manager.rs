// src/bin/zone_monitor/active_order_manager.rs
// Manages tracking of active trades and matching them with pending orders

use chrono::{DateTime, Utc};
use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::fs;
use tracing::{debug, error, info, warn};

// Import the pending order structures
use crate::pending_order_manager::{
    BookedOrdersContainer, BookedPendingOrder, PendingOrdersContainer,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedOrder {
    pub order_id: u64,
    pub position_id: Option<u64>, // Will be set when order becomes position
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub symbol_id: u32,
    pub volume: u32,
    pub trade_side: u32,
    pub limit_price: f64,
    pub order_status: u32,
    pub status: String, // "pending", "filled", "cancelled"
    pub created_at: DateTime<Utc>,
    pub filled_at: Option<DateTime<Utc>>,
    pub filled_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedOrdersContainer {
    pub last_updated: DateTime<Utc>,
    pub tracked_orders: HashMap<String, TrackedOrder>, // order_id -> TrackedOrder
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Position {
    pub positionId: u64,
    pub symbolId: u32,
    pub volume: u32,
    pub tradeSide: u32,
    pub price: f64, // This is the entry price for positions
    pub openTimestamp: u64,
    pub status: u32,
    pub swap: f64,
    pub commission: f64,
    pub usedMargin: f64,

    // Make these optional since API conditionally provides them
    // Some positions have SL/TP, others don't
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopLoss: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub takeProfit: Option<f64>,
}

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
struct PositionsResponse {
    pub positions: Vec<Position>,
    pub orders: Vec<Order>,
}

#[derive(Debug, Clone)]
pub struct ActiveOrderManager {
    api_url: String,
    http_client: reqwest::Client,
    pub tracked_orders_file: String,
    symbol_id_mapping: HashMap<u32, String>,
    last_update: std::sync::Arc<tokio::sync::Mutex<chrono::DateTime<chrono::Utc>>>,
}

impl ActiveOrderManager {
    pub fn new() -> Self {
        let api_url = std::env::var("POSITIONS_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8000/positions".to_string());

        let tracked_orders_file = std::env::var("TRACKED_ORDERS_FILE")
            .unwrap_or_else(|_| "shared_tracked_orders.json".to_string());

        // Symbol ID mapping from trading engine
        let mut symbol_id_mapping = HashMap::new();
        symbol_id_mapping.insert(185, "EURUSD_SB".to_string());
        symbol_id_mapping.insert(199, "GBPUSD_SB".to_string());
        symbol_id_mapping.insert(226, "USDJPY_SB".to_string());
        symbol_id_mapping.insert(222, "USDCHF_SB".to_string());
        symbol_id_mapping.insert(158, "AUDUSD_SB".to_string());
        symbol_id_mapping.insert(221, "USDCAD_SB".to_string());
        symbol_id_mapping.insert(211, "NZDUSD_SB".to_string());
        symbol_id_mapping.insert(175, "EURGBP_SB".to_string());
        symbol_id_mapping.insert(177, "EURJPY_SB".to_string());
        symbol_id_mapping.insert(173, "EURCHF_SB".to_string());
        symbol_id_mapping.insert(171, "EURAUD_SB".to_string());
        symbol_id_mapping.insert(172, "EURCAD_SB".to_string());
        symbol_id_mapping.insert(180, "EURNZD_SB".to_string());
        symbol_id_mapping.insert(192, "GBPJPY_SB".to_string());
        symbol_id_mapping.insert(191, "GBPCHF_SB".to_string());
        symbol_id_mapping.insert(189, "GBPAUD_SB".to_string());
        symbol_id_mapping.insert(190, "GBPCAD_SB".to_string());
        symbol_id_mapping.insert(195, "GBPNZD_SB".to_string());
        symbol_id_mapping.insert(155, "AUDJPY_SB".to_string());
        symbol_id_mapping.insert(156, "AUDNZD_SB".to_string());
        symbol_id_mapping.insert(153, "AUDCAD_SB".to_string());
        symbol_id_mapping.insert(210, "NZDJPY_SB".to_string());
        symbol_id_mapping.insert(162, "CADJPY_SB".to_string());
        symbol_id_mapping.insert(163, "CHFJPY_SB".to_string());
        symbol_id_mapping.insert(205, "NAS100_SB".to_string());
        symbol_id_mapping.insert(220, "US500_SB".to_string());

        info!("📋 Active Order Manager initialized:");
        info!("   API URL: {}", api_url);
        info!("   Tracked orders file: {}", tracked_orders_file);
        info!("   Symbol mappings: {}", symbol_id_mapping.len());

        Self {
            api_url,
            http_client: reqwest::Client::new(),
            tracked_orders_file,
            symbol_id_mapping,
            last_update: std::sync::Arc::new(tokio::sync::Mutex::new(
                Utc::now() - chrono::Duration::minutes(2),
            )),
        }
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
                            info!("✅ Successfully fetched positions and orders");
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

    pub async fn update_booked_orders(&self) -> Result<(), String> {
        // Use the last_update mutex to prevent concurrent execution
        let _lock = self.last_update.lock().await;
        let positions_response = self.fetch_positions().await?;

        info!(
            "📋 Found {} positions and {} pending orders",
            positions_response.positions.len(),
            positions_response.orders.len()
        );
        
        // Load existing booked orders
        let mut booked_orders_container: BookedOrdersContainer =
            match fs::read_to_string("shared_booked_orders.json").await {
                Ok(content) => serde_json::from_str(&content).unwrap_or(BookedOrdersContainer {
                    last_updated: Utc::now(),
                    booked_orders: HashMap::new(),
                }),
                Err(_) => BookedOrdersContainer {
                    last_updated: Utc::now(),
                    booked_orders: HashMap::new(),
                },
            };
        
        info!("📋 DEBUG: Starting enrichment check for {} positions", positions_response.positions.len());

        for position in &positions_response.positions {
            let position_id_str = position.positionId.to_string();
            info!("📋 DEBUG: Checking position {} - already tracked: {}", 
                position_id_str,
                booked_orders_container.booked_orders.values()
                    .any(|booked| booked.ctrader_order_id == position_id_str));
        }

        // Load pending orders for enrichment
        let pending_orders_container: Option<PendingOrdersContainer> =
            match fs::read_to_string("shared_pending_orders.json").await {
                Ok(content) => serde_json::from_str(&content).ok(),
                Err(e) => {
                    warn!("📋 Could not load pending orders for enrichment: {}", e);
                    None
                }
            };

        // NEW: Identify orders that are no longer pending (they filled)
        let filled_order_ids =
            self.identify_filled_orders(&positions_response, &pending_orders_container);
        if !filled_order_ids.is_empty() {
            info!(
                "📋 Detected {} newly filled orders: {:?}",
                filled_order_ids.len(),
                filled_order_ids
            );
            // We'll mark these as filled after processing positions
        }

        let mut updates_made = false;

        // Create a set of current position IDs for tracking closures
        let current_position_ids: std::collections::HashSet<String> = positions_response
            .positions
            .iter()
            .map(|pos| pos.positionId.to_string())
            .collect();

        // Check for closed positions
        for (_zone_id, booked_order) in &mut booked_orders_container.booked_orders {
            if booked_order.status == "FILLED" && !booked_order.ctrader_order_id.is_empty() {
                if !current_position_ids.contains(&booked_order.ctrader_order_id) {
                    // Position was closed
                    booked_order.status = "CLOSED".to_string();
                    booked_order.closed_at = Some(Utc::now());
                    updates_made = true;
                    info!(
                        "🔒 Position {} ({}) closed",
                        booked_order.ctrader_order_id, booked_order.symbol
                    );
                }
            }
        }

        // Check each current position and enrich with pending order data
        for position in &positions_response.positions {
            let symbol_name = self.symbol_id_mapping.get(&position.symbolId)
                .map(|s| s.replace("_SB", "")) // Remove _SB suffix for consistency
                .unwrap_or_else(|| format!("UNKNOWN_{}", position.symbolId));
            
            let position_id_str = position.positionId.to_string();
            
            // Check if we already have this exact position tracked by position ID
            let already_tracked = booked_orders_container.booked_orders.values()
                .any(|booked| booked.ctrader_order_id == position_id_str);
            
            if !already_tracked {
                // Try to find matching pending order for enrichment using improved algorithm
                let enrichment_data = if let Some(ref pending_container) = pending_orders_container {
                    self.find_matching_pending_order(&pending_container, &symbol_name, position)
                } else {
                    None
                };

                // Create new booked order for this position
                let (zone_id, timeframe, zone_info) = if let Some(pending_data) = &enrichment_data {
                    // Use enriched data from pending order
                    (
                        pending_data.zone_id.clone(),
                        pending_data.timeframe.clone(),
                        Some((
                            pending_data.zone_strength,
                            pending_data.touch_count,
                            pending_data.zone_high,
                            pending_data.zone_low,
                            pending_data.zone_type.clone()
                        ))
                    )
                } else {
                    // Fallback to generated data for positions without matching pending orders
                    (
                        format!("pos_{}_{}", position.positionId, symbol_name),
                        "market".to_string(),
                        None
                    )
                };

                let order_type = if position.tradeSide == 1 { "BUY" } else { "SELL" };

                // Fix: Convert u64 timestamp to i64 for DateTime::from_timestamp
                let timestamp_i64 = (position.openTimestamp / 1000) as i64;
                
                let mut booked_order = BookedPendingOrder {
                    zone_id: zone_id.clone(),
                    symbol: symbol_name.clone(),
                    timeframe,
                    order_type: order_type.to_string(),
                    entry_price: position.price,
                    lot_size: (position.volume as f64 / 100.0) as i32,
                    
                    // Handle optional stop loss and take profit from API
                    stop_loss: position.stopLoss.unwrap_or(0.0),
                    take_profit: position.takeProfit.unwrap_or(0.0),
                    
                    ctrader_order_id: position_id_str.clone(),
                    booked_at: DateTime::from_timestamp(timestamp_i64, 0)
                        .unwrap_or(Utc::now()),
                    status: "FILLED".to_string(),
                    filled_at: Some(DateTime::from_timestamp(timestamp_i64, 0)
                        .unwrap_or(Utc::now())),
                    filled_price: Some(position.price),
                    closed_at: None,
                    
                    // Enriched zone data - will be populated below if match found
                    zone_type: None,
                    zone_high: None,
                    zone_low: None,
                    zone_strength: None,
                    touch_count: None,
                    distance_when_placed: None,
                    original_zone_id: None,
                };

                // Add enriched zone information if available
                if let Some((zone_strength, touch_count, zone_high, zone_low, zone_type)) = zone_info {
                    booked_order.zone_type = Some(zone_type.clone());
                    booked_order.zone_high = Some(zone_high);
                    booked_order.zone_low = Some(zone_low);
                    booked_order.zone_strength = Some(zone_strength);
                    booked_order.touch_count = Some(touch_count);
                    
                    if let Some(pending_data) = &enrichment_data {
                        booked_order.distance_when_placed = Some(pending_data.distance_when_placed);
                        booked_order.original_zone_id = Some(pending_data.zone_id.clone());
                    }
                    
                    info!("✅ Enriched booked order {} with zone data: strength={}, touches={}, type={} (from pending order: {})", 
                          zone_id, 
                          zone_strength, 
                          touch_count, 
                          zone_type,
                          enrichment_data.as_ref().map(|d| d.zone_id.as_str()).unwrap_or("unknown"));
                } else {
                    warn!("⚠️ Could not enrich position {} - no matching pending order found", position.positionId);
                }
                
                booked_orders_container.booked_orders.insert(zone_id.clone(), booked_order);
                updates_made = true;
                
                info!("✅ Added booked order: {} position {} at {:.5} (SL: {}, TP: {}) - zone: {}", 
                      symbol_name, 
                      position.positionId, 
                      position.price,
                      position.stopLoss.map(|sl| format!("{:.5}", sl)).unwrap_or("None".to_string()),
                      position.takeProfit.map(|tp| format!("{:.5}", tp)).unwrap_or("None".to_string()),
                      zone_id);
            } 
        
        }

        // NEW: Mark filled orders in the pending orders file
        if !filled_order_ids.is_empty() {
            info!(
                "📋 Marking {} orders as filled in pending orders file",
                filled_order_ids.len()
            );
            if let Err(e) = self.mark_filled_orders_in_file(&filled_order_ids).await {
                error!("❌ Failed to mark orders as filled: {}", e);
            }
        }

        // Only save if updates were made
        if updates_made {
            booked_orders_container.last_updated = Utc::now();
            let json_content = serde_json::to_string_pretty(&booked_orders_container)
                .map_err(|e| format!("Failed to serialize booked orders: {}", e))?;

            fs::write("shared_booked_orders.json", json_content)
                .await
                .map_err(|e| format!("Failed to write booked orders file: {}", e))?;

            info!("💾 Booked orders updated successfully");
        } else {
            debug!("📋 No booked order updates needed");
        }

        Ok(())
    }

    /// Identify which pending orders are no longer pending (they filled)
    fn identify_filled_orders(
        &self,
        positions_response: &PositionsResponse,
        pending_container: &Option<PendingOrdersContainer>,
    ) -> Vec<String> {
        let mut filled_order_ids = Vec::new();

        if let Some(pending_container) = pending_container {
            // Get current pending order IDs from broker
            let broker_pending_ids: std::collections::HashSet<String> = positions_response
                .orders
                .iter()
                .map(|order| order.orderId.to_string())
                .collect();

            // Check which of our local pending orders are no longer pending at broker
            for pending_order in pending_container.orders.values() {
                if let Some(ref ctrader_id) = pending_order.ctrader_order_id {
                    if pending_order.status == "PENDING" && !broker_pending_ids.contains(ctrader_id)
                    {
                        // This order was pending but is no longer at the broker - it filled
                        filled_order_ids.push(ctrader_id.clone());
                    }
                }
            }
        }

        filled_order_ids
    }

    async fn mark_filled_orders_in_file(&self, filled_order_ids: &[String]) -> Result<(), String> {
        if filled_order_ids.is_empty() {
            return Ok(());
        }

        // Load pending orders file
        let mut pending_container: PendingOrdersContainer =
            match fs::read_to_string("shared_pending_orders.json").await {
                Ok(content) => serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse pending orders: {}", e))?,
                Err(_) => return Ok(()), // File doesn't exist, nothing to update
            };

        let mut updates_made = false;

        // Mark orders as filled
        for order_id in filled_order_ids {
            for pending_order in pending_container.orders.values_mut() {
                if let Some(ref ctrader_id) = pending_order.ctrader_order_id {
                    if ctrader_id == order_id && pending_order.status == "PENDING" {
                        info!(
                            "📋 Marking order {} (zone: {}) as FILLED",
                            order_id, pending_order.zone_id
                        );
                        pending_order.status = "FILLED".to_string();
                        updates_made = true;
                        break;
                    }
                }
            }
        }

        // Save updated file
        if updates_made {
            pending_container.last_updated = Utc::now();
            let json_content = serde_json::to_string_pretty(&pending_container)
                .map_err(|e| format!("Failed to serialize pending orders: {}", e))?;

            fs::write("shared_pending_orders.json", json_content)
                .await
                .map_err(|e| format!("Failed to write pending orders file: {}", e))?;

            info!(
                "💾 Updated {} orders to FILLED status",
                filled_order_ids.len()
            );
        }

        Ok(())
    }

    /// Find matching pending order for a position to enrich the booked order
    fn find_matching_pending_order<'a>(
        &self,
        pending_container: &'a PendingOrdersContainer,
        symbol_name: &str,
        position: &Position
    ) -> Option<&'a crate::pending_order_manager::PendingOrder> {
        
        let position_side = if position.tradeSide == 1 { "BUY" } else { "SELL" };
        let position_price = position.price;
        let position_timestamp = DateTime::from_timestamp((position.openTimestamp / 1000) as i64, 0)
            .unwrap_or(Utc::now());
        
        info!("🔍 Looking for pending order match for {} {} position at {:.5} (opened: {})", 
              symbol_name, position_side, position_price, 
              position_timestamp.format("%H:%M:%S"));

        let mut best_match: Option<&crate::pending_order_manager::PendingOrder> = None;
        let mut best_score = f64::MAX; // Lower score = better match
        
        for pending_order in pending_container.orders.values() {
            // 1. Symbol must match exactly
            if pending_order.symbol != symbol_name {
                continue;
            }
            
            // 2. Trade side must match
            let pending_side = if pending_order.order_type.contains("BUY") { "BUY" } else { "SELL" };
            if pending_side != position_side {
                continue;
            }
            
            // 3. Order must have been placed BEFORE position opened
            if pending_order.placed_at > position_timestamp {
                continue;
            }
            
            // 4. Skip orders that are not FILLED status (shouldn't have become positions)
            if pending_order.status != "FILLED" {
                continue;
            }
            
            // 5. Calculate price proximity score
            let price_diff = (pending_order.entry_price - position_price).abs();
            let pip_value = self.get_pip_value(symbol_name);
            let price_diff_pips = price_diff / pip_value;
            
            // 6. Calculate time proximity score (closer in time = better match)
            let time_diff_minutes = position_timestamp
                .signed_duration_since(pending_order.placed_at)
                .num_minutes();
            
            // Create composite matching score
            // Price difference (pips) + time penalty
            let mut match_score = price_diff_pips;
            
            // Add time penalty: 0.1 points per minute difference
            if time_diff_minutes > 0 {
                match_score += (time_diff_minutes as f64) * 0.1;
            }
            
            info!("   📊 Candidate: {} order at {:.5} (placed: {}, price_diff: {:.2} pips, time_diff: {}min, score: {:.2})",
                  pending_order.zone_id,
                  pending_order.entry_price,
                  pending_order.placed_at.format("%H:%M:%S"),
                  price_diff_pips,
                  time_diff_minutes,
                  match_score
            );
            
            // Accept matches within reasonable tolerance
            let max_price_diff_pips = 5.0; // Allow up to 5 pips difference
            let max_time_diff_hours = 24; // Allow matches within 24 hours
            
            if price_diff_pips <= max_price_diff_pips && 
               time_diff_minutes <= (max_time_diff_hours * 60) &&
               match_score < best_score {
                best_match = Some(pending_order);
                best_score = match_score;
                
                info!("   ⭐ New best match found! Score: {:.2}", match_score);
            }
        }
        
        if let Some(matched_order) = best_match {
            info!("🔗 Best match found: {} order {} -> position {} (score: {:.2})", 
                  symbol_name,
                  matched_order.zone_id, 
                  position.positionId,
                  best_score);
            Some(matched_order)
        } else {
            warn!("🔍 No matching pending order found for {} position {} at {:.5}", 
                  symbol_name, position.positionId, position_price);
            None
        }
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
