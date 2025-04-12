// PASTE THIS ENTIRE FILE CONTENT INTO: src/detect.rs

use log::{info, warn, debug, error};
// Only need imports for the *single* pattern handled in the simplified _fetch_and_detect_core now
use crate::patterns::{
    PatternRecognizer, FiftyPercentBeforeBigBarRecognizer // Removed others from top level
    // If you add back patterns to the match, re-import their structs here
};
// Trade imports moved into detect_patterns where they are used
use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize}; // Added Serialize
use serde_json::{json, Value};
use dotenv::dotenv;
use reqwest;
use csv::ReaderBuilder;
use std::io::Cursor;
use std::error::Error;
use crate::types::{ // Imports for bulk handler types
    BulkSymbolTimeframesRequestItem, BulkResultItem, BulkResultData,
    BulkActiveZonesResponse, EnrichedZone, deserialize_raw_zone_value
};
use futures::future::join_all; // For concurrent tasks

// --- Structs ---
#[derive(Deserialize, Serialize, Clone, Debug)] // Added Clone
pub struct CandleData {
    pub time: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)] // Added Serialize & Clone
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

// --- Custom Error Enum ---
#[derive(Debug)]
pub enum CoreError { // Keep pub if needed by other modules, otherwise private
    EnvVar(String),
    Request(reqwest::Error),
    Csv(csv::Error),
    Config(String),
}
impl std::fmt::Display for CoreError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{:?}", self) } }
impl Error for CoreError { fn source(&self) -> Option<&(dyn Error + 'static)> { None } }
impl From<std::env::VarError> for CoreError { fn from(err: std::env::VarError) -> Self { CoreError::EnvVar(err.to_string()) } }
impl From<reqwest::Error> for CoreError { fn from(err: reqwest::Error) -> Self { CoreError::Request(err) } }
impl From<csv::Error> for CoreError { fn from(err: csv::Error) -> Self { CoreError::Csv(err) } }


// --- INTERNAL CORE Logic Function (SIMPLIFIED MATCH) ---
// Contains the minimal change to only handle the specific pattern(s) needed right now.
async fn _fetch_and_detect_core(query: &ChartQuery) -> Result<(Vec<CandleData>, Box<dyn PatternRecognizer>, Value), CoreError> {

    info!("[_fetch_and_detect_core] Starting for: {:?}", query);
    dotenv().ok();

    // --- Data Fetching & Parsing (Unchanged from your original) ---
    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG")?;
    let token = std::env::var("INFLUXDB_TOKEN")?;
    let bucket = std::env::var("INFLUXDB_BUCKET")?;
    let symbol = if query.symbol.ends_with("_SB") { query.symbol.clone() } else { format!("{}_SB", query.symbol) };
    let timeframe = query.timeframe.to_lowercase();
    let flux_query = format!(
         r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r["_measurement"] == "trendbar") |> filter(fn: (r) => r["symbol"] == "{}") |> filter(fn: (r) => r["timeframe"] == "{}") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
        bucket, query.start_time, query.end_time, symbol, timeframe
    );
    // info!("[_fetch_and_detect_core] Sending Flux Query:\n{}", flux_query); // Optional: reduce log noise
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response_text = client.post(&url).bearer_auth(&token).json(&serde_json::json!({"query": flux_query, "type": "flux"})).send().await?.text().await?;
    // info!("[_fetch_and_detect_core] Response length: {}", response_text.len()); // Optional: reduce log noise
    let mut candles = Vec::<CandleData>::new();
    if !response_text.trim().is_empty() {
        let cursor = Cursor::new(response_text.as_bytes());
        let mut rdr = ReaderBuilder::new().has_headers(true).flexible(true).from_reader(cursor);
        let headers = rdr.headers()?.clone();
        let get_idx = |name: &str| headers.iter().position(|h| h == name);
        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) =
            (get_idx("_time"), get_idx("open"), get_idx("high"), get_idx("low"), get_idx("close"), get_idx("volume")) {
            for result in rdr.records() {
                 match result {
                     Ok(record) => {
                         let parse_f64 = |idx: usize| record.get(idx).and_then(|v| v.parse::<f64>().ok());
                         let parse_u32 = |idx: usize| record.get(idx).and_then(|v| v.parse::<u32>().ok());
                         if record.len() > t_idx && record.len() > o_idx && record.len() > h_idx && record.len() > l_idx && record.len() > c_idx && record.len() > v_idx {
                             let time = record.get(t_idx).unwrap_or("").to_string();
                             if !time.is_empty() { candles.push(CandleData { time, open: parse_f64(o_idx).unwrap_or(0.0), high: parse_f64(h_idx).unwrap_or(0.0), low: parse_f64(l_idx).unwrap_or(0.0), close: parse_f64(c_idx).unwrap_or(0.0), volume: parse_u32(v_idx).unwrap_or(0) }); }
                         } else { warn!("Skipping record with unexpected length: {}", record.len()); }
                     },
                     Err(e) => warn!("[_fetch_and_detect_core] CSV record error: {}", e),
                 }
             }
        } else { return Err(CoreError::Config("CSV header mismatch".to_string())); }
    }
    info!("[_fetch_and_detect_core] Parsed {} candles for {}/{}", candles.len(), query.symbol, query.timeframe);


