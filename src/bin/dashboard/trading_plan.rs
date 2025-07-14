use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct TradingSetup {
    pub symbol: String,
    pub timeframe: String,
    pub sl: f64,
    pub tp: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TradingPlan {
    #[serde(rename = "top5Setups")]
    pub top_setups: Vec<TradingSetup>,
}

impl TradingPlan {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Option<Self> {
        let data = fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }
} 