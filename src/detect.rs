use log::{debug, error, info, warn};
use crate::patterns::{
    FiftyPercentBeforeBigBarRecognizer,
    PatternRecognizer,
};
use crate::types::{
    deserialize_raw_zone_value,
    BulkActiveZonesResponse,
    BulkResultData,
    BulkResultItem,
    BulkSymbolTimeframesRequestItem,
    EnrichedZone,
    SymbolZoneResponse,    // Imported from types.rs
    TimeframeZoneResponse, // Imported from types.rs
};
use actix_web::{web, HttpResponse, Responder};
use csv::ReaderBuilder;
use dotenv::dotenv;
use futures::future::join_all;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::error::Error;
use std::io::Cursor;

// --- Structs ---
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CandleData {
    pub time: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
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

#[derive(serde::Deserialize, Debug)]
pub struct SingleZoneQueryParams {
    symbol: String,
    timeframe: String,
    pattern: Option<String>,
    start_time: String,
    end_time: String,
}

#[derive(serde::Serialize)]
struct ErrorResponse {
    error: String,
}

// --- Custom Error Enum ---
#[derive(Debug)]
pub enum CoreError {
    EnvVar(String),
    Request(reqwest::Error),
    Csv(csv::Error),
    Config(String),
    Processing(String),
}
impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{:?}", self) }
}
impl Error for CoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> { None }
}
impl From<std::env::VarError> for CoreError {
    fn from(err: std::env::VarError) -> Self { CoreError::EnvVar(err.to_string()) }
}
impl From<reqwest::Error> for CoreError {
    fn from(err: reqwest::Error) -> Self { CoreError::Request(err) }
}
impl From<csv::Error> for CoreError {
    fn from(err: csv::Error) -> Self { CoreError::Csv(err) }
}

// --- INTERNAL CORE Logic Function ---
pub async fn _fetch_and_detect_core(
    query: &ChartQuery,
    caller_id: &str
) -> Result<(Vec<CandleData>, Box<dyn PatternRecognizer>, Value), CoreError> {
    info!("[{}] -> Called _fetch_and_detect_core for: {}/{} (Pattern: {})", caller_id, query.symbol, query.timeframe, query.pattern);
    dotenv().ok();

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
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response_text = client.post(&url).bearer_auth(&token).json(&serde_json::json!({"query": flux_query, "type": "flux"})).send().await?.text().await?;
    let mut candles = Vec::<CandleData>::new();
    if !response_text.trim().is_empty() {
        let cursor = Cursor::new(response_text.as_bytes());
        let mut rdr = ReaderBuilder::new().has_headers(true).flexible(true).from_reader(cursor);
        let headers = rdr.headers()?.clone();
        let get_idx = |name: &str| headers.iter().position(|h| h == name);
        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) = ( get_idx("_time"), get_idx("open"), get_idx("high"), get_idx("low"), get_idx("close"), get_idx("volume")) {
            for result in rdr.records() {
                match result {
                    Ok(record) => {
                        let parse_f64 = |idx: usize| record.get(idx).and_then(|v| v.parse::<f64>().ok());
                        let parse_u32 = |idx: usize| record.get(idx).and_then(|v| v.parse::<u32>().ok());
                        if record.len() > t_idx && record.len() > o_idx && record.len() > h_idx && record.len() > l_idx && record.len() > c_idx && record.len() > v_idx {
                            let time = record.get(t_idx).unwrap_or("").to_string();
                            if !time.is_empty() {
                                candles.push(CandleData { time, open: parse_f64(o_idx).unwrap_or(0.0), high: parse_f64(h_idx).unwrap_or(0.0), low: parse_f64(l_idx).unwrap_or(0.0), close: parse_f64(c_idx).unwrap_or(0.0), volume: parse_u32(v_idx).unwrap_or(0), });
                            }
                        } else { warn!("Skipping record with unexpected length: {}", record.len()); }
                    }
                    Err(e) => warn!("[_fetch_and_detect_core] CSV record error: {}", e),
                }
            }
        } else { return Err(CoreError::Config("CSV header mismatch".to_string())); }
    }
    info!("[_fetch_and_detect_core] Parsed {} candles for {}/{}", candles.len(), query.symbol, query.timeframe);

    let recognizer: Box<dyn PatternRecognizer> = match query.pattern.as_str() {
        "fifty_percent_before_big_bar" => Box::new(FiftyPercentBeforeBigBarRecognizer::default()),
        other_pattern => {
            warn!("Pattern '{}' is not currently configured in _fetch_and_detect_core match statement.", other_pattern);
            return Err(CoreError::Config(format!("Pattern '{}' is not supported by this endpoint.", other_pattern)));
        }
    };
    info!("[_fetch_and_detect_core] Using recognizer: {}", query.pattern);

    let detection_results = recognizer.detect(&candles);
    info!("[_fetch_and_detect_core] Detection complete.");
    Ok((candles, recognizer, detection_results))
}

