// src/detect.rs
use crate::zones::patterns::{FiftyPercentBeforeBigBarRecognizer, PatternRecognizer};
use crate::types::{
    // Your existing, working types:
    // deserialize_raw_zone_value,
    BulkActiveZonesResponse,
    BulkResultData,
    BulkResultItem,
    BulkSymbolTimeframesRequestItem,
    EnrichedZone,
    SymbolZoneResponse,
    TimeframeZoneResponse,
    TouchPoint,

    // --- ADDED FOR THE NEW ENDPOINT ---
    FindAndVerifyZoneQueryParams,
    FindAndVerifyZoneResult,
    FindAndVerifyZoneQueryParamsSerializable,
    FoundZoneActivityDetails,
    // --- END ADDED ---
};

use actix_web::{web, HttpResponse, Responder};
use csv::ReaderBuilder;
use dotenv::dotenv;
use futures::future::join_all;
use log::{debug, info, warn};
use reqwest;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::error::Error;
use std::io::Cursor;

use std::fs::OpenOptions;
use std::io::Write;

use crate::zones::zone_detection::{ZoneDetectionEngine, ZoneDetectionRequest, InfluxDataFetcher};

fn debug_to_file(message: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("zone_debug.log")
        .expect("Failed to open debug log file");
    
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
    writeln!(file, "[{}] {}", timestamp, message).expect("Failed to write to debug log");
}


// --- Structs ---
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct CandleData {
     #[serde(rename = "_time")]
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
    #[serde(default)]
    pub max_touch_count: Option<i64>,
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl Error for CoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
impl From<std::env::VarError> for CoreError {
    fn from(err: std::env::VarError) -> Self {
        CoreError::EnvVar(err.to_string())
    }
}
impl From<reqwest::Error> for CoreError {
    fn from(err: reqwest::Error) -> Self {
        CoreError::Request(err)
    }
}
impl From<csv::Error> for CoreError {
    fn from(err: csv::Error) -> Self {
        CoreError::Csv(err)
    }
}

// --- INTERNAL CORE Logic Function ---
pub async fn _fetch_and_detect_core(
    query: &ChartQuery,
    caller_id: &str,
) -> Result<(Vec<CandleData>, Box<dyn PatternRecognizer>, Value), CoreError> {
    info!(
        "[{}] -> Called _fetch_and_detect_core for: {}/{} (Pattern: {})",
        caller_id, query.symbol, query.timeframe, query.pattern
    );
    dotenv().ok();

    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG")?;
    let token = std::env::var("INFLUXDB_TOKEN")?;
    let bucket = std::env::var("INFLUXDB_BUCKET")?;
    let symbol = if query.symbol.ends_with("_SB") {
        query.symbol.clone()
    } else {
        format!("{}_SB", query.symbol)
    };
    let timeframe = query.timeframe.to_lowercase();
    let flux_query = format!(
        r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r["_measurement"] == "trendbar") |> filter(fn: (r) => r["symbol"] == "{}") |> filter(fn: (r) => r["timeframe"] == "{}") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
        bucket, query.start_time, query.end_time, symbol, timeframe
    );
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response_text = client
        .post(&url)
        .bearer_auth(&token)
        .json(&serde_json::json!({"query": flux_query, "type": "flux"}))
        .send()
        .await?
        .text()
        .await?;
    let mut candles = Vec::<CandleData>::new();
    if !response_text.trim().is_empty() {
        let cursor = Cursor::new(response_text.as_bytes());
        let mut rdr = ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(cursor);
        let headers = rdr.headers()?.clone();
        let get_idx = |name: &str| headers.iter().position(|h| h == name);
        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) = (
            get_idx("_time"),
            get_idx("open"),
            get_idx("high"),
            get_idx("low"),
            get_idx("close"),
            get_idx("volume"),
        ) {
            for result in rdr.records() {
                match result {
                    Ok(record) => {
                        let parse_f64 =
                            |idx: usize| record.get(idx).and_then(|v| v.parse::<f64>().ok());
                        let parse_u32 =
                            |idx: usize| record.get(idx).and_then(|v| v.parse::<u32>().ok());
                        if record.len() > t_idx
                            && record.len() > o_idx
                            && record.len() > h_idx
                            && record.len() > l_idx
                            && record.len() > c_idx
                            && record.len() > v_idx
                        {
                            let time = record.get(t_idx).unwrap_or("").to_string();
                            if !time.is_empty() {
                                candles.push(CandleData {
                                    time,
                                    open: parse_f64(o_idx).unwrap_or(0.0),
                                    high: parse_f64(h_idx).unwrap_or(0.0),
                                    low: parse_f64(l_idx).unwrap_or(0.0),
                                    close: parse_f64(c_idx).unwrap_or(0.0),
                                    volume: parse_u32(v_idx).unwrap_or(0),
                                });
                            }
                        } else {
                            warn!("Skipping record with unexpected length: {}", record.len());
                        }
                    }
                    Err(e) => warn!("[_fetch_and_detect_core] CSV record error: {}", e),
                }
            }
        } else {
            return Err(CoreError::Config("CSV header mismatch".to_string()));
        }
    }
    info!(
        "[_fetch_and_detect_core] Parsed {} candles for {}/{}",
        candles.len(),
        query.symbol,
        query.timeframe
    );

    let recognizer: Box<dyn PatternRecognizer> = match query.pattern.as_str() {
        "fifty_percent_before_big_bar" => Box::new(FiftyPercentBeforeBigBarRecognizer::default()),
        other_pattern => {
            warn!("Pattern '{}' is not currently configured in _fetch_and_detect_core match statement.", other_pattern);
            return Err(CoreError::Config(format!(
                "Pattern '{}' is not supported by this endpoint.",
                other_pattern
            )));
        }
    };
    info!(
        "[_fetch_and_detect_core] Using recognizer: {}",
        query.pattern
    );

    let detection_results = recognizer.detect(&candles);
    info!("[_fetch_and_detect_core] Detection complete.");
    Ok((candles, recognizer, detection_results))
}

// --- detect_patterns ---
pub async fn detect_patterns(query: web::Query<ChartQuery>) -> impl Responder {
    // Only import TradeConfig if trading logic is used
    use crate::trading::trades::TradeConfig;

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
            debug_to_file(&format!("[detect_patterns] Core error: {}", e));
            match e {
                CoreError::Config(msg)
                    if msg.starts_with("Pattern")
                        && msg.ends_with("not supported by this endpoint.") =>
                {
                    HttpResponse::BadRequest().json(ErrorResponse { error: msg })
                }
                CoreError::Config(msg) => HttpResponse::InternalServerError().json(ErrorResponse {
                    error: format!("Configuration error: {}", msg),
                }),
                CoreError::Request(rq_e) => {
                    HttpResponse::InternalServerError().json(ErrorResponse {
                        error: format!("Data source request error: {}", rq_e),
                    })
                }
                CoreError::Csv(csv_e) => HttpResponse::InternalServerError().json(ErrorResponse {
                    error: format!("Data parsing error: {}", csv_e),
                }),
                CoreError::EnvVar(env_e) => {
                    HttpResponse::InternalServerError().json(ErrorResponse {
                        error: format!("Server configuration error: {}", env_e),
                    })
                }
                CoreError::Processing(msg) => {
                    HttpResponse::InternalServerError().json(ErrorResponse {
                        error: format!("Processing error: {}", msg),
                    })
                }
            }
        }
    }
}

// --- Helper function to check zone activity ---
pub fn is_zone_still_active(zone: &Value, candles: &[CandleData], is_supply: bool) -> bool {
    // --- START: Enhanced Logging ---
    debug_to_file(&format!("[DETECT_IS_ACTIVE_FORCED_LOG] CALLED! Zone RawStartTime: {:?}, IsSupply: {}. Candles provided: {}",
        zone.get("start_time").and_then(|v| v.as_str()),
        is_supply,
        candles.len()
    ));
    let zone_id_for_log = zone.get("zone_id") // Try to get generated ID if already present (e.g. from bulk)
        .or_else(|| zone.get("start_time")) // Fallback to start_time for identification
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN_ZONE_ID_OR_START_TIME");

    let source_detection_method = zone.get("detection_method").and_then(|v|v.as_str()).unwrap_or("N/A");

    // Log the call and key parameters
    log::info!(
        "[IS_ZONE_ACTIVE_CHECK] Called for Zone (ID/StartTime: '{}', Method: '{}'), IsSupply: {}. Total candles provided for check: {}.",
        zone_id_for_log,
        source_detection_method,
        is_supply,
        candles.len()
    );
    if candles.len() > 0 {
        log::info!(
            "[IS_ZONE_ACTIVE_CHECK] Candles range from {} to {}",
            candles.first().unwrap().time,
            candles.last().unwrap().time
        );
    }
    // --- END: Enhanced Logging ---

    let start_idx = match zone.get("start_idx").and_then(|v| v.as_u64()) {
        Some(idx) => idx as usize,
        None => {
            warn!("[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}'): Missing 'start_idx'. Marking INACTIVE.", zone_id_for_log);
            return false;
        }
    };
    let zone_high = match zone.get("zone_high").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => {
            warn!("[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}'): Missing/invalid 'zone_high'. Marking INACTIVE.", zone_id_for_log);
            return false;
        }
    };
    let zone_low = match zone.get("zone_low").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => {
            warn!("[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}'): Missing/invalid 'zone_low'. Marking INACTIVE.", zone_id_for_log);
            return false;
        }
    };

    // The "fifty_candle" is at `candles[start_idx]`
    // The "big_bar" (which completes the zone formation) is at `candles[start_idx + 1]`
    // So, activity check should start from the candle *after* the big_bar: `start_idx + 2`
    let check_start_idx = start_idx.saturating_add(2);

    // Log zone boundaries and where checking starts
    let formation_fifty_candle_time = candles.get(start_idx).map_or("N/A_IDX", |c| c.time.as_str());
    let formation_big_bar_time = candles.get(start_idx + 1).map_or("N/A_IDX", |c| c.time.as_str());
    log::info!(
        "[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}'): FiftyCandleTime: {}, BigBarTime: {}. ZoneDef [Low: {:.5}, High: {:.5}]. Activity check starts at index: {} (Time: {}).",
        zone_id_for_log,
        formation_fifty_candle_time,
        formation_big_bar_time,
        zone_low,
        zone_high,
        check_start_idx,
        candles.get(check_start_idx).map_or("N/A_IDX_PAST_END", |c| c.time.as_str())
    );


    if check_start_idx >= candles.len() {
        log::info!("[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}'): No candles available after formation pattern (check_start_idx {} >= candles.len {}). Zone remains ACTIVE.",
            zone_id_for_log, check_start_idx, candles.len());
        return true; // No candles after formation to invalidate it
    }

    for i in check_start_idx..candles.len() {
        let candle = &candles[i];
        if !candle.high.is_finite() || !candle.low.is_finite() || !candle.close.is_finite() {
            warn!("[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}'): Subsequent candle #{} (Time: {}) has non-finite OHLC values. Skipping this candle for check.",
                zone_id_for_log, i, candle.time);
            continue;
        }

        // Verbose log for each candle being checked against the zone
        log::trace!(
            "[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}'): Checking subsequent candle #{} (Time: {}): Open={:.5}, High={:.5}, Low={:.5}, Close={:.5}",
            zone_id_for_log, i, candle.time, candle.open, candle.high, candle.low, candle.close
        );

        if is_supply {
            if candle.high > zone_high {
                log::info!(
                    "[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}', Type: SUPPLY): INVALIDATED by candle #{} (Time: {}). Candle_High ({:.5}) > Zone_High ({:.5}). Marking INACTIVE.",
                    zone_id_for_log, i, candle.time, candle.high, zone_high
                );
                return false;
            }
        } else { // is_demand
            if candle.low < zone_low {
                log::info!(
                    "[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}', Type: DEMAND): INVALIDATED by candle #{} (Time: {}). Candle_Low ({:.5}) < Zone_Low ({:.5}). Marking INACTIVE.",
                    zone_id_for_log, i, candle.time, candle.low, zone_low
                );
                return false;
            }
        }
    }

    log::info!(
        "[IS_ZONE_ACTIVE_CHECK] Zone (ID/StartTime: '{}'): Remained valid through all {} subsequent candles checked. Marking ACTIVE.",
        zone_id_for_log, candles.len().saturating_sub(check_start_idx)
    );
    return true;
}