    // --- Recognizer Selection (SIMPLIFIED) ---
     let recognizer: Box<dyn PatternRecognizer> = match query.pattern.as_str() {
         // --- ONLY Handle the pattern currently in use ---
         "fifty_percent_before_big_bar" => Box::new(FiftyPercentBeforeBigBarRecognizer::default()),
         // If "supply_demand_zone" is ALSO needed AND uses default(), add its arm here:
         // "supply_demand_zone" => Box::new(EnhancedSupplyDemandZoneRecognizer::default()),

         // --- Return error for ALL other patterns ---
         other_pattern => {
             warn!("Pattern '{}' is not currently configured in _fetch_and_detect_core match statement.", other_pattern);
             return Err(CoreError::Config(format!("Pattern '{}' is not supported by this endpoint.", other_pattern)));
         }
     };
     info!("[_fetch_and_detect_core] Using recognizer: {}", query.pattern);

    // --- Run Detection ---
    let detection_results = recognizer.detect(&candles);
    info!("[_fetch_and_detect_core] Detection complete.");
    Ok((candles, recognizer, detection_results))
}


// --- detect_patterns (Unchanged from your original) ---
pub async fn detect_patterns(query: web::Query<ChartQuery>) -> impl Responder {
     // Moved imports here
     use crate::trades::{TradeConfig, Trade, TradeSummary};
     use crate::trading::TradeExecutor;

     info!("[detect_patterns] Handling /analyze request: {:?}", query);
     match _fetch_and_detect_core(&query).await {
        Ok((candles, recognizer, mut pattern_result)) => {
            if candles.is_empty() {
                 info!("[detect_patterns] No candle data found. Returning 'not found'.");
                 return HttpResponse::NotFound().json(json!({
                     "error": "No data found",
                     "message": format!("No candles found for symbol {}_SB with timeframe {} in the specified time range.", query.symbol, query.timeframe)
                 }));
             }

            let enable_trading = query.enable_trading.unwrap_or(false);
            if enable_trading {
                 info!("[detect_patterns] Trading enabled.");
                let trade_config = TradeConfig {
                    enabled: true,
                    lot_size: query.lot_size.unwrap_or(0.01),
                    default_stop_loss_pips: query.stop_loss_pips.unwrap_or(20.0),
                    default_take_profit_pips: query.take_profit_pips.unwrap_or(40.0),
                    enable_trailing_stop: query.enable_trailing_stop.unwrap_or(false),
                    ..TradeConfig::default()
                 };
                let (trades, summary) = recognizer.trade(&candles, trade_config);
                if let Value::Object(map) = &mut pattern_result {
                    map.insert("trades".to_string(), json!(trades));
                    map.insert("trade_summary".to_string(), json!(summary));
                     info!("[detect_patterns] Trading analysis added.");
                } else {
                     warn!("[detect_patterns] Pattern result not object, cannot add trading info.");
                 }
            }

            HttpResponse::Ok().json(pattern_result)
        }
        Err(e) => {
             error!("[detect_patterns] Core error: {}", e);
             match e {
                 CoreError::Config(msg) if msg.starts_with("Unknown pattern") => HttpResponse::BadRequest().body(msg),
                 CoreError::Config(msg) if msg == "CSV header mismatch" => HttpResponse::InternalServerError().body("CSV header mismatch error"),
                 CoreError::Request(rq_e) => HttpResponse::InternalServerError().body(format!("Data source request error: {}", rq_e)),
                 CoreError::Csv(csv_e) => HttpResponse::InternalServerError().body(format!("Data parsing error: {}", csv_e)),
                 CoreError::EnvVar(env_e) => HttpResponse::InternalServerError().body(format!("Server configuration error: {}", env_e)),
                 _ => HttpResponse::InternalServerError().body(format!("Internal processing error: {}", e))
             }
        }
    }
}