// --- detect_patterns ---
pub async fn detect_patterns(query: web::Query<ChartQuery>) -> impl Responder {
    // Only import TradeConfig if trading logic is used
    use crate::trades::TradeConfig;

    info!("[detect_patterns] Handling /analyze request: {:?}", query);
    match _fetch_and_detect_core(&query, "detect_patterns").await {
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
                // No specific 'use' needed here for Trade/TradeSummary if recognizer.trade() returns them
                info!("[detect_patterns] Trading enabled.");
                let trade_config = TradeConfig {
                    enabled: true,
                    lot_size: query.lot_size.unwrap_or(0.01),
                    default_stop_loss_pips: query.stop_loss_pips.unwrap_or(20.0),
                    default_take_profit_pips: query.take_profit_pips.unwrap_or(40.0),
                    enable_trailing_stop: query.enable_trailing_stop.unwrap_or(false),
                    ..TradeConfig::default()
                };
                // Call requires `recognizer.trade` to be implemented correctly
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
                CoreError::Config(msg) if msg.starts_with("Pattern") && msg.ends_with("not supported by this endpoint.") => HttpResponse::BadRequest().json(ErrorResponse { error: msg }),
                CoreError::Config(msg) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Configuration error: {}", msg) }),
                CoreError::Request(rq_e) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Data source request error: {}", rq_e) }),
                CoreError::Csv(csv_e) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Data parsing error: {}", csv_e) }),
                CoreError::EnvVar(env_e) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Server configuration error: {}", env_e) }),
                CoreError::Processing(msg) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Processing error: {}", msg) }),
            }
        }
    }
}

// --- Helper function to check zone activity ---
pub fn is_zone_still_active(zone: &Value, candles: &[CandleData], is_supply: bool) -> bool {
    let start_idx = match zone.get("start_idx").and_then(|v| v.as_u64()) { Some(idx) => idx as usize, None => { warn!("is_zone_still_active: Missing 'start_idx'"); return false; } };
    let zone_high = match zone.get("zone_high").and_then(|v| v.as_f64()) { Some(val) if val.is_finite() => val, _ => { warn!("is_zone_still_active: Missing/invalid 'zone_high'"); return false; } };
    let zone_low = match zone.get("zone_low").and_then(|v| v.as_f64()) { Some(val) if val.is_finite() => val, _ => { warn!("is_zone_still_active: Missing/invalid 'zone_low'"); return false; } };
    let check_start_idx = start_idx.saturating_add(2);
    if check_start_idx >= candles.len() { return true; }
    for i in check_start_idx..candles.len() {
        let candle = &candles[i];
        if !candle.high.is_finite() || !candle.low.is_finite() || !candle.close.is_finite() { continue; }
        if is_supply { if candle.high > zone_high { return false; } } else { if candle.low < zone_low { return false; } }
    }
    true
}

