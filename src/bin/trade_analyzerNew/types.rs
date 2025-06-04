// types.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ZoneData {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_high: f64,
    pub zone_low: f64,
    pub zone_type: String, // supply/demand
    pub strength_score: f64,
    pub is_active: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PriceCandle {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Debug, Serialize)]
pub struct TradeResult {
    pub entry_time: DateTime<Utc>,
    pub symbol: String,
    pub timeframe: String,
    pub zone_id: String,
    pub action: String, // BUY/SELL
    pub entry_price: f64,
    pub exit_time: Option<DateTime<Utc>>,
    pub exit_price: Option<f64>,
    pub exit_reason: String, // TP_HIT, SL_HIT, ZONE_INVALIDATED, STILL_OPEN
    pub pnl_pips: Option<f64>,
    pub duration_minutes: Option<i64>,
    pub zone_strength: f64,
}

#[derive(Debug, Clone)]
pub struct OpenTrade {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub action: String,
    pub entry_time: DateTime<Utc>,
    pub entry_price: f64,
    pub take_profit: f64,
    pub stop_loss: f64,
    pub zone_strength: f64,
}