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
                // Improved fallback: Try to infer timeframe from existing pending orders for this symbol
                    let position_timestamp = DateTime::from_timestamp((position.openTimestamp / 1000) as i64, 0)
                        .unwrap_or(Utc::now());
                    
                    let fallback_timeframe = if let Some(ref pending_container) = pending_orders_container {
                        // Look for any recent pending orders for the same symbol to infer likely timeframe
                        let mut timeframe_candidates: Vec<&str> = pending_container.orders.values()
                            .filter(|order| order.symbol == symbol_name)
                            .filter(|order| {
                                // Only consider orders from the past 7 days
                                let days_diff = position_timestamp
                                    .signed_duration_since(order.placed_at)
                                    .num_days();
                                days_diff.abs() <= 7
                            })
                            .map(|order| order.timeframe.as_str())
                            .collect();
                        
                        // Sort by frequency and pick most common timeframe
                        timeframe_candidates.sort();
                        timeframe_candidates.dedup();
                        
                        if !timeframe_candidates.is_empty() {
                            // Pick the first (alphabetically) as a reasonable default
                            // This tends to favor shorter timeframes like "1H" over "4H"
                            timeframe_candidates[0].to_string()
                        } else {
                            // No recent orders found - use "unknown" instead of "market"
                            "unknown".to_string()
                        }
                    } else {
                        "unknown".to_string()
                    };
                    
                    warn!("⚠️ Creating fallback entry for position {} with timeframe '{}' (no matching pending order found)", 
                          position.positionId, fallback_timeframe);
                    
                    (
                        format!("pos_{}_{}", position.positionId, symbol_name),
                        fallback_timeframe,
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
    /// Uses improved flexible matching based on symbol, direction, price, and timing
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
        
        info!("🔍 Looking for pending order match for {} {} position {} at {:.5} (opened: {})", 
              symbol_name, position_side, position.positionId, position_price, 
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
            
            // 3. Order must have been placed BEFORE position opened (with some tolerance)
            let time_diff_seconds = position_timestamp
                .signed_duration_since(pending_order.placed_at)
                .num_seconds();
            
            // Allow orders placed up to 10 minutes after position time to account for timing differences
            if time_diff_seconds < -600 {
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
            let time_diff_minutes = time_diff_seconds.abs() / 60;
            
            // Create composite matching score with improved weighting
            // Price difference (pips) is the primary factor
            let mut match_score = price_diff_pips;
            
            // Add time penalty: 0.05 points per minute difference (reduced from 0.1)
            match_score += (time_diff_minutes as f64) * 0.05;
            
            // Bonus for exact cTrader order ID match (if available)
            let ctrader_id_bonus = if let Some(ref pending_ctrader_id) = pending_order.ctrader_order_id {
                let position_id_str = position.positionId.to_string();
                if pending_ctrader_id == &position_id_str {
                    // Exact match - heavily favor this
                    match_score -= 100.0;
                    true
                } else {
                    // Check for numerically close IDs (within 1000 of each other)
                    if let (Ok(pending_id_num), Ok(position_id_num)) = 
                        (pending_ctrader_id.parse::<u64>(), position_id_str.parse::<u64>()) {
                        let id_diff = (pending_id_num as i64 - position_id_num as i64).abs();
                        if id_diff <= 1000 {
                            // Close numeric IDs - give bonus
                            match_score -= 50.0 - (id_diff as f64 * 0.05);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
            } else {
                false
            };
            
            info!("   📊 Candidate: {} order at {:.5} (placed: {}, ctrader_id: {:?}, price_diff: {:.2} pips, time_diff: {}min, score: {:.2}, id_bonus: {})",
                  pending_order.zone_id,
                  pending_order.entry_price,
                  pending_order.placed_at.format("%H:%M:%S"),
                  pending_order.ctrader_order_id,
                  price_diff_pips,
                  time_diff_minutes,
                  match_score,
                  ctrader_id_bonus
            );
            
            // More lenient acceptance criteria - focus on price and time proximity
            let max_price_diff_pips = 10.0; // Increased from 5.0 pips
            let max_time_diff_hours = 48; // Increased from 24 hours
            
            if price_diff_pips <= max_price_diff_pips && 
               time_diff_minutes <= (max_time_diff_hours * 60) &&
               match_score < best_score {
                best_match = Some(pending_order);
                best_score = match_score;
                
                info!("   ⭐ New best match found! Score: {:.2} (price_diff: {:.2} pips, time_diff: {}min)", 
                      match_score, price_diff_pips, time_diff_minutes);
            }
        }
        
        if let Some(matched_order) = best_match {
            info!("🔗 MATCH FOUND: {} order {} -> position {} (score: {:.2}, zone_id: {}, timeframe: {})", 
                  symbol_name,
                  matched_order.ctrader_order_id.as_ref().unwrap_or(&"unknown".to_string()),
                  position.positionId,
                  best_score,
                  matched_order.zone_id,
                  matched_order.timeframe);
            Some(matched_order)
        } else {
            warn!("🔍 NO MATCH: No suitable pending order found for {} position {} at {:.5} - will create fallback entry", 
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
    
    /// Retroactively enrich existing booked orders that have "market" or "unknown" timeframes
    /// This is useful for fixing historical data after matching algorithm improvements
    pub async fn retroactive_enrichment(&self) -> Result<usize, String> {
        info!("🔄 Starting retroactive enrichment of existing booked orders...");
        
        // Load existing booked orders
        let mut booked_orders_container: BookedOrdersContainer =
            match fs::read_to_string("shared_booked_orders.json").await {
                Ok(content) => serde_json::from_str(&content).map_err(|e| {
                    format!("Failed to parse booked orders: {}", e)
                })?,
                Err(e) => {
                    return Err(format!("Failed to read booked orders file: {}", e));
                }
            };
            
        // Load pending orders for enrichment
        let pending_orders_container: PendingOrdersContainer =
            match fs::read_to_string("shared_pending_orders.json").await {
                Ok(content) => serde_json::from_str(&content).map_err(|e| {
                    format!("Failed to parse pending orders: {}", e)
                })?,
                Err(e) => {
                    return Err(format!("Failed to read pending orders file: {}", e));
                }
            };
            
        let mut enriched_count = 0;
        
        // Find booked orders that need enrichment (those with "market" or "unknown" timeframes)
        let orders_to_enrich: Vec<(String, BookedPendingOrder)> = booked_orders_container
            .booked_orders
            .iter()
            .filter(|(_, order)| {
                order.timeframe == "market" || order.timeframe == "unknown" || order.zone_type.is_none()
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
            
        info!("📋 Found {} booked orders that could benefit from retroactive enrichment", orders_to_enrich.len());
        
        for (zone_id, mut booked_order) in orders_to_enrich {
            // Skip if this order doesn't look like a fallback position entry
            if !zone_id.starts_with("pos_") && booked_order.zone_type.is_some() {
                continue;
            }
            
            info!("🔍 Attempting to enrich order: {} (symbol: {}, timeframe: {}, ctrader_id: {})", 
                  zone_id, booked_order.symbol, booked_order.timeframe, booked_order.ctrader_order_id);
            
            // Create a mock position from the booked order data to use with our matching logic
            let mock_position = Position {
                positionId: booked_order.ctrader_order_id.parse::<u64>().unwrap_or(0),
                symbolId: self.symbol_id_mapping.iter()
                    .find(|(_, symbol)| symbol.replace("_SB", "") == booked_order.symbol)
                    .map(|(id, _)| *id)
                    .unwrap_or(0),
                volume: (booked_order.lot_size as u32) * 100, // Convert back to volume
                tradeSide: if booked_order.order_type == "BUY" { 1 } else { 2 },
                price: booked_order.entry_price,
                openTimestamp: booked_order.filled_at
                    .unwrap_or(booked_order.booked_at)
                    .timestamp() as u64 * 1000, // Convert to milliseconds
                status: 1,
                swap: 0.0,
                commission: 0.0,
                usedMargin: 0.0,
                stopLoss: if booked_order.stop_loss != 0.0 { Some(booked_order.stop_loss) } else { None },
                takeProfit: if booked_order.take_profit != 0.0 { Some(booked_order.take_profit) } else { None },
            };
            
            // Try to find a matching pending order using our improved algorithm
            if let Some(matching_pending) = self.find_matching_pending_order(
                &pending_orders_container, 
                &booked_order.symbol, 
                &mock_position
            ) {
                // Enrich the booked order with data from the matching pending order
                booked_order.timeframe = matching_pending.timeframe.clone();
                booked_order.zone_type = Some(matching_pending.zone_type.clone());
                booked_order.zone_high = Some(matching_pending.zone_high);
                booked_order.zone_low = Some(matching_pending.zone_low);
                booked_order.zone_strength = Some(matching_pending.zone_strength);
                booked_order.touch_count = Some(matching_pending.touch_count);
                booked_order.distance_when_placed = Some(matching_pending.distance_when_placed);
                booked_order.original_zone_id = Some(matching_pending.zone_id.clone());
                
                // Update the zone_id to match the pending order if it was a fallback
                let new_zone_id = if zone_id.starts_with("pos_") {
                    matching_pending.zone_id.clone()
                } else {
                    zone_id.clone()
                };
                
                // IMPORTANT: Update the zone_id field in the booked_order itself
                booked_order.zone_id = new_zone_id.clone();
                
                // Remove old entry and add enriched entry
                booked_orders_container.booked_orders.remove(&zone_id);
                booked_orders_container.booked_orders.insert(new_zone_id.clone(), booked_order);
                
                enriched_count += 1;
                
                info!("✅ Successfully enriched order {} -> {} with timeframe '{}', zone_type: '{}'", 
                      zone_id, new_zone_id, matching_pending.timeframe, matching_pending.zone_type);
            } else {
                // Try to infer timeframe from other orders for the same symbol
                let inferred_timeframe = pending_orders_container.orders.values()
                    .filter(|order| order.symbol == booked_order.symbol)
                    .filter(|order| {
                        let time_diff = booked_order.booked_at
                            .signed_duration_since(order.placed_at)
                            .num_days()
                            .abs();
                        time_diff <= 7 // Within 7 days
                    })
                    .map(|order| order.timeframe.as_str())
                    .next(); // Take the first one found
                    
                if let Some(timeframe) = inferred_timeframe {
                    if booked_order.timeframe == "market" || booked_order.timeframe == "unknown" {
                        booked_order.timeframe = timeframe.to_string();
                        booked_orders_container.booked_orders.insert(zone_id.clone(), booked_order);
                        enriched_count += 1;
                        
                        info!("📊 Inferred timeframe '{}' for order {} based on nearby orders", 
                              timeframe, zone_id);
                    }
                } else {
                    info!("❌ Could not enrich order {}: no matching pending order or nearby timeframe found", zone_id);
                }
            }
        }
        
        // Save the updated booked orders if any changes were made
        if enriched_count > 0 {
            booked_orders_container.last_updated = Utc::now();
            let json_content = serde_json::to_string_pretty(&booked_orders_container)
                .map_err(|e| format!("Failed to serialize booked orders: {}", e))?;

            fs::write("shared_booked_orders.json", json_content)
                .await
                .map_err(|e| format!("Failed to write booked orders file: {}", e))?;

            info!("💾 Retroactive enrichment completed: {} orders enhanced", enriched_count);
        } else {
            info!("📋 No orders needed retroactive enrichment");
        }
        
        Ok(enriched_count)
    }
}