// --- get_active_zones_handler ---
pub async fn get_active_zones_handler(query: web::Query<ChartQuery>) -> impl Responder {
    info!("[get_active_zones] Handling /active-zones request: {:?}", query);
    match _fetch_and_detect_core(&query, "get_active_zones_handler").await {
        Ok((candles, _recognizer, mut detection_results)) => {
            debug!("[get_active_zones] Result from _fetch_and_detect_core (before enrichment): Candles: {}, Results JSON: {}", candles.len(), serde_json::to_string(&detection_results).unwrap_or_else(|e| format!("Error serializing: {}", e)));
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
                    for zone_json in zones.iter_mut() {
                        let is_active = is_zone_still_active(zone_json, &candles, is_supply);
                        if let Some(obj) = zone_json.as_object_mut() {
                            obj.insert("is_active".to_string(), json!(is_active));
                            if let Some(start_idx) = obj.get("start_idx").and_then(|v| v.as_u64()) {
                                let zone_candles = candles.get(start_idx as usize..).unwrap_or(&[]).to_vec();
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
                 CoreError::Config(msg) if msg.starts_with("Pattern") && msg.ends_with("not supported by this endpoint.") => HttpResponse::BadRequest().json(ErrorResponse { error: msg }),
                CoreError::Config(msg) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Configuration error: {}", msg) }),
                CoreError::Request(rq_e) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Data source request error: {}", rq_e) }),
                CoreError::Csv(csv_e) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Data parsing error: {}", csv_e) }),
                CoreError::EnvVar(env_e) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Server configuration error: {}", env_e) }),
                CoreError::Processing(msg) => HttpResponse::InternalServerError().json(ErrorResponse { error: format!("Processing error: {}", msg) }),
            }
        }
    }
}

// --- get_bulk_multi_tf_active_zones_handler (Unchanged) ---
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
            let current_query = ChartQuery {
                symbol: current_symbol.clone(), timeframe: current_timeframe.clone(),
                start_time: global_query.start_time.clone(), end_time: global_query.end_time.clone(),
                pattern: global_query.pattern.clone(),
                enable_trading: None, lot_size: None, stop_loss_pips: None, take_profit_pips: None, enable_trailing_stop: None,
            };
            if current_query.pattern != "fifty_percent_before_big_bar" {
                warn!("[get_bulk_multi_tf] Skipping task for unsupported pattern '{}' for {}/{}", current_query.pattern, current_symbol, current_timeframe);
                tasks.push(tokio::spawn(async move { BulkResultItem { symbol: current_symbol, timeframe: current_timeframe, status: "Error: Pattern Not Supported".to_string(), data: None, error_message: Some(format!("Pattern '{}' not supported by this endpoint configuration.", current_query.pattern)), } }));
                continue;
            }
            let task = tokio::spawn(async move {
                let caller_task_id = format!("bulk_task_{}/{}", current_symbol, current_timeframe);
                info!("[Task {}/{}] Processing pattern '{}'...", current_symbol, current_timeframe, current_query.pattern);
                match _fetch_and_detect_core(&current_query, &caller_task_id).await {
                    Ok((candles, _recognizer, detection_results_json)) => {
                        if candles.is_empty() {
                            info!("[Task {}/{}] No candle data found.", current_symbol, current_timeframe);
                            return BulkResultItem { symbol: current_symbol, timeframe: current_timeframe, status: "No Data".to_string(), data: None, error_message: Some("No candle data found.".to_string()), };
                        }
                        info!("[Task {}/{}] Enriching results ({} candles)...", current_symbol, current_timeframe, candles.len());
                        let mut final_supply_zones: Vec<EnrichedZone> = Vec::new();
                        let mut final_demand_zones: Vec<EnrichedZone> = Vec::new();
                        let mut enrichment_error: Option<String> = None;
                        let candle_count = candles.len();
                        
                        let mut total_zones = 0;
                        let mut active_zones_count = 0;
                        let mut failed_deser_count = 0;
                        let mut invalid_index_count = 0;
                        
                        let mut process_and_enrich_zones = |zones_option: Option<&Vec<Value>>, is_supply: bool| -> Result<Vec<EnrichedZone>, String> {
                            let mut processed_zones: Vec<EnrichedZone> = Vec::new();
                            let zone_type = if is_supply { "supply_zones" } else { "demand_zones" };
                            
                            if let Some(zones) = zones_option {
                                let zone_count = zones.len();
                                total_zones += zone_count;
                                info!("[API] [Task {}/{}] Processing {} {} zones", current_symbol, current_timeframe, zone_count, zone_type);
                                
                                let mut local_active = 0;
                                let mut local_inactive = 0;
                                let mut local_deser_fail = 0;
                                let mut local_idx_invalid = 0;
                                
                                for zone_json in zones.iter() {
                                    let is_active = is_zone_still_active(zone_json, &candles, is_supply);
                                    
                                    if is_active {
                                        local_active += 1;
                                    } else {
                                        local_inactive += 1;
                                    }
                                    
                                    // Validate index
                                    let valid_index = zone_json.get("start_idx").and_then(|v| v.as_u64()).map(|start_idx| {
                                        if candle_count > start_idx as usize {
                                            true
                                        } else {
                                            local_idx_invalid += 1;
                                            invalid_index_count += 1;
                                            warn!("[API] [Task {}/{}] Invalid start_idx {} >= candle_count {}", 
                                                current_symbol, current_timeframe, start_idx, candle_count);
                                            false
                                        }
                                    }).unwrap_or(false);
                                    
                                    // Calculate bars_active only if index is valid
                                    let bars_active_duration: Option<u64> = if is_active && valid_index {
                                        zone_json.get("start_idx").and_then(|v| v.as_u64()).map(|start_idx| {
                                            candle_count as u64 - start_idx
                                        })
                                    } else { 
                                        None 
                                    };
                                    
                                    match deserialize_raw_zone_value(zone_json) {
                                        Ok(mut enriched_zone_struct) => {
                                            if enriched_zone_struct.zone_id.is_none() {
                                                info!("[API_ID_CHECK] Symbol: {}, TF: {}, TypeForID: '{}', StartTime: {:?}, High: {:?}, Low: {:?}, RawJsonType: {:?}",
                                                current_symbol,        // Log captured variable
                                                current_timeframe,     // Log captured variable
                                                // Determine type for ID *here* right before logging/using it:
                                                if zone_json.get("type").and_then(|v| v.as_str()).unwrap_or("").contains("supply") { "supply" } else { "demand" },
                                                enriched_zone_struct.start_time.as_deref(),      // Option<&str>
                                                enriched_zone_struct.zone_high,       // Option<f64>
                                                enriched_zone_struct.zone_low,        // Option<f64>
                                                zone_json.get("type").and_then(|v| v.as_str())); // Log raw type field
                                          // --- END OF INSERTED LOGGING BLOCK ---
                                                // --- INSERT THIS LINE ---
                                                let symbol_for_id = if current_symbol.ends_with("_SB") { current_symbol.clone() } else { format!("{}_SB", current_symbol) };
                                                // --- END INSERTION ---
                                                let zone_id = generate_deterministic_zone_id(
                                                    &symbol_for_id,
                                                    &current_timeframe,
                                                    if zone_json.get("type") // Check the type field in the raw JSON
                                                            .and_then(|v| v.as_str()) // Get it as a string slice
                                                            .unwrap_or("") // Use an empty string if missing
                                                            .contains("supply") // Check if the string CONTAINS "supply"
                                                    {
                                                        "supply" // If yes, use "supply"
                                                    } else {
                                                        "demand" // Otherwise, default to "demand"
                                                    },
                                                    
                                                    enriched_zone_struct.start_time.as_deref(),
                                                    enriched_zone_struct.zone_high,
                                                    enriched_zone_struct.zone_low
                                                );
                                                info!("[API_ID_CHECK] Zone_ID: {}", zone_id);
                                                enriched_zone_struct.zone_id = Some(zone_id);
                                            }
                                            enriched_zone_struct.is_active = is_active;
                                            enriched_zone_struct.bars_active = bars_active_duration;
                                            let calculated_touches = calculate_touches_within_range(
                                                zone_json, // Pass the raw JSON
                                                &candles,  // Pass the full candle slice
                                                is_supply, // Pass supply flag
                                                candle_count // Pass candle count
                                            );
                                            enriched_zone_struct.touch_count = Some(calculated_touches);
                                            enriched_zone_struct.strength_score = Some(100.0);
                                            enriched_zone_struct.is_active = is_active; // Set based on separate calculation
                                            enriched_zone_struct.bars_active = bars_active_duration;
                                            processed_zones.push(enriched_zone_struct);
                                        }
                                        Err(e) => { 
                                            local_deser_fail += 1;
                                            failed_deser_count += 1;
                                            error!("[API] [Task {}/{}] Deserialize failed: {}. Value: {:?}", 
                                                current_symbol, current_timeframe, e, zone_json); 
                                            // Don't return an error, just log it
                                        }
                                    }
                                }
                                
                                info!("[API] [Task {}/{}] {} zones - {} active, {} inactive, {} failed deser, {} invalid idx", 
                                    current_symbol, current_timeframe, zone_type, local_active, local_inactive, 
                                    local_deser_fail, local_idx_invalid);
                                active_zones_count += local_active;
                            }
                            
                            Ok(processed_zones)
                        };
                        if let Some(data) = detection_results_json.get("data").and_then(|d| d.get("price")) {
                             match data.get("supply_zones").and_then(|sz| sz.get("zones")).and_then(|z| z.as_array()) {
                                Some(raw_zones) => { match process_and_enrich_zones(Some(raw_zones), true) { Ok(zones) => final_supply_zones = zones, Err(e) => enrichment_error = Some(e), } }
                                None => debug!("[Task {}/{}] No supply zones path.", current_symbol, current_timeframe),
                             }
                             if enrichment_error.is_none() {
                                match data.get("demand_zones").and_then(|dz| dz.get("zones")).and_then(|z| z.as_array()) {
                                    Some(raw_zones) => { match process_and_enrich_zones(Some(raw_zones), false) { Ok(zones) => final_demand_zones = zones, Err(e) => enrichment_error = Some(e), } }
                                    None => debug!("[Task {}/{}] No demand zones path.", current_symbol, current_timeframe),
                                }
                             }
                        } else { 
                            warn!("[Task {}/{}] Missing 'data.price' path in results: {:?}", current_symbol, current_timeframe, detection_results_json); 
                            enrichment_error = Some("Missing 'data.price' structure.".to_string()); 
                        }
                        
                        // After processing both types of zones:
                        info!("[API] [Task {}/{}] SUMMARY: {} total zones, {} active zones, {} failed deserialization, {} invalid indices",
                            current_symbol, current_timeframe, total_zones, active_zones_count, failed_deser_count, invalid_index_count);
                        
                        if let Some(err_msg) = enrichment_error {
                             BulkResultItem { symbol: current_symbol, timeframe: current_timeframe, status: "Error: Enrichment Failed".to_string(), data: None, error_message: Some(err_msg), }
                        } else {
                            info!("[API] [Task {}/{}] Before filtering: {} supply zones, {} demand zones", 
                                current_symbol, current_timeframe, final_supply_zones.len(), final_demand_zones.len());
                            
                            let active_supply_zones = final_supply_zones.into_iter().filter(|zone| zone.is_active).collect::<Vec<_>>();
                            let active_demand_zones = final_demand_zones.into_iter().filter(|zone| zone.is_active).collect::<Vec<_>>();
                            
                            info!("[API] [Task {}/{}] After active filtering: {} active supply, {} active demand zones.", 
                                current_symbol, current_timeframe, active_supply_zones.len(), active_demand_zones.len());
                            
                            BulkResultItem { symbol: current_symbol, timeframe: current_timeframe, status: "Success".to_string(), data: Some(BulkResultData { supply_zones: active_supply_zones, demand_zones: active_demand_zones, }), error_message: None, }
                        }
                    }
                    Err(core_err) => {
                        error!("[Task {}/{}] Core error: {}", current_symbol, current_timeframe, core_err);
                        let (status, msg) = match core_err {
                            CoreError::Config(e) => ("Error: Config Failed".to_string(), e),
                            CoreError::Request(e) => { ("Error: Fetch Failed".to_string(), e.to_string()) }
                            CoreError::Csv(e) => ("Error: Parse Failed".to_string(), e.to_string()),
                            CoreError::EnvVar(e) => ("Error: Server Config Failed".to_string(), e),
                            CoreError::Processing(e) => ("Error: Processing Failed".to_string(), e),
                        };
                        BulkResultItem { status, data: None, error_message: Some(msg), symbol: current_symbol, timeframe: current_timeframe, }
                    }
                }
            });
            tasks.push(task);
        }
    }
    let task_results = join_all(tasks).await;
    let results: Vec<BulkResultItem> = task_results.into_iter().map(|res| match res {
        Ok(item) => item,
        Err(e) => BulkResultItem { symbol: "Unknown".into(), timeframe: "Unknown".into(), status: "Error: Task Panic".into(), data: None, error_message: Some(format!("Task panicked: {}", e)), },
    }).collect();
    
    // Add a summary count of all zones
    let total_active_zones: usize = results.iter()
        .filter_map(|item| item.data.as_ref().map(|data| data.supply_zones.len() + data.demand_zones.len()))
        .sum();
    
    info!("[API] FINAL SUMMARY: Completed processing {} combinations with {} total active zones returned.", 
        results.len(), total_active_zones);
    
    let response = BulkActiveZonesResponse { results, query_params: Some(global_query.into_inner()), symbols: HashMap::new() };
    HttpResponse::Ok().json(response)
}

