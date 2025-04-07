// PASTE THIS ENTIRE BLOCK INTO: src/detect.rs
// This version uses a shared internal function `_fetch_and_detect_core`.

use log::{info, warn, debug, error}; // Add info and debug levels
use crate::patterns::{PatternRecognizer, /* Add specific recognizer types needed */ FiftyPercentBeforeBigBarRecognizer, EnhancedSupplyDemandZoneRecognizer /* etc */};
use crate::trades::{TradeConfig, Trade, TradeSummary}; // Needed for detect_patterns
use crate::trading::TradeExecutor; // Needed for detect_patterns
use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use dotenv::dotenv;
use reqwest;
use csv::ReaderBuilder;
use std::io::Cursor;
use std::error::Error; // Use standard Error trait

// --- Structs (Ensure CandleData and ChartQuery have Debug derived) ---
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CandleData {
    pub time: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u32,
}

#[derive(Deserialize, Debug)] // Ensure Debug is derived
pub struct ChartQuery {
    pub start_time: String,
    pub end_time: String,
    pub symbol: String,
    pub timeframe: String,
    pub pattern: String,
    pub enable_trading: Option<bool>,
    pub lot_size: Option<f64>,
    pub stop_loss_pips: Option<f64>,
    pub take_profit_pips: Option<f64>,
    pub enable_trailing_stop: Option<bool>,
}

// --- Optional: Custom Error Enum ---
#[derive(Debug)]
enum CoreError {
    EnvVar(String),
    Request(reqwest::Error),
    Csv(csv::Error),
    Config(String),
}
impl std::fmt::Display for CoreError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{:?}", self) } }
impl Error for CoreError { fn source(&self) -> Option<&(dyn Error + 'static)> { None } } // Simplified for brevity
impl From<std::env::VarError> for CoreError { fn from(err: std::env::VarError) -> Self { CoreError::EnvVar(err.to_string()) } }
impl From<reqwest::Error> for CoreError { fn from(err: reqwest::Error) -> Self { CoreError::Request(err) } }
impl From<csv::Error> for CoreError { fn from(err: csv::Error) -> Self { CoreError::Csv(err) } }
// --- End Error Enum ---


// --- INTERNAL CORE Logic Function ---
// Fetches data, selects recognizer, runs detect.
// Returns: Candles, Recognizer instance (for trading), Raw Detection Results
async fn _fetch_and_detect_core(query: &ChartQuery) -> Result<(Vec<CandleData>, Box<dyn PatternRecognizer>, Value), CoreError> {
     use crate::patterns::{ // Ensure all recognizer types are in scope
        BigBarRecognizer, BullishEngulfingRecognizer, CombinedDemandRecognizer, ConsolidationRecognizer,
        DemandMoveAwayRecognizer, DropBaseRallyRecognizer, FlexibleDemandZoneRecognizer,
        FiftyPercentBodyCandleRecognizer, PinBarRecognizer, PriceSmaCrossRecognizer, RallyRecognizer,
        SimpleSupplyDemandZoneRecognizer, SpecificTimeEntryRecognizer, SupplyZoneRecognizer /* etc */
    };

    info!("[_fetch_and_detect_core] Starting for: {:?}", query);
    dotenv().ok();

    // --- Data Fetching & Parsing (Common Logic - ensure this is EXACTLY from your working version) ---
    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG")?;
    let token = std::env::var("INFLUXDB_TOKEN")?;
    let bucket = std::env::var("INFLUXDB_BUCKET")?;
    let symbol = if query.symbol.ends_with("_SB") { query.symbol.clone() } else { format!("{}_SB", query.symbol) };
    let timeframe = query.timeframe.to_lowercase(); // <--- ADD THIS LINE
    let flux_query = format!( /* ... exact flux query ... */
         r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r["_measurement"] == "trendbar") |> filter(fn: (r) => r["symbol"] == "{}") |> filter(fn: (r) => r["timeframe"] == "{}") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
        bucket, query.start_time, query.end_time, symbol, timeframe
    );
    info!("[_fetch_and_detect_core] Sending Flux Query:\n{}", flux_query);
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response_text = client.post(&url).bearer_auth(&token).json(&serde_json::json!({"query": flux_query, "type": "flux"})).send().await?.text().await?;
    info!("[_fetch_and_detect_core] Response length: {}", response_text.len()); // Log length instead of content
    let mut candles = Vec::<CandleData>::new();
    // --- Exact CSV Parsing Logic ---
     if !response_text.trim().is_empty() {
        let cursor = Cursor::new(response_text.as_bytes());
        let mut rdr = ReaderBuilder::new().has_headers(true).flexible(true).from_reader(cursor);
        let headers = rdr.headers()?.clone();
        let get_idx = |name: &str| headers.iter().position(|h| h == name);
        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) =
            (get_idx("_time"), get_idx("open"), get_idx("high"), get_idx("low"), get_idx("close"), get_idx("volume")) {
            for result in rdr.records() {
                 match result {
                     Ok(record) => { /* ... exact safe parsing logic ... */
                         let parse_f64 = |idx: usize| record.get(idx).and_then(|v| v.parse::<f64>().ok());
                         let parse_u32 = |idx: usize| record.get(idx).and_then(|v| v.parse::<u32>().ok());
                         if record.len() > t_idx && record.len() > o_idx && record.len() > h_idx && record.len() > l_idx && record.len() > c_idx && record.len() > v_idx {
                             let time = record.get(t_idx).unwrap_or("").to_string();
                             if !time.is_empty() { candles.push(CandleData { time, open: parse_f64(o_idx).unwrap_or(0.0), high: parse_f64(h_idx).unwrap_or(0.0), low: parse_f64(l_idx).unwrap_or(0.0), close: parse_f64(c_idx).unwrap_or(0.0), volume: parse_u32(v_idx).unwrap_or(0) }); }
                         }
                     },
                     Err(e) => warn!("[_fetch_and_detect_core] CSV record error: {}", e),
                 }
             }
        } else { return Err(CoreError::Config("CSV header mismatch".to_string())); }
    }
    info!("[_fetch_and_detect_core] Parsed {} candles.", candles.len());
    // --- End Fetching/Parsing Block ---


