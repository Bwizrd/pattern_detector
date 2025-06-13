// src/bin/zone_monitor/trading_engine.rs
// Trading execution system for zone-based signals with real API integration

use crate::types::{PriceUpdate, ZoneAlert};
use chrono::{Datelike, Timelike};
use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Trade {
    pub id: String,
    pub zone_id: String,
    pub symbol: String,
    pub trade_type: String, // "buy" or "sell"
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub lot_size: f64,
    pub status: String, // "pending", "open", "closed", "cancelled"
    pub opened_at: chrono::DateTime<chrono::Utc>,
    pub closed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub profit_loss: Option<f64>,
    pub risk_reward_ratio: f64,
    pub timeframe: String,
    pub ctrader_order_id: Option<String>,

}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TradingConfig {
    pub enabled: bool,
    pub lot_size: f64,
    pub stop_loss_pips: f64,
    pub take_profit_pips: f64,
    pub max_daily_trades: i32,
    pub max_trades_per_symbol: i32,
    pub allowed_symbols: Vec<String>,
    pub allowed_timeframes: Vec<String>,
    pub min_risk_reward: f64,
    pub trading_start_hour: u8,
    pub trading_end_hour: u8,
    pub api_bridge_url: String,
}

#[derive(Debug)]
pub struct TradeEvaluationResult {
    pub can_trade: bool,
    pub rejection_reason: Option<String>,
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            lot_size: 1000.0,
            stop_loss_pips: 20.0,
            take_profit_pips: 40.0,
            max_daily_trades: 10,
            max_trades_per_symbol: 2,
            allowed_symbols: vec!["EURUSD".to_string(), "GBPUSD".to_string()],
            allowed_timeframes: vec!["30m".to_string(), "1h".to_string(), "4h".to_string()],
            min_risk_reward: 1.5,
            trading_start_hour: 8,
            trading_end_hour: 17,
            api_bridge_url: "http://localhost:8000".to_string(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TradingStats {
    pub total_trades: i32,
    pub open_trades: i32,
    pub closed_trades: i32,
    pub winning_trades: i32,
    pub losing_trades: i32,
    pub total_profit_loss: f64,
    pub win_rate: f64,
    pub daily_trades: i32,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

impl Default for TradingStats {
    fn default() -> Self {
        Self {
            total_trades: 0,
            open_trades: 0,
            closed_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            total_profit_loss: 0.0,
            win_rate: 0.0,
            daily_trades: 0,
            last_updated: chrono::Utc::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeResult {
    pub trade: Option<Trade>,
    pub success: bool,
    pub rejection_reason: Option<String>,
    pub execution_result: String, // "success", "rejected", "failed"
}

impl TradeResult {
    pub fn success(trade: Trade) -> Self {
        Self {
            trade: Some(trade),
            success: true,
            rejection_reason: None,
            execution_result: "success".to_string(),
        }
    }

    pub fn rejected(reason: String) -> Self {
        Self {
            trade: None,
            success: false,
            rejection_reason: Some(reason),
            execution_result: "rejected".to_string(),
        }
    }

    pub fn failed(reason: String) -> Self {
        Self {
            trade: None,
            success: false,
            rejection_reason: Some(reason),
            execution_result: "failed".to_string(),
        }
    }
}

// API Request/Response structures
#[derive(Debug, Serialize)]
struct PlaceOrderRequest {
    #[serde(rename = "symbolId")]
    symbol_id: u32,
    #[serde(rename = "tradeSide")]
    trade_side: u32, // 1 for buy, 2 for sell
    volume: f64,
    #[serde(rename = "stopLoss")]
    stop_loss: f64,
    #[serde(rename = "takeProfit")]
    take_profit: f64,
}

#[derive(Debug, Deserialize)]
struct PlaceOrderResponse {
    success: bool,
    #[serde(rename = "orderId")]
    order_id: Option<String>,
    message: Option<String>,
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
}

#[derive(Debug)]
pub struct TradingEngine {
    config: Arc<RwLock<TradingConfig>>,
    active_trades: Arc<RwLock<HashMap<String, Trade>>>,
    trade_history: Arc<RwLock<Vec<Trade>>>,
    stats: Arc<RwLock<TradingStats>>,
    last_reset_day: Arc<RwLock<u32>>,
    http_client: reqwest::Client,
    symbol_mappings: HashMap<String, u32>,
}

impl Clone for TradingEngine {
    fn clone(&self) -> Self {
        Self {
            config: Arc::clone(&self.config),
            active_trades: Arc::clone(&self.active_trades),
            trade_history: Arc::clone(&self.trade_history),
            stats: Arc::clone(&self.stats),
            last_reset_day: Arc::clone(&self.last_reset_day),
            http_client: self.http_client.clone(),
            symbol_mappings: self.symbol_mappings.clone(),
        }
    }
}

impl TradingEngine {
    pub fn new() -> Self {
        let config = TradingConfig {
            enabled: std::env::var("TRADING_ENABLED")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            lot_size: std::env::var("TRADING_LOT_SIZE")
                .unwrap_or_else(|_| "1000".to_string())
                .parse()
                .unwrap_or(1000.0),
            stop_loss_pips: std::env::var("TRADING_STOP_LOSS_PIPS")
                .unwrap_or_else(|_| "20.0".to_string())
                .parse()
                .unwrap_or(20.0),
            take_profit_pips: std::env::var("TRADING_TAKE_PROFIT_PIPS")
                .unwrap_or_else(|_| "40.0".to_string())
                .parse()
                .unwrap_or(40.0),
            max_daily_trades: std::env::var("TRADING_MAX_DAILY_TRADES")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .unwrap_or(10),
            max_trades_per_symbol: std::env::var("TRADING_MAX_TRADES_PER_SYMBOL")
                .unwrap_or_else(|_| "2".to_string())
                .parse()
                .unwrap_or(2),
            allowed_symbols: std::env::var("TRADING_ALLOWED_SYMBOLS")
                .unwrap_or_else(|_| "EURUSD,GBPUSD,USDJPY,USDCHF,AUDUSD,USDCAD,NZDUSD,EURGBP,EURJPY,EURCHF,EURAUD,EURCAD,EURNZD,GBPJPY,GBPCHF,GBPAUD,GBPCAD,GBPNZD,AUDJPY,AUDNZD,AUDCAD,NZDJPY,CADJPY,CHFJPY".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            allowed_timeframes: std::env::var("TRADING_ALLOWED_TIMEFRAMES")
                .unwrap_or_else(|_| "30m,1h,4h".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            min_risk_reward: std::env::var("TRADING_MIN_RISK_REWARD")
                .unwrap_or_else(|_| "1.5".to_string())
                .parse()
                .unwrap_or(1.5),
            trading_start_hour: std::env::var("TRADING_START_HOUR")
                .unwrap_or_else(|_| "8".to_string())
                .parse()
                .unwrap_or(8),
            trading_end_hour: std::env::var("TRADING_END_HOUR")
                .unwrap_or_else(|_| "17".to_string())
                .parse()
                .unwrap_or(17),
            api_bridge_url: std::env::var("CTRADER_API_BRIDGE_URL")
                .unwrap_or_else(|_| "http://localhost:8000".to_string()),
        };

        // Your symbol mappings
        let mut symbol_mappings = HashMap::new();
        symbol_mappings.insert("EURUSD_SB".to_string(), 185);
        symbol_mappings.insert("GBPUSD_SB".to_string(), 199);
        symbol_mappings.insert("USDJPY_SB".to_string(), 226);
        symbol_mappings.insert("USDCHF_SB".to_string(), 222);
        symbol_mappings.insert("AUDUSD_SB".to_string(), 158);
        symbol_mappings.insert("USDCAD_SB".to_string(), 221);
        symbol_mappings.insert("NZDUSD_SB".to_string(), 211);
        symbol_mappings.insert("EURGBP_SB".to_string(), 175);
        symbol_mappings.insert("EURJPY_SB".to_string(), 177);
        symbol_mappings.insert("EURCHF_SB".to_string(), 173);
        symbol_mappings.insert("EURAUD_SB".to_string(), 171);
        symbol_mappings.insert("EURCAD_SB".to_string(), 172);
        symbol_mappings.insert("EURNZD_SB".to_string(), 180);
        symbol_mappings.insert("GBPJPY_SB".to_string(), 192);
        symbol_mappings.insert("GBPCHF_SB".to_string(), 191);
        symbol_mappings.insert("GBPAUD_SB".to_string(), 189);
        symbol_mappings.insert("GBPCAD_SB".to_string(), 190);
        symbol_mappings.insert("GBPNZD_SB".to_string(), 195);
        symbol_mappings.insert("AUDJPY_SB".to_string(), 155);
        symbol_mappings.insert("AUDNZD_SB".to_string(), 156);
        symbol_mappings.insert("AUDCAD_SB".to_string(), 153);
        symbol_mappings.insert("NZDJPY_SB".to_string(), 210);
        symbol_mappings.insert("CADJPY_SB".to_string(), 162);
        symbol_mappings.insert("CHFJPY_SB".to_string(), 163);
        symbol_mappings.insert("NAS100_SB".to_string(), 205);
        symbol_mappings.insert("US500_SB".to_string(), 220);

        info!("💰 Trading Engine initialized:");
        info!("   Enabled: {}", config.enabled);
        info!("   Lot size: {:.0}", config.lot_size);
        info!("   Stop loss: {:.1} pips", config.stop_loss_pips);
        info!("   Take profit: {:.1} pips", config.take_profit_pips);
        info!("   Max daily trades: {}", config.max_daily_trades);
        info!("   API Bridge: {}", config.api_bridge_url);
        info!("   Symbols available: {}", symbol_mappings.len());

        Self {
            config: Arc::new(RwLock::new(config)),
            active_trades: Arc::new(RwLock::new(HashMap::new())),
            trade_history: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(TradingStats::default())),
            last_reset_day: Arc::new(RwLock::new(chrono::Utc::now().day())),
            http_client: reqwest::Client::new(),
            symbol_mappings,
        }
    }

    // Calculate lot size based on symbol type
    fn get_lot_size_for_symbol(&self, base_symbol: &str, default_lot_size: f64) -> f64 {
        if base_symbol.contains("JPY") {
            std::env::var("TRADING_JPY_LOT_SIZE")
                .unwrap_or_else(|_| "100".to_string())
                .parse()
                .unwrap_or(100.0)
        } else if base_symbol.contains("NAS100") || base_symbol.contains("US500") {
            std::env::var("TRADING_INDEX_LOT_SIZE")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .unwrap_or(10.0)
        } else {
            default_lot_size
        }
    }

    // Main function to evaluate and execute trades
    pub async fn evaluate_and_execute_trade(
        &self,
        signal: &ZoneAlert,
        current_price_data: &PriceUpdate,
    ) -> TradeResult {
        let config = self.config.read().await;

        if !config.enabled {
            debug!("💰 Trading disabled, skipping signal");
            return TradeResult::rejected("trading_disabled".to_string());
        }

        // Reset daily counter if new day
        self.reset_daily_counter_if_needed().await;

        // Check trading constraints with detailed reasons
        let evaluation = self.can_trade_with_reason(&signal, &config).await;
        if !evaluation.can_trade {
            return TradeResult::rejected(
                evaluation
                    .rejection_reason
                    .unwrap_or_else(|| "unknown_constraint".to_string()),
            );
        }

        // Generate trade from signal - now with better error handling
        let trade = match self
            .generate_trade_from_signal(signal, current_price_data, &config)
            .await
        {
            Ok(trade) => trade,
            Err(e) => {
                return TradeResult::rejected(e);
            }
        };

        drop(config);

        // Execute the trade via API
        match self.execute_trade_via_api(trade).await {
            Ok(executed_trade) => {
                info!(
                    "✅ Trade executed successfully: {} (Order ID: {:?})",
                    executed_trade.id, executed_trade.ctrader_order_id
                );
                TradeResult::success(executed_trade)
            }
            Err(e) => {
                error!("❌ Failed to execute trade: {}", e);

                // Categorize the error for better rejection reasons
                let rejection_reason = if e.contains("Error connecting to trading service") {
                    "api_connection_error".to_string()
                } else if e.contains("timeout") {
                    "api_timeout".to_string()
                } else if e.contains("UNKNOWN_ERROR") {
                    "api_unknown_error".to_string()
                } else if e.contains("insufficient") {
                    "insufficient_funds".to_string()
                } else if e.contains("market closed") || e.contains("trading hours") {
                    "market_closed".to_string()
                } else {
                    format!("api_error_{}", e.replace(" ", "_").to_lowercase())
                };

                TradeResult::failed(rejection_reason)
            }
        }
    }
    // Check if we can trade based on constraints
    async fn can_trade_with_reason(
        &self,
        signal: &ZoneAlert,
        config: &TradingConfig,
    ) -> TradeEvaluationResult {
        // Check symbol allowlist (signal comes without _SB, config has without _SB)
        if !config.allowed_symbols.contains(&signal.symbol) {
            info!("🚫 Symbol {} not in allowed list", signal.symbol);
            return TradeEvaluationResult {
                can_trade: false,
                rejection_reason: Some(format!("symbol_not_allowed_{}", signal.symbol)),
            };
        }

        // Check symbol mapping (need _SB version for mapping)
        let sb_symbol = format!("{}_SB", signal.symbol);
        if !self.symbol_mappings.contains_key(&sb_symbol) {
            warn!("🚫 No symbol mapping found for {}", sb_symbol);
            return TradeEvaluationResult {
                can_trade: false,
                rejection_reason: Some(format!("symbol_mapping_missing_{}", signal.symbol)),
            };
        }

        // Check trading hours
        let now = chrono::Utc::now();
        let hour = now.hour() as u8;
        if hour < config.trading_start_hour || hour > config.trading_end_hour {
            info!(
                "🚫 Outside trading hours: {} (allowed: {}:00-{}:00)",
                hour, config.trading_start_hour, config.trading_end_hour
            );
            return TradeEvaluationResult {
                can_trade: false,
                rejection_reason: Some(format!("outside_trading_hours_{}h", hour)),
            };
        }

        // Check daily limits
        let stats = self.stats.read().await;
        if stats.daily_trades >= config.max_daily_trades {
            info!(
                "🚫 Daily trade limit reached: {}/{}",
                stats.daily_trades, config.max_daily_trades
            );
            return TradeEvaluationResult {
                can_trade: false,
                rejection_reason: Some(format!(
                    "daily_limit_reached_{}_of_{}",
                    stats.daily_trades, config.max_daily_trades
                )),
            };
        }

        // Check per-symbol limits
        let active_trades = self.active_trades.read().await;
        let symbol_count = active_trades
            .values()
            .filter(|t| t.symbol == signal.symbol && t.status == "open")
            .count();

        if symbol_count as i32 >= config.max_trades_per_symbol {
            info!(
                "🚫 Max trades per symbol reached for {}: {}/{}",
                signal.symbol, symbol_count, config.max_trades_per_symbol
            );
            return TradeEvaluationResult {
                can_trade: false,
                rejection_reason: Some(format!(
                    "symbol_limit_reached_{}_{}_of_{}",
                    signal.symbol, symbol_count, config.max_trades_per_symbol
                )),
            };
        }

        TradeEvaluationResult {
            can_trade: true,
            rejection_reason: None,
        }
    }

    // Generate trade from signal
    async fn generate_trade_from_signal(
        &self,
        signal: &ZoneAlert,
        price_data: &PriceUpdate,
        config: &TradingConfig,
    ) -> Result<Trade, String> {
        let current_price = (price_data.bid + price_data.ask) / 2.0;
        let sb_symbol = format!("{}_SB", signal.symbol);
        let symbol_id = *self
            .symbol_mappings
            .get(&sb_symbol)
            .ok_or_else(|| format!("symbol_mapping_not_found_{}", signal.symbol))?;

        // Get pip value for the symbol
        let pip_value = if signal.symbol.contains("JPY") {
            0.01
        } else {
            0.0001
        };

        // Determine trade direction based on zone type
        // Send positive pip distances - TypeScript bridge will convert to relative values
        let (trade_type, entry_price, stop_loss_distance, take_profit_distance) = match signal.zone_type.as_str() {
            "demand_zone" => {
                let entry = current_price;
                // For BUY: Send positive pip distances
                let sl_distance = config.stop_loss_pips * pip_value;  // Always positive
                let tp_distance = config.take_profit_pips * pip_value; // Always positive
                ("buy".to_string(), entry, sl_distance, tp_distance)
            }
            "supply_zone" => {
                let entry = current_price;
                // For SELL: Send positive pip distances
                let sl_distance = config.stop_loss_pips * pip_value;  // Always positive
                let tp_distance = config.take_profit_pips * pip_value; // Always positive
                ("sell".to_string(), entry, sl_distance, tp_distance)
            }
            _ => {
                return Err(format!("unknown_zone_type_{}", signal.zone_type));
            }
        };

        // Calculate lot size for this symbol
        let lot_size = self.get_lot_size_for_symbol(&signal.symbol, config.lot_size);

        // Calculate risk/reward ratio
        let risk_pips = config.stop_loss_pips;
        let reward_pips = config.take_profit_pips;
        let risk_reward_ratio = reward_pips / risk_pips;

        if risk_reward_ratio < config.min_risk_reward {
            return Err(format!(
                "risk_reward_too_low_{:.2}_min_{:.2}",
                risk_reward_ratio, config.min_risk_reward
            ));
        }

        Ok(Trade {
            id: Uuid::new_v4().to_string(),
            zone_id: signal.zone_id.clone(),
            symbol: signal.symbol.clone(),
            trade_type,
            entry_price,
            stop_loss: stop_loss_distance,
            take_profit: take_profit_distance,
            lot_size,
            status: "pending".to_string(),
            opened_at: chrono::Utc::now(),
            closed_at: None,
            profit_loss: None,
            risk_reward_ratio,
            timeframe: "unknown".to_string(),
            ctrader_order_id: None,
        })
    }

    // Execute trade via cTrader API
    async fn execute_trade_via_api(&self, mut trade: Trade) -> Result<Trade, String> {
        let config = self.config.read().await;

        info!(
            "🚀 Executing {} trade: {} @ {:.5} (SL distance: {:.5}, TP distance: {:.5})",
            trade.trade_type.to_uppercase(),
            trade.symbol,
            trade.entry_price,
            trade.stop_loss,
            trade.take_profit
        );

        let trade_side = match trade.trade_type.as_str() {
            "buy" => 1u32,
            "sell" => 2u32,
            _ => return Err("Invalid trade type".to_string()),
        };

        let sb_symbol = format!("{}_SB", trade.symbol);
        let symbol_id = *self
            .symbol_mappings
            .get(&sb_symbol)
            .ok_or("Symbol mapping not found")?;

        let request = PlaceOrderRequest {
            symbol_id,
            trade_side,
            volume: trade.lot_size,
            stop_loss: trade.stop_loss,
            take_profit: trade.take_profit,
        };

        let url = format!("{}/placeOrder", config.api_bridge_url);

        info!(
            "🌐 API call: {} with symbol_id={}, trade_side={}, volume={}, SL_distance={:.5}, TP_distance={:.5}",
            url,
            request.symbol_id,
            request.trade_side,
            request.volume,
            request.stop_loss,
            request.take_profit
        );

        drop(config);

        // Make API call
        let response = self
            .http_client
            .post(&url)
            .json(&request)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("API request failed: {}", e))?;

        let response_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        info!("📄 API response: {}", response_text);

        let api_response: PlaceOrderResponse = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if api_response.success {
            trade.status = "open".to_string();
            trade.ctrader_order_id = api_response.order_id;

            // Store in active trades
            {
                let mut active_trades = self.active_trades.write().await;
                active_trades.insert(trade.id.clone(), trade.clone());
            }

            // Update stats
            {
                let mut stats = self.stats.write().await;
                stats.total_trades += 1;
                stats.open_trades += 1;
                stats.daily_trades += 1;
                stats.last_updated = chrono::Utc::now();
            }

            info!(
                "✅ Trade opened: {} {} {:.0} lots @ {:.5} (Order ID: {:?})",
                trade.symbol,
                trade.trade_type.to_uppercase(),
                trade.lot_size,
                trade.entry_price,
                trade.ctrader_order_id
            );

            Ok(trade)
        } else {
            let error_msg = api_response
                .message
                .or(api_response.error_code)
                .unwrap_or_else(|| "Unknown API error".to_string());
            Err(error_msg)
        }
    }

    // Reset daily counter if new day
    async fn reset_daily_counter_if_needed(&self) {
        let current_day = chrono::Utc::now().day();
        let mut last_reset_day = self.last_reset_day.write().await;

        if current_day != *last_reset_day {
            let mut stats = self.stats.write().await;
            stats.daily_trades = 0;
            *last_reset_day = current_day;
            info!("🔄 Daily trade counter reset for new day");
        }
    }

    // Getter methods for monitoring
    pub async fn get_trading_stats(&self) -> TradingStats {
        self.stats.read().await.clone()
    }

    pub async fn get_active_trades(&self) -> HashMap<String, Trade> {
        self.active_trades.read().await.clone()
    }

    pub async fn get_trade_history(&self) -> Vec<Trade> {
        self.trade_history.read().await.clone()
    }

    pub async fn get_config(&self) -> TradingConfig {
        self.config.read().await.clone()
    }

    // Close trade manually
    pub async fn close_trade(
        &self,
        trade_id: &str,
        close_price: f64,
        reason: &str,
    ) -> Result<Trade, String> {
        let mut trade = {
            let mut active_trades = self.active_trades.write().await;
            active_trades.remove(trade_id).ok_or("Trade not found")?
        };

        // Calculate P&L
        let price_diff = match trade.trade_type.as_str() {
            "buy" => close_price - trade.entry_price,
            "sell" => trade.entry_price - close_price,
            _ => 0.0,
        };

        let profit_loss = price_diff * 10000.0; // Convert to pips

        trade.status = "closed".to_string();
        trade.closed_at = Some(chrono::Utc::now());
        trade.profit_loss = Some(profit_loss);

        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.open_trades = stats.open_trades.saturating_sub(1);
            stats.closed_trades += 1;
            stats.total_profit_loss += profit_loss;

            if profit_loss > 0.0 {
                stats.winning_trades += 1;
            } else {
                stats.losing_trades += 1;
            }

            if stats.closed_trades > 0 {
                stats.win_rate = (stats.winning_trades as f64 / stats.closed_trades as f64) * 100.0;
            }

            stats.last_updated = chrono::Utc::now();
        }

        // Store in history
        {
            let mut history = self.trade_history.write().await;
            history.push(trade.clone());
        }

        info!(
            "💰 Trade closed: {} P&L: {:.1} pips ({})",
            trade.symbol, profit_loss, reason
        );

        Ok(trade)
    }
}
