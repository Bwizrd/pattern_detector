// src/types.rs
// Contains shared data structures used by both the web server handlers and potentially the generator.

pub use crate::api::detect::{CandleData, ChartQuery}; // Imports from detect.rs
use log::{warn, debug};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap; // Needed for HashMap fields
// --- Structs used as Input Parameters (e.g., for web requests) ---

#[derive(Deserialize, Debug, Clone)]
pub struct BulkSymbolTimeframesRequestItem {
    pub symbol: String,
    pub timeframes: Vec<String>,
}

// --- Core Data Structures ---

// Note: Ensure CandleData is defined correctly, usually in detect.rs or here.
// If it's in detect.rs, the import `use crate::api::detect::CandleData;` at the top is correct.
// If you define it here, remove the import and ensure it has necessary derives.
// Example (assuming it's imported from detect.rs):
// pub use crate::api::detect::CandleData; // Re-export if needed elsewhere easily

// *** NEW: Struct for touch point data ***
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TouchPoint {
    pub time: String, // ISO 8601 time of the candle where touch occurred
    pub price: f64,   // Price level of the touch (proximal line)
}

// --- Structs primarily used for Zone Data Processing & Output ---

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct EnrichedZone {
    // --- Fields typically set by the recognizer ---
    pub zone_id: Option<String>, // Unique ID for the zone
    #[serde(skip_serializing_if = "Option::is_none")] // Add this for consistency
    pub symbol: Option<String>, // <<< ADDED
    #[serde(skip_serializing_if = "Option::is_none")] // Add this for consistency
    pub timeframe: Option<String>, // <<< ADDED
    pub start_idx: Option<u64>,  // Index of the first candle forming the zone
    pub end_idx: Option<u64>,    // Index of the LAST candle forming the zone (e.g., the big bar)
    pub start_time: Option<String>, // ISO 8601 format string of start_idx candle
    pub end_time: Option<String>, // ISO 8601 format string of end_idx candle
    pub zone_high: Option<f64>,  // Top boundary of the zone
    pub zone_low: Option<f64>,   // Bottom boundary of the zone
    pub fifty_percent_line: Option<f64>, // Midpoint of the zone
    pub detection_method: Option<String>, // How the zone was identified

    #[serde(rename = "type", skip_serializing_if = "Option::is_none")] // Added skip_serializing_if
    pub zone_type: Option<String>, // e.g., "supply_zone", "demand_zone". Renamed from 'type'.

    #[serde(skip_serializing_if = "Option::is_none")] // Added skip_serializing_if
    pub extended: Option<bool>, // Was the zone extended during detection?
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

    // --- *** NEW FIELDS *** ---
    /// Index of the first candle that invalidated the zone (broke distal). None if active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_end_idx: Option<u64>,

    /// Time (ISO 8601) of the first candle that invalidated the zone. Same as invalidation_time. None if active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_end_time: Option<String>,
    // --- *** END NEW FIELDS *** ---

    // --- *** NEW FIELD for Touch Points *** ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub touch_points: Option<Vec<TouchPoint>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_candles: Option<Vec<serde_json::Value>>,
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
        Ok(zone) => {
            // Post-deserialization checks or defaults if needed
            if zone.zone_id.is_none() {
                debug!(
                    "Zone missing zone_id (will be generated during enrichment). Time: {:?}",
                    zone_json.get("start_time")
                );
            }
            if zone.zone_type.is_none() {
                // Try to infer from detection_method if possible, or rely on enrichment context
                debug!(
                    "deserialize_raw_zone_value: Zone missing zone_type. JSON: {:?}",
                    zone_json.get("detection_method")
                );
                // Enrichment logic should set this based on context (supply/demand loop)
            }
            if zone.end_idx.is_none() {
                debug!(
                    "deserialize_raw_zone_value: Zone missing end_idx. JSON: {:?}",
                    zone_json
                );
                // This is critical for enrichment, maybe return error? Or let enrichment handle Option?
                // Returning Ok for now, enrichment will fail later if None.
            }
            // `is_active` should default to false due to `#[serde(default)]` if missing

            Ok(zone)
        }
        Err(e) => {
            // Log the specific error and the JSON that failed
            let error_msg = format!(
                "deserialize_raw_zone_value error: {}. Failed JSON: {}",
                e, zone_json
            );
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct FindAndVerifyZoneQueryParams {
    pub symbol: String,
    pub timeframe: String,
    pub target_formation_start_time: String, // RFC3339
    pub pattern: String,
    pub fetch_window_start_time: String, // RFC3339
    pub fetch_window_end_time: String,   // RFC3339
}

#[derive(Serialize, Debug, Clone)]
pub struct FindAndVerifyZoneQueryParamsSerializable {
    // For echoing in response
    pub symbol: String,
    pub timeframe: String,
    pub target_formation_start_time: String,
    pub pattern: String,
    pub fetch_window_start_time: String,
    pub fetch_window_end_time: String,
}

#[derive(Serialize, Debug, Default, Clone)]
pub struct FoundZoneActivityDetails {
    // Details of THE ONE zone if found
    pub detected_zone_actual_start_time: Option<String>,
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub detection_method: Option<String>,
    pub zone_type_detected: Option<String>,
    pub start_idx_in_fetched_slice: Option<u64>,
    pub end_idx_in_fetched_slice: Option<u64>,
    pub is_active: bool,
    pub touch_count: i64,
    pub bars_active_after_formation: Option<u64>,
    pub invalidation_candle_time: Option<String>,
    pub first_touch_candle_time: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct FindAndVerifyZoneResult {
    // Overall response for the endpoint
    pub query_params: FindAndVerifyZoneQueryParamsSerializable,
    pub fetched_candles_count: usize,
    pub found_zone_details: Option<FoundZoneActivityDetails>,
    pub message: String,
    pub error_details: Option<String>,
    pub recognizer_raw_output_for_target_zone: Option<serde_json::Value>, // For debugging
                                                                          // We can add overall_fetch_start/end if needed later, from the logic function
}

#[derive(serde::Deserialize, Debug, Clone, Default)]
pub struct ZoneCsvRecord {
    // --- Columns from pivot rowKey ---
    // These MUST match the column names produced by the pivot if they are part of rowKey
    #[serde(rename = "_time")]
    pub time: Option<String>,
    pub symbol: Option<String>,
    pub timeframe: Option<String>,
    pub pattern: Option<String>,
    pub zone_type: Option<String>,

    // --- Columns from pivot columnKey (original _field values) ---
    // Add ALL fields you expect after pivot. Use Option<String> for initial parsing.
    pub bars_active: Option<String>,
    pub detection_method: Option<String>,
    pub end_time_rfc3339: Option<String>,
    pub fifty_percent_line: Option<String>,
    pub formation_candles_json: Option<String>,
    pub is_active: Option<String>, // Will be "0" or "1"
    pub start_time_rfc3339: Option<String>,
    pub start_idx: Option<String>, // ADD THIS (as String for initial parse from CSV)
    pub end_idx: Option<String>,   // ADD THIS (as String for initial parse from CSV)
    pub strength_score: Option<String>,
    pub touch_count: Option<String>,
    pub zone_high: Option<String>,
    pub zone_id: Option<String>, // zone_id is now a regular column
    pub zone_low: Option<String>,

    // --- Standard InfluxDB meta columns (often present in CSV, can be ignored) ---
    #[serde(default, rename = "_start")]
    pub _start: Option<String>,
    #[serde(default, rename = "_stop")]
    pub _stop: Option<String>,
    #[serde(default, rename = "result")]
    pub _result_influx: Option<String>, // Avoid conflict if StoredZone has 'result'
    #[serde(default)]
    pub table: Option<String>, // Table ID is usually numeric
    #[serde(default, rename = "_measurement")]
    pub _measurement: Option<String>,
}

pub fn map_csv_to_stored_zone(csv_record: ZoneCsvRecord) -> crate::types::StoredZone {
    // Use fully qualified StoredZone
    let parse_optional_f64 = |s: Option<String>| -> Option<f64> {
        s.and_then(|str_val| str_val.trim().parse::<f64>().ok())
    };
    let parse_optional_i64 = |s: Option<String>| -> Option<i64> {
        s.and_then(|str_val| str_val.trim().parse::<i64>().ok())
    };
    let parse_optional_u64 = |s: Option<String>| -> Option<u64> {
        s.and_then(|str_val| str_val.trim().parse::<u64>().ok())
    };
    let is_active_bool = csv_record
        .is_active
        .as_deref()
        .map_or(false, |s| s.trim() == "1");

    let formation_candles_vec: Vec<crate::types::CandleData> = csv_record
        .formation_candles_json
        .as_ref()
        .and_then(|json_str| match serde_json::from_str(json_str.trim()) {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!(
                    "[MAP_CSV] Failed to parse formation_candles_json string: {}. JSON: '{}'",
                    e,
                    json_str
                );
                None
            }
        })
        .unwrap_or_default();

    crate::types::StoredZone {
        zone_id: csv_record
            .zone_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        symbol: csv_record.symbol.map(|s| s.trim().to_string()),
        timeframe: csv_record.timeframe.map(|s| s.trim().to_string()),
        pattern: csv_record.pattern.map(|s| s.trim().to_string()),
        zone_type: csv_record.zone_type.map(|s| s.trim().to_string()),
        start_time: csv_record.time.map(|s| s.trim().to_string()), // Map Influx _time
        end_time: csv_record.end_time_rfc3339.map(|s| s.trim().to_string()),
        zone_high: parse_optional_f64(csv_record.zone_high),
        zone_low: parse_optional_f64(csv_record.zone_low),
        fifty_percent_line: parse_optional_f64(csv_record.fifty_percent_line),
        detection_method: csv_record.detection_method.map(|s| s.trim().to_string()),
        is_active: is_active_bool,
        bars_active: parse_optional_u64(csv_record.bars_active),
        touch_count: parse_optional_i64(csv_record.touch_count),
        strength_score: parse_optional_f64(csv_record.strength_score),
        formation_candles: formation_candles_vec,
        start_idx: parse_optional_u64(csv_record.start_idx), // MODIFY THIS
        end_idx: parse_optional_u64(csv_record.end_idx),     // MODIFY THIS
        extended: None,
        extension_percent: None,
    }
}
