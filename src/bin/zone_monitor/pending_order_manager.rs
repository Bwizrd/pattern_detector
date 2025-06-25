// src/bin/zone_monitor/pending_order_manager.rs
// Manages automatic limit order placement for zones within specified distance

use crate::types::{Zone, PriceUpdate};
use std::collections::HashMap;
use std::env;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::{debug, error, info, warn};
use reqwest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOrder {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub order_type: String,        // "BUY_LIMIT" or "SELL_LIMIT"
    pub entry_price: f64,          // The limit price
    pub lot_size: i32,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub ctrader_order_id: Option<String>, // Order ID returned by cTrader
    pub placed_at: DateTime<Utc>,
    pub status: String,            // "PENDING", "FILLED", "CANCELLED", "FAILED"
    pub zone_high: f64,
    pub zone_low: f64,
    pub zone_strength: f64,
    pub touch_count: i32,
    pub distance_when_placed: f64, // Distance in pips when order was placed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOrdersContainer {
    pub last_updated: DateTime<Utc>,
    pub orders: HashMap<String, PendingOrder>, // zone_id -> PendingOrder
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookedPendingOrder {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub order_type: String,        // "BUY_LIMIT" or "SELL_LIMIT"
    pub entry_price: f64,
    pub lot_size: i32,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub ctrader_order_id: String,
    pub booked_at: DateTime<Utc>,
    pub status: String,            // "PENDING", "FILLED", "CANCELLED"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookedOrdersContainer {
    pub last_updated: DateTime<Utc>,
    pub booked_orders: HashMap<String, BookedPendingOrder>, // zone_id -> BookedPendingOrder
}

#[derive(Debug)]
pub struct PendingOrderManager {
    enabled: bool,
    distance_threshold_pips: f64,
    shared_file_path: String,
    booked_orders_file_path: String,
    default_lot_size: i32,
    default_sl_pips: f64,
    default_tp_pips: f64,
    ctrader_api_url: String,
    pending_orders: HashMap<String, PendingOrder>, // zone_id -> order
    booked_orders: HashMap<String, BookedPendingOrder>, // zone_id -> successfully booked pending order
    http_client: reqwest::Client,
    symbol_ids: HashMap<String, i32>, // symbol -> symbol_id mapping
    allowed_timeframes: Vec<String>, // allowed timeframes for trading
}

#[derive(Debug, Serialize)]
struct PlacePendingOrderRequest {
    symbolId: i32,             // Symbol ID (number)
    tradeSide: i32,            // 1 = BUY, 2 = SELL
    volume: f64,               // Volume
    entryPrice: f64,           // The limit price
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
    pub fn new() -> Self {
        let enabled = env::var("ENABLE_LIMIT_ORDERS")
            .unwrap_or_else(|_| "false".to_string())
            .trim()
            .to_lowercase() == "true";

        let distance_threshold_pips = env::var("LIMIT_ORDER_DISTANCE_PIPS")
            .unwrap_or_else(|_| "20.0".to_string())
            .parse::<f64>()
            .unwrap_or(20.0);

        let shared_file_path = env::var("LIMIT_ORDER_SHARED_FILE")
            .unwrap_or_else(|_| "shared_pending_orders.json".to_string());

        let booked_orders_file_path = env::var("BOOKED_ORDERS_SHARED_FILE")
            .unwrap_or_else(|_| "shared_booked_orders.json".to_string());

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

        if enabled {
            info!("📋 Pending Order Manager initialized:");
            info!("   Enabled: {}", enabled);
            info!("   Distance threshold: {:.1} pips", distance_threshold_pips);
            info!("   Shared file: {}", shared_file_path);
            info!("   Default lot size: {}", default_lot_size);
            info!("   Default SL: {:.1} pips", default_sl_pips);
            info!("   Default TP: {:.1} pips", default_tp_pips);
            info!("   cTrader API: {}", ctrader_api_url);
            info!("   Allowed timeframes: {:?}", allowed_timeframes);
        } else {
            info!("📋 Pending Order Manager disabled");
        }

        Self {
            enabled,
            distance_threshold_pips,
            shared_file_path,
            booked_orders_file_path,
            default_lot_size,
            default_sl_pips,
            default_tp_pips,
            ctrader_api_url,
            pending_orders: HashMap::new(),
            booked_orders: HashMap::new(),
            http_client: reqwest::Client::new(),
            symbol_ids: Self::init_symbol_ids(),
            allowed_timeframes,
        }
    }

    /// Initialize symbol ID mapping (matching Angular CURRENCIES_MAP)
    fn init_symbol_ids() -> HashMap<String, i32> {
        let mut symbol_ids = HashMap::new();
        
        // Using the correct symbol IDs from Angular CURRENCIES_MAP
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

    /// Load existing pending orders from shared file
    pub async fn load_pending_orders(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        match fs::read_to_string(&self.shared_file_path).await {
            Ok(content) => {
                let container: PendingOrdersContainer = serde_json::from_str(&content)?;
                self.pending_orders = container.orders;
                info!("📋 Loaded {} pending orders from {}", self.pending_orders.len(), self.shared_file_path);
            },
            Err(_) => {
                // File doesn't exist yet, start with empty orders
                info!("📋 No existing pending orders file found, starting fresh");
            }
        }

        Ok(())
    }

    /// Save pending orders to shared file
    async fn save_pending_orders(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let container = PendingOrdersContainer {
            last_updated: Utc::now(),
            orders: self.pending_orders.clone(),
        };

        let json_content = serde_json::to_string_pretty(&container)?;
        fs::write(&self.shared_file_path, json_content).await?;

        debug!("📋 Saved {} pending orders to {}", self.pending_orders.len(), self.shared_file_path);
        Ok(())
    }

    /// Save booked orders to shared file
    async fn save_booked_orders(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let container = BookedOrdersContainer {
            last_updated: Utc::now(),
            booked_orders: self.booked_orders.clone(),
        };

        let json_content = serde_json::to_string_pretty(&container)?;
        fs::write(&self.booked_orders_file_path, json_content).await?;

        debug!("📋 Saved {} booked orders to {}", self.booked_orders.len(), self.booked_orders_file_path);
        Ok(())
    }

    /// Check zones and place pending orders for eligible ones
    pub async fn check_and_place_orders(&mut self, price_update: &PriceUpdate, zones: &[Zone]) {
        if !self.enabled {
            return;
        }

        let current_price = (price_update.bid + price_update.ask) / 2.0;

        for zone in zones {
            if !zone.is_active {
                continue;
            }

            // Skip if we already have a pending order for this zone
            if self.pending_orders.contains_key(&zone.id) {
                continue;
            }

            // Skip if we already have a pending order for this symbol/timeframe combination
            let symbol_timeframe_key = format!("{}_{}", zone.symbol, zone.timeframe);
            if self.pending_orders.values().any(|order| 
                format!("{}_{}", order.symbol, order.timeframe) == symbol_timeframe_key
            ) {
                continue;
            }

            // Skip if we already have a successfully booked order for this symbol/timeframe combination
            if self.booked_orders.values().any(|order| 
                format!("{}_{}", order.symbol, order.timeframe) == symbol_timeframe_key && 
                order.status == "PENDING"
            ) {
                continue;
            }

            // Skip if zone timeframe is not in allowed timeframes
            if !self.allowed_timeframes.contains(&zone.timeframe) {
                continue;
            }

            let distance_pips = self.calculate_distance_to_zone(current_price, zone);

            // Check if zone is within our distance threshold
            if distance_pips <= self.distance_threshold_pips {
                if let Err(e) = self.place_pending_order_for_zone(price_update, zone, distance_pips).await {
                    error!("📋 Failed to place pending order for zone {}: {}", zone.id, e);
                }
            }
        }

        // Save updated orders to files
        if let Err(e) = self.save_pending_orders().await {
            error!("📋 Failed to save pending orders: {}", e);
        }
        if let Err(e) = self.save_booked_orders().await {
            error!("📋 Failed to save booked orders: {}", e);
        }
    }

    /// Place a pending order for a specific zone
    async fn place_pending_order_for_zone(
        &mut self, 
        price_update: &PriceUpdate, 
        zone: &Zone, 
        distance_pips: f64
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        
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
            zone.low  // For supply zones, sell when price reaches the low (proximal)
        } else {
            zone.high // For demand zones, buy when price reaches the high (proximal)
        };

        // Calculate stop loss and take profit
        let pip_value = self.get_pip_value(&price_update.symbol);
        let (stop_loss, take_profit) = if is_supply {
            // SELL order: SL above entry, TP below entry
            let sl = entry_price + (self.default_sl_pips * pip_value);
            let tp = entry_price - (self.default_tp_pips * pip_value);
            (sl, tp)
        } else {
            // BUY order: SL below entry, TP above entry
            let sl = entry_price - (self.default_sl_pips * pip_value);
            let tp = entry_price + (self.default_tp_pips * pip_value);
            (sl, tp)
        };

        // Volume calculation - exactly matching Angular logic
        let display_volume = 0.1; // Base display volume
        let volume = self.convert_display_to_ctrader_volume(display_volume, symbol_id);
        
        // Validate volume before proceeding (matching Angular logic)
        if !self.validate_volume(symbol_id, volume) {
            let min_volume = self.get_minimum_volume(symbol_id);
            return Err(format!("Volume {} is below minimum {} for symbol {}", volume, min_volume, symbol_id).into());
        }

        // Round prices to correct decimal places for cTrader
        let (rounded_entry, rounded_sl, rounded_tp) = self.round_prices_for_symbol(&price_update.symbol, entry_price, stop_loss, take_profit);

        // Create the pending order request
        let request = PlacePendingOrderRequest {
            symbolId: symbol_id,
            tradeSide: trade_side,
            volume,
            entryPrice: rounded_entry,
            stopLoss: Some(rounded_sl),
            takeProfit: Some(rounded_tp),
        };

        info!("📋 Placing {} pending order for zone {} at {:.5} (SL: {:.5}, TP: {:.5}) - Volume: {}", 
              order_type_str, zone.id, entry_price, stop_loss, take_profit, volume);
        
        let request_json = serde_json::to_string_pretty(&request).unwrap_or_else(|_| "Failed to serialize".to_string());
        info!("📋 Request JSON: {}", request_json);

        // Call cTrader API
        let api_url = format!("{}/placePendingOrder", self.ctrader_api_url);
        let response = self.http_client
            .post(&api_url)
            .json(&request)
            .send()
            .await?;

        let response_text = response.text().await?;
        
        // Try to parse as the new API response format
        let api_response: PlacePendingOrderResponse = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse API response: {} - Response: {}", e, response_text))?;

        // Create pending order record
        let mut pending_order = PendingOrder {
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
            pending_order.status = "PENDING".to_string();
            pending_order.ctrader_order_id = Some(order_id.clone());
            info!("✅ Pending order placed successfully! Zone: {}, Order ID: {}", zone.id, order_id);
            
            // Store the successful pending order
            self.pending_orders.insert(zone.id.clone(), pending_order.clone());
            
            // Also record it as a successfully booked order
            let booked_order = BookedPendingOrder {
                zone_id: zone.id.clone(),
                symbol: price_update.symbol.clone(),
                timeframe: zone.timeframe.clone(),
                order_type: order_type_str.to_string(),
                entry_price,
                lot_size: self.default_lot_size,
                stop_loss,
                take_profit,
                ctrader_order_id: order_id,
                booked_at: Utc::now(),
                status: "PENDING".to_string(),
            };
            self.booked_orders.insert(zone.id.clone(), booked_order);
            
        } else {
            pending_order.status = "FAILED".to_string();
            let error_msg = api_response.error_code
                .unwrap_or_else(|| api_response.message);
            warn!("❌ Failed to place pending order for zone {}: {}", zone.id, error_msg);
            
            // Store the failed order too (to prevent retries)
            self.pending_orders.insert(zone.id.clone(), pending_order);
            
            return Err(error_msg.into());
        }

        Ok(())
    }

    /// Calculate distance from current price to zone proximal line
    fn calculate_distance_to_zone(&self, current_price: f64, zone: &Zone) -> f64 {
        let is_supply = zone.zone_type.to_lowercase().contains("supply");
        let proximal_line = if is_supply {
            zone.low  // Supply zone: proximal = low
        } else {
            zone.high // Demand zone: proximal = high
        };
        
        let pip_value = self.get_pip_value(&zone.symbol);
        let distance_pips = (current_price - proximal_line).abs() / pip_value;
        
        distance_pips
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

    /// Round prices to correct decimal places for cTrader API
    fn round_prices_for_symbol(&self, symbol: &str, entry: f64, sl: f64, tp: f64) -> (f64, f64, f64) {
        let decimal_places = if symbol.contains("JPY") {
            3 // JPY pairs: 3 decimal places (e.g., 110.123)
        } else {
            match symbol {
                "NAS100" => 1,        // Index: 1 decimal place
                "US500" => 1,         // Index: 1 decimal place
                "GBPCHF" | "GBPCHF_SB" => 3, // GBPCHF: 3 decimal places based on error
                "EURCHF" | "EURCHF_SB" => 3, // CHF pairs likely similar 
                "USDCHF" | "USDCHF_SB" => 3, // CHF pairs likely similar
                _ => 5,               // Most forex pairs: 5 decimal places
            }
        };

        let multiplier = 10_f64.powi(decimal_places);
        let round_price = |price: f64| (price * multiplier).round() / multiplier;

        (round_price(entry), round_price(sl), round_price(tp))
    }

    /// Get count of pending orders
    pub fn get_pending_orders_count(&self) -> usize {
        self.pending_orders.len()
    }

    /// Check if a zone has a pending order
    pub fn has_pending_order(&self, zone_id: &str) -> bool {
        self.pending_orders.contains_key(zone_id)
    }

    /// Get pending order for a zone
    pub fn get_pending_order(&self, zone_id: &str) -> Option<&PendingOrder> {
        self.pending_orders.get(zone_id)
    }

    /// Remove a pending order (e.g., when it gets filled or cancelled)
    pub async fn remove_pending_order(&mut self, zone_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.pending_orders.remove(zone_id).is_some() {
            info!("📋 Removed pending order for zone {}", zone_id);
            self.save_pending_orders().await?;
        }
        Ok(())
    }

    /// Update order status
    pub async fn update_order_status(&mut self, zone_id: &str, new_status: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(order) = self.pending_orders.get_mut(zone_id) {
            order.status = new_status.to_string();
            info!("📋 Updated order status for zone {} to {}", zone_id, new_status);
            self.save_pending_orders().await?;
        }
        Ok(())
    }

    /// Get all pending orders
    pub fn get_all_pending_orders(&self) -> &HashMap<String, PendingOrder> {
        &self.pending_orders
    }

    /// Check if manager is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Convert display volume to cTrader volume (matching Angular logic)
    fn convert_display_to_ctrader_volume(&self, display_volume: f64, symbol_id: i32) -> f64 {
        if self.is_jpy_pair(symbol_id) {
            display_volume * 100.0  // JPY: 0.1 → 10 
        } else if self.is_index_pair(symbol_id) {
            display_volume * 10.0   // Indices: 0.1 → 1.0
        } else {
            display_volume * 10000.0  // Forex: 0.1 → 1000
        }
    }

    /// Check if symbol is JPY pair (matching Angular logic)
    fn is_jpy_pair(&self, symbol_id: i32) -> bool {
        // JPY pairs from the currency map
        matches!(symbol_id, 226 | 177 | 192 | 155 | 210 | 162 | 163)
        // 226=USDJPY, 177=EURJPY, 192=GBPJPY, 155=AUDJPY, 210=NZDJPY, 162=CADJPY, 163=CHFJPY
    }

    /// Check if symbol is index pair (matching Angular logic)
    fn is_index_pair(&self, symbol_id: i32) -> bool {
        matches!(symbol_id, 220 | 205)  // 220=US500, 205=NAS100
    }

    /// Validate volume (matching Angular logic) 
    fn validate_volume(&self, symbol_id: i32, volume: f64) -> bool {
        let min_volume = self.get_minimum_volume(symbol_id);
        volume >= min_volume
    }

    /// Get minimum volume for symbol (matching Angular logic)
    fn get_minimum_volume(&self, symbol_id: i32) -> f64 {
        match symbol_id {
            220 => 1.0,    // US500
            205 => 1.0,    // NAS100
            _ => 0.01,     // Default for forex pairs
        }
    }
}