// --- get_active_zones_handler ---
pub async fn get_active_zones_handler(query: web::Query<ChartQuery>) -> impl Responder {
    info!(
        "[get_active_zones] Handling /active-zones request: {:?}",
        query
    );
     let max_touch_count = query.max_touch_count;
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

            info!(
                "[get_active_zones] Enriching detection results ({} candles) with max_touch_count filter: {:?}...",
                candles.len(),
                max_touch_count
            );
            
            let process_zones = |zones_option: Option<&mut Vec<Value>>, is_supply: bool| {
                if let Some(zones) = zones_option {
                    // First enrich all zones with activity status and candles
                    for zone_json in zones.iter_mut() {
                        let is_active = is_zone_still_active(zone_json, &candles, is_supply);
                        if let Some(obj) = zone_json.as_object_mut() {
                            obj.insert("is_active".to_string(), json!(is_active));
                            if let Some(start_idx) = obj.get("start_idx").and_then(|v| v.as_u64()) {
                                let zone_candles =
                                    candles.get(start_idx as usize..).unwrap_or(&[]).to_vec();
                                obj.insert("candles".to_string(), json!(zone_candles));
                            }
                        }
                    }
                    
                    // Apply touch count filtering if specified
                    if let Some(max_touches) = max_touch_count {
                        let initial_count = zones.len();
                        zones.retain(|zone_json| {
                            if let Some(touch_count) = zone_json.get("touch_count").and_then(|v| v.as_i64()) {
                                let should_keep = touch_count <= max_touches;
                                if !should_keep {
                                    debug!(
                                        "[get_active_zones] Filtering out zone with touch_count: {} > max: {} (zone_id: {:?})",
                                        touch_count,
                                        max_touches,
                                        zone_json.get("zone_id")
                                    );
                                }
                                should_keep
                            } else {
                                // If touch_count is missing, assume it's 0 and keep the zone
                                debug!(
                                    "[get_active_zones] Zone missing touch_count, keeping zone_id: {:?}",
                                    zone_json.get("zone_id")
                                );
                                true
                            }
                        });
                        let filtered_count = zones.len();
                        info!(
                            "[get_active_zones] Touch count filtering: {} -> {} zones (max_touches: {})",
                            initial_count, filtered_count, max_touches
                        );
                    }
                }
            };
            
            if let Some(data) = detection_results
                .get_mut("data")
                .and_then(|d| d.get_mut("price"))
            {
                process_zones(
                    data.get_mut("supply_zones")
                        .and_then(|sz| sz.get_mut("zones"))
                        .and_then(|z| z.as_array_mut()),
                    true,
                );
                process_zones(
                    data.get_mut("demand_zones")
                        .and_then(|dz| dz.get_mut("zones"))
                        .and_then(|z| z.as_array_mut()),
                    false,
                );
            } else {
                warn!(
                    "[get_active_zones] Could not find 'data.price...zones' to enrich in {:?}",
                    detection_results
                );
            }
            
            info!("[get_active_zones] Enrichment complete.");
            HttpResponse::Ok().json(detection_results)
        }
        Err(e) => {
            debug_to_file(&format!("[get_active_zones] Core error: {}", e));
            match e {
                CoreError::Config(msg)
                    if msg.starts_with("Pattern")
                        && msg.ends_with("not supported by this endpoint.") =>
                {
                    HttpResponse::BadRequest().json(ErrorResponse { error: msg })
                }
                CoreError::Config(msg) => HttpResponse::InternalServerError().json(ErrorResponse {
                    error: format!("Configuration error: {}", msg),
                }),
                CoreError::Request(rq_e) => {
                    HttpResponse::InternalServerError().json(ErrorResponse {
                        error: format!("Data source request error: {}", rq_e),
                    })
                }
                CoreError::Csv(csv_e) => HttpResponse::InternalServerError().json(ErrorResponse {
                    error: format!("Data parsing error: {}", csv_e),
                }),
                CoreError::EnvVar(env_e) => {
                    HttpResponse::InternalServerError().json(ErrorResponse {
                        error: format!("Server configuration error: {}", env_e),
                    })
                }
                CoreError::Processing(msg) => {
                    HttpResponse::InternalServerError().json(ErrorResponse {
                        error: format!("Processing error: {}", msg),
                    })
                }
            }
        }
    }
}

pub async fn get_bulk_multi_tf_active_zones_handler(
    global_query: web::Query<ChartQuery>,
    items: web::Json<Vec<BulkSymbolTimeframesRequestItem>>,
) -> impl Responder {
    let request_items = items.into_inner();
    
    debug_to_file(&format!("🚀 [BULK_HANDLER] ENTRY: {} symbol requests", request_items.len()));
    
    info!(
        "[get_bulk_multi_tf] Handling request for {} symbol entries. Query: {:?}",
        request_items.len(),
        global_query
    );
    
    // Create shared components
    let zone_engine = ZoneDetectionEngine::new();
    let data_fetcher = match InfluxDataFetcher::new() {
        Ok(fetcher) => fetcher,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to initialize data fetcher: {}", e),
            });
        }
    };
    
    let mut tasks = Vec::new();
    
    for item in request_items {
        let symbol_for_tasks = item.symbol.clone();
        for timeframe in item.timeframes {
            let current_symbol = symbol_for_tasks.clone();
            let current_timeframe = timeframe.clone();
            
            // Clone dependencies for the task
            let engine = zone_engine.clone(); // If you make ZoneDetectionEngine Clone
            let fetcher = data_fetcher.clone(); // If you make InfluxDataFetcher Clone
            let query = global_query.clone();
            
            let task = tokio::spawn(async move {
                let caller_task_id = format!("bulk_task_{}/{}", current_symbol, current_timeframe);
                
                debug_to_file(&format!("📋 [TASK_START] {}/{} - Fetching candles", current_symbol, current_timeframe));
                
                // Fetch candles using shared fetcher
                let candles = match fetcher.fetch_candles(
                    &current_symbol,
                    &current_timeframe,
                    &query.start_time,
                    &query.end_time,
                ).await {
                    Ok(candles) => candles,
                    Err(e) => {
                        debug_to_file(&format!("📋 [TASK_ERROR] {}/{} - Fetch failed: {}", current_symbol, current_timeframe, e));
                        return BulkResultItem {
                            symbol: current_symbol,
                            timeframe: current_timeframe,
                            status: "Error: Data Fetch Failed".to_string(),
                            data: None,
                            error_message: Some(format!("Failed to fetch candles: {}", e)),
                        };
                    }
                };
                
                if candles.is_empty() {
                    debug_to_file(&format!("📋 [TASK_EMPTY] {}/{} - No candle data", current_symbol, current_timeframe));
                    return BulkResultItem {
                        symbol: current_symbol,
                        timeframe: current_timeframe,
                        status: "No Data".to_string(),
                        data: None,
                        error_message: Some("No candle data found.".to_string()),
                    };
                }
                
                debug_to_file(&format!("📋 [TASK_DETECT] {}/{} - Running zone detection on {} candles", 
                       current_symbol, current_timeframe, candles.len()));
                
                // Run zone detection using shared engine
                let detection_request = ZoneDetectionRequest {
                    symbol: current_symbol.clone(),
                    timeframe: current_timeframe.clone(),
                    pattern: query.pattern.clone(),
                    candles,
                    max_touch_count: query.max_touch_count,
                };
                
                match engine.detect_zones(detection_request) {
                    Ok(result) => {
                        // Filter for active zones only (to match existing behavior)
                        let active_supply_zones = result.supply_zones.into_iter()
                            .filter(|zone| zone.is_active)
                            .collect::<Vec<_>>();
                        let active_demand_zones = result.demand_zones.into_iter()
                            .filter(|zone| zone.is_active)
                            .collect::<Vec<_>>();
                        
                        debug_to_file(&format!("📋 [TASK_SUCCESS] {}/{} - Returning {} supply, {} demand zones", 
                               current_symbol, current_timeframe, active_supply_zones.len(), active_demand_zones.len()));
                        
                        BulkResultItem {
                            symbol: current_symbol,
                            timeframe: current_timeframe,
                            status: "Success".to_string(),
                            data: Some(BulkResultData {
                                supply_zones: active_supply_zones,
                                demand_zones: active_demand_zones,
                            }),
                            error_message: None,
                        }
                    }
                    Err(e) => {
                        debug_to_file(&format!("📋 [TASK_DETECT_ERROR] {}/{} - Detection failed: {}", 
                               current_symbol, current_timeframe, e));
                        BulkResultItem {
                            symbol: current_symbol,
                            timeframe: current_timeframe,
                            status: "Error: Zone Detection Failed".to_string(),
                            data: None,
                            error_message: Some(format!("Zone detection failed: {}", e)),
                        }
                    }
                }
            });
            
            tasks.push(task);
        }
    }
    
    // Wait for all tasks to complete
    let task_results = join_all(tasks).await;
    let results: Vec<BulkResultItem> = task_results
        .into_iter()
        .map(|res| match res {
            Ok(item) => item,
            Err(e) => {
                debug_to_file(&format!("🚀 [BULK_HANDLER] Task panicked: {}", e));
                BulkResultItem {
                    symbol: "Unknown".into(),
                    timeframe: "Unknown".into(),
                    status: "Error: Task Panic".into(),
                    data: None,
                    error_message: Some(format!("Task panicked: {}", e)),
                }
            }
        })
        .collect();
    
    let total_active_zones: usize = results
        .iter()
        .filter_map(|item| item.data.as_ref().map(|data| data.supply_zones.len() + data.demand_zones.len()))
        .sum();
    
    debug_to_file(&format!("🚀 [BULK_HANDLER] FINAL: {} combinations processed, {} total active zones", 
           results.len(), total_active_zones));
    
    let response = BulkActiveZonesResponse {
        results,
        query_params: Some(global_query.into_inner()),
        symbols: HashMap::new(),
    };
    
    HttpResponse::Ok().json(response)
}