// PASTE THIS ENTIRE FUNCTION AT THE END OF src/detect.rs

// New handler for debugging single symbol/timeframe using duplicated logic approach
pub async fn debug_bulk_zones_handler(
    query: web::Query<SingleZoneQueryParams>,
) -> impl Responder {
    // Extract query parameters
    let symbol = query.symbol.clone();
    let timeframe = query.timeframe.clone();
    let pattern = query.pattern.clone().unwrap_or_else(|| "fifty_percent_before_big_bar".to_string());
    let start_time = query.start_time.clone();
    let end_time = query.end_time.clone();

    info!("[debug_bulk_zones_handler] Debug request for Symbol: {}, Timeframe: {}, Pattern: {}, Start: {}, End: {}",
        symbol, timeframe, pattern, start_time, end_time);

    // Create the ChartQuery needed for _fetch_and_detect_core
    let core_query = ChartQuery {
        symbol: symbol.clone(),
        timeframe: timeframe.clone(),
        start_time,
        end_time,
        pattern,
        enable_trading: None, lot_size: None, stop_loss_pips: None,
        take_profit_pips: None, enable_trailing_stop: None,
    };

    let caller_id = format!("debug_task_{}/{}", symbol, timeframe);

    // --- Call the PRIVATE _fetch_and_detect_core (defined above in this file) ---
    match _fetch_and_detect_core(&core_query, &caller_id).await {
        Ok((candles, _recognizer, detection_results_json)) => {
            if candles.is_empty() {
                info!("[debug_bulk_zones_handler] No candle data found for {}/{}.", symbol, timeframe);
                 return HttpResponse::NotFound().json(ErrorResponse {
                    error: format!("No candle data found for Symbol: {}, Timeframe: {}", symbol, timeframe),
                });
            }

            // --- DUPLICATED Enrichment Logic ---
            info!("[debug_bulk_zones_handler] Enriching results for {}/{} ({} candles)...", symbol, timeframe, candles.len());
            let mut final_supply_zones: Vec<EnrichedZone> = Vec::new(); // Use crate::types:: path if needed
            let mut final_demand_zones: Vec<EnrichedZone> = Vec::new(); // Use crate::types:: path if needed
            let mut enrichment_error: Option<String> = None;
            let candle_count = candles.len();

            // This closure uses the PRIVATE is_zone_still_active (defined above in this file)
             let process_and_enrich_zones = |zones_option: Option<&Vec<Value>>, is_supply: bool| -> Result<Vec<EnrichedZone>, String> {
                 let mut processed_zones: Vec<EnrichedZone> = Vec::new();
                 if let Some(zones) = zones_option {
                     for zone_json in zones.iter() {
                         // CALL THE PRIVATE FUNCTION: is_zone_still_active
                         let is_active = is_zone_still_active(zone_json, &candles, is_supply);
                         let bars_active_duration: Option<u64> = if is_active { zone_json.get("start_idx").and_then(|v| v.as_u64()).and_then(|start_idx| { if candle_count > start_idx as usize { Some(candle_count as u64 - start_idx) } else { warn!("[debug] Invalid start_idx {} >= candle_count {}", start_idx, candle_count); None } }) } else { None };
                         // Call public function from types module
                         match crate::types::deserialize_raw_zone_value(zone_json) {
                             Ok(mut enriched_zone_struct) => {
                                 enriched_zone_struct.is_active = is_active;
                                 enriched_zone_struct.bars_active = bars_active_duration;
                                 processed_zones.push(enriched_zone_struct);
                             }
                             Err(e) => { error!("[debug] Deserialize failed: {}. Value: {:?}", e, zone_json); return Err(format!("Deserialize failed: {}", e)); }
                         }
                     }
                 }
                 Ok(processed_zones)
            };

            // Navigate results JSON
            if let Some(data) = detection_results_json.get("data").and_then(|d| d.get("price")) {
                 match data.get("supply_zones").and_then(|sz| sz.get("zones")).and_then(|z| z.as_array()) {
                    Some(raw_zones) => { match process_and_enrich_zones(Some(raw_zones), true) { Ok(zones) => final_supply_zones = zones, Err(e) => enrichment_error = Some(e), } }
                    None => debug!("[debug] No supply zones path for {}/{}.", symbol, timeframe),
                 }
                 if enrichment_error.is_none() {
                    match data.get("demand_zones").and_then(|dz| dz.get("zones")).and_then(|z| z.as_array()) {
                        Some(raw_zones) => { match process_and_enrich_zones(Some(raw_zones), false) { Ok(zones) => final_demand_zones = zones, Err(e) => enrichment_error = Some(e), } }
                        None => debug!("[debug] No demand zones path for {}/{}.", symbol, timeframe),
                    }
                 }
            } else { warn!("[debug] Missing 'data.price' path in results for {}/{}: {:?}", symbol, timeframe, detection_results_json); enrichment_error = Some("Missing 'data.price' structure.".to_string()); }

            // --- Package Result (FILTERING HERE) ---
            if let Some(err_msg) = enrichment_error {
                error!("[debug_bulk_zones_handler] Enrichment failed for {}/{}: {}", symbol, timeframe, err_msg);
                HttpResponse::InternalServerError().json(ErrorResponse {
                    error: format!("Enrichment Failed: {}", err_msg),
                })
            } else {
                 info!("[debug_bulk_zones_handler] Filtering for active zones for {}/{}...", symbol, timeframe);
                 let active_supply_zones = final_supply_zones.into_iter().filter(|zone| zone.is_active).collect::<Vec<_>>();
                 let active_demand_zones = final_demand_zones.into_iter().filter(|zone| zone.is_active).collect::<Vec<_>>();
                 info!("[debug_bulk_zones_handler] Found {} active supply, {} active demand zones for {}/{}.", active_supply_zones.len(), active_demand_zones.len(), symbol, timeframe);

                // Use structs from types.rs
                let timeframe_data = crate::types::TimeframeZoneResponse {
                    supply_zones: active_supply_zones,
                    demand_zones: active_demand_zones,
                };
                let symbol_data = crate::types::SymbolZoneResponse {
                    timeframes: HashMap::from([(timeframe.clone(), timeframe_data)]),
                };
                let mut response = crate::types::BulkActiveZonesResponse::default(); // Assumes Default derive
                response.symbols.insert(symbol.clone(), symbol_data);

                HttpResponse::Ok().json(response)
            }
        }
        Err(core_err) => { // Handle CoreError from this file's _fetch_and_detect_core
            error!("[debug_bulk_zones_handler] Core error for {}/{}: {}", symbol, timeframe, core_err);
            // Use local CoreError variants directly
            let (mut status_code, error_message) = match core_err {
                CoreError::Config(e) if e.starts_with("Pattern") => (HttpResponse::BadRequest(), e),
                CoreError::Config(e) => (HttpResponse::InternalServerError(), format!("Configuration Error: {}", e)),
                CoreError::Request(e) => (HttpResponse::InternalServerError(), format!("Data Fetch Error: {}", e)),
                CoreError::Csv(e) => (HttpResponse::InternalServerError(), format!("Data Parse Error: {}", e)),
                CoreError::EnvVar(e) => (HttpResponse::InternalServerError(), format!("Server Config Error: {}", e)),
                CoreError::Processing(e) => (HttpResponse::InternalServerError(), format!("Processing Error: {}", e)),
            };
            status_code.json(ErrorResponse { error: error_message })
        }
    }
}

