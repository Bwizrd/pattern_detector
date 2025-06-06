// src/bin/zone_monitor/trading_engine.rs
// Trading execution system for zone-based signals

use crate::types::{PriceUpdate, ZoneAlert};
use chrono::{Datelike, Timelike};
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
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            lot_size: 0.01,
            stop_loss_pips: 20.0,
            take_profit_pips: 40.0,
            max_daily_trades: 10,
            max_trades_per_symbol: 2,
            allowed_symbols: vec!["EURUSD".to_string(), "GBPUSD".to_string()],
            allowed_timeframes: vec!["30m".to_string(), "1h".to_string(), "4h".to_string()],
            min_risk_reward: 1.5,
            trading_start_hour: 8,
            trading_end_hour: 17,
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

#[derive(Debug)]
pub struct TradingEngine {
    config: Arc<RwLock<TradingConfig>>,
    active_trades: Arc<RwLock<HashMap<String, Trade>>>,
    trade_history: Arc<RwLock<Vec<Trade>>>,
    stats: Arc<RwLock<TradingStats>>,
    last_reset_day: Arc<RwLock<u32>>,
}

impl Clone for TradingEngine {
    fn clone(&self) -> Self {
        Self {
            config: Arc::clone(&self.config),
            active_trades: Arc::clone(&self.active_trades),
            trade_history: Arc::clone(&self.trade_history),
            stats: Arc::clone(&self.stats),
            last_reset_day: Arc::clone(&self.last_reset_day),
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
                .unwrap_or_else(|_| "0.01".to_string())
                .parse()
                .unwrap_or(0.01),
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
                .unwrap_or_else(|_| "EURUSD,GBPUSD,AUDUSD,USDCAD".to_string())
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
        };

        info!("💰 Trading Engine initialized:");
        info!("   Enabled: {}", config.enabled);
        info!("   Lot size: {:.2}", config.lot_size);
        info!("   Stop loss: {:.1} pips", config.stop_loss_pips);
        info!("   Take profit: {:.1} pips", config.take_profit_pips);
        info!("   Max daily trades: {}", config.max_daily_trades);
        info!("   Allowed symbols: {:?}", config.allowed_symbols);
        info!("   Allowed timeframes: {:?}", config.allowed_timeframes);

        Self {
            config: Arc::new(RwLock::new(config)),
            active_trades: Arc::new(RwLock::new(HashMap::new())),
            trade_history: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(TradingStats::default())),
            last_reset_day: Arc::new(RwLock::new(chrono::Utc::now().day())),
        }
    }

    // Main function to evaluate and execute trades
    pub async fn evaluate_and_execute_trade(
        &self,
        signal: &ZoneAlert,
        current_price_data: &PriceUpdate,
    ) -> Option<Trade> {
        let config = self.config.read().await;

        if !config.enabled {
            debug!("💰 Trading disabled, skipping signal");
            return None;
        }

        // Reset daily counter if new day
        self.reset_daily_counter_if_needed().await;

        // Check trading constraints
        if !self.can_trade(&signal, &config).await {
            return None;
        }

        // Generate trade from signal
        let trade = self
            .generate_trade_from_signal(signal, current_price_data, &config)
            .await?;

        // Execute the trade
        match self.execute_trade(trade).await {
            Ok(executed_trade) => {
                info!("✅ Trade executed successfully: {}", executed_trade.id);
                Some(executed_trade)
            }
            Err(e) => {
                error!("❌ Failed to execute trade: {}", e);
                None
            }
        }
    }

    // Check if we can trade based on constraints
    async fn can_trade(&self, signal: &ZoneAlert, config: &TradingConfig) -> bool {
        // Check symbol allowlist
        if !config.allowed_symbols.contains(&signal.symbol) {
            info!("🚫 Symbol {} not in allowed list", signal.symbol);
            return false;
        }

        // Check timeframe allowlist (when available)
        // For now, assume all signals are valid since timeframe is "unknown"

        // Check trading hours
        let now = chrono::Utc::now();
        let hour = now.hour() as u8;
        if hour < config.trading_start_hour || hour > config.trading_end_hour {
            info!(
                "🚫 Outside trading hours: {} (allowed: {}:00-{}:00)",
                hour, config.trading_start_hour, config.trading_end_hour
            );
            return false;
        }

        // Check daily limits
        let stats = self.stats.read().await;
        if stats.daily_trades >= config.max_daily_trades {
            info!(
                "🚫 Daily trade limit reached: {}/{}",
                stats.daily_trades, config.max_daily_trades
            );
            return false;
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
            return false;
        }

        true
    }

    // Generate trade from signal
    async fn generate_trade_from_signal(
        &self,
        signal: &ZoneAlert,
        price_data: &PriceUpdate,
        config: &TradingConfig,
    ) -> Option<Trade> {
        let current_price = (price_data.bid + price_data.ask) / 2.0;

        // Determine trade direction based on zone type
        let (trade_type, entry_price, stop_loss, take_profit) = match signal.zone_type.as_str() {
            "demand_zone" => {
                // Price at demand zone - expect bounce up (BUY)
                let entry = current_price;
                let sl = entry - (config.stop_loss_pips / 10000.0); // Fixed 20 pips below entry
                let tp = entry + (config.take_profit_pips / 10000.0); // Fixed 40 pips above entry
                ("buy".to_string(), entry, sl, tp)
            }
            "supply_zone" => {
                // Price at supply zone - expect bounce down (SELL)
                let entry = current_price;
                let sl = entry + (config.stop_loss_pips / 10000.0); // Fixed 20 pips above entry
                let tp = entry - (config.take_profit_pips / 10000.0); // Fixed 40 pips below entry
                ("sell".to_string(), entry, sl, tp)
            }
            _ => {
                warn!("🚫 Unknown zone type: {}", signal.zone_type);
                return None;
            }
        };

        // Calculate risk/reward ratio
        // Calculate risk/reward ratio (should always be 2.0 now)
        let risk = (entry_price - stop_loss).abs() * 10000.0;
        let reward = (take_profit - entry_price).abs() * 10000.0;
        let risk_reward_ratio = if risk > 0.0 { reward / risk } else { 0.0 };

        if risk_reward_ratio < config.min_risk_reward {
            info!(
                "🚫 Risk/reward too low: {:.2} < {:.2}",
                risk_reward_ratio, config.min_risk_reward
            );
            return None;
        }

        Some(Trade {
            id: Uuid::new_v4().to_string(),
            zone_id: signal.zone_id.clone(),
            symbol: signal.symbol.clone(),
            trade_type,
            entry_price,
            stop_loss,
            take_profit,
            lot_size: config.lot_size,
            status: "pending".to_string(),
            opened_at: chrono::Utc::now(),
            closed_at: None,
            profit_loss: None,
            risk_reward_ratio,
            timeframe: "unknown".to_string(), // Will be updated when zone JSON includes timeframes
        })
    }

    // Execute trade (simulate for now)
    async fn execute_trade(&self, mut trade: Trade) -> Result<Trade, String> {
        info!(
            "🎯 Executing {} trade: {} @ {:.5} (SL: {:.5}, TP: {:.5})",
            trade.trade_type.to_uppercase(),
            trade.symbol,
            trade.entry_price,
            trade.stop_loss,
            trade.take_profit
        );

        // Simulate trade execution
        trade.status = "open".to_string();

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
            "✅ Trade opened: {} {} {:.2} lots @ {:.5}",
            trade.symbol,
            trade.trade_type.to_uppercase(),
            trade.lot_size,
            trade.entry_price
        );

        Ok(trade)
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

    // Close trade manually (for testing or SL/TP hit)
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

        // Simple P&L calculation (not accounting for lot size multiplier)
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
