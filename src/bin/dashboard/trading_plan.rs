use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OptimizedCombination {
    pub symbol: String,
    pub timeframe: String,
    #[serde(rename = "stopLoss")]
    pub stop_loss: f64,
    #[serde(rename = "takeProfit")]
    pub take_profit: f64,
    #[serde(rename = "winRate")]
    pub win_rate: f64,
    #[serde(rename = "totalPnL")]
    pub total_pnl: f64,
    pub profit_factor: Option<f64>,
    #[serde(rename = "totalTrades")]
    pub total_trades: i32,
    #[serde(rename = "winningTrades")]
    pub winning_trades: i32,
    #[serde(rename = "losingTrades")]
    pub losing_trades: i32,
    #[serde(rename = "avgDuration")]
    pub avg_duration: String,
    #[serde(rename = "lotSize")]
    pub lot_size: f64,
    pub pattern: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DateRange {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(rename = "exportedAt")]
    pub exported_at: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TradingPlan {
    #[serde(rename = "optimizationDate")]
    pub optimization_date: String,
    #[serde(rename = "totalCombinations")]
    pub total_combinations: i32,
    #[serde(rename = "totalPnL")]
    pub total_pnl: f64,
    #[serde(rename = "dateRange")]
    pub date_range: DateRange,
    #[serde(rename = "bestCombinations")]
    pub best_combinations: HashMap<String, OptimizedCombination>,
    pub metadata: Metadata,
}

impl TradingPlan {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Option<Self> {
        let data = fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }
} 