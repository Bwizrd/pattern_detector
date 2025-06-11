// src/trading/backtest/backtest_api.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize}; // For analytics

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MultiBacktestRequest {
    pub start_time: String,
    pub end_time: String,
    pub symbols: Vec<String>,
    pub pattern_timeframes: Vec<String>,
    
    // Trading Parameters (optional - fall back to env vars)
    pub stop_loss_pips: Option<f64>,
    pub take_profit_pips: Option<f64>,
    pub lot_size: Option<f64>,
    
    // Trading Rules (optional - fall back to env vars)
    pub max_daily_trades: Option<u32>,
    pub max_trades_per_symbol: Option<u32>,
    pub max_trades_per_zone: Option<usize>,
    pub min_zone_strength: Option<f64>,
    pub max_touch_count_for_trading: Option<i64>,
    
    // Time/Day Filtering (optional - fall back to env vars)
    pub allowed_trade_days: Option<Vec<String>>,
    pub allowed_symbols: Option<Vec<String>>,
    pub allowed_timeframes: Option<Vec<String>>,
    pub trade_start_hour_utc: Option<u32>,
    pub trade_end_hour_utc: Option<u32>,

    // Rejected trades tracking
    pub show_rejected_trades: Option<bool>,
}

impl MultiBacktestRequest {
    /// Get stop loss pips with environment variable fallback
    pub fn get_stop_loss_pips(&self) -> f64 {
        self.stop_loss_pips.unwrap_or_else(|| {
            std::env::var("TRADING_STOP_LOSS_PIPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(20.0)
        })
    }
    
    /// Get take profit pips with environment variable fallback
    pub fn get_take_profit_pips(&self) -> f64 {
        self.take_profit_pips.unwrap_or_else(|| {
            std::env::var("TRADING_TAKE_PROFIT_PIPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10.0)
        })
    }
    
    /// Get lot size with environment variable fallback
    pub fn get_lot_size(&self) -> f64 {
        self.lot_size.unwrap_or_else(|| {
            std::env::var("TRADING_LOT_SIZE")
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|size| size / 1000.0) // Convert from 1000 to 1.0
                .unwrap_or(0.01)
        })
    }
    
    /// Get max daily trades with environment variable fallback
    pub fn get_max_daily_trades(&self) -> u32 {
        self.max_daily_trades.unwrap_or_else(|| {
            std::env::var("TRADING_MAX_DAILY_TRADES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10)
        })
    }
    
    /// Get max trades per symbol with environment variable fallback
    pub fn get_max_trades_per_symbol(&self) -> u32 {
        self.max_trades_per_symbol.unwrap_or_else(|| {
            std::env::var("TRADING_MAX_TRADES_PER_SYMBOL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(2)
        })
    }
    
    /// Get max trades per zone with environment variable fallback
    pub fn get_max_trades_per_zone(&self) -> usize {
        self.max_trades_per_zone.unwrap_or_else(|| {
            std::env::var("TRADING_MAX_TRADES_PER_ZONE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3)
        })
    }
    
    /// Get min zone strength with environment variable fallback
    pub fn get_min_zone_strength(&self) -> f64 {
        self.min_zone_strength.unwrap_or_else(|| {
            std::env::var("TRADING_MIN_ZONE_STRENGTH")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(100.0)
        })
    }
    
    /// Get max touch count with environment variable fallback
    pub fn get_max_touch_count_for_trading(&self) -> i64 {
        self.max_touch_count_for_trading.unwrap_or_else(|| {
            std::env::var("MAX_TOUCH_COUNT_FOR_TRADING")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(4)
        })
    }

    
    /// Get allowed symbols with environment variable fallback
    pub fn get_allowed_symbols(&self) -> Option<Vec<String>> {
        self.allowed_symbols.clone().or_else(|| {
            std::env::var("TRADING_ALLOWED_SYMBOLS")
                .ok()
                .map(|s| s.split(',').map(|sym| sym.trim().to_string()).collect())
        })
    }
    
    /// Get allowed timeframes with environment variable fallback
    pub fn get_allowed_timeframes(&self) -> Option<Vec<String>> {
        self.allowed_timeframes.clone().or_else(|| {
            std::env::var("TRADING_ALLOWED_TIMEFRAMES")
                .ok()
                .map(|s| s.split(',').map(|tf| tf.trim().to_string()).collect())
        })
    }
    
    /// Get allowed trade days with environment variable fallback
    pub fn get_allowed_trade_days(&self) -> Option<Vec<String>> {
        self.allowed_trade_days.clone().or_else(|| {
            std::env::var("TRADING_ALLOWED_WEEKDAYS")
                .ok()
                .map(|s| s.split(',').map(|day| day.trim().to_string()).collect())
        })
    }
    
    /// Get trade start hour with environment variable fallback
    pub fn get_trade_start_hour_utc(&self) -> Option<u32> {
        self.trade_start_hour_utc.or_else(|| {
            std::env::var("TRADING_START_HOUR")
                .ok()
                .and_then(|s| s.parse().ok())
        })
    }
    
    /// Get trade end hour with environment variable fallback
    pub fn get_trade_end_hour_utc(&self) -> Option<u32> {
        self.trade_end_hour_utc.or_else(|| {
            std::env::var("TRADING_END_HOUR")
                .ok()
                .and_then(|s| s.parse().ok())
        })
    }

    /// Get show rejected trades with default fallback
    pub fn get_show_rejected_trades(&self) -> bool {
        self.show_rejected_trades.unwrap_or(false)
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct IndividualTradeResult {
    pub symbol: String,
    pub timeframe: String, // The timeframe the pattern was detected on
    pub zone_id: Option<String>,
    pub direction: String, // "Long" or "Short"
    pub entry_time: DateTime<Utc>,
    pub entry_price: f64,
    pub exit_time: Option<DateTime<Utc>>,
    pub exit_price: Option<f64>,
    pub pnl_pips: Option<f64>,
    pub exit_reason: Option<String>, // "Take Profit", "Stop Loss", "End of Data"
    pub zone_strength: Option<f64>,
    pub touch_count: Option<i32>,
    // For analytics
    pub entry_day_of_week: Option<String>, // e.g., "Monday"
    pub entry_hour_of_day: Option<u32>,    // 0-23
}

#[derive(Serialize, Debug, Clone)]
pub struct RejectedTradeResult {
    pub symbol: String,
    pub timeframe: String,
    pub zone_id: String,
    pub direction: String,
    pub rejection_time: DateTime<Utc>,
    pub rejection_reason: String,
    pub zone_strength: Option<f64>,
    pub touch_count: Option<i32>,
    pub zone_price: Option<f64>,
    pub entry_candle_index: Option<usize>,
}

#[derive(Serialize, Debug, Clone)]
pub struct SymbolTimeframeSummary {
    pub symbol: String,
    pub timeframe: String,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate_percent: f64,
    pub total_pnl_pips: f64,
    pub profit_factor: f64,
    // Could add more specific stats here if needed per symbol/TF
}

#[derive(Serialize, Debug, Clone)]
pub struct OverallBacktestSummary {
    pub total_trades: usize,
    pub total_pnl_pips: f64,
    pub overall_win_rate_percent: f64,
    pub overall_profit_factor: f64,
    pub average_trade_duration_str: String,
    pub overall_winning_trades: usize,
    pub overall_losing_trades: usize,
    pub trades_by_symbol_percent: std::collections::HashMap<String, f64>, // {"EURUSD_SB": 20.0, ...}
    pub trades_by_timeframe_percent: std::collections::HashMap<String, f64>, // {"30m": 10.0, ...}
    pub trades_by_day_of_week_percent: std::collections::HashMap<String, f64>, // {"Monday": 25.0, ...}
    pub pnl_by_day_of_week_pips: std::collections::HashMap<String, f64>,
    pub trades_by_hour_of_day_percent: std::collections::HashMap<u32, f64>, // {9: 15.0 (for 9 AM)}
    pub pnl_by_hour_of_day_pips: std::collections::HashMap<u32, f64>,
    // Add more aggregated stats as needed
}

#[derive(Serialize, Debug, Clone)]
pub struct RejectionSummary {
    pub total_rejections: usize,
    pub rejections_by_reason: std::collections::HashMap<String, usize>,
    pub rejections_by_symbol: std::collections::HashMap<String, usize>,
}

// ⭐ NEW: Trading rules and environment tracking structs
#[derive(Debug, Serialize, Clone)]
pub struct TradingRulesSnapshot {
    // From TradeConfig
    pub trading_enabled: bool,
    pub lot_size: f64,
    pub default_stop_loss_pips: f64,
    pub default_take_profit_pips: f64,
    pub max_trades_per_zone: usize, // ⭐ RENAMED from max_trades_per_pattern
    
    // From environment variables (parsed values)
    pub trading_max_daily_trades: Option<u32>,
    pub trading_max_trades_per_symbol: Option<u32>,
    pub trading_min_zone_strength: Option<f64>,
    pub max_touch_count_for_trading: Option<i64>,
    pub trading_max_trades_per_zone_env: Option<usize>, // ⭐ NEW: From TRADING_MAX_TRADES_PER_ZONE
    pub trading_allowed_symbols: Option<Vec<String>>,
    pub trading_allowed_timeframes: Option<Vec<String>>,
    pub trading_allowed_weekdays: Option<Vec<String>>,
    pub trading_start_hour_utc: Option<u32>,
    pub trading_end_hour_utc: Option<u32>,
}

#[derive(Debug, Serialize, Clone)]
pub struct EnvironmentVariablesSnapshot {
    // Raw environment variable values (as strings)
    pub trading_max_daily_trades: Option<String>,
    pub trading_max_trades_per_symbol: Option<String>,
    pub trading_max_trades_per_zone: Option<String>, // ⭐ RENAMED from TRADING_MAX_TRADES_PER_ZONE
    pub trading_min_zone_strength: Option<String>,
    pub max_touch_count_for_trading: Option<String>,
    pub trading_allowed_symbols: Option<String>,
    pub trading_allowed_timeframes: Option<String>,
    pub trading_allowed_weekdays: Option<String>,
    pub trading_start_hour_utc: Option<String>,
    pub trading_end_hour_utc: Option<String>,
    pub trading_enabled: Option<String>,
}

// ⭐ UPDATED: RequestParametersSnapshot with all new fields
#[derive(Debug, Serialize, Clone)]
pub struct RequestParametersSnapshot {
    pub symbols: Vec<String>,
    pub pattern_timeframes: Vec<String>,
    pub start_time: String,
    pub end_time: String,
    pub lot_size: Option<f64>,
    pub stop_loss_pips: Option<f64>,
    pub take_profit_pips: Option<f64>,
    pub max_daily_trades: Option<u32>,
    pub max_trades_per_symbol: Option<u32>,
    pub max_trades_per_zone: Option<usize>,
    pub min_zone_strength: Option<f64>,
    pub max_touch_count_for_trading: Option<i64>,
    pub allowed_trade_days: Option<Vec<String>>,
    pub allowed_symbols: Option<Vec<String>>,
    pub allowed_timeframes: Option<Vec<String>>,
    pub trade_start_hour_utc: Option<u32>,
    pub trade_end_hour_utc: Option<u32>,
}

#[derive(Serialize, Debug, Clone)]
pub struct MultiBacktestResponse {
    pub overall_summary: OverallBacktestSummary,
    pub detailed_summaries: Vec<SymbolTimeframeSummary>,
    pub all_trades: Vec<IndividualTradeResult>, // Important for granular analysis
    
    // Rejected trades tracking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_trades: Option<Vec<RejectedTradeResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_summary: Option<RejectionSummary>,
    
    // ⭐ NEW: Complete trading rules transparency
    pub trading_rules_applied: TradingRulesSnapshot,
    pub environment_variables: EnvironmentVariablesSnapshot,
    pub request_parameters: RequestParametersSnapshot,
}