    // --- Recognizer Selection ---
     let recognizer: Box<dyn PatternRecognizer> = match query.pattern.as_str() {
         // ... Copy the exact match statement from detect_patterns ...
         "fifty_percent_before_big_bar" => Box::new(FiftyPercentBeforeBigBarRecognizer::default()),
         "supply_demand_zone" => Box::new(EnhancedSupplyDemandZoneRecognizer::default()),
         // etc...
         _ => return Err(CoreError::Config(format!("Unknown pattern: {}", query.pattern))),
     };
     info!("[_fetch_and_detect_core] Using recognizer: {}", query.pattern);

    // --- Run Detection ---
    // IMPORTANT: Run detect even if candles is empty, let the recognizer handle it.
    let detection_results = recognizer.detect(&candles);
    info!("[_fetch_and_detect_core] Detection complete.");

    Ok((candles, recognizer, detection_results)) // Return essentials
}


// --- detect_patterns (for /analyze) ---
// Calls the core function, handles no-data case, adds trading info, returns response.
// External behavior is IDENTICAL to before.
pub async fn detect_patterns(query: web::Query<ChartQuery>) -> impl Responder {
     info!("[detect_patterns] Handling /analyze request: {:?}", query);
     match _fetch_and_detect_core(&query).await {
        // Receive candles, recognizer instance, and raw results
        Ok((candles, recognizer, mut pattern_result)) => {

             // --- Handle No Candles Case ---
             // Check *after* calling core logic, matching original behavior
            if candles.is_empty() {
                 info!("[detect_patterns] No candle data found by core logic. Returning 'not found'.");
                 // Return the exact same response your original function did for this case
                 return HttpResponse::NotFound().json(json!({
                     "error": "No data found",
                     "message": format!("No candles found for symbol {}_SB with timeframe {} in the specified time range.", query.symbol, query.timeframe)
                 }));
             }

            // --- Original Trading Logic ---
            let enable_trading = query.enable_trading.unwrap_or(false);
            if enable_trading {
                 info!("[detect_patterns] Trading enabled.");
                 // Use the exact TradeConfig setup from your original function
                let trade_config = TradeConfig { enabled: true, /* ... */
                    lot_size: query.lot_size.unwrap_or(0.01),
                    default_stop_loss_pips: query.stop_loss_pips.unwrap_or(20.0),
                    default_take_profit_pips: query.take_profit_pips.unwrap_or(40.0),
                    enable_trailing_stop: query.enable_trailing_stop.unwrap_or(false),
                    ..TradeConfig::default()
                 };
                let (trades, summary) = recognizer.trade(&candles, trade_config); // Use returned recognizer
                if let Value::Object(map) = &mut pattern_result { // Mutate the result
                    map.insert("trades".to_string(), json!(trades));
                    map.insert("trade_summary".to_string(), json!(summary));
                     info!("[detect_patterns] Trading analysis added.");
                } else {
                     warn!("[detect_patterns] Pattern result not object, cannot add trading info.");
                 }
            }
            // --- End Original Trading Logic ---

            HttpResponse::Ok().json(pattern_result) // Return potentially modified result
        }
        Err(e) => {
             error!("[detect_patterns] Core error: {}", e);
             // Map CoreError to the exact HTTP response detect_patterns originally returned
             match e {
                 CoreError::Config(msg) if msg.starts_with("Unknown pattern") => HttpResponse::BadRequest().body(msg),
                 CoreError::Config(msg) if msg == "CSV header mismatch" => HttpResponse::InternalServerError().body("CSV header mismatch error"), // Adjust body text if needed
                 CoreError::Request(rq_e) => HttpResponse::InternalServerError().body(format!("Data source request error: {}", rq_e)),
                 CoreError::Csv(csv_e) => HttpResponse::InternalServerError().body(format!("Data parsing error: {}", csv_e)),
                 CoreError::EnvVar(env_e) => HttpResponse::InternalServerError().body(format!("Server configuration error: {}", env_e)),
                 // Add other specific mappings if needed
                 _ => HttpResponse::InternalServerError().body(format!("Internal processing error: {}", e))
             }
        }
    }
}


