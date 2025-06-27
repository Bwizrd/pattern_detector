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
    pub status: String,            // "PENDING", "FILLED", "CLOSED", "CANCELLED"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filled_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filled_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,
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

// cTrader API response structures for fetching and cancelling orders
#[derive(Debug, Deserialize)]
struct CTraderPendingOrdersResponse {
    success: bool,
    orders: Vec<CTraderPendingOrder>,
    count: i32,
}

#[derive(Debug, Deserialize)]
struct CTraderPendingOrder {
    #[serde(rename = "orderId")]
    order_id: i64,
    #[serde(rename = "symbolId")]
    symbol_id: i32,
    volume: f64,
    #[serde(rename = "tradeSide")]
    trade_side: i32,
    #[serde(rename = "orderType")]
    order_type: i32,
    #[serde(rename = "limitPrice")]
    limit_price: f64,
    #[serde(rename = "stopLoss")]
    stop_loss: f64,
    #[serde(rename = "takeProfit")]
    take_profit: f64,
    #[serde(rename = "orderStatus")]
    order_status: i32,
    #[serde(rename = "executedVolume")]
    executed_volume: f64,
    #[serde(rename = "openTimestamp")]
    open_timestamp: i64,
    #[serde(rename = "lastUpdateTimestamp")]
    last_update_timestamp: i64,
    #[serde(rename = "clientOrderId")]
    client_order_id: String,
    #[serde(rename = "timeInForce")]
    time_in_force: i32,
}

