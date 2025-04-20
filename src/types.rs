// src/types.rs
// Contains shared data structures used by both the web server handlers and potentially the generator.

use serde::{Deserialize, Serialize};
use crate::detect::{CandleData, ChartQuery}; // Imports from detect.rs
use std::collections::HashMap; // Needed for HashMap fields

// --- Structs used as Input Parameters (e.g., for web requests) ---

#[derive(Deserialize, Debug, Clone)]
pub struct BulkSymbolTimeframesRequestItem {
    pub symbol: String,
    pub timeframes: Vec<String>,
}

// --- Core Data Structures ---

// Note: Ensure CandleData is defined correctly, usually in detect.rs or here.
// If it's in detect.rs, the import `use crate::detect::CandleData;` at the top is correct.
// If you define it here, remove the import and ensure it has necessary derives.
// Example (assuming it's imported from detect.rs):
// pub use crate::detect::CandleData; // Re-export if needed elsewhere easily


// --- Structs primarily used for Zone Data Processing & Output ---

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct EnrichedZone {
    // Fields typically deserialized from the pattern recognizer's JSON output
    pub start_idx: Option<u64>,
    pub end_idx: Option<u64>,
    pub start_time: Option<String>, // ISO 8601 format string
    pub end_time: Option<String>,   // ISO 8601 format string (for formation end)
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub fifty_percent_line: Option<f64>,
    pub detection_method: Option<String>,
    pub quality_score: Option<f64>,
    pub strength: Option<String>,
    pub extended: Option<bool>,         // Was the zone extended during detection?
    pub extension_percent: Option<f64>, // How much was it extended?

    // Fields added during enrichment process
    #[serde(default)] // Ensures this defaults to false if missing during deserialization
    pub is_active: bool, // Status calculated based on later price action

    #[serde(skip_serializing_if = "Option::is_none")] // Don't include in JSON if None
    pub bars_active: Option<u64>, // How many bars the zone remained active (calculated for bulk/debug)
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
    pub status: String, // e.g., "Success", "No Data", "Error: ..."
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<BulkResultData>, // Contains zones only on success
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>, // Description if status indicates an error
}

// --- Structs specifically needed for the debug endpoint's desired response format ---
// These structure the response to group by symbol, then timeframe.

#[derive(Serialize, Clone, Debug, Default)]
pub struct TimeframeZoneResponse {
    // Holds the zones for a single timeframe, matching BulkResultData structure
    pub supply_zones: Vec<EnrichedZone>,
    pub demand_zones: Vec<EnrichedZone>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct SymbolZoneResponse {
    // Maps timeframe strings (e.g., "4h") to their zone data
    pub timeframes: HashMap<String, TimeframeZoneResponse>,
}

// --- Main Response Structures for API Endpoints ---

// Response for the BULK endpoint (/bulk-multi-tf-active-zones)
// AND ALSO used as the structure for the DEBUG endpoint (/debug-bulk-zone)
#[derive(Serialize, Debug, Default)] // Add Default derive
pub struct BulkActiveZonesResponse {
    // Field specifically used by the debug endpoint to structure its single result
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub symbols: HashMap<String, SymbolZoneResponse>,

    // Original fields for the bulk endpoint response
    pub results: Vec<BulkResultItem>, // List of results for each requested symbol/timeframe combo
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_params: Option<ChartQuery>, // Echo back the global query parameters used
}

// --- Helper Functions ---

// Public helper function to deserialize raw JSON into EnrichedZone
// Used by handlers that process results from _fetch_and_detect_core
pub fn deserialize_raw_zone_value(zone_value: &serde_json::Value) -> Result<EnrichedZone, serde_json::Error> {
    serde_json::from_value::<EnrichedZone>(zone_value.clone())
}


// --- StoredZone (Keep this here if you added it previously for the generator) ---
// --- If you are ONLY restoring the debug endpoint, you can COMMENT OUT or REMOVE this struct ---
/*
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct StoredZone {
    // Base fields from detection
    pub start_idx: Option<u64>,
    pub end_idx: Option<u64>,
    pub start_time: Option<String>,
    pub end_time: Option<String>, // Formation end time from detector
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub fifty_percent_line: Option<f64>,
    pub detection_method: Option<String>,
    pub quality_score: Option<f64>,
    pub strength: Option<String>,
    pub extended: Option<bool>,
    pub extension_percent: Option<f64>,
    // Status determined by the generator run
    pub is_active: bool, // As of the end_time of the generator run
    // Specific candles forming the zone
    #[serde(default)]
    pub formation_candles: Vec<CandleData>,
}
*/