// --- Helper function to check zone activity (Keep as is) ---
fn is_zone_still_active(zone: &Value, candles: &[CandleData], is_supply: bool) -> bool {
    // 1. Safely extract zone details
    let start_idx = match zone.get("start_idx").and_then(|v| v.as_u64()) {
        Some(idx) => idx as usize,
        None => { warn!("is_zone_still_active: Missing 'start_idx'"); return false; }
    };
    let zone_high = match zone.get("zone_high").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => { warn!("is_zone_still_active: Missing/invalid 'zone_high' for start_idx {}", start_idx); return false; }
    };
    let zone_low = match zone.get("zone_low").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => { warn!("is_zone_still_active: Missing/invalid 'zone_low' for start_idx {}", start_idx); return false; }
    };
    // --- Log Zone Details ---
    debug!(
        "is_zone_still_active: Checking zone start_idx={}, type={}, zone_low={:.5}, zone_high={:.5}",
        start_idx, if is_supply { "Supply" } else { "Demand" }, zone_low, zone_high
    );
    // ---

    // 2. Determine where to start checking
    let check_start_idx = start_idx.saturating_add(2);

    // 3. Handle edge case
    if check_start_idx >= candles.len() {
        debug!("is_zone_still_active: Zone start_idx={} ACTIVE (no candles after formation: check_start_idx={}, len={})", start_idx, check_start_idx, candles.len());
        return true;
    }

    // 4. Iterate through subsequent candles
    debug!("is_zone_still_active: Zone start_idx={} - Checking candles from index {} to {}", start_idx, check_start_idx, candles.len() - 1);
    for i in check_start_idx..candles.len() {
        let candle = &candles[i];

        if !candle.high.is_finite() || !candle.low.is_finite() || !candle.close.is_finite() {
             warn!("is_zone_still_active: Invalid candle data at index {}", i);
             continue;
        }

        // --- Log Candle Details Being Checked ---
        debug!(
            "  - Checking candle idx={}: low={:.5}, high={:.5}, close={:.5}",
            i, candle.low, candle.high, candle.close
        );
        // ---

        if is_supply {
            // --- Check Supply Invalidation ---
            if candle.high > zone_high {
                info!( // Use INFO for invalidation events
                    "is_zone_still_active: Supply zone start_idx={} INACTIVE at candle_idx={} (candle_high {:.5} > zone_high {:.5})",
                    start_idx, i, candle.high, zone_high
                );
                return false; // Zone broken
            } else {
                 // debug!("    Supply condition met: candle_high {:.5} <= zone_high {:.5}", candle.high, zone_high);
            }
        } else { // is_demand
            // --- Check Demand Invalidation ---
            if candle.low < zone_low {
                 info!( // Use INFO for invalidation events
                    "is_zone_still_active: Demand zone start_idx={} INACTIVE at candle_idx={} (candle_low {:.5} < zone_low {:.5})",
                    start_idx, i, candle.low, zone_low
                 );
                return false; // Zone broken
            } else {
                 // Optional: Log why it *didn't* invalidate if needed for deep debugging
                 // debug!(
                 //    "    Demand condition met: candle_low {:.5} >= zone_low {:.5}",
                 //    candle.low, zone_low
                 // );
            }
             // --- Add check for close below low as well? ---
             // You mentioned it closed below too. While low < zone_low should be sufficient,
             // we can add an explicit check for close if the low check fails unexpectedly.
             // Generally, if low < zone_low doesn't trigger, close < zone_low won't either,
             // unless zone_low is somehow *between* the candle's low and close (which is impossible).
             // Sticking to `candle.low < zone_low` is standard.
        }
    }

    // 5. If loop completes, zone is active
    debug!("is_zone_still_active: Zone start_idx={} ACTIVE (loop completed without invalidation)", start_idx);
    true
}


