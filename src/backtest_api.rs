use chrono::{DateTime, Utc, Weekday};
use serde::{Deserialize, Serialize}; // For analytics

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MultiBacktestRequest {
    pub start_time: String,
    pub end_time: String,
    pub symbols: Vec<String>,
    pub pattern_timeframes: Vec<String>,
    pub stop_loss_pips: f64,
    pub take_profit_pips: f64,
    pub lot_size: Option<f64>, // Or f64 if always required
    pub allowed_trade_days: Option<Vec<String>>, // NEW: e.g., ["Mon", "Tue", "Wed"]
    pub trade_end_hour_utc: Option<u32>, // NEW: e.g., 17 (for up to 17:59:59 UTC)
}

#[derive(Serialize, Debug, Clone)]
pub struct IndividualTradeResult {
    pub symbol: String,
    pub timeframe: String, // The timeframe the pattern was detected on
    pub direction: String, // "Long" or "Short"
    pub entry_time: DateTime<Utc>,
    pub entry_price: f64,
    pub exit_time: Option<DateTime<Utc>>,
    pub exit_price: Option<f64>,
    pub pnl_pips: Option<f64>,
    pub exit_reason: Option<String>, // "Take Profit", "Stop Loss", "End of Data"
    // For analytics
    pub entry_day_of_week: Option<String>, // e.g., "Monday"
    pub entry_hour_of_day: Option<u32>,    // 0-23
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
pub struct MultiBacktestResponse {
    pub overall_summary: OverallBacktestSummary,
    pub detailed_summaries: Vec<SymbolTimeframeSummary>,
    pub all_trades: Vec<IndividualTradeResult>, // Important for granular analysis
}
