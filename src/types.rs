// src/types.rs
use serde::{Deserialize, Serialize};
use crate::detect::{CandleData, ChartQuery}; // Import from detect

// --- Input ---
#[derive(Deserialize, Debug, Clone)]
pub struct BulkSymbolTimeframesRequestItem {
    pub symbol: String,
    pub timeframes: Vec<String>,
}

// --- Output ---
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct EnrichedZone {
    // Fields from recognizer JSON (use Option<T> and matching names)
    pub start_idx: Option<u64>,
    pub end_idx: Option<u64>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub fifty_percent_line: Option<f64>,
    pub detection_method: Option<String>,
    pub quality_score: Option<f64>,
    pub strength: Option<String>,
    pub extended: Option<bool>,
    pub extension_percent: Option<f64>,
    // ... other common fields ...

    // Added fields
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub candles: Vec<CandleData>,
}

#[derive(Serialize, Debug, Default)]
pub struct BulkResultData {
    pub supply_zones: Vec<EnrichedZone>,
    pub demand_zones: Vec<EnrichedZone>,
}

#[derive(Serialize, Debug)]
pub struct BulkResultItem {
    pub symbol: String,
    pub timeframe: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<BulkResultData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct BulkActiveZonesResponse {
    pub results: Vec<BulkResultItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_params: Option<ChartQuery>,
}

// Helper
pub fn deserialize_raw_zone_value(zone_value: &serde_json::Value) -> Result<EnrichedZone, serde_json::Error> {
    serde_json::from_value::<EnrichedZone>(zone_value.clone())
}