// --- Helper function to check zone activity (Unchanged from your original) ---
fn is_zone_still_active(zone: &Value, candles: &[CandleData], is_supply: bool) -> bool {
    let start_idx = match zone.get("start_idx").and_then(|v| v.as_u64()) {
        Some(idx) => idx as usize,
        None => { warn!("is_zone_still_active: Missing 'start_idx' in zone: {:?}", zone); return false; }
    };
    let zone_high = match zone.get("zone_high").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => { warn!("is_zone_still_active: Missing/invalid 'zone_high' for start_idx {} in zone: {:?}", start_idx, zone); return false; }
    };
    let zone_low = match zone.get("zone_low").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => { warn!("is_zone_still_active: Missing/invalid 'zone_low' for start_idx {} in zone: {:?}", start_idx, zone); return false; }
    };
    // debug!("is_zone_still_active: Checking zone start_idx={}, type={}, zone_low={:.5}, zone_high={:.5}", start_idx, if is_supply { "Supply" } else { "Demand" }, zone_low, zone_high);
    let check_start_idx = start_idx.saturating_add(2);
    if check_start_idx >= candles.len() {
        // debug!("is_zone_still_active: Zone start_idx={} ACTIVE (no candles after formation)", start_idx);
        return true;
    }
    // debug!("is_zone_still_active: Zone start_idx={} - Checking candles from index {} to {}", start_idx, check_start_idx, candles.len() - 1);
    for i in check_start_idx..candles.len() {
        let candle = &candles[i];
        if !candle.high.is_finite() || !candle.low.is_finite() || !candle.close.is_finite() {
             warn!("is_zone_still_active: Invalid candle data at index {}", i);
             continue;
        }
        // debug!("  - Checking candle idx={}: low={:.5}, high={:.5}, close={:.5}", i, candle.low, candle.high, candle.close);
        if is_supply {
            if candle.high > zone_high {
                // info!("is_zone_still_active: Supply zone start_idx={} INACTIVE at candle_idx={}", start_idx, i);
                return false;
            }
        } else {
            if candle.low < zone_low {
                 // info!("is_zone_still_active: Demand zone start_idx={} INACTIVE at candle_idx={}", start_idx, i);
                return false;
            }
        }
    }
    // debug!("is_zone_still_active: Zone start_idx={} ACTIVE (loop completed)", start_idx);
    true
}


// --- get_active_zones_handler (Unchanged from your original) ---
pub async fn get_active_zones_handler(query: web::Query<ChartQuery>) -> impl Responder {
     info!("[get_active_zones] Handling /active-zones request: {:?}", query);
     match _fetch_and_detect_core(&query).await {
        Ok((candles, _recognizer, mut detection_results)) => {
            if candles.is_empty() {
                 info!("[get_active_zones] No candle data found. Returning empty structure.");
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
            let process_zones = |zones_option: Option<&mut Vec<Value>>, is_supply: bool| {
                 if let Some(zones) = zones_option {
                     // info!("[get_active_zones] Enriching {} {} zones.", zones.len(), if is_supply {"supply"} else {"demand"});
                     for zone_json in zones.iter_mut() {
                         let is_active = is_zone_still_active(zone_json, &candles, is_supply);
                         if let Some(obj) = zone_json.as_object_mut() {
                             obj.insert("is_active".to_string(), json!(is_active));
                             // info!("[get_active_zones] Inserted is_active={} for zone starting at {:?}", is_active, obj.get("start_idx"));
                             if let Some(start_idx) = obj.get("start_idx").and_then(|v| v.as_u64()) {
                                 let zone_candles = candles.get(start_idx as usize ..).unwrap_or(&[]).to_vec();
                                 obj.insert("candles".to_string(), json!(zone_candles));
                             }
                         }
                     }
                 }
            };
             if let Some(data) = detection_results.get_mut("data").and_then(|d| d.get_mut("price")) {
                 process_zones(data.get_mut("supply_zones").and_then(|sz| sz.get_mut("zones")).and_then(|z| z.as_array_mut()), true);
                 process_zones(data.get_mut("demand_zones").and_then(|dz| dz.get_mut("zones")).and_then(|z| z.as_array_mut()), false);
             } else { warn!("[get_active_zones] Could not find 'data.price...zones' to enrich in {:?}", detection_results); }


            info!("[get_active_zones] Enrichment complete.");
            HttpResponse::Ok().json(detection_results)
        }
        Err(e) => {
             error!("[get_active_zones] Core error: {}", e);
             match e {
                 CoreError::Config(msg) if msg.starts_with("Unknown pattern") => HttpResponse::BadRequest().body(msg),
                 CoreError::Config(msg) if msg == "CSV header mismatch" => HttpResponse::InternalServerError().body("CSV header mismatch error"),
                 CoreError::Request(rq_e) => HttpResponse::InternalServerError().body(format!("Data source request error: {}", rq_e)),
                 CoreError::Csv(csv_e) => HttpResponse::InternalServerError().body(format!("Data parsing error: {}", csv_e)),
                 CoreError::EnvVar(env_e) => HttpResponse::InternalServerError().body(format!("Server configuration error: {}", env_e)),
                 _ => HttpResponse::InternalServerError().body(format!("Internal processing error: {}", e))
             }
         }
     }
}


// --- NEW BULK HANDLER (Filters for active zones, syntax corrected) ---
pub async fn get_bulk_multi_tf_active_zones_handler(
    global_query: web::Query<ChartQuery>,
    items: web::Json<Vec<BulkSymbolTimeframesRequestItem>>,
) -> impl Responder {
    let request_items = items.into_inner();
    info!("[get_bulk_multi_tf] Handling request for {} symbol entries. Query: {:?}", request_items.len(), global_query);

    let mut tasks = Vec::new();

    for item in request_items {
        let symbol_for_tasks = item.symbol.clone();
        for timeframe in item.timeframes {
            let current_symbol = symbol_for_tasks.clone();
            let current_timeframe = timeframe.clone();
            let current_query = ChartQuery { // Needs ChartQuery: Clone + Serialize
                symbol: current_symbol.clone(),
                timeframe: current_timeframe.clone(),
                start_time: global_query.start_time.clone(),
                end_time: global_query.end_time.clone(),
                pattern: global_query.pattern.clone(),
                enable_trading: None, lot_size: None, stop_loss_pips: None, take_profit_pips: None, enable_trailing_stop: None,
            };

            // Optional: Early check if pattern is supported
             if current_query.pattern != "fifty_percent_before_big_bar" /* && current_query.pattern != "supply_demand_zone" // Add other allowed patterns here */ {
                 warn!("[get_bulk_multi_tf] Skipping task for unsupported pattern '{}' for {}/{}",
                    current_query.pattern, current_symbol, current_timeframe);
                 tasks.push(tokio::spawn(async move {
                     BulkResultItem {
                        symbol: current_symbol, timeframe: current_timeframe,
                        status: "Error: Pattern Not Supported".to_string(),
                        data: None,
                        error_message: Some(format!("Pattern '{}' not supported by this endpoint configuration.", current_query.pattern)),
                     }
                 }));
                 continue; // Skip to next iteration
             }

            let task = tokio::spawn(async move {
                info!("[Task {}/{}] Processing pattern '{}'...", current_symbol, current_timeframe, current_query.pattern);

                // Calls the SIMPLIFIED _fetch_and_detect_core
                match _fetch_and_detect_core(&current_query).await { // CORRECT variable name
                    Ok((candles, _recognizer, detection_results_json)) => {
                        if candles.is_empty() {
                             info!("[Task {}/{}] No candle data found.", current_symbol, current_timeframe);
                             return BulkResultItem { symbol: current_symbol, timeframe: current_timeframe, status: "No Data".to_string(), data: None, error_message: Some("No candle data found.".to_string()), };
                        }

                        // --- Enrichment logic ---
                        info!("[Task {}/{}] Enriching results ({} candles)...", current_symbol, current_timeframe, candles.len());
                        let mut final_supply_zones: Vec<EnrichedZone> = Vec::new();
                        let mut final_demand_zones: Vec<EnrichedZone> = Vec::new();
                        let mut enrichment_error: Option<String> = None;

                        let process_and_enrich_zones = |zones_option: Option<&Vec<Value>>, is_supply: bool| -> Result<Vec<EnrichedZone>, String> {
                            let mut processed_zones: Vec<EnrichedZone> = Vec::new();
                            if let Some(zones) = zones_option {
                                for zone_json in zones.iter() {
                                    let is_active = is_zone_still_active(zone_json, &candles, is_supply);
                                    let zone_candles: Vec<CandleData> = match zone_json.get("start_idx").and_then(|v| v.as_u64()) {
                                        Some(start_idx) => candles.get(start_idx as usize ..).unwrap_or(&[]).to_vec(),
                                        None => { warn!("[Task {}/{}] Zone missing 'start_idx'", current_symbol, current_timeframe); Vec::new() }
                                    };
                                    match deserialize_raw_zone_value(zone_json) {
                                        Ok(mut enriched_zone_struct) => {
                                            enriched_zone_struct.is_active = is_active;
                                            enriched_zone_struct.candles = zone_candles; // Still include candles for active zones
                                            processed_zones.push(enriched_zone_struct);
                                        }
                                        Err(e) => {
                                            error!("[Task {}/{}] Deserialize failed: {}. Value: {:?}", current_symbol, current_timeframe, e, zone_json);
                                            return Err(format!("Deserialize failed: {}", e));
                                        }
                                    }
                                }
                            }
                            Ok(processed_zones)
                        };

                        if let Some(data) = detection_results_json.get("data").and_then(|d| d.get("price")) {
                            // Process Supply Zones
                            match data.get("supply_zones").and_then(|sz| sz.get("zones")).and_then(|z| z.as_array()) {
                                Some(raw_zones) => {
                                    match process_and_enrich_zones(Some(raw_zones), true) {
                                        Ok(zones) => final_supply_zones = zones,
                                        Err(e) => enrichment_error = Some(e),
                                    }
                                },
                                None => debug!("[Task {}/{}] No supply zones path.", current_symbol, current_timeframe),
                            }
                            // Process Demand Zones if no previous error
                            if enrichment_error.is_none() {
                                match data.get("demand_zones").and_then(|dz| dz.get("zones")).and_then(|z| z.as_array()) {
                                    Some(raw_zones) => {
                                        match process_and_enrich_zones(Some(raw_zones), false) {
                                            Ok(zones) => final_demand_zones = zones,
                                            Err(e) => enrichment_error = Some(e),
                                        }
                                    },
                                   None => debug!("[Task {}/{}] No demand zones path.", current_symbol, current_timeframe),
                                }
                            }
                        } else {
                            warn!("[Task {}/{}] Missing 'data.price' path in results: {:?}", current_symbol, current_timeframe, detection_results_json);
                            enrichment_error = Some("Missing 'data.price' structure.".to_string());
                        }

                        // --- Package Result (FILTERING HERE) ---
                        if let Some(err_msg) = enrichment_error {
                             // Return value for enrichment error case
                             BulkResultItem {
                                symbol: current_symbol, timeframe: current_timeframe,
                                status: "Error: Enrichment Failed".to_string(), data: None, error_message: Some(err_msg),
                             }
                        } else {
                            // Filter for active zones
                            info!("[Task {}/{}] Filtering for active zones...", current_symbol, current_timeframe);
                            let active_supply_zones = final_supply_zones
                                .into_iter()
                                .filter(|zone| zone.is_active) // Keep only active ones
                                .collect::<Vec<_>>();
                            let active_demand_zones = final_demand_zones
                                .into_iter()
                                .filter(|zone| zone.is_active) // Keep only active ones
                                .collect::<Vec<_>>();
                            info!("[Task {}/{}] Found {} active supply, {} active demand zones.",
                                current_symbol, current_timeframe, active_supply_zones.len(), active_demand_zones.len());

                            // Return value for success case, using FILTERED zones
                             BulkResultItem {
                                symbol: current_symbol, timeframe: current_timeframe, status: "Success".to_string(),
                                data: Some(BulkResultData {
                                    supply_zones: active_supply_zones,
                                    demand_zones: active_demand_zones
                                }),
                                error_message: None,
                            }
                        }
                    }, // End Ok arm
                    Err(core_err) => { // Handle CoreError
                        error!("[Task {}/{}] Core error: {}", current_symbol, current_timeframe, core_err);
                        let (status, msg) = match core_err {
                             CoreError::Config(e) => ("Error: Config Failed".to_string(), e),
                             CoreError::Request(e) => ("Error: Fetch Failed".to_string(), e.to_string()),
                             CoreError::Csv(e) => ("Error: Parse Failed".to_string(), e.to_string()),
                             CoreError::EnvVar(e) => ("Error: Server Config Failed".to_string(), e),
                        };
                         BulkResultItem { status, data: None, error_message: Some(msg), symbol: current_symbol, timeframe: current_timeframe }
                    } // End Err arm
                } // End match _fetch_and_detect_core
            }); // End of tokio::spawn closure
            tasks.push(task);
        } // End loop timeframes
    } // End loop symbols

    // --- Collect Task Results ---
    let task_results = join_all(tasks).await;
    let results: Vec<BulkResultItem> = task_results.into_iter().map(|res| {
        match res { Ok(item) => item, Err(e) => BulkResultItem{symbol:"Unknown".into(),timeframe:"Unknown".into(),status:"Error: Task Panic".into(),data:None,error_message:Some(format!("Task panicked: {}",e))}}
     }).collect();
    info!("[get_bulk_multi_tf] Completed processing {} combinations.", results.len());
    // --- Create Final Response ---
    let response = BulkActiveZonesResponse {
        results,
        query_params: Some(global_query.into_inner()),
    };
    HttpResponse::Ok().json(response)
}