pub fn generate_deterministic_zone_id(
    symbol: &str,
    timeframe: &str,
    zone_type: &str,
    start_time: Option<&str>,
    zone_high: Option<f64>,
    zone_low: Option<f64>,
) -> String {
    use sha2::{Digest, Sha256};

    const PRECISION: usize = 6; // Or your desired precision
    let high_str = zone_high.map_or("0.0".to_string(), |v| format!("{:.prec$}", v, prec = PRECISION));
    let low_str = zone_low.map_or("0.0".to_string(), |v| format!("{:.prec$}", v, prec = PRECISION));
    
    // Create a unique ID based on the zone's key properties
    let id_input = format!("{}_{}_{}_{}_{}_{}", 
        symbol, 
        timeframe, 
        zone_type,
        start_time.unwrap_or("unknown"),
        high_str, // Use formatted float string
        low_str   // Use formatted float string
    );
    
    let mut hasher = Sha256::new();
    hasher.update(id_input.as_bytes());
    let result = hasher.finalize();
    let hex_id = format!("{:x}", result);
    
    // Use the first 16 chars of the hash for the ID
    hex_id[..16].to_string()
}

/// Calculates the number of times a zone was "touched" by candles *after* its formation,
/// within a given candle set. Stops counting after the zone is invalidated.
/// NOTE: This only counts touches within the provided `candles` slice.
pub fn calculate_touches_within_range(
    zone_json: &Value,        // Raw JSON to get indices and levels reliably
    candles: &[CandleData],   // The slice of candles to check against
    is_supply: bool,
    candle_count: usize,      // Pass candle_count for efficiency
) -> i64 { // Return i64 to match DB type eventually

    // --- Get necessary values from the zone JSON ---
    // Use end_idx if available and reliable, otherwise fallback needs care
    let end_idx = match zone_json.get("end_idx").and_then(|v| v.as_u64()) {
        Some(idx) => idx as usize,
        None => {
            warn!("calculate_touches: Missing 'end_idx' in zone: {:?}. Cannot calculate touches.", zone_json);
            return 0; // Cannot calculate without end index
        }
    };
    let zone_high = match zone_json.get("zone_high").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => { warn!("calculate_touches: Missing/invalid 'zone_high' in zone: {:?}", zone_json); return 0; }
    };
    let zone_low = match zone_json.get("zone_low").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => { warn!("calculate_touches: Missing/invalid 'zone_low' in zone: {:?}", zone_json); return 0; }
    };
    // --- End Get necessary values ---


    let mut touch_count: i64 = 0;
    let mut is_still_valid = true; // Track if zone has been invalidated

    // Start checking candles *after* the zone formation (end_idx + 1)
    let check_start_idx = end_idx + 1;

    if check_start_idx >= candle_count {
        return 0; // No candles after formation in this range
    }

    for i in check_start_idx..candle_count {
        let candle = &candles[i];

        // Skip candles with bad data
        if !candle.high.is_finite() || !candle.low.is_finite() {
            continue;
        }

        // 1. Check for Invalidation FIRST
        if is_supply {
            // Supply invalidated if high breaks above zone high
            if candle.high > zone_high {
                is_still_valid = false;
                // debug!("calculate_touches: Supply zone invalidated at index {} (High: {} > Zone High: {})", i, candle.high, zone_high);
                break; // Stop counting touches after invalidation
            }
        } else {
            // Demand invalidated if low breaks below zone low
            if candle.low < zone_low {
                is_still_valid = false;
                // debug!("calculate_touches: Demand zone invalidated at index {} (Low: {} < Zone Low: {})", i, candle.low, zone_low);
                break; // Stop counting touches after invalidation
            }
        }

        // 2. If still valid, check for a Touch
        if is_still_valid {
            if is_supply {
                // Supply Touch: High enters zone (>= low) but does NOT break high (already checked above)
                if candle.high >= zone_low {
                    touch_count += 1;
                    // debug!("calculate_touches: Supply touch detected at index {}", i);
                }
            } else {
                // Demand Touch: Low enters zone (<= high) but does NOT break low (already checked above)
                if candle.low <= zone_high {
                    touch_count += 1;
                    // debug!("calculate_touches: Demand touch detected at index {}", i);
                }
            }
        }
    } // End loop through subsequent candles

    touch_count
}