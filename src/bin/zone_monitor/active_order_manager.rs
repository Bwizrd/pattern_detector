// src/bin/zone_monitor/active_order_manager.rs
// Manages tracking of active trades and matching them with pending orders

use chrono::{DateTime, Utc};
use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::fs;
use tracing::{error, info, warn, debug};

// Import the pending order structures
use crate::pending_order_manager::{PendingOrdersContainer, PendingOrder, BookedOrdersContainer, BookedPendingOrder};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedOrder {
    pub order_id: u64,
    pub position_id: Option<u64>,  // Will be set when order becomes position
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub symbol_id: u32,
    pub volume: u32,
    pub trade_side: u32,
    pub limit_price: f64,
    pub order_status: u32,
    pub status: String,  // "pending", "filled", "cancelled"
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
    pub stopLoss: f64,
    pub takeProfit: f64,
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
            last_update: std::sync::Arc::new(tokio::sync::Mutex::new(Utc::now() - chrono::Duration::minutes(2))),
        }
    }

    pub async fn fetch_positions(&self) -> Result<PositionsResponse, String> {
        info!("📋 Fetching positions from API: {}", self.api_url);

        match self.http_client
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
        
        info!("📋 Found {} positions", positions_response.positions.len());
        
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
                }
            };

        let mut updates_made = false;
        
        // Create a set of current position IDs for tracking closures
        let current_position_ids: std::collections::HashSet<String> = positions_response.positions
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
                    info!("🔒 Position {} ({}) closed", booked_order.ctrader_order_id, booked_order.symbol);
                }
            }
        }
        
        // Check each current position
        for position in &positions_response.positions {
            let symbol_name = self.symbol_id_mapping.get(&position.symbolId)
                .map(|s| s.replace("_SB", "")) // Remove _SB suffix for consistency
                .unwrap_or_else(|| format!("UNKNOWN_{}", position.symbolId));
            
            let position_id_str = position.positionId.to_string();
            
            // Check if we already have this exact position tracked by position ID
            let already_tracked = booked_orders_container.booked_orders.values()
                .any(|booked| booked.ctrader_order_id == position_id_str);
            
            if !already_tracked {
                // Create new booked order for this position
                let zone_id = format!("pos_{}_{}", position.positionId, symbol_name);
                let order_type = if position.tradeSide == 1 { "BUY" } else { "SELL" };
                
                let booked_order = BookedPendingOrder {
                    zone_id: zone_id.clone(),
                    symbol: symbol_name.clone(),
                    timeframe: "market".to_string(), // These are market positions, not pending orders
                    order_type: order_type.to_string(),
                    entry_price: position.price,
                    lot_size: (position.volume as f64 / 1000.0) as i32,
                    stop_loss: position.stopLoss,
                    take_profit: position.takeProfit,
                    ctrader_order_id: position_id_str,
                    booked_at: Utc::now(),
                    status: "FILLED".to_string(),
                    filled_at: Some(Utc::now()),
                    filled_price: Some(position.price),
                    closed_at: None,
                };
                
                booked_orders_container.booked_orders.insert(zone_id.clone(), booked_order);
                updates_made = true;
                
                info!("✅ Added new booked order: {} position {} at {}", symbol_name, position.positionId, position.price);
            }
        }
        
        // Only save if updates were made - don't overwrite constantly
        if updates_made {
            booked_orders_container.last_updated = Utc::now();
            let json_content = serde_json::to_string_pretty(&booked_orders_container)
                .map_err(|e| format!("Failed to serialize booked orders: {}", e))?;
            
            fs::write("shared_booked_orders.json", json_content).await
                .map_err(|e| format!("Failed to write booked orders file: {}", e))?;
            
            info!("💾 Booked orders updated successfully");
        } else {
            // Only log this at debug level to reduce noise
            debug!("📋 No updates needed");
        }

        Ok(())
    }
}
