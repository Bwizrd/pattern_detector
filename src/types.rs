// src/types.rs
// Contains shared data structures used by both the web server handlers and potentially the generator.

use serde::{Deserialize, Serialize};
use crate::detect::{CandleData, ChartQuery}; // Imports from detect.rs
use std::collections::HashMap; // Needed for HashMap fields
use log::{warn};
use serde_json::Value; 
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
    // --- Fields typically set by the recognizer ---
    pub zone_id: Option<String>, // Unique ID for the zone
    pub start_idx: Option<u64>,  // Index of the first candle forming the zone
    pub end_idx: Option<u64>,    // Index of the LAST candle forming the zone (e.g., the big bar)
    pub start_time: Option<String>, // ISO 8601 format string of start_idx candle
    pub end_time: Option<String>,   // ISO 8601 format string of end_idx candle
    pub zone_high: Option<f64>,     // Top boundary of the zone
    pub zone_low: Option<f64>,      // Bottom boundary of the zone
    pub fifty_percent_line: Option<f64>, // Midpoint of the zone
    pub detection_method: Option<String>, // How the zone was identified

    #[serde(rename = "type", skip_serializing_if = "Option::is_none")] // Added skip_serializing_if
    pub zone_type: Option<String>, // e.g., "supply_zone", "demand_zone". Renamed from 'type'.

    #[serde(skip_serializing_if = "Option::is_none")] // Added skip_serializing_if
    pub extended: Option<bool>,         // Was the zone extended during detection?
    #[serde(skip_serializing_if = "Option::is_none")] // Added skip_serializing_if
    pub extension_percent: Option<f64>, // How much was it extended?

    // --- Fields calculated during enrichment ---
    #[serde(default)] // Defaults to false if missing. Calculated based on distal line status.
    pub is_active: bool,

    #[serde(skip_serializing_if = "Option::is_none")] // Don't include in JSON if None
    pub bars_active: Option<u64>, // How many bars passed *after* formation before the zone was invalidated (distal break).

    #[serde(skip_serializing_if = "Option::is_none")]
    pub touch_count: Option<i64>, // Number of times price entered the zone (touched proximal) before consumption/invalidation.

    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength_score: Option<f64>, // A calculated score (e.g., based on touches, formation quality).

    // --- NEW FIELDS ---
    /// Time (ISO 8601) of the first candle that invalidated the zone (broke the distal line). None if not invalidated yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidation_time: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_data_candle: Option<String>,

    // Optional: Include candles involved in the zone state? Might make JSON large.
    // #[serde(skip_serializing_if = "Option::is_none")]
    // pub candles: Option<Vec<CandleData>>,
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
pub fn deserialize_raw_zone_value(zone_json: &Value) -> Result<EnrichedZone, String> {
    // Attempt direct deserialization first
    match serde_json::from_value::<EnrichedZone>(zone_json.clone()) {
        Ok(mut zone) => {
            // Post-deserialization checks or defaults if needed
            if zone.zone_id.is_none() {
                // We expect the enrichment logic to add this now if missing
                warn!("deserialize_raw_zone_value: Zone missing zone_id (will be generated during enrichment). JSON: {:?}", zone_json.get("start_time"));
            }
             if zone.zone_type.is_none() {
                // Try to infer from detection_method if possible, or rely on enrichment context
                warn!("deserialize_raw_zone_value: Zone missing zone_type. JSON: {:?}", zone_json.get("detection_method"));
                 // Enrichment logic should set this based on context (supply/demand loop)
             }
            if zone.end_idx.is_none() {
                 warn!("deserialize_raw_zone_value: Zone missing end_idx. JSON: {:?}", zone_json);
                 // This is critical for enrichment, maybe return error? Or let enrichment handle Option?
                 // Returning Ok for now, enrichment will fail later if None.
            }
             // `is_active` should default to false due to `#[serde(default)]` if missing

            Ok(zone)
        }
        Err(e) => {
            // Log the specific error and the JSON that failed
            let error_msg = format!("deserialize_raw_zone_value error: {}. Failed JSON: {}", e, zone_json);
            warn!("{}", error_msg); // Use warn or error as appropriate
            Err(error_msg)
        }
    }
}

// --- StoredZone (Keep this here if you added it previously for the generator) ---
// --- If you are ONLY restoring the debug endpoint, you can COMMENT OUT or REMOVE this struct ---

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct StoredZone {
    // Base fields from detection
    pub zone_id: Option<String>, // Unique ID for the zone
    pub start_idx: Option<u64>,
    pub end_idx: Option<u64>,
    pub start_time: Option<String>,
    pub end_time: Option<String>, // Formation end time from detector
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub fifty_percent_line: Option<f64>,
    pub detection_method: Option<String>,
    pub extended: Option<bool>,
    pub extension_percent: Option<f64>,
    // Status determined by the generator run
    #[serde(default)]
    pub is_active: bool, // As of the end_time of the generator run
    // Specific candles forming the zone
    #[serde(skip_serializing_if = "Option::is_none")] // Keep Option for flexibility
    pub bars_active: Option<u64>, // <<< Ensure this exists
    
    #[serde(default)]
    pub formation_candles: Vec<CandleData>,
    // --- ADD THESE FIELDS ---
    #[serde(skip_serializing_if = "Option::is_none")] // Don't expect in input JSON yet
    pub touch_count: Option<i64>, // Use i64 for InfluxDB integer type
    #[serde(skip_serializing_if = "Option::is_none")] // Don't expect in input JSON yet
    pub strength_score: Option<f64>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeframe: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_type: Option<String>,
    // --- END ADDED FIELDS ---
}