// --- NEW Handler for /active-zones ---
// Calls the core function, handles no-data case, enriches results, returns response.
pub async fn get_active_zones_handler(query: web::Query<ChartQuery>) -> impl Responder {
     info!("[get_active_zones] Handling /active-zones request: {:?}", query);
     match _fetch_and_detect_core(&query).await {
         // Receive candles and RAW detection_results. Ignore recognizer.
        Ok((candles, _recognizer, mut detection_results)) => { // Make results mutable

             // --- Handle No Candles Case ---
            if candles.is_empty() {
                 info!("[get_active_zones] No candle data found by core logic. Returning empty structure.");
                 // Return the specific structure desired for this endpoint when no data
                 return HttpResponse::Ok().json(json!({
                     "pattern": query.pattern,
                     "status": "All Zones with Activity Status and Candles",
                     "time_range": { "start": query.start_time, "end": query.end_time },
                     "symbol": query.symbol,
                     "timeframe": query.timeframe,
                     "supply_zones": [],
                     "demand_zones": [],
                     "message": "No candle data found for the specified parameters."
                 }));
             }

             info!("[get_active_zones] Enriching detection results ({} candles)...", candles.len());
             // --- Enrichment Logic (Modifies detection_results in place) ---
            let process_zones = |zones_option: Option<&mut Vec<Value>>, is_supply: bool| {
                 if let Some(zones) = zones_option {
                     info!("[get_active_zones] Enriching {} {} zones.", zones.len(), if is_supply {"supply"} else {"demand"});
                     for zone_json in zones.iter_mut() { // Iterate mutably
                         let is_active = is_zone_still_active(zone_json, &candles, is_supply);
                         if let Some(obj) = zone_json.as_object_mut() {
                             obj.insert("is_active".to_string(), json!(is_active));
                             info!("[get_active_zones] Inserted is_active={} for zone starting at {:?}", is_active, obj.get("start_idx"));
                             if let Some(start_idx) = obj.get("start_idx").and_then(|v| v.as_u64()) {
                                 let zone_candles = candles.get(start_idx as usize ..).unwrap_or(&[]).to_vec();
                                 obj.insert("candles".to_string(), json!(zone_candles));
                             } // Handle missing start_idx if needed
                         }
                     }
                 } // Handle case where zones_option is None if structure might vary
            };

            // Get mutable access and call enrichment
             if let Some(data) = detection_results.get_mut("data").and_then(|d| d.get_mut("price")) {
                 process_zones(data.get_mut("supply_zones").and_then(|sz| sz.get_mut("zones")).and_then(|z| z.as_array_mut()), true);
                 process_zones(data.get_mut("demand_zones").and_then(|dz| dz.get_mut("zones")).and_then(|z| z.as_array_mut()), false);
             } else { warn!("[get_active_zones] Could not find expected structure 'data.price...zones' to enrich."); }


            info!("[get_active_zones] Enrichment complete.");

        //     // --- ADD LOGGING BLOCK ---
        //     match serde_json::to_string_pretty(&detection_results) {
        //         Ok(json_string) => {
        //             info!("[get_active_zones] Final JSON structure to be sent (pretty):\n{}", json_string);
        //         }
        //         Err(e) => {
        //             error!("[get_active_zones] Failed to serialize final JSON for logging: {}", e);
        //         }
        //    }
           // --- END LOGGING BLOCK ---

             // Return the modified detection_results
             HttpResponse::Ok().json(detection_results)
        }
        Err(e) => {
             error!("[get_active_zones] Core error: {}", e);
             // Map CoreError to appropriate HTTP responses (can be same as detect_patterns)
             match e {
                 CoreError::Config(msg) if msg.starts_with("Unknown pattern") => HttpResponse::BadRequest().body(msg),
                 CoreError::Config(msg) if msg == "CSV header mismatch" => HttpResponse::InternalServerError().body("CSV header mismatch error"),
                  // Add other specific mappings as needed, matching detect_patterns where appropriate
                 _ => HttpResponse::InternalServerError().body(format!("Internal processing error: {}", e))
             }
         }
     }
}