// PASTE THIS ENTIRE FUNCTION AT THE END OF src/detect.rs

// New handler for debugging single symbol/timeframe using duplicated logic approach
// PASTE THIS CORRECTED FUNCTION INTO src/detect.rs, replacing the previous debug handler

pub async fn debug_bulk_zones_handler(query: web::Query<SingleZoneQueryParams>) -> impl Responder {
    // Extract query parameters
    let symbol = query.symbol.clone();
    let timeframe = query.timeframe.clone();
    let pattern = query
        .pattern
        .clone()
        .unwrap_or_else(|| "fifty_percent_before_big_bar".to_string());
    let start_time = query.start_time.clone();
    let end_time = query.end_time.clone();

    info!("[debug_bulk_zones] Debug request for Symbol: {}, Timeframe: {}, Pattern: {}, Start: {}, End: {}",
        symbol, timeframe, pattern, start_time, end_time);

    // Ensure symbol has _SB suffix if needed by core logic
    let core_symbol = if symbol.ends_with("_SB") {
        symbol.clone()
    } else {
        format!("{}_SB", symbol)
    };

    // Create the ChartQuery needed for _fetch_and_detect_core
    let core_query = ChartQuery {
        symbol: core_symbol.clone(), // Use potentially suffixed symbol
        timeframe: timeframe.clone(),
        start_time,
        end_time,
        pattern,
        enable_trading: None,
        lot_size: None,
        stop_loss_pips: None,
        take_profit_pips: None,
        enable_trailing_stop: None,
        max_touch_count: None,
    };

    // Check pattern support early
    if core_query.pattern != "fifty_percent_before_big_bar" {
        warn!(
            "[debug_bulk_zones] Unsupported pattern '{}' requested.",
            core_query.pattern
        );
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: format!(
                "Pattern '{}' not supported by this endpoint configuration.",
                core_query.pattern
            ),
        });
    }

    let caller_id = format!("debug_task_{}/{}", symbol, timeframe);

    // --- Call the PRIVATE _fetch_and_detect_core ---
    match _fetch_and_detect_core(&core_query, &caller_id).await {
        Ok((candles, _recognizer, detection_results_json)) => {
            if candles.is_empty() {
                info!(
                    "[debug_bulk_zones] No candle data found for {}/{}.",
                    symbol, timeframe
                );
                return HttpResponse::NotFound().json(ErrorResponse {
                    error: format!(
                        "No candle data found for Symbol: {}, Timeframe: {}",
                        symbol, timeframe
                    ),
                });
            }

            // --- FIXED Enrichment Logic ---
            info!(
                "[debug_bulk_zones] Enriching results for {}/{} ({} candles)...",
                symbol, timeframe, candles.len()
            );
            let mut final_supply_zones: Vec<EnrichedZone> = Vec::new();
            let mut final_demand_zones: Vec<EnrichedZone> = Vec::new();
            let mut enrichment_error: Option<String> = None;
            let candle_count = candles.len();
            let last_candle_timestamp = candles.last().map(|c| c.time.clone());

            // --- FIXED Enrichment Closure ---
            let process_and_enrich_zones =
                |zones_option: Option<&Vec<Value>>,
                 is_supply: bool,
                 last_candle_time_for_zone: Option<String>|
                 -> Result<Vec<EnrichedZone>, String> {
                    let mut processed_zones: Vec<EnrichedZone> = Vec::new();
                    let zone_type_str = if is_supply { "supply" } else { "demand" };
                    let mut zone_counter = 0;

                    if let Some(zones) = zones_option {
                        info!(
                            "[Task {}/{}] Processing {} {} zones found by recognizer.",
                            symbol, timeframe,
                            zones.len(),
                            zone_type_str
                        );
                        for zone_json in zones.iter() {
                            zone_counter += 1;
                            // --- 1. Deserialize the base zone ---
                            let mut enriched_zone_struct =
                                match crate::types::deserialize_raw_zone_value(zone_json) {
                                    Ok(mut z) => {
                                        if z.zone_id.is_none() {
                                            let symbol_for_id = &core_symbol;
                                            let zone_type_for_id = if is_supply { "supply" } else { "demand" };
                                            let id = generate_deterministic_zone_id(
                                                symbol_for_id,
                                                &timeframe,
                                                zone_type_for_id,
                                                z.start_time.as_deref(),
                                                z.zone_high,
                                                z.zone_low,
                                            );
                                            info!("[Task {}/{}] Generated Zone ID {} for {} zone #{}", symbol, timeframe, id, zone_type_str, zone_counter);
                                            z.zone_id = Some(id);
                                        }
                                        if z.zone_type.is_none() {
                                            z.zone_type = Some(format!("{}_zone", zone_type_str));
                                        }
                                        z
                                    }
                                    Err(e) => {
                                        debug_to_file(&format!("[Task {}/{}] Deserialize failed for {} zone #{}: {}. Value: {:?}", symbol, timeframe, zone_type_str, zone_counter, e, zone_json));
                                        continue; // Skip this zone if deserialization fails
                                    }
                                };
                            
                            enriched_zone_struct.last_data_candle = last_candle_time_for_zone.clone();
                            let zone_id_log = enriched_zone_struct.zone_id.as_deref().unwrap_or("UNKNOWN");

                            // --- 2. Get Essential Info for Tracking ---
                            let formation_end_idx = match enriched_zone_struct.start_idx {
                                Some(start_idx) => start_idx as usize + 1,
                                None => {
                                    warn!("[Task {}/{}] {} zone {} (ID: {}) missing 'end_idx'. Skipping.", symbol, timeframe, zone_type_str, zone_counter, zone_id_log);
                                    continue;
                                }
                            };
                            let zone_high = match enriched_zone_struct.zone_high {
                                Some(val) if val.is_finite() => val,
                                _ => {
                                    warn!("[Task {}/{}] {} zone {} (ID: {}) missing/invalid 'zone_high'. Skipping.", symbol, timeframe, zone_type_str, zone_counter, zone_id_log);
                                    continue;
                                }
                            };
                            let zone_low = match enriched_zone_struct.zone_low {
                                Some(val) if val.is_finite() => val,
                                _ => {
                                    warn!("[Task {}/{}] {} zone {} (ID: {}) missing/invalid 'zone_low'. Skipping.", symbol, timeframe, zone_type_str, zone_counter, zone_id_log);
                                    continue;
                                }
                            };
                            let (proximal_line, distal_line) = if is_supply {
                                (zone_low, zone_high) // Supply: Proximal=Low, Distal=High
                            } else {
                                (zone_high, zone_low) // Demand: Proximal=High, Distal=Low
                            };

                            debug!("[Task {}/{}] Enriching {} zone #{} (ID: {}), FormedEndIdx: {}, H: {}, L: {}, P: {}, D: {}", symbol, timeframe, zone_type_str, zone_counter, zone_id_log, formation_end_idx, zone_high, zone_low, proximal_line, distal_line);

                            // --- 3. Initialize Tracking Variables ---
                            let mut is_currently_active = true; // Assume active initially
                            let mut touch_count: i64 = 0;
                            let mut first_invalidation_idx: Option<usize> = None; // Distal break ONLY
                            let mut current_touch_points: Vec<TouchPoint> = Vec::new();

                            // --- 4. Loop Through Candles *After* Formation ---
                            let check_start_idx = formation_end_idx + 1;
                            
                            // FIXED: Determine correct initial state based on first candle after formation
                            let mut is_outside_zone = true;
                            if check_start_idx < candle_count {
                                let first_candle_after_formation = &candles[check_start_idx];
                                
                                if is_supply {
                                    // For supply zone: if first candle's high reaches proximal line, we start "inside"
                                    is_outside_zone = first_candle_after_formation.high < proximal_line;
                                } else {
                                    // For demand zone: if first candle's low reaches proximal line, we start "inside"
                                    is_outside_zone = first_candle_after_formation.low > proximal_line;
                                }
                                
                                debug!("[Task {}/{}] Zone {} initial state after formation: {} (first candle H:{}, L:{})", 
                                       symbol, timeframe, zone_counter,
                                       if is_outside_zone { "OUTSIDE" } else { "INSIDE" },
                                       first_candle_after_formation.high, first_candle_after_formation.low);
                            }

                            if check_start_idx < candle_count {
                                for i in check_start_idx..candle_count {
                                    // Stop checking if the zone is no longer active (distal break)
                                    if !is_currently_active {
                                        break;
                                    }

                                    let candle = &candles[i];
                                    if !candle.high.is_finite() || !candle.low.is_finite() || !candle.close.is_finite() {
                                        continue; // Skip malformed candles
                                    }

                                    let mut interaction_occurred = false;
                                    let mut interaction_price = 0.0;

                                    // --- Check for Invalidation (Distal Breach) ---
                                    if is_supply {
                                        // Supply invalidated if candle HIGH goes ABOVE distal line (zone_high)
                                        if candle.high > distal_line {
                                            is_currently_active = false;
                                            if first_invalidation_idx.is_none() {
                                                first_invalidation_idx = Some(i);
                                            }
                                            debug!("[Task {}/{}] Supply zone {} (ID: {}) invalidated at index {}", symbol, timeframe, zone_counter, zone_id_log, i);
                                            break; // Stop processing this zone after invalidation
                                        }
                                    } else {
                                        // Demand invalidated if candle LOW goes BELOW distal line (zone_low)
                                        if candle.low < distal_line {
                                            is_currently_active = false;
                                            if first_invalidation_idx.is_none() {
                                                first_invalidation_idx = Some(i);
                                            }
                                            debug!("[Task {}/{}] Demand zone {} (ID: {}) invalidated at index {}", symbol, timeframe, zone_counter, zone_id_log, i);
                                            break; // Stop processing this zone after invalidation
                                        }
                                    }

                                    // --- Check for Interaction (Proximal Breach) ---
                                    if is_supply {
                                        // Supply interaction if candle HIGH reaches or crosses proximal line (zone_low)
                                        if candle.high >= proximal_line {
                                            interaction_occurred = true;
                                            interaction_price = candle.high; // Price at interaction point
                                        }
                                    } else {
                                        // Demand interaction if candle LOW reaches or crosses proximal line (zone_high)
                                        if candle.low <= proximal_line {
                                            interaction_occurred = true;
                                            interaction_price = candle.low; // Price at interaction point
                                        }
                                    }

                                    // --- Record Interaction and Increment Count ONLY IF transitioning from OUTSIDE ---
                                    if interaction_occurred && is_outside_zone {
                                        touch_count += 1; // Count this as one "touch" event (entry)
                                        current_touch_points.push(TouchPoint {
                                            time: candle.time.clone(),
                                            price: interaction_price,
                                        });
                                        debug!("[Task {}/{}] Zone {} (ID: {}) ENTRY recorded at index {}. Price: {}. Total Touches: {}", symbol, timeframe, zone_counter, zone_id_log, i, interaction_price, touch_count);
                                    }

                                    // --- Update State for the NEXT candle's check ---
                                    is_outside_zone = !interaction_occurred;

                                } // End candle loop
                            } else {
                                debug!("[Task {}/{}] Zone {} (ID: {}) - No candles after formation index {}.", symbol, timeframe, zone_counter, zone_id_log, formation_end_idx);
                            }

                            // --- 5. Finalize Enrichment ---
                            enriched_zone_struct.is_active = is_currently_active;
                            enriched_zone_struct.touch_count = Some(touch_count); // Set the final calculated touch count
                            if !current_touch_points.is_empty() {
                                enriched_zone_struct.touch_points = Some(current_touch_points);
                            } else {
                                enriched_zone_struct.touch_points = None; // Explicitly set to None if no touches were recorded
                            }

                            // Populate invalidation/end fields only if the zone was invalidated
                            if !is_currently_active {
                                enriched_zone_struct.invalidation_time = first_invalidation_idx
                                    .and_then(|idx| candles.get(idx)) // Safely get candle
                                    .map(|c| c.time.clone());
                                enriched_zone_struct.zone_end_idx = first_invalidation_idx.map(|idx| idx as u64);
                                enriched_zone_struct.zone_end_time = enriched_zone_struct.invalidation_time.clone(); // Reuse invalidation time
                            } else {
                                // Ensure these are None if the zone is still active
                                enriched_zone_struct.invalidation_time = None;
                                enriched_zone_struct.zone_end_idx = None;
                                enriched_zone_struct.zone_end_time = None;
                            }

                            // FIXED: Calculate bars_active correctly
                            if !is_currently_active {
                                // Zone was invalidated - calculate from formation to invalidation
                                if let Some(invalidation_idx) = first_invalidation_idx {
                                    if invalidation_idx > check_start_idx {
                                        enriched_zone_struct.bars_active = Some((invalidation_idx - check_start_idx) as u64);
                                    } else {
                                        enriched_zone_struct.bars_active = Some(0);
                                    }
                                } else {
                                    enriched_zone_struct.bars_active = Some(0);
                                }
                            } else {
                                // Zone is still active - calculate from formation to end of available data
                                if candle_count > check_start_idx {
                                    enriched_zone_struct.bars_active = Some((candle_count - check_start_idx) as u64);
                                } else {
                                    enriched_zone_struct.bars_active = Some(0);
                                }
                            }

                            // Calculate strength score based on the corrected touch count
                            let score_raw = 100.0 - (touch_count as f64 * 10.0); // Example scoring
                            enriched_zone_struct.strength_score = Some(score_raw.max(0.0)); // Ensure score is not negative

                            debug!("[Task/Debug] Zone Final Status: Active={}, Touches={}, BarsActive={:?}, InvalidatedT={:?}, ZoneEndIdx={:?}, ZoneEndTime={:?}",
                            enriched_zone_struct.is_active, enriched_zone_struct.touch_count.unwrap_or(0), enriched_zone_struct.bars_active,
                            enriched_zone_struct.invalidation_time.is_some(), // Use invalidation_time for consistency in log
                            enriched_zone_struct.zone_end_idx, enriched_zone_struct.zone_end_time);

                                                        // Add debug candle data for analysis
                            let mut debug_candles = Vec::new();
                            let start_idx = enriched_zone_struct.start_idx.unwrap() as usize;

                            debug_to_file(&format!("🕯️ [ZONE_{}] Adding debug candles for zone ID: {}", zone_counter, zone_id_log));

                            // Add formation candles (always 2 candles)
                            if start_idx < candles.len() {
                                let fifty_candle = &candles[start_idx];
                                debug_candles.push(json!({
                                    "idx": start_idx,
                                    "time": fifty_candle.time,
                                    "open": fifty_candle.open,
                                    "high": fifty_candle.high,
                                    "low": fifty_candle.low,
                                    "close": fifty_candle.close
                                }));
                                debug_to_file(&format!("🕯️   [{}] FORMATION-50%: {} O:{:.5} H:{:.5} L:{:.5} C:{:.5}", 
                                            start_idx, fifty_candle.time, fifty_candle.open, fifty_candle.high, fifty_candle.low, fifty_candle.close));
                            }

                            if start_idx + 1 < candles.len() {
                                let big_candle = &candles[start_idx + 1];
                                debug_candles.push(json!({
                                    "idx": start_idx + 1,
                                    "time": big_candle.time,
                                    "open": big_candle.open,
                                    "high": big_candle.high,
                                    "low": big_candle.low,
                                    "close": big_candle.close
                                }));
                                debug_to_file(&format!("🕯️   [{}] FORMATION-BIG: {} O:{:.5} H:{:.5} L:{:.5} C:{:.5}", 
                                            start_idx + 1, big_candle.time, big_candle.open, big_candle.high, big_candle.low, big_candle.close));
                            }

                            // Add activity candles (from formation end + 1 to end of data or invalidation)
                            let activity_start = start_idx + 2;
                            let activity_end = if !is_currently_active && first_invalidation_idx.is_some() {
                                first_invalidation_idx.unwrap()
                            } else {
                                candles.len()
                            };

                            debug_to_file(&format!("🕯️ [ZONE_{}] Activity candles from {} to {} (total: {})", 
                                        zone_counter, activity_start, activity_end - 1, activity_end - activity_start));

                            for i in activity_start..activity_end {
                                if i < candles.len() {
                                    let activity_candle = &candles[i];
                                    debug_candles.push(json!({
                                        "idx": i,
                                        "time": activity_candle.time,
                                        "open": activity_candle.open,
                                        "high": activity_candle.high,
                                        "low": activity_candle.low,
                                        "close": activity_candle.close
                                    }));
                                }
                            }

                            debug_to_file(&format!("🕯️ [ZONE_{}] Total debug candles added: {}", zone_counter, debug_candles.len()));

                            // Add debug candles to the zone
                            enriched_zone_struct.debug_candles = Some(debug_candles);

                            processed_zones.push(enriched_zone_struct);
                        } // End loop through zones (for zone_json in zones.iter())
                    } else {
                        // This case means the key (e.g., "supply_zones") existed, but its "zones" array was null or missing.
                        info!(
                            "[Task {}/{}] No raw {} zones found in detection results structure (key existed but 'zones' was empty/null).",
                            symbol, timeframe, zone_type_str
                        );
                    }
                    Ok(processed_zones)
                };

            // --- Execute Enrichment ---
            if let Some(data) = detection_results_json.get("data").and_then(|d| d.get("price")) {
                match data.get("supply_zones").and_then(|sz| sz.get("zones")).and_then(|z| z.as_array()) {
                    Some(raw_zones) => match process_and_enrich_zones(
                        Some(raw_zones),
                        true,
                        last_candle_timestamp.clone(),
                    ) {
                        Ok(zones) => final_supply_zones = zones,
                        Err(e) => enrichment_error = Some(e),
                    },
                    None => debug!("[debug] No supply zones path for {}/{}.", symbol, timeframe),
                }
                if enrichment_error.is_none() {
                    match data.get("demand_zones").and_then(|dz| dz.get("zones")).and_then(|z| z.as_array()) {
                        Some(raw_zones) => match process_and_enrich_zones(
                            Some(raw_zones),
                            false,
                            last_candle_timestamp.clone(),
                        ) {
                            Ok(zones) => final_demand_zones = zones,
                            Err(e) => enrichment_error = Some(e),
                        },
                        None => {
                            debug!("[debug] No demand zones path for {}/{}.", symbol, timeframe)
                        }
                    }
                }
            } else {
                warn!(
                    "[debug] Missing 'data.price' path in results for {}/{}: {:?}",
                    symbol, timeframe, detection_results_json
                );
                enrichment_error = Some("Missing 'data.price' structure.".to_string());
            }

            // --- Package Result ---
            if let Some(err_msg) = enrichment_error {
                debug_to_file(&format!(
                    "[debug_bulk_zones] Enrichment failed for {}/{}: {}",
                    symbol, timeframe, err_msg
                ));
                HttpResponse::InternalServerError().json(ErrorResponse {
                    error: format!("Enrichment Failed: {}", err_msg),
                })
            } else {
                info!(
                    "[debug_bulk_zones] Filtering for active zones for {}/{}...",
                    symbol, timeframe
                );
                let active_supply_zones = final_supply_zones
                    .into_iter()
                    .filter(|zone| zone.is_active)
                    .collect::<Vec<_>>();
                let active_demand_zones = final_demand_zones
                    .into_iter()
                    .filter(|zone| zone.is_active)
                    .collect::<Vec<_>>();
                info!(
                    "[debug_bulk_zones] Found {} active supply, {} active demand zones for {}/{}.",
                    active_supply_zones.len(),
                    active_demand_zones.len(),
                    symbol,
                    timeframe
                );

                // Use structs from types.rs - Structure matches BulkActiveZonesResponse for consistency
                let timeframe_data = TimeframeZoneResponse {
                    supply_zones: active_supply_zones,
                    demand_zones: active_demand_zones,
                };
                let symbol_data = SymbolZoneResponse {
                    // Use original user-requested timeframe string as key
                    timeframes: HashMap::from([(timeframe.clone(), timeframe_data)]),
                };
                let mut response = BulkActiveZonesResponse::default(); // Assumes Default derive
                                                                       // Use original user-requested symbol string as key
                response.symbols.insert(symbol.clone(), symbol_data);
                // Optionally add query params if needed for debug response clarity
                // response.query_params = Some(query.into_inner()); // Need to reconstruct ChartQuery if desired

                HttpResponse::Ok().json(response)
            }
        }
        Err(core_err) => {
            // Handle CoreError from this file's _fetch_and_detect_core
            debug_to_file(&format!(
                "[debug_bulk_zones] Core error for {}/{}: {}",
                symbol, timeframe, core_err
            ));
            let (mut status_code, error_message) = match core_err {
                // Removed the specific pattern check here, as it's done earlier
                CoreError::Config(e) => (
                    HttpResponse::InternalServerError(),
                    format!("Configuration Error: {}", e),
                ),
                CoreError::Request(e) => (
                    HttpResponse::InternalServerError(),
                    format!("Data Fetch Error: {}", e),
                ),
                CoreError::Csv(e) => (
                    HttpResponse::InternalServerError(),
                    format!("Data Parse Error: {}", e),
                ),
                CoreError::EnvVar(e) => (
                    HttpResponse::InternalServerError(),
                    format!("Server Config Error: {}", e),
                ),
                CoreError::Processing(e) => (
                    HttpResponse::InternalServerError(),
                    format!("Processing Error: {}", e),
                ),
            };
            status_code.json(ErrorResponse {
                error: error_message,
            })
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

    const PRECISION: usize = 8; // Or your desired precision

    // ⭐ CRITICAL FIX: Always standardize symbol by removing _SB suffix
    let clean_symbol = symbol.replace("_SB", "");
    
    // STANDARDIZE TIMEFRAME: Always lowercase
    let clean_timeframe = timeframe.to_lowercase();
    
    // STANDARDIZE ZONE TYPE: Always lowercase
    let clean_zone_type = zone_type.to_lowercase();

    let high_str = zone_high.map_or("0.0".to_string(), |v| {
        if v.is_finite() {
            format!("{:.prec$}", v, prec = PRECISION)
        } else {
            "0.0".to_string()
        }
    });
    let low_str = zone_low.map_or("0.0".to_string(), |v| {
        if v.is_finite() {
            format!("{:.prec$}", v, prec = PRECISION)
        } else {
            "0.0".to_string()
        }
    });
    let clean_start_time = start_time.unwrap_or("unknown");

    // Create deterministic ID input
    let id_input = format!(
        "{}_{}_{}_{}_{}_{}",
        clean_symbol,      // ⭐ Now always "USDJPY" regardless of input
        clean_timeframe,   // Always lowercase "1h"
        clean_zone_type,   // Always lowercase "demand"
        clean_start_time,
        high_str,
        low_str
    );


    let mut hasher = Sha256::new();
    hasher.update(id_input.as_bytes());
    let result = hasher.finalize();
    let hex_id = format!("{:x}", result);

    // Use the first 16 chars of the hash for the ID
     let final_id = hex_id[..16].to_string();
    
    #[cfg(debug_assertions)]
    {
        crate::api::detect::debug_to_file(&format!(
            "🔧 [ZONE_ID_STANDARD] Generated ID: '{}'", final_id
        ));
    }

    final_id
}

/// Calculates the number of times a zone was "touched" by candles *after* its formation,
/// within a given candle set. Stops counting after the zone is invalidated.
/// NOTE: This only counts touches within the provided `candles` slice.
pub fn calculate_touches_within_range(
    zone_json: &Value,      // Raw JSON to get indices and levels reliably
    candles: &[CandleData], // The slice of candles to check against
    is_supply: bool,
    candle_count: usize, // Pass candle_count for efficiency
) -> i64 {
    // Return i64 to match DB type eventually

    // --- Get necessary values from the zone JSON ---
    // Use end_idx if available and reliable, otherwise fallback needs care
    let end_idx = match zone_json.get("end_idx").and_then(|v| v.as_u64()) {
        Some(idx) => idx as usize,
        None => {
            warn!(
                "calculate_touches: Missing 'end_idx' in zone: {:?}. Cannot calculate touches.",
                zone_json
            );
            return 0; // Cannot calculate without end index
        }
    };
    let zone_high = match zone_json.get("zone_high").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => {
            warn!(
                "calculate_touches: Missing/invalid 'zone_high' in zone: {:?}",
                zone_json
            );
            return 0;
        }
    };
    let zone_low = match zone_json.get("zone_low").and_then(|v| v.as_f64()) {
        Some(val) if val.is_finite() => val,
        _ => {
            warn!(
                "calculate_touches: Missing/invalid 'zone_low' in zone: {:?}",
                zone_json
            );
            return 0;
        }
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

pub async fn find_and_verify_zone_handler(
    query: web::Query<FindAndVerifyZoneQueryParams>, // Uses the new struct
) -> impl Responder {
    info!("[HANDLER_FIND_VERIFY] Request: {:?}", query);

    let serializable_query_params = FindAndVerifyZoneQueryParamsSerializable {
        symbol: query.symbol.clone(),
        timeframe: query.timeframe.clone(),
        target_formation_start_time: query.target_formation_start_time.clone(),
        pattern: query.pattern.clone(),
        fetch_window_start_time: query.fetch_window_start_time.clone(),
        fetch_window_end_time: query.fetch_window_end_time.clone(),
    };

    // Call the core logic function (we'll define its signature next)
    match execute_find_and_verify_zone_logic(&query).await {
        Ok(mut result_payload) => { // result_payload is FindAndVerifyZoneResult
            // Ensure query_params in the payload is correctly set if not done by logic fn
            result_payload.query_params = serializable_query_params;
            HttpResponse::Ok().json(result_payload)
        }
        Err(e) => { // Assuming execute_find_and_verify_zone_logic returns Result<FindAndVerifyZoneResult, CoreError>
            debug_to_file(&format!("[HANDLER_FIND_VERIFY] Core error: {}", e));
            let err_msg = format!("Failed to process find-and-verify request: {}", e);
            
            // Build an error response using FindAndVerifyZoneResult
            HttpResponse::InternalServerError().json(FindAndVerifyZoneResult {
                query_params: serializable_query_params,
                fetched_candles_count: 0,
                found_zone_details: None,
                message: "Error during processing.".to_string(),
                error_details: Some(err_msg),
                recognizer_raw_output_for_target_zone: None,
            })
        }
    }
}

async fn execute_find_and_verify_zone_logic(
    query_params: &FindAndVerifyZoneQueryParams,
) -> Result<FindAndVerifyZoneResult, CoreError> {
    info!("[LOGIC_FIND_VERIFY] Processing for target start time: {} for S/TF: {}/{}",
        query_params.target_formation_start_time, query_params.symbol, query_params.timeframe);

    // 1. Prepare ChartQuery for _fetch_and_detect_core
    let chart_query_for_fetch = ChartQuery {
        symbol: query_params.symbol.clone(),
        timeframe: query_params.timeframe.clone(),
        start_time: query_params.fetch_window_start_time.clone(),
        end_time: query_params.fetch_window_end_time.clone(),
        pattern: query_params.pattern.clone(), // Use the specified pattern
        enable_trading: None, lot_size: None, stop_loss_pips: None, take_profit_pips: None, enable_trailing_stop: None,
        max_touch_count: None,
    };

    let serializable_query_params = FindAndVerifyZoneQueryParamsSerializable {
        symbol: query_params.symbol.clone(),
        timeframe: query_params.timeframe.clone(),
        target_formation_start_time: query_params.target_formation_start_time.clone(),
        pattern: query_params.pattern.clone(),
        fetch_window_start_time: query_params.fetch_window_start_time.clone(),
        fetch_window_end_time: query_params.fetch_window_end_time.clone(),
    };

    // 2. Fetch candles and run recognizer
    let (all_fetched_candles, _recognizer, recognizer_output_value) =
        match _fetch_and_detect_core(&chart_query_for_fetch, "find_and_verify_logic").await {
            Ok(data) => data,
            Err(e) => {
                debug_to_file(&format!("[LOGIC_FIND_VERIFY] Error from _fetch_and_detect_core: {}", e));
                return Err(e); // Propagate CoreError
            }
        };

    if all_fetched_candles.is_empty() {
        info!("[LOGIC_FIND_VERIFY] No candles fetched in the window for {}/{}", query_params.symbol, query_params.timeframe);
        return Ok(FindAndVerifyZoneResult {
            query_params: serializable_query_params,
            fetched_candles_count: 0,
            found_zone_details: None,
            message: "No candles fetched in the specified window.".to_string(),
            error_details: None,
            recognizer_raw_output_for_target_zone: Some(recognizer_output_value), // Still include recognizer output
        });
    }
    let candle_count_total = all_fetched_candles.len();
    info!("[LOGIC_FIND_VERIFY] Fetched {} candles. Recognizer output keys: {:?}",
        candle_count_total,
        recognizer_output_value.as_object().map_or_else(|| Vec::new(), |o| o.keys().cloned().collect::<Vec<_>>())
    );


    // 3. Find the specific zone matching target_formation_start_time
    let mut target_zone_raw_json: Option<serde_json::Value> = None;

    // Expected path to zones array: recognizer_output_value["data"]["price"]["supply_zones" or "demand_zones"]["zones"]
    if let Some(data_val) = recognizer_output_value.get("data") {
        if let Some(price_val) = data_val.get("price") {
            for zone_kind_key in ["supply_zones", "demand_zones"] { // Check both supply and demand
                if let Some(zones_of_kind_val) = price_val.get(zone_kind_key) {
                    if let Some(zones_array_val) = zones_of_kind_val.get("zones") {
                        if let Some(zones_array) = zones_array_val.as_array() {
                            for zone_json in zones_array {
                                if let Some(zone_start_str) = zone_json.get("start_time").and_then(|st| st.as_str()) {
                                    if zone_start_str == query_params.target_formation_start_time {
                                        target_zone_raw_json = Some(zone_json.clone());
                                        info!("[LOGIC_FIND_VERIFY] Found target zone starting at {}: Kind: {}", zone_start_str, zone_kind_key);
                                        break; // Found our specific zone
                                    }
                                }
                            }
                        }
                    }
                }
                if target_zone_raw_json.is_some() {
                    break; // Stop checking other kinds if found
                }
            }
        }
    }

    if target_zone_raw_json.is_none() {
        info!("[LOGIC_FIND_VERIFY] No zone found by recognizer starting exactly at {}", query_params.target_formation_start_time);
        return Ok(FindAndVerifyZoneResult {
            query_params: serializable_query_params,
            fetched_candles_count: candle_count_total,
            found_zone_details: None,
            message: format!("No zone detected by pattern '{}' starting exactly at {}.", query_params.pattern, query_params.target_formation_start_time),
            error_details: None,
            recognizer_raw_output_for_target_zone: Some(recognizer_output_value), // Show what was found
        });
    }

    // 4. If found, perform enrichment (This is where we adapt logic from bulk handler's closure)
   let raw_zone_to_enrich = target_zone_raw_json.as_ref().unwrap();

    // --- START: Adapted Enrichment Logic ---
    let mut activity_details = FoundZoneActivityDetails::default();

    let detection_method_str = raw_zone_to_enrich
        .get("detection_method")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown from recognizer");

    // Prefer "type" field from recognizer if it exists, otherwise infer from detection_method
    let zone_type_explicit = raw_zone_to_enrich
        .get("type") // e.g., "supply_zone", "demand_zone"
        .and_then(|v| v.as_str());

    let is_supply: bool; // Will be used for activity logic
    let zone_type_str: String;

    if let Some(zt_explicit) = zone_type_explicit {
        if zt_explicit.to_lowercase().contains("supply") {
            is_supply = true;
            zone_type_str = "supply".to_string();
        } else if zt_explicit.to_lowercase().contains("demand") {
            is_supply = false;
            zone_type_str = "demand".to_string();
        } else {
            // Fallback if "type" field is ambiguous
            is_supply = detection_method_str.to_lowercase().contains("supply") ||
                        detection_method_str.to_lowercase().contains("bearish");
            zone_type_str = if is_supply { "supply".to_string() } else { "demand".to_string() };
            warn!("[LOGIC_FIND_VERIFY] Ambiguous 'type' field ('{}') from recognizer, inferred type based on detection_method.", zt_explicit);
        }
    } else {
        // Infer from detection_method if "type" field is missing
        is_supply = detection_method_str.to_lowercase().contains("supply") ||
                    detection_method_str.to_lowercase().contains("bearish");
        zone_type_str = if is_supply { "supply".to_string() } else { "demand".to_string() };
    }

    // activity_details.is_supply_zone_detected = Some(is_supply); // OLD
    activity_details.zone_type_detected = Some(zone_type_str); // NEW
    activity_details.detection_method = Some(detection_method_str.to_string());


    activity_details.detected_zone_actual_start_time = raw_zone_to_enrich.get("start_time").and_then(|v| v.as_str()).map(String::from);
    activity_details.zone_high = raw_zone_to_enrich.get("zone_high").and_then(|v| v.as_f64());
    activity_details.zone_low = raw_zone_to_enrich.get("zone_low").and_then(|v| v.as_f64());
    activity_details.start_idx_in_fetched_slice = raw_zone_to_enrich.get("start_idx").and_then(|v| v.as_u64());
    activity_details.end_idx_in_fetched_slice = raw_zone_to_enrich.get("end_idx").and_then(|v| v.as_u64());

    let (zone_high, zone_low, start_idx_from_json, end_idx_from_json) = match (
        activity_details.zone_high,
        activity_details.zone_low,
        activity_details.start_idx_in_fetched_slice,
        activity_details.end_idx_in_fetched_slice,
    ) {
        (Some(h), Some(l), Some(s), Some(e)) if h.is_finite() && l.is_finite() => (h, l, s as usize, e as usize),
        _ => {
            let err_msg = format!("Target zone JSON missing essential fields (high, low, start_idx, end_idx): {:?}", raw_zone_to_enrich);
            debug_to_file(&format!("[LOGIC_FIND_VERIFY] {}", err_msg));
            return Ok(FindAndVerifyZoneResult {
                query_params: serializable_query_params,
                fetched_candles_count: candle_count_total,
                found_zone_details: None, // Could put partial details here if desired
                message: "Error enriching target zone.".to_string(),
                error_details: Some(err_msg),
                recognizer_raw_output_for_target_zone: Some(recognizer_output_value),
            });
        }
    };

    // Ensure indices are within bounds of the fetched candles
    if end_idx_from_json >= candle_count_total || start_idx_from_json > end_idx_from_json {
        let err_msg = format!(
            "Invalid indices from recognizer for target zone: start_idx={}, end_idx={}, candle_count={}. Zone JSON: {:?}",
            start_idx_from_json, end_idx_from_json, candle_count_total, raw_zone_to_enrich
        );
        debug_to_file(&format!("[LOGIC_FIND_VERIFY] {}", err_msg));
         return Ok(FindAndVerifyZoneResult {
            query_params: serializable_query_params,
            fetched_candles_count: candle_count_total,
            found_zone_details: None,
            message: "Invalid indices from recognizer for target zone.".to_string(),
            error_details: Some(err_msg),
            recognizer_raw_output_for_target_zone: Some(recognizer_output_value),
        });
    }


    activity_details.is_active = true; // Assume active
    let (proximal_line, distal_line) = if is_supply {
        (zone_low, zone_high)
    } else {
        (zone_high, zone_low)
    };

    // Activity check starts *after* the zone is fully formed (after end_idx_from_json)
    let check_start_idx_in_slice = end_idx_from_json + 1;
    let mut is_currently_outside_zone = true;

    if check_start_idx_in_slice < candle_count_total {
        for i in check_start_idx_in_slice..candle_count_total {
            let candle = &all_fetched_candles[i];
             if !candle.high.is_finite() || !candle.low.is_finite() || !candle.close.is_finite() {
                warn!("[LOGIC_FIND_VERIFY] Candle at index {} (time: {}) has non-finite values during activity check. Skipping.", i, candle.time);
                continue;
            }

            activity_details.bars_active_after_formation = Some((i - check_start_idx_in_slice + 1) as u64);

            if is_supply {
                if candle.high > distal_line {
                    activity_details.is_active = false;
                    activity_details.invalidation_candle_time = Some(candle.time.clone());
                    debug!("[LOGIC_FIND_VERIFY] Supply zone invalidated at candle {}: CH {} > Distal {}", candle.time, candle.high, distal_line);
                    break;
                }
            } else {
                if candle.low < distal_line {
                    activity_details.is_active = false;
                    activity_details.invalidation_candle_time = Some(candle.time.clone());
                    debug!("[LOGIC_FIND_VERIFY] Demand zone invalidated at candle {}: CL {} < Distal {}", candle.time, candle.low, distal_line);
                    break;
                }
            }

            let mut interaction_occurred = false;
            if is_supply {
                if candle.high >= proximal_line { interaction_occurred = true; }
            } else {
                if candle.low <= proximal_line { interaction_occurred = true; }
            }

            if interaction_occurred && is_currently_outside_zone {
                activity_details.touch_count += 1;
                if activity_details.first_touch_candle_time.is_none() {
                    activity_details.first_touch_candle_time = Some(candle.time.clone());
                }
                debug!("[LOGIC_FIND_VERIFY] Zone touched at candle {}. Touches: {}", candle.time, activity_details.touch_count);
            }
            is_currently_outside_zone = !interaction_occurred;
        }
    } else {
        activity_details.bars_active_after_formation = Some(0);
        info!("[LOGIC_FIND_VERIFY] No candles available after zone formation (end_idx: {}) to check activity.", end_idx_from_json);
    }
     if activity_details.is_active && activity_details.bars_active_after_formation.is_none() {
         if check_start_idx_in_slice < candle_count_total {
            activity_details.bars_active_after_formation = Some((candle_count_total - check_start_idx_in_slice) as u64);
         } else {
            activity_details.bars_active_after_formation = Some(0);
         }
    }
    // --- END: Adapted Enrichment Logic ---

    info!("[LOGIC_FIND_VERIFY] Final activity for target zone: active={}, touches={}", activity_details.is_active, activity_details.touch_count);

    Ok(FindAndVerifyZoneResult {
        query_params: serializable_query_params,
        fetched_candles_count: candle_count_total,
        found_zone_details: Some(activity_details),
        message: "Target zone found and analyzed.".to_string(),
        error_details: None,
        recognizer_raw_output_for_target_zone: Some(raw_zone_to_enrich.clone()), // Include the specific zone's JSON
    })
}

pub async fn fetch_candles_direct(
    http_client: &HttpClient,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    bucket: &str,
    measurement: &str, // This would be your trendbar measurement, e.g., "trendbar"
    symbol: &str,
    timeframe: &str,
    start_time: &str, // Can be relative like "-10d" or RFC3339
    end_time: &str,   // Can be "now()" or RFC3339
) -> Result<Vec<CandleData>, String> {
    if symbol.is_empty() || timeframe.is_empty() {
        warn!("[FETCH_CANDLES_DIRECT] Symbol or timeframe is empty, returning no candles.");
        return Ok(vec![]);
    }

    // Construct the Flux query to fetch candles
    // This assumes your trendbar data has fields: open, high, low, close, volume
    // and tags: symbol, timeframe. Adjust if your schema is different.
    let flux_query = format!(
        r#"from(bucket: "{}")
           |> range(start: {}, stop: {})
           |> filter(fn: (r) => r["_measurement"] == "{}")
           |> filter(fn: (r) => r["symbol"] == "{}")
           |> filter(fn: (r) => r["timeframe"] == "{}")
           |> toFloat() // Ensure numeric fields are float
           |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value")
           |> sort(columns: ["_time"])"#,
        bucket, start_time, end_time, measurement, symbol, timeframe
    );

    debug!("[FETCH_CANDLES_DIRECT] Fetching candles for {}/{}, Query: {}", symbol, timeframe, flux_query);
    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);

    let response = match http_client.post(&query_url)
        .bearer_auth(influx_token)
        .header("Accept", "application/csv")
        .header("Content-Type", "application/json")
        .json(&json!({"query": flux_query, "type": "flux"}))
        .send().await
    {
        Ok(resp) => resp,
        Err(e) => return Err(format!("HTTP request to InfluxDB failed: {}", e)),
    };

    let status = response.status();
    if !status.is_success() {
        let err_body = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        debug_to_file(&format!("[FETCH_CANDLES_DIRECT] InfluxDB query failed (status {}): {}. Query: {}", status, err_body, flux_query));
        return Err(format!("InfluxDB query failed (status {}): {}", status, err_body));
    }

    let csv_text = match response.text().await {
        Ok(text) => text,
        Err(e) => return Err(format!("Failed to read InfluxDB response text: {}", e)),
    };

    let mut candles = Vec::new();
    if csv_text.lines().skip_while(|l| l.starts_with('#') || l.is_empty()).count() > 1 { // Check if more than just header
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true) // Useful if there are extra columns in CSV not in CandleData
            .comment(Some(b'#'))
            .from_reader(csv_text.as_bytes());

        // CandleData needs to derive Deserialize and its fields must match
        // the pivoted CSV column names (e.g., _time, open, high, low, close, volume)
        // or use #[serde(rename = "...")]
        for result in rdr.deserialize::<CandleData>() {
            match result {
                Ok(candle) => candles.push(candle),
                Err(e) => {
                    warn!("[FETCH_CANDLES_DIRECT] CSV deserialize error for a candle: {}. Row might be skipped.", e);
                }
            }
        }
    } else {
        debug!("[FETCH_CANDLES_DIRECT] No data rows in CSV response for {}/{}", symbol, timeframe);
    }
    debug!("[FETCH_CANDLES_DIRECT] Parsed {} candles for {}/{}", candles.len(), symbol, timeframe);
    Ok(candles)
}

// Add this to detect.rs - extracted from get_bulk_multi_tf_active_zones_handler
pub fn enrich_zones_with_activity(
    raw_zones_option: Option<&Vec<serde_json::Value>>,
    is_supply: bool,
    all_candles: &[CandleData],
    total_candle_count: usize,
    last_candle_timestamp: Option<String>,
    symbol: &str,
    timeframe: &str,
) -> Result<Vec<EnrichedZone>, String> {
    let mut enriched_zones: Vec<EnrichedZone> = Vec::new();
    let zone_type_str = if is_supply { "supply" } else { "demand" };
    
    debug!("[ENRICH_GENERATOR] Processing {} zones for {}/{}", zone_type_str, symbol, timeframe);
    
    if let Some(raw_zones) = raw_zones_option {
        debug!("[ENRICH_GENERATOR] Found {} raw {} zones", raw_zones.len(), zone_type_str);
        
        for (zone_counter, zone_json) in raw_zones.iter().enumerate() {
            debug!("[ENRICH_GENERATOR] Processing {} zone #{}", zone_type_str, zone_counter + 1);
            
            // Deserialize the zone
            let mut enriched_zone = match crate::types::deserialize_raw_zone_value(zone_json) {
                Ok(mut z) => {
                    // Generate zone_id if missing
                    if z.zone_id.is_none() {
                        let id = generate_deterministic_zone_id(
                            symbol, timeframe, 
                            if is_supply { "supply" } else { "demand" },
                            z.start_time.as_deref(), z.zone_high, z.zone_low,
                        );
                        z.zone_id = Some(id);
                    }
                    // Set zone_type if missing
                    if z.zone_type.is_none() {
                        z.zone_type = Some(format!("{}_zone", zone_type_str));
                    }
                    z
                }
                Err(e) => {
                    warn!("[ENRICH_GENERATOR] Failed to deserialize zone #{}: {}", zone_counter + 1, e);
                    continue;
                }
            };
            
            // CRITICAL: Clear any existing end time data (match bulk handler)
            enriched_zone.end_time = None;
            enriched_zone.end_idx = None;
            enriched_zone.zone_end_time = None;
            enriched_zone.zone_end_idx = None;
            enriched_zone.invalidation_time = None;
            enriched_zone.last_data_candle = last_candle_timestamp.clone();
            
            let zone_id_log = enriched_zone.zone_id.as_deref().unwrap_or("UNKNOWN");
            
            // Get formation end index (MATCH BULK HANDLER LOGIC)
            let formation_end_idx = match enriched_zone.start_idx {
                Some(start_idx) => start_idx as usize + 1, // Formation ends after the big bar
                None => {
                    warn!("[ENRICH_GENERATOR] Zone {} missing start_idx", zone_id_log);
                    continue;
                }
            };
            
            // Validate zone prices
            let zone_high = match enriched_zone.zone_high {
                Some(val) if val.is_finite() => val,
                _ => {
                    warn!("[ENRICH_GENERATOR] Zone {} invalid zone_high", zone_id_log);
                    continue;
                }
            };
            
            let zone_low = match enriched_zone.zone_low {
                Some(val) if val.is_finite() => val,
                _ => {
                    warn!("[ENRICH_GENERATOR] Zone {} invalid zone_low", zone_id_log);
                    continue;
                }
            };
            
            // Set proximal/distal lines (MATCH BULK HANDLER)
            let (proximal_line, distal_line) = if is_supply {
                (zone_low, zone_high)
            } else {
                (zone_high, zone_low)
            };
            
            debug!("[ENRICH_GENERATOR] Zone {} - Formation ends at idx {}, High: {:.5}, Low: {:.5}", 
                   zone_id_log, formation_end_idx, zone_high, zone_low);
            
            // CRITICAL: Activity check starts AFTER formation (MATCH BULK HANDLER)
            let check_start_idx = formation_end_idx + 1;
            
            // Initialize tracking variables
            let mut is_currently_active = true;
            let mut touch_count: i64 = 0;
            let mut first_invalidation_idx: Option<usize> = None;
            let mut current_touch_points: Vec<TouchPoint> = Vec::new();
            
            debug!("[ENRICH_GENERATOR] Zone {} activity check starts at index: {}", zone_id_log, check_start_idx);
            
            // CRITICAL: Determine initial state (MATCH BULK HANDLER EXACTLY)
            let mut is_outside_zone = true;
            if check_start_idx < total_candle_count {
                let first_candle_after_formation = &all_candles[check_start_idx];
                if is_supply {
                    is_outside_zone = first_candle_after_formation.high < proximal_line;
                } else {
                    is_outside_zone = first_candle_after_formation.low > proximal_line;
                }
                debug!("[ENRICH_GENERATOR] Zone {} initial state: {} (first candle H:{:.5} L:{:.5})", 
                       zone_id_log, if is_outside_zone { "OUTSIDE" } else { "INSIDE" },
                       first_candle_after_formation.high, first_candle_after_formation.low);
            } else {
                warn!("[ENRICH_GENERATOR] Zone {} has no candles after formation!", zone_id_log);
            }
            
            // Process candles after formation (MATCH BULK HANDLER LOGIC)
            if check_start_idx < total_candle_count {
                for i in check_start_idx..total_candle_count {
                    if !is_currently_active { break; }
                    
                    let candle = &all_candles[i];
                    if !candle.high.is_finite() || !candle.low.is_finite() || !candle.close.is_finite() {
                        continue;
                    }
                    
                    let mut interaction_occurred = false;
                    let mut interaction_price = 0.0;
                    
                    // CRITICAL: Check for invalidation FIRST (MATCH BULK HANDLER)
                    if is_supply {
                        if candle.high > distal_line {
                            is_currently_active = false;
                            first_invalidation_idx = Some(i);
                            enriched_zone.invalidation_time = Some(candle.time.clone());
                            enriched_zone.end_time = Some(candle.time.clone());
                            enriched_zone.end_idx = Some(i as u64);
                            debug!("[ENRICH_GENERATOR] Zone {} INVALIDATED at [{}]: {:.5} > {:.5}", 
                                   zone_id_log, i, candle.high, distal_line);
                            break;
                        }
                    } else {
                        if candle.low < distal_line {
                            is_currently_active = false;
                            first_invalidation_idx = Some(i);
                            enriched_zone.invalidation_time = Some(candle.time.clone());
                            enriched_zone.end_time = Some(candle.time.clone());
                            enriched_zone.end_idx = Some(i as u64);
                            debug!("[ENRICH_GENERATOR] Zone {} INVALIDATED at [{}]: {:.5} < {:.5}", 
                                   zone_id_log, i, candle.low, distal_line);
                            break;
                        }
                    }
                    
                    // Check for interaction with proximal line
                    if is_supply {
                        if candle.high >= proximal_line {
                            interaction_occurred = true;
                            interaction_price = candle.high;
                        }
                    } else {
                        if candle.low <= proximal_line {
                            interaction_occurred = true;
                            interaction_price = candle.low;
                        }
                    }
                    
                    // CRITICAL: Count touches only when transitioning from outside to inside (MATCH BULK HANDLER)
                    if interaction_occurred && is_outside_zone {
                        touch_count += 1;
                        current_touch_points.push(TouchPoint {
                            time: candle.time.clone(),
                            price: interaction_price,
                        });
                        debug!("[ENRICH_GENERATOR] Zone {} TOUCH #{} at [{}]: {:.5}", 
                               zone_id_log, touch_count, i, interaction_price);
                    }
                    
                    // Update state for next iteration
                    is_outside_zone = !interaction_occurred;
                }
            }
            
            // CRITICAL: Calculate bars_active correctly (MATCH BULK HANDLER EXACTLY)
            if !is_currently_active {
                // Zone was invalidated - count bars from activity start to invalidation
                if let Some(invalidation_idx) = first_invalidation_idx {
                    if invalidation_idx > check_start_idx {
                        enriched_zone.bars_active = Some((invalidation_idx - check_start_idx) as u64);
                    } else {
                        enriched_zone.bars_active = Some(0);
                    }
                } else {
                    enriched_zone.bars_active = Some(0);
                }
            } else {
                // Zone is still active - count bars from activity start to end of data
                if total_candle_count > check_start_idx {
                    enriched_zone.bars_active = Some((total_candle_count - check_start_idx) as u64);
                } else {
                    enriched_zone.bars_active = Some(0);
                }
            }
            
            // Finalize enrichment (MATCH BULK HANDLER)
            enriched_zone.is_active = is_currently_active;
            enriched_zone.touch_count = Some(touch_count);
            
            if !current_touch_points.is_empty() {
                enriched_zone.touch_points = Some(current_touch_points);
            }
            
            // Calculate strength score
            let score_raw = 100.0 - (touch_count as f64 * 10.0);
            enriched_zone.strength_score = Some(score_raw.max(0.0));
            
            debug!("[ENRICH_GENERATOR] Zone {} FINAL: Active={} Touches={} BarsActive={:?} EndTime={:?}", 
                   zone_id_log, enriched_zone.is_active, touch_count, 
                   enriched_zone.bars_active, enriched_zone.end_time);
            
            enriched_zones.push(enriched_zone);
        }
    } else {
        debug!("[ENRICH_GENERATOR] No {} zones found", zone_type_str);
    }
    
    Ok(enriched_zones)
}

pub fn debug_zone_enrichment_step_by_step(
    zone_json: &Value,
    candles: &[CandleData],
    is_supply: bool,
    symbol: &str,
    timeframe: &str,
) {
    debug_to_file("=================================================================");
    debug_to_file(&format!("🔍 ZONE ANALYSIS: {}/{}", symbol, timeframe));
    debug_to_file("=================================================================");
    
    // Extract zone data
    let zone_high = zone_json.get("zone_high").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let zone_low = zone_json.get("zone_low").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let start_idx = zone_json.get("start_idx").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let end_idx = zone_json.get("end_idx").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let start_time = zone_json.get("start_time").and_then(|v| v.as_str()).unwrap_or("unknown");
    
    debug_to_file(&format!("🔍 Zone Data:"));
    debug_to_file(&format!("   - High: {:.5}, Low: {:.5}", zone_high, zone_low));
    debug_to_file(&format!("   - Start Index: {}, End Index: {}", start_idx, end_idx));
    debug_to_file(&format!("   - Start Time: {}", start_time));
    debug_to_file(&format!("   - Total Candles Available: {}", candles.len()));
    
    if end_idx >= candles.len() {
        debug_to_file(&format!("❌ CRITICAL ERROR: end_idx ({}) >= candles.len() ({})", end_idx, candles.len()));
        return;
    }
    
    // Show formation candles
    debug_to_file(&format!("🔍 Formation Candles:"));
    for i in start_idx..=end_idx.min(candles.len() - 1) {
        let candle = &candles[i];
        debug_to_file(&format!("   [{:3}] {}: O:{:.5} H:{:.5} L:{:.5} C:{:.5}", 
                      i, candle.time, candle.open, candle.high, candle.low, candle.close));
    }
    
    // Define zone logic
    let (proximal_line, distal_line) = if is_supply {
        (zone_low, zone_high) // Supply: Proximal=Low, Distal=High
    } else {
        (zone_high, zone_low) // Demand: Proximal=High, Distal=Low
    };
    
    debug_to_file(&format!("🔍 Zone Logic:"));
    debug_to_file(&format!("   - Zone Type: {}", if is_supply { "SUPPLY" } else { "DEMAND" }));
    debug_to_file(&format!("   - Proximal Line (entry): {:.5}", proximal_line));
    debug_to_file(&format!("   - Distal Line (break): {:.5}", distal_line));
    
    // Check candles after formation
    let check_start_idx = end_idx + 1;
    debug_to_file(&format!("🔍 Activity Check:"));
    debug_to_file(&format!("   - Checking from index: {}", check_start_idx));
    
    if check_start_idx >= candles.len() {
        debug_to_file(&format!("❌ NO CANDLES AFTER FORMATION! check_start_idx ({}) >= candles.len() ({})", 
                      check_start_idx, candles.len()));
        return;
    }
    
    // Show first 10 candles after formation
    let show_count = (candles.len() - check_start_idx).min(10);
    debug_to_file(&format!("🔍 First {} candles after formation:", show_count));
    for i in 0..show_count {
        let idx = check_start_idx + i;
        let candle = &candles[idx];
        debug_to_file(&format!("   [{:3}] {}: H:{:.5} L:{:.5} C:{:.5}", 
                      idx, candle.time, candle.high, candle.low, candle.close));
    }
    
    // Determine initial state
    let first_candle = &candles[check_start_idx];
    let initial_is_outside = if is_supply {
        first_candle.high < proximal_line
    } else {
        first_candle.low > proximal_line
    };
    
    debug_to_file(&format!("🔍 Initial State Analysis:"));
    debug_to_file(&format!("   - First candle after formation [{}]: {}", check_start_idx, first_candle.time));
    debug_to_file(&format!("   - First candle High: {:.5}, Low: {:.5}", first_candle.high, first_candle.low));
    debug_to_file(&format!("   - Comparison: {} zone, proximal line = {:.5}", 
                  if is_supply { "Supply" } else { "Demand" }, proximal_line));
    if is_supply {
        debug_to_file(&format!("   - Supply logic: first_candle.high ({:.5}) < proximal ({:.5}) = {}", 
                      first_candle.high, proximal_line, first_candle.high < proximal_line));
    } else {
        debug_to_file(&format!("   - Demand logic: first_candle.low ({:.5}) > proximal ({:.5}) = {}", 
                      first_candle.low, proximal_line, first_candle.low > proximal_line));
    }
    debug_to_file(&format!("   - Initial state: {}", if initial_is_outside { "OUTSIDE" } else { "INSIDE" }));
    
    // Simulate the enrichment logic step by step
    let mut is_currently_active = true;
    let mut touch_count: i64 = 0;
    let mut is_outside_zone = initial_is_outside;
    let mut first_invalidation_idx: Option<usize> = None;
    
    debug_to_file(&format!("🔍 Step-by-Step Touch Analysis:"));
    
    let analysis_count = (candles.len() - check_start_idx).min(15); // Analyze first 15 candles
    for i in 0..analysis_count {
        let idx = check_start_idx + i;
        if !is_currently_active {
            debug_to_file(&format!("   [{}] ❌ Zone already invalidated, stopping analysis", idx));
            break;
        }
        
        let candle = &candles[idx];
        debug_to_file(&format!("   [{}] Analyzing candle: {}", idx, candle.time));
        debug_to_file(&format!("        High: {:.5}, Low: {:.5}, Close: {:.5}", 
                      candle.high, candle.low, candle.close));
        debug_to_file(&format!("        Current state: {}", if is_outside_zone { "OUTSIDE" } else { "INSIDE" }));
        
        // Check for invalidation first
        let mut invalidated_this_candle = false;
        if is_supply {
            if candle.high > distal_line {
                invalidated_this_candle = true;
                is_currently_active = false;
                first_invalidation_idx = Some(idx);
                debug_to_file(&format!("        ❌ SUPPLY ZONE INVALIDATED: High {:.5} > Distal {:.5}", 
                              candle.high, distal_line));
            }
        } else {
            if candle.low < distal_line {
                invalidated_this_candle = true;
                is_currently_active = false;
                first_invalidation_idx = Some(idx);
                debug_to_file(&format!("        ❌ DEMAND ZONE INVALIDATED: Low {:.5} < Distal {:.5}", 
                              candle.low, distal_line));
            }
        }
        
        if invalidated_this_candle {
            break;
        }
        
        // Check for interaction
        let mut interaction_occurred = false;
        let mut interaction_price = 0.0;
        
        if is_supply {
            if candle.high >= proximal_line {
                interaction_occurred = true;
                interaction_price = candle.high;
                debug_to_file(&format!("        🎯 SUPPLY INTERACTION: High {:.5} >= Proximal {:.5}", 
                              candle.high, proximal_line));
            } else {
                debug_to_file(&format!("        ⭕ NO INTERACTION: High {:.5} < Proximal {:.5}", 
                              candle.high, proximal_line));
            }
        } else {
            if candle.low <= proximal_line {
                interaction_occurred = true;
                interaction_price = candle.low;
                debug_to_file(&format!("        🎯 DEMAND INTERACTION: Low {:.5} <= Proximal {:.5}", 
                              candle.low, proximal_line));
            } else {
                debug_to_file(&format!("        ⭕ NO INTERACTION: Low {:.5} > Proximal {:.5}", 
                              candle.low, proximal_line));
            }
        }
        
        // Check if we should count this as a touch
        if interaction_occurred && is_outside_zone {
            touch_count += 1;
            debug_to_file(&format!("        ✅ TOUCH COUNTED: #{} (was outside: true, price: {:.5})", 
                          touch_count, interaction_price));
        } else if interaction_occurred && !is_outside_zone {
            debug_to_file(&format!("        ⏸️  NO TOUCH: Interaction occurred but already inside zone"));
        } else {
            debug_to_file(&format!("        ⭕ NO TOUCH: No interaction with zone"));
        }
        
        // Update state for next candle
        let old_state = is_outside_zone;
        is_outside_zone = !interaction_occurred;
        debug_to_file(&format!("        📍 State for next candle: {} → {}", 
                      if old_state { "OUTSIDE" } else { "INSIDE" },
                      if is_outside_zone { "OUTSIDE" } else { "INSIDE" }));
        
        debug_to_file(""); // Empty line for readability
    }
    
    // Calculate final stats
    let bars_active = if !is_currently_active {
        first_invalidation_idx.map(|idx| if idx > check_start_idx { idx - check_start_idx } else { 0 }).unwrap_or(0)
    } else {
        if candles.len() > check_start_idx { candles.len() - check_start_idx } else { 0 }
    };
    
    debug_to_file(&format!("🔍 FINAL RESULTS:"));
    debug_to_file(&format!("   - Touch Count: {}", touch_count));
    debug_to_file(&format!("   - Bars Active: {}", bars_active));
    debug_to_file(&format!("   - Is Active: {}", is_currently_active));
    debug_to_file(&format!("   - Invalidation Index: {:?}", first_invalidation_idx));
    debug_to_file(&format!("   - Total candles analyzed: {}", analysis_count));
    debug_to_file("=================================================================");
    debug_to_file(""); // Empty line
}