#[derive(Debug, Deserialize)]
struct CancelOrderResponse {
    success: bool,
    message: String,
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
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
                debug!("📋 Zone {} already has a pending order, skipping", zone.id);
                continue;
            }

            // Check if we already have ANY pending order for this symbol/timeframe combination
            let symbol_timeframe_key = format!("{}_{}", zone.symbol, zone.timeframe);
            let has_blocking_order = self.pending_orders.values().any(|order| 
                format!("{}_{}", order.symbol, order.timeframe) == symbol_timeframe_key
            );
            
            if has_blocking_order {
                debug!("📋 Skipping zone {} - already have pending order for {}_{}", zone.id, zone.symbol, zone.timeframe);
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
        // Note: booked_orders are now managed by active_order_manager, not here
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
            
            // Don't add to booked_orders yet - only add when the order is actually FILLED
            // The active_order_manager will move orders from pending to booked when they fill
            
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

    /// Fetch all pending orders from cTrader API
    async fn fetch_ctrader_pending_orders(&self) -> Result<Vec<CTraderPendingOrder>, Box<dyn std::error::Error + Send + Sync>> {
        let api_url = format!("{}/pendingOrders", self.ctrader_api_url);
        let response = self.http_client
            .get(&api_url)
            .send()
            .await?;

        let response_text = response.text().await?;
        let api_response: CTraderPendingOrdersResponse = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse cTrader pending orders response: {} - Response: {}", e, response_text))?;

        if api_response.success {
            Ok(api_response.orders)
        } else {
            Err("Failed to fetch pending orders from cTrader".into())
        }
    }

    /// Cancel an order via cTrader API
    async fn cancel_ctrader_order(&self, order_id: i64) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let api_url = format!("{}/cancelOrder/{}", self.ctrader_api_url, order_id);
        
        info!("📋 Calling DELETE {}", api_url);
        
        let response = self.http_client
            .delete(&api_url)
            .send()
            .await?;

        let response_text = response.text().await?;
        
        // Try to parse the response
        let api_response: CancelOrderResponse = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse cancel order response: {} - Response: {}", e, response_text))?;

        if api_response.success {
            info!("✅ Successfully cancelled cTrader order {}", order_id);
            Ok(true)
        } else {
            warn!("❌ Failed to cancel cTrader order {}: {}", order_id, api_response.message);
            Ok(false)
        }
    }

    /// Check if order is old and moved away, then cancel it
    async fn should_cancel_order(&self, order: &CTraderPendingOrder, current_prices: &HashMap<String, f64>) -> (bool, String) {
        // Convert timestamp from milliseconds to seconds
        let order_timestamp = order.open_timestamp / 1000;
        let order_time = DateTime::from_timestamp(order_timestamp, 0)
            .unwrap_or_else(|| Utc::now());
        
        let now = Utc::now();
        let age_hours = now.signed_duration_since(order_time).num_hours();

        // Check if order is older than 12 hours
        if age_hours < 12 {
            return (false, format!("Order only {} hours old", age_hours));
        }

        // Find the symbol for this order
        let symbol = self.symbol_ids.iter()
            .find(|(_, &id)| id == order.symbol_id)
            .map(|(symbol, _)| symbol.clone())
            .unwrap_or_else(|| format!("UNKNOWN_{}", order.symbol_id));

        // Get current price for this symbol
        let current_price = match current_prices.get(&symbol).or_else(|| {
            // Try without _SB suffix
            let symbol_without_sb = symbol.replace("_SB", "");
            current_prices.get(&symbol_without_sb)
        }) {
            Some(&price) => price,
            None => {
                warn!("📋 No current price available for symbol {}, skipping cancel check", symbol);
                return (false, "No current price available".to_string());
            }
        };

        // Calculate distance from current price to order entry price
        let entry_price = order.limit_price;
        let pip_value = self.get_pip_value(&symbol);
        let distance_pips = (current_price - entry_price).abs() / pip_value;

        // Cancel if more than 50 pips away and older than 12 hours
        if distance_pips > 50.0 {
            (true, format!("{} hours old, {:.1} pips away from entry {:.5} (current: {:.5})", 
                          age_hours, distance_pips, entry_price, current_price))
        } else {
            (false, format!("{} hours old but only {:.1} pips away", age_hours, distance_pips))
        }
    }

    /// Periodic cleanup of old orders that have moved away
    pub async fn periodic_cleanup_old_orders(&mut self, current_prices: &HashMap<String, f64>) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(0);
        }

        info!("📋 Starting periodic cleanup of old pending orders...");

        // Fetch current orders from cTrader
        let ctrader_orders = match self.fetch_ctrader_pending_orders().await {
            Ok(orders) => orders,
            Err(e) => {
                error!("📋 Failed to fetch pending orders from cTrader: {}", e);
                return Err(e);
            }
        };

        let mut cancelled_count = 0;
        let mut orders_to_remove = Vec::new();

        for ctrader_order in ctrader_orders {
            let (should_cancel, reason) = self.should_cancel_order(&ctrader_order, current_prices).await;
            
            if should_cancel {
                info!("📋 Cancelling order {} - Reason: {}", ctrader_order.order_id, reason);
                
                match self.cancel_ctrader_order(ctrader_order.order_id).await {
                    Ok(true) => {
                        cancelled_count += 1;
                        
                        // Find and mark local order for removal
                        if let Some((zone_id, _)) = self.pending_orders.iter()
                            .find(|(_, order)| {
                                order.ctrader_order_id.as_ref()
                                    .map(|id| id.parse::<i64>().unwrap_or(-1) == ctrader_order.order_id)
                                    .unwrap_or(false)
                            }) {
                            orders_to_remove.push(zone_id.clone());
                        }
                    },
                    Ok(false) => {
                        warn!("📋 Failed to cancel order {}", ctrader_order.order_id);
                    },
                    Err(e) => {
                        error!("📋 Error cancelling order {}: {}", ctrader_order.order_id, e);
                    }
                }
            } else {
                debug!("📋 Order {} kept - Reason: {}", ctrader_order.order_id, reason);
            }
        }

        // Remove cancelled orders from local storage
        for zone_id in orders_to_remove {
            self.pending_orders.remove(&zone_id);
            self.booked_orders.remove(&zone_id);
            info!("📋 Removed cancelled order for zone {} from local storage", zone_id);
        }

        // Save updated orders
        if cancelled_count > 0 {
            if let Err(e) = self.save_pending_orders().await {
                error!("📋 Failed to save pending orders after cleanup: {}", e);
            }
            // Note: booked_orders are now managed by active_order_manager, not here
        }

        info!("📋 Periodic cleanup completed - Cancelled {} old orders", cancelled_count);
        Ok(cancelled_count)
    }

    /// Synchronize local pending orders with broker's actual pending orders
    pub async fn synchronize_pending_orders(&mut self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(0);
        }

        info!("📋 Synchronizing local pending orders with broker...");

        // Fetch current orders from cTrader
        let ctrader_orders = match self.fetch_ctrader_pending_orders().await {
            Ok(orders) => orders,
            Err(e) => {
                error!("📋 Failed to fetch pending orders from cTrader: {}", e);
                return Err(e);
            }
        };

        // Create a set of actual pending order IDs at the broker
        let broker_order_ids: std::collections::HashSet<i64> = ctrader_orders
            .iter()
            .map(|order| order.order_id)
            .collect();

        info!("📋 Broker has {} pending orders: {:?}", broker_order_ids.len(), broker_order_ids);
        info!("📋 Local storage has {} pending orders", self.pending_orders.len());

        // Find local orders that are no longer pending at the broker
        let mut stale_zone_ids = Vec::new();
        
        for (zone_id, local_order) in &self.pending_orders {
            if let Some(ctrader_order_id) = &local_order.ctrader_order_id {
                match ctrader_order_id.parse::<i64>() {
                    Ok(order_id) => {
                        if !broker_order_ids.contains(&order_id) {
                            info!("📋 Local order {} (cTrader ID: {}) no longer exists at broker - marking for removal", 
                                  zone_id, order_id);
                            stale_zone_ids.push(zone_id.clone());
                        } else {
                            debug!("📋 Order {} (cTrader ID: {}) confirmed at broker", zone_id, order_id);
                        }
                    },
                    Err(_) => {
                        warn!("📋 Invalid cTrader order ID format for zone {}: {}", zone_id, ctrader_order_id);
                        stale_zone_ids.push(zone_id.clone());
                    }
                }
            } else {
                warn!("📋 Local order {} has no cTrader order ID - marking for removal", zone_id);
                stale_zone_ids.push(zone_id.clone());
            }
        }

        // Remove stale orders from local storage
        let removed_count = stale_zone_ids.len();
        for zone_id in stale_zone_ids {
            if let Some(removed_order) = self.pending_orders.remove(&zone_id) {
                info!("📋 Removed stale pending order: {} {} {} (cTrader ID: {})", 
                      removed_order.symbol, 
                      removed_order.timeframe,
                      removed_order.order_type,
                      removed_order.ctrader_order_id.as_deref().unwrap_or("N/A"));
            }
            // Also remove from booked orders if it exists there
            self.booked_orders.remove(&zone_id);
        }

        // Save updated orders if any were removed
        if removed_count > 0 {
            if let Err(e) = self.save_pending_orders().await {
                error!("📋 Failed to save pending orders after sync: {}", e);
            }
            // Note: booked_orders are now managed by active_order_manager, not here
        }

        info!("📋 Synchronization completed - Removed {} stale orders, {} orders remain", 
              removed_count, self.pending_orders.len());
        Ok(removed_count)
    }

    /// One-off cleanup when blocked by same symbol+timeframe
    pub async fn cleanup_blocking_orders(&mut self, symbol: &str, timeframe: &str, current_prices: &HashMap<String, f64>) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(false);
        }

        info!("📋 Checking for blocking orders for {}_{}", symbol, timeframe);

        // Find existing order for this symbol+timeframe
        let blocking_zone_id = self.pending_orders.iter()
            .find(|(_, order)| order.symbol == symbol && order.timeframe == timeframe)
            .map(|(zone_id, _)| zone_id.clone());

        let blocking_zone_id = match blocking_zone_id {
            Some(id) => id,
            None => {
                debug!("📋 No blocking order found for {}_{}", symbol, timeframe);
                return Ok(false);
            }
        };

        let blocking_order = self.pending_orders.get(&blocking_zone_id).unwrap().clone();
        
        // Check if this order should be cancelled
        if let Some(ctrader_order_id) = &blocking_order.ctrader_order_id {
            let order_id = match ctrader_order_id.parse::<i64>() {
                Ok(id) => id,
                Err(_) => {
                    warn!("📋 Invalid order ID format: {}", ctrader_order_id);
                    return Ok(false);
                }
            };

            // Fetch this specific order from cTrader to get current details
            let ctrader_orders = self.fetch_ctrader_pending_orders().await?;
            let ctrader_order = ctrader_orders.iter()
                .find(|order| order.order_id == order_id);

            if let Some(ctrader_order) = ctrader_order {
                let (should_cancel, reason) = self.should_cancel_order(ctrader_order, current_prices).await;
                
                if should_cancel {
                    info!("📋 Cancelling blocking order {} for {}_{} - Reason: {}", order_id, symbol, timeframe, reason);
                    
                    match self.cancel_ctrader_order(order_id).await {
                        Ok(true) => {
                            // Remove from local storage
                            self.pending_orders.remove(&blocking_zone_id);
                            self.booked_orders.remove(&blocking_zone_id);
                            
                            // Save updated orders
                            if let Err(e) = self.save_pending_orders().await {
                                error!("📋 Failed to save pending orders after cleanup: {}", e);
                            }
                            // Note: booked_orders are now managed by active_order_manager, not here
                            
                            info!("✅ Successfully cancelled blocking order for {}_{}", symbol, timeframe);
                            return Ok(true);
                        },
                        Ok(false) => {
                            warn!("📋 Failed to cancel blocking order {}", order_id);
                        },
                        Err(e) => {
                            error!("📋 Error cancelling blocking order {}: {}", order_id, e);
                        }
                    }
                } else {
                    info!("📋 Blocking order {} for {}_{} is still valid - Reason: {}", order_id, symbol, timeframe, reason);
                }
            } else {
                warn!("📋 Blocking order {} not found in cTrader, removing from local storage", order_id);
                self.pending_orders.remove(&blocking_zone_id);
                self.booked_orders.remove(&blocking_zone_id);
                
                if let Err(e) = self.save_pending_orders().await {
                    error!("📋 Failed to save pending orders after cleanup: {}", e);
                }
                // Note: booked_orders are now managed by active_order_manager, not here
                
                return Ok(true);
            }
        }

        Ok(false)
    }
}
