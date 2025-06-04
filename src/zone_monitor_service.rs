// // src/zone_monitor_service.rs
// use crate::types::{CandleData, StoredZone};
// use std::collections::HashMap;
// use std::env;
// use std::sync::Arc;
// use tokio::sync::Mutex;

// use chrono::{Datelike, Utc, Weekday};
// use futures_util::StreamExt;
// use log::{debug, error, info, warn};
// use reqwest::Client as HttpClient;
// use serde_json::json;
// use tokio_tungstenite::tungstenite::error::Error as WsError;
// use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
// use url::Url;

// // Using StoredZone from types.rs
// // These are needed by fetch_and_cache_active_zones for parsing CSV from InfluxDB
// // Ensure these are pub in main.rs or moved to a shared module like types.rs
// use crate::detect::fetch_candles_direct;
// use crate::types::{map_csv_to_stored_zone, ZoneCsvRecord};

// const TS_WEBSOCKET_URL: &str = "ws://localhost:8081";
// const ORDER_PLACEMENT_URL_BASE: &str = "http://localhost:"; // Port will be added from env

// use crate::proximity_detector::ProximityDetector;

// // Add this field to your LiveZoneState struct:
// #[derive(Clone, Debug)]
// pub struct LiveZoneState {
//     pub zone_data: StoredZone,
//     pub live_touches_this_cycle: i64,
//     pub trade_attempted_this_cycle: bool,
//     pub is_outside_zone: bool,
//     // Add this field:
//     pub proximity_detector: std::sync::Arc<tokio::sync::Mutex<ProximityDetector>>,
// }

// #[derive(serde::Deserialize, Debug, Clone)]
// #[serde(rename_all = "camelCase")]
// pub struct BarUpdateData {
//     time: String,
//     open: f64,
//     high: f64,
//     low: f64,
//     close: f64,
//     volume: u64,
//     symbol_id: u32,
//     symbol: String,
//     timeframe: String,
//     is_new_bar: bool,
// }

// #[derive(serde::Deserialize, Debug, Clone)]
// struct WebSocketMessage {
//     #[serde(rename = "type")]
//     msg_type: String,
//     data: Option<BarUpdateData>,
// }

// #[derive(Clone, Debug)]
// pub struct LiveZoneState {
//     pub zone_data: StoredZone,
//     pub live_touches_this_cycle: i64,
//     pub trade_attempted_this_cycle: bool,
//     pub is_outside_zone: bool,
// }

// pub type ActiveZoneCache = Arc<Mutex<HashMap<String, LiveZoneState>>>;

// // ============================================================================
// // ADD THESE SOUND FUNCTIONS TO YOUR zone_monitor_service.rs FILE
// // Place them AFTER your existing imports and BEFORE the fetch_and_cache_active_zones function
// // ============================================================================

// // macOS-only sound notification functions
// fn play_startup_sound() {
//     info!("[STARTUP_SOUND] 🎵 Zone Monitor is ready - playing startup sound!");
//     std::process::Command::new("afplay")
//         .arg("/System/Library/Sounds/Ping.aiff")
//         .spawn()
//         .ok();
// }

// fn play_trade_success_sound(trade_side: i32, symbol: &str) {
//     let trade_type = if trade_side == 0 { "BUY" } else { "SELL" };
//     info!(
//         "[TRADE_SOUND] 🔊 Playing {} trade success sound for {}",
//         trade_type, symbol
//     );

//     match trade_side {
//         0 => {
//             // BUY trade - Glass sound (positive/rising)
//             std::process::Command::new("afplay")
//                 .arg("/System/Library/Sounds/Glass.aiff")
//                 .spawn()
//                 .ok();
//         }
//         1 => {
//             // SELL trade - Purr sound (different tone)
//             std::process::Command::new("afplay")
//                 .arg("/System/Library/Sounds/Purr.aiff")
//                 .spawn()
//                 .ok();
//         }
//         _ => {
//             // Default - Blow sound
//             std::process::Command::new("afplay")
//                 .arg("/System/Library/Sounds/Blow.aiff")
//                 .spawn()
//                 .ok();
//         }
//     }
// }

// fn play_trade_error_sound() {
//     warn!("[TRADE_SOUND] 🔇 Playing trade error sound");
//     std::process::Command::new("afplay")
//         .arg("/System/Library/Sounds/Sosumi.aiff")
//         .spawn()
//         .ok();
// }

// // Updated logging function with sound (REPLACE your existing log_trade_api_response function with this)
// fn log_trade_api_response_with_sound(
//     zone_id: &str,
//     symbol: &str,
//     success: bool,
//     status_code: Option<u16>,
//     response_body: &str,
//     trade_side: i32, // 0 = BUY, 1 = SELL
// ) {
//     let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
//     let status = if success { "SUCCESS" } else { "FAILED" };
//     let code_str = status_code.map_or("N/A".to_string(), |c| c.to_string());

//     let log_line = format!(
//         "[{}] TRADE_API_RESPONSE | Zone: {} | Symbol: {} | Status: {} | Code: {} | Response: {}\n",
//         timestamp,
//         zone_id,
//         symbol,
//         status,
//         code_str,
//         response_body.chars().take(200).collect::<String>()
//     );

//     if let Ok(mut writer) = TRADE_LOG_FILE.lock() {
//         if let Err(e) = writer.write_all(log_line.as_bytes()) {
//             error!("[TRADE_LOG_ERROR] Failed to write API response: {}", e);
//         }
//         let _ = writer.flush();
//     }

//     // PLAY SOUND based on result
//     if success {
//         play_trade_success_sound(trade_side, symbol);
//     } else {
//         play_trade_error_sound();
//     }
// }

// // Helper function to build OR filter strings for Flux
// fn build_or_filter(tag_name: &str, values: &[String]) -> String {
//     if values.is_empty() {
//         return "true".to_string(); // Pass all if no specific values to filter by
//     }
//     values
//         .iter()
//         .map(|val| format!("r.{} == \"{}\"", tag_name, val.replace("\"", "\\\""))) // Escape quotes in val
//         .collect::<Vec<String>>()
//         .join(" or ")
// }

// async fn fetch_and_cache_active_zones(
//     passed_in_cache: ActiveZoneCache,
//     http_client: &HttpClient, 
//     influx_host: &str,
//     influx_org: &str,
//     influx_token: &str,
//     zone_bucket: &str,
//     zone_measurement: &str,
//     target_symbols: &[String],
//     target_pattern_timeframes: &[String],
// ) -> Result<usize, String> {
//     info!(
//         "[ZONE_MONITOR_CACHE] Refreshing cache from InfluxDB. Symbols: {:?}, TFs: {:?}",
//         target_symbols, target_pattern_timeframes
//     );

//     if target_symbols.is_empty() && target_pattern_timeframes.is_empty() {
//         warn!("[ZONE_MONITOR_CACHE] No target symbols or TFs specified for specific filtering in cache refresh. Will fetch all active zones based on query lookback.");
//     }

//     let symbol_filter_str = build_or_filter("symbol", target_symbols);
//     let timeframe_filter_str = build_or_filter("timeframe", target_pattern_timeframes);

//     let query_lookback_days_str =
//         env::var("ZONE_MONITOR_DB_QUERY_LOOKBACK_DAYS").unwrap_or_else(|_| "35".to_string());
//     let query_lookback_days = query_lookback_days_str.parse::<i64>().unwrap_or(35);

//     let flux_query = format!(
//         r#"
//         from(bucket: "{zone_bucket}")
//           |> range(start: -{query_lookback_days}d)
//           |> filter(fn: (r) => r._measurement == "{zone_measurement}")
//           |> filter(fn: (r) => exists r.symbol and ({symbol_filter_str}))      // CORRECTED: Use symbol_filter_str
//           |> filter(fn: (r) => exists r.timeframe and ({timeframe_filter_str}))  // CORRECTED: Use timeframe_filter_str
//           |> pivot(
//               rowKey:["_time", "symbol", "timeframe", "pattern", "zone_type", "zone_id"], 
//               columnKey: ["_field"],
//               valueColumn: "_value"
//              )
//           |> filter(fn: (r) => exists r.is_active and r.is_active == 1) 
//           |> sort(columns: ["_time"], desc: true)
//         "#,
//         zone_bucket = zone_bucket,
//         query_lookback_days = query_lookback_days,
//         zone_measurement = zone_measurement,
//         symbol_filter_str = symbol_filter_str,      
//         timeframe_filter_str = timeframe_filter_str 
//     );

//     let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);
//     debug!(
//         "[ZONE_MONITOR_CACHE] Influx query for cache refresh: {}",
//         flux_query
//     );

//     let response_text = match http_client
//         .post(&query_url)
//         .bearer_auth(influx_token)
//         .header("Accept", "application/csv")
//         .header("Content-Type", "application/json")
//         .json(&json!({ "query": flux_query, "type": "flux" }))
//         .send()
//         .await
//     {
//         Ok(response) => {
//             let status = response.status();
//             if status.is_success() {
//                 response
//                     .text()
//                     .await
//                     .map_err(|e| format!("Influx response text error: {}", e))?
//             } else {
//                 let err_text = response
//                     .text()
//                     .await
//                     .unwrap_or_else(|_| "Unknown error body (failed to read)".to_string());
//                 error!(
//                     "[ZONE_MONITOR_CACHE] Influx query failed (status {}): {}. Query: {}",
//                     status, err_text, flux_query
//                 );
//                 return Err(format!(
//                     "Influx query failed (status {}): {}",
//                     status, err_text
//                 ));
//             }
//         }
//         Err(e) => {
//             error!(
//                 "[ZONE_MONITOR_CACHE] Influx request failed: {}. Query: {}",
//                 e, flux_query
//             );
//             return Err(format!("Influx request failed: {}", e));
//         }
//     };

//     if response_text.len() < 5000 { 
//         log::info!("[RAW_CSV_DEBUG] START --- Raw CSV for Zone Cache ({} bytes) --- START", response_text.len());
//         log::info!("{}", response_text);
//         log::info!("[RAW_CSV_DEBUG] END --- Raw CSV for Zone Cache --- END");
//     } else {
//         log::info!("[RAW_CSV_DEBUG] Raw CSV for Zone Cache is large ({} bytes). Logging first 2000 and last 1000 chars.", response_text.len());
//         log::info!("[RAW_CSV_DEBUG] First 2000 chars:\n{}", &response_text[..2000.min(response_text.len())]);
//         log::info!("[RAW_CSV_DEBUG] Last 1000 chars:\n{}", &response_text[(response_text.len() - 1000.min(response_text.len()))..]);
//     }

//     let mut processed_lines: Vec<String> = Vec::new();
//     let mut actual_header_line: Option<String> = None;
//     let mut first_header_found_and_added = false;

//     for line_content in response_text.lines() {
//         let trimmed_line_content = line_content.trim();
//         if trimmed_line_content.is_empty() {
//             if !first_header_found_and_added && trimmed_line_content.starts_with('#') {
//                  processed_lines.push(trimmed_line_content.to_string());
//             }
//             continue; 
//         }
//         if trimmed_line_content.starts_with('#') {
//             if !first_header_found_and_added {
//                 processed_lines.push(trimmed_line_content.to_string());
//             } 
//             continue;
//         }

//         if !first_header_found_and_added {
//             actual_header_line = Some(trimmed_line_content.to_string());
//             processed_lines.push(trimmed_line_content.to_string());
//             first_header_found_and_added = true;
//             info!("[ZONE_MONITOR_CACHE_CSV_PROCESS] Identified primary header: {}", trimmed_line_content);
//         } else {
//             if Some(trimmed_line_content) != actual_header_line.as_deref() {
//                 processed_lines.push(trimmed_line_content.to_string());
//             } else {
//                 info!("[ZONE_MONITOR_CACHE_CSV_PROCESS] Skipping repeated header line in CSV: {}", trimmed_line_content);
//             }
//         }
//     }

//     let cleaned_csv_text = processed_lines.join("\n");
//     let mut fetched_zones_from_db: Vec<StoredZone> = Vec::new();

//     if cleaned_csv_text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).count() > 1 {
//         let mut rdr = csv::ReaderBuilder::new()
//             .has_headers(true) 
//             .flexible(true)    
//             .comment(Some(b'#')) 
//             .from_reader(cleaned_csv_text.as_bytes());

//         for (idx, result) in rdr.deserialize::<ZoneCsvRecord>().enumerate() {
//             match result {
//                 Ok(csv_rec) => {
//                     log::debug!("[ZONE_MONITOR_CACHE_CSV_DEBUG] Deserialized (cleaned) CSV Record #{}: {:?}", idx + 1, csv_rec);
//                     let stored_zone = map_csv_to_stored_zone(csv_rec); 
//                     log::debug!("[ZONE_MONITOR_CACHE_MAP_DEBUG] Mapped StoredZone #{}: {:?}", idx + 1, stored_zone);
//                     fetched_zones_from_db.push(stored_zone);
//                 }
//                 Err(e) => warn!("[ZONE_MONITOR_CACHE] CSV deserialize error (from cleaned CSV) for row #{}: {}", idx + 1, e),
//             }
//         }
//     } else {
//         info!("[ZONE_MONITOR_CACHE] No data rows in (cleaned) CSV response from InfluxDB.");
//     }

//     let mut cache = passed_in_cache.lock().await;
//     cache.clear(); 

//     log::info!("[CACHE_POPULATE] About to populate cache. Number of StoredZones from DB parse (after cleaning): {}", fetched_zones_from_db.len());
//     let proximity_detector = std::sync::Arc::new(tokio::sync::Mutex::new(ProximityDetector::new()));

//     for zone_to_cache in fetched_zones_from_db {
//         if let Some(id_to_log) = &zone_to_cache.zone_id {
//             log::info!(
//                 "[CACHE_INSERT_CHECK] Zone ID: {:?}, Symbol: {:?}, TF: {:?}, Type: {:?}, \
//                 DB_StartTime: {:?}, DB_ZoneHigh: {:?}, DB_ZoneLow: {:?}, DB_TouchCount: {:?}, \
//                 DB_IsActive: {}, DB_StartIdx: {:?}, DB_EndIdx: {:?}",
//                 id_to_log,
//                 zone_to_cache.symbol,
//                 zone_to_cache.timeframe,
//                 zone_to_cache.zone_type,
//                 zone_to_cache.start_time,
//                 zone_to_cache.zone_high,
//                 zone_to_cache.zone_low,
//                 zone_to_cache.touch_count,
//                 zone_to_cache.is_active,
//                 zone_to_cache.start_idx,
//                 zone_to_cache.end_idx
//             );

//             if zone_to_cache.zone_type.as_deref().unwrap_or("").to_lowercase().contains("supply") &&
//                zone_to_cache.zone_high.is_some() &&
//                zone_to_cache.zone_high == Some(0.0) {
//                 log::error!(
//                     "[CACHE_INSERT_ERROR_DETECTED] SUPPLY ZONE ID {:?} HAS ZONE_HIGH OF 0.0 BEFORE CACHE INSERT! StoredZone: {:?}",
//                     id_to_log,
//                     &zone_to_cache
//                 );
//             }
//             if zone_to_cache.touch_count.is_none() {
//                  log::warn!(
//                     "[CACHE_INSERT_WARN_DETECTED] ZONE ID {:?} HAS touch_count of None BEFORE CACHE INSERT! StoredZone: {:?}",
//                     id_to_log,
//                     &zone_to_cache
//                  );
//             }
            
//             // Ensure LiveZoneState is correctly referenced
//             // If LiveZoneState is in the same module (zone_monitor_service.rs), this is fine.
//             // Otherwise, use fully qualified path e.g. crate::types::LiveZoneState or crate::LiveZoneState
//             cache.insert(
//                 id_to_log.clone(),
//                 LiveZoneState { 
//                     zone_data: zone_to_cache,
//                     live_touches_this_cycle: 0,
//                     trade_attempted_this_cycle: false,
//                     is_outside_zone: true, 
//                     proximity_detector: std::sync::Arc::clone(&proximity_detector),
//                 },
//             );
//         } else {
//             log::warn!("[CACHE_POPULATE] Encountered a StoredZone from DB parse (after cleaning) with no zone_id. Skipping cache insert. Zone: {:?}", zone_to_cache);
//         }
//     }
//     let final_cache_size = cache.len();
//     info!(
//         "[ZONE_MONITOR_CACHE] Refreshed cache with {} zones.",
//         final_cache_size
//     );
//     Ok(final_cache_size)
// }

// pub async fn run_zone_monitor_service(passed_in_cache: ActiveZoneCache) {
//     info!("[ZONE_MONITOR] Starting Zone Monitor Service...");

//     let influx_host = env::var("INFLUXDB_HOST").expect("INFLUXDB_HOST not set for Zone Monitor");
//     let influx_org = env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG not set for Zone Monitor");
//     let influx_token = env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN for Zone Monitor not set");
//     let zone_bucket = env::var("GENERATOR_WRITE_BUCKET")
//         .expect("GENERATOR_WRITE_BUCKET not set for Zone Monitor");
//     let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT")
//         .expect("GENERATOR_ZONE_MEASUREMENT not set for Zone Monitor");
//     let ts_app_port = env::var("TYPESCRIPT_APP_PORT").unwrap_or_else(|_| "3001".to_string());
//     let order_placement_url = format!("{}{}/placeOrder", ORDER_PLACEMENT_URL_BASE, ts_app_port);

//     let monitor_target_symbols: Vec<String> = env::var("ZONE_MONITOR_SYMBOLS")
//         .unwrap_or_else(|_| "EURUSD_SB,USDJPY_SB,GBPJPY_SB,AUDJPY_SB,CHFJPY_SB".to_string())
//         .split(',')
//         .map(|s| s.trim().to_string())
//         .collect();
//     let monitor_pattern_timeframes: Vec<String> = env::var("ZONE_MONITOR_PATTERNTFS")
//         .unwrap_or_else(|_| "1h,4h,1d".to_string())
//         .split(',')
//         .map(|s| s.trim().to_string())
//         .collect();

//     let sl_pips: f64 = env::var("STRATEGY_SL_PIPS")
//         .unwrap_or_else(|_| "4.0".to_string())
//         .parse()
//         .expect("Invalid STRATEGY_SL_PIPS");
//     let tp_pips: f64 = env::var("STRATEGY_TP_PIPS")
//         .unwrap_or_else(|_| "70.0".to_string())
//         .parse()
//         .expect("Invalid STRATEGY_TP_PIPS");
//     let lot_size: f64 = env::var("STRATEGY_LOT_SIZE")
//         .unwrap_or_else(|_| "0.01".to_string())
//         .parse()
//         .expect("Invalid STRATEGY_LOT_SIZE");
//     let allowed_days_str: Vec<String> = env::var("STRATEGY_ALLOWED_DAYS")
//         .unwrap_or_else(|_| "Mon,Tue,Wed".to_string())
//         .split(',')
//         .map(|s| s.trim().to_string())
//         .collect();
//     let allowed_days: Vec<Weekday> = parse_allowed_days_from_strings(&allowed_days_str);
//     let historical_touch_limit: i64 = env::var("STRATEGY_MAX_HIST_TOUCHES")
//         .unwrap_or_else(|_| "2".to_string())
//         .parse()
//         .expect("Invalid STRATEGY_MAX_HIST_TOUCHES");

//     let http_client_arc = Arc::new(HttpClient::new());

//     if let Err(e) = fetch_and_cache_active_zones(
//         Arc::clone(&passed_in_cache),
//         &http_client_arc,
//         &influx_host,
//         &influx_org,
//         &influx_token,
//         &zone_bucket,
//         &zone_measurement,
//         &monitor_target_symbols,
//         &monitor_pattern_timeframes,
//     )
//     .await
//     {
//         error!("[ZONE_MONITOR] Failed initial zone cache load: {}. Service may continue with an empty cache.", e);
//     }
//     play_startup_sound();

//     let cache_clone_for_proximity_cleanup = Arc::clone(&passed_in_cache);
//     tokio::spawn(async move {
//         let mut cleanup_interval = tokio::time::interval(tokio::time::Duration::from_secs(3600)); // Every hour
//         loop {
//             cleanup_interval.tick().await;
//             debug!("[PROXIMITY_CLEANUP] Running proximity detector cleanup...");
            
//             let cache_guard = cache_clone_for_proximity_cleanup.lock().await;
//             if let Some((_, first_zone)) = cache_guard.iter().next() {
//                 let proximity_detector_arc = std::sync::Arc::clone(&first_zone.proximity_detector);
//                 drop(cache_guard);
                
//                 let mut proximity_detector = proximity_detector_arc.lock().await;
//                 proximity_detector.cleanup_old_states();
//             }
//         }
//     });

//     let cache_clone_for_refresh = Arc::clone(&passed_in_cache);
//     let http_client_for_refresh = Arc::clone(&http_client_arc);
//     let influx_details_for_refresh = (
//         influx_host.clone(),
//         influx_org.clone(),
//         influx_token.clone(),
//         zone_bucket.clone(),
//         zone_measurement.clone(),
//     );
//     let symbols_for_refresh = monitor_target_symbols.clone();
//     let timeframes_for_refresh = monitor_pattern_timeframes.clone();

//     tokio::spawn(async move {
//         let refresh_interval_secs_str =
//             env::var("ZONE_MONITOR_CACHE_REFRESH_SECS").unwrap_or_else(|_| "60".to_string());
//         let refresh_interval_secs = refresh_interval_secs_str.parse::<u64>().unwrap_or(60);
//         info!(
//             "[ZONE_MONITOR_CACHE_REFRESH_TASK] Starting periodic cache refresh every {} seconds.",
//             refresh_interval_secs
//         );
//         let mut interval =
//             tokio::time::interval(tokio::time::Duration::from_secs(refresh_interval_secs));
//         loop {
//             interval.tick().await;
//             debug!("[ZONE_MONITOR_CACHE_REFRESH_TASK] Triggering periodic cache refresh...");
//             if let Err(e) = fetch_and_cache_active_zones(
//                 Arc::clone(&cache_clone_for_refresh),
//                 &http_client_for_refresh,
//                 &influx_details_for_refresh.0,
//                 &influx_details_for_refresh.1,
//                 &influx_details_for_refresh.2,
//                 &influx_details_for_refresh.3,
//                 &influx_details_for_refresh.4,
//                 &symbols_for_refresh,
//                 &timeframes_for_refresh,
//             )
//             .await
//             {
//                 error!(
//                     "[ZONE_MONITOR_CACHE_REFRESH_TASK] Error during periodic cache refresh: {}",
//                     e
//                 );
//             }
//         }
//     });

//     loop {
//         info!(
//             "[ZONE_MONITOR] Attempting to connect to TypeScript WebSocket Price Stream at {}...",
//             TS_WEBSOCKET_URL
//         );
//         match connect_async(Url::parse(TS_WEBSOCKET_URL).expect("Invalid WebSocket URL")).await {
//             Ok((ws_stream, _response)) => {
//                 info!(
//                     "[ZONE_MONITOR] Successfully connected to TypeScript WebSocket Price Stream."
//                 );
//                 let (_ws_sender, mut ws_receiver) = ws_stream.split();
//                 while let Some(msg_result) = ws_receiver.next().await {
//                     match msg_result {
//                         Ok(Message::Text(text)) => {
//                             match serde_json::from_str::<WebSocketMessage>(&text) {
//                                 Ok(ws_msg) => {
//                                     if ws_msg.msg_type == "BAR_UPDATE" {
//                                         if let Some(bar_data) = ws_msg.data {
//                                             process_bar_live_touch(
//                                                 bar_data,
//                                                 Arc::clone(&passed_in_cache),
//                                                 sl_pips,
//                                                 tp_pips,
//                                                 lot_size,
//                                                 &allowed_days,
//                                                 historical_touch_limit,
//                                                 &order_placement_url,
//                                                 &http_client_arc,
//                                             )
//                                             .await;
//                                         }
//                                     } else if ws_msg.msg_type == "CONNECTED" {
//                                         info!("[ZONE_MONITOR] Confirmed CONNECTED from TS WSS.");
//                                     }
//                                 }
//                                 Err(e) => {
//                                     warn!("[ZONE_MONITOR] Deserialize error: {}. Raw: {}", e, text);
//                                 }
//                             }
//                         }
//                         Ok(Message::Close(_)) => {
//                             warn!("[ZONE_MONITOR] TS WSS closed.");
//                             break;
//                         }
//                         Ok(Message::Ping(_ping_data)) => {
//                             debug!("[ZONE_MONITOR] Ping from TS WSS.");
//                         }
//                         Ok(Message::Pong(_)) => {
//                             debug!("[ZONE_MONITOR] Pong from TS WSS.");
//                         }
//                         Ok(msg) => {
//                             debug!("[ZONE_MONITOR] Other TS WSS msg: {:?}", msg);
//                         }
//                         Err(e) => {
//                             match e {
//                                 WsError::ConnectionClosed
//                                 | WsError::Protocol(_)
//                                 | WsError::AlreadyClosed => {
//                                     warn!("[ZONE_MONITOR] WebSocket connection error (likely closed): {}", e);
//                                 }
//                                 _ => {
//                                     error!("[ZONE_MONITOR] WebSocket read error: {}", e);
//                                 }
//                             }
//                             break;
//                         }
//                     }
//                 }
//             }
//             Err(e) => {
//                 error!("[ZONE_MONITOR] Failed to connect to TS WSS: {}", e);
//             }
//         }
//         info!("[ZONE_MONITOR] Disconnected from TS WSS. Retrying in 5 seconds...");
//         tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
//     }
// }

// pub async fn process_bar_live_touch(
//     bar: BarUpdateData,
//     zone_cache: ActiveZoneCache,
//     _sl_pips: f64,
//     _tp_pips: f64,
//     lot_size: f64,
//     allowed_days: &[Weekday],
//     historical_touch_limit: i64,
//     order_placement_url: &str,
//     http_client: &Arc<HttpClient>,
// ) {
//     // Add debug function for writing to test file
//     let write_debug = |msg: &str| {
//         use std::fs::OpenOptions;
//         use std::io::Write;
//         if let Ok(mut file) = OpenOptions::new()
//             .create(true)
//             .append(true)
//             .open("zone_monitor_test_results.txt")
//         {
//             let _ = writeln!(file, "{}", msg);
//         }
//     };

//     write_debug(&format!(
//         "🚀 PROCESS_BAR_LIVE_TOUCH CALLED - Bar: {} at {}",
//         bar.symbol, bar.time
//     ));

//     check_proximity_alerts(&bar, &zone_cache).await;

//     let mut cache_guard = zone_cache.lock().await;

//     write_debug(&format!("📦 Cache has {} zones", cache_guard.len()));

//     if cache_guard.is_empty() {
//         write_debug("❌ CACHE IS EMPTY - NO ZONES TO PROCESS!");
//         return;
//     }

//     for (zone_id, live_zone_state) in cache_guard.iter_mut() {
//         write_debug(&format!("🎯 Processing zone: {:?}", zone_id));
//         write_debug(&format!(
//             "   Zone active: {}",
//             live_zone_state.zone_data.is_active
//         ));
//         write_debug(&format!(
//             "   Zone symbol: {:?}",
//             live_zone_state.zone_data.symbol
//         ));
//         write_debug(&format!("   Bar symbol: {}", bar.symbol));

//         // CRITICAL FIX: Only check active status and symbol match first
//         // DO NOT check trade_attempted_this_cycle yet - invalidation must be checked first
//         if !live_zone_state.zone_data.is_active {
//             write_debug("   ⏭️ SKIPPING: Zone is not active");
//             continue;
//         }

//         if let Some(zone_symbol) = &live_zone_state.zone_data.symbol {
//             if zone_symbol != &bar.symbol {
//                 write_debug(&format!(
//                     "   ⏭️ SKIPPING: Symbol mismatch - zone: {}, bar: {}",
//                     zone_symbol, bar.symbol
//                 ));
//                 continue;
//             }

//             if let (Some(zone_high_val), Some(zone_low_val), Some(zone_type_str)) = (
//                 &live_zone_state.zone_data.zone_high,
//                 &live_zone_state.zone_data.zone_low,
//                 &live_zone_state.zone_data.zone_type,
//             ) {
//                 let proximal_line: f64;
//                 let trade_side_for_api: i32;
//                 let entry_price_check_against_bar: f64;

//                 if zone_type_str.eq_ignore_ascii_case("supply")
//                     || zone_type_str.eq_ignore_ascii_case("supply_zone")
//                 {
//                     proximal_line = *zone_low_val;
//                     trade_side_for_api = 1;
//                     entry_price_check_against_bar = bar.high;
//                 } else if zone_type_str.eq_ignore_ascii_case("demand")
//                     || zone_type_str.eq_ignore_ascii_case("demand_zone")
//                 {
//                     proximal_line = *zone_high_val;
//                     trade_side_for_api = 0;
//                     entry_price_check_against_bar = bar.low;
//                 } else {
//                     warn!(
//                         "[LIVE_TOUCH] Unknown zone_type: {} for zone ID: {:?}",
//                         zone_type_str, live_zone_state.zone_data.zone_id
//                     );
//                     continue;
//                 }

//                 // CRITICAL FIX: Check for invalidation FIRST (matches Generator logic)
//                 let zone_type_is_supply = zone_type_str.eq_ignore_ascii_case("supply")
//                     || zone_type_str.eq_ignore_ascii_case("supply_zone");

//                 let distal_line = if zone_type_is_supply {
//                     *zone_high_val // For supply zones, distal line is the high
//                 } else {
//                     *zone_low_val // For demand zones, distal line is the low
//                 };

//                 // DEBUG: Write all values to test file
//                 write_debug("🔍 INVALIDATION CHECK");
//                 write_debug(&format!(
//                     "   Zone ID: {:?}",
//                     live_zone_state.zone_data.zone_id
//                 ));
//                 write_debug(&format!(
//                     "   Zone Type: {} (is_supply: {})",
//                     zone_type_str, zone_type_is_supply
//                 ));
//                 write_debug(&format!("   Bar Time: {}", bar.time));
//                 write_debug(&format!("   Bar: High={:.6}, Low={:.6}", bar.high, bar.low));
//                 write_debug(&format!(
//                     "   Zone: High={:.6}, Low={:.6}",
//                     zone_high_val, zone_low_val
//                 ));
//                 write_debug(&format!("   Proximal Line: {:.6}", proximal_line));
//                 write_debug(&format!("   Distal Line: {:.6}", distal_line));

//                 // Check if zone should be invalidated
//                 let should_invalidate = if zone_type_is_supply {
//                     let result = bar.high > distal_line;
//                     write_debug(&format!(
//                         "   Supply invalidation check: {:.6} > {:.6} = {}",
//                         bar.high, distal_line, result
//                     ));
//                     result
//                 } else {
//                     let result = bar.low < distal_line;
//                     write_debug(&format!(
//                         "   Demand invalidation check: {:.6} < {:.6} = {}",
//                         bar.low, distal_line, result
//                     ));
//                     result
//                 };

//                 if should_invalidate {
//                     write_debug("🚨 INVALIDATION TRIGGERED! Setting zone inactive.");
//                     live_zone_state.zone_data.is_active = false;
//                     info!("[ZONE_INVALIDATED] Zone {} invalidated by bar high/low: {:.5}/{:.5}, distal: {:.5}",
//                           live_zone_state.zone_data.zone_id.as_deref().unwrap_or("N/A"), 
//                           bar.high, bar.low, distal_line);
//                     continue; // Skip to next zone, this one is invalidated
//                 } else {
//                     write_debug("✅ No invalidation - zone remains active");
//                 }

//                 // CRITICAL FIX: NOW check trade attempt status AFTER invalidation check
//                 write_debug(&format!(
//                     "   Trade attempted: {}",
//                     live_zone_state.trade_attempted_this_cycle
//                 ));
//                 if live_zone_state.trade_attempted_this_cycle {
//                     write_debug("   ⏭️ SKIPPING: Trade already attempted this cycle");
//                     continue;
//                 }

//                 let touched_this_bar = (trade_side_for_api == 1
//                     && entry_price_check_against_bar >= proximal_line)
//                     || (trade_side_for_api == 0 && entry_price_check_against_bar <= proximal_line);

//                 write_debug(&format!(
//                     "   Touch check: entry_price={:.6}, proximal={:.6}, touched={}",
//                     entry_price_check_against_bar, proximal_line, touched_this_bar
//                 ));
//                 write_debug(&format!(
//                     "   Was outside zone: {}",
//                     live_zone_state.is_outside_zone
//                 ));

//                 // MAJOR FIX: Only count touch if we were previously outside the zone (matches Generator logic)
//                 if touched_this_bar && live_zone_state.is_outside_zone {
//                     live_zone_state.live_touches_this_cycle += 1;
//                     let historical_touches = live_zone_state.zone_data.touch_count.unwrap_or(0);
//                     let zone_id_str = live_zone_state
//                         .zone_data
//                         .zone_id
//                         .as_deref()
//                         .unwrap_or("N/A");

//                     write_debug(&format!(
//                         "   ✨ TOUCH COUNTED! Live touches: {}, Historical: {}",
//                         live_zone_state.live_touches_this_cycle, historical_touches
//                     ));

//                     info!("[LIVE_TOUCH] Zone {} ({}) for {} LIVE touched. Live/Hist Touches: {}/{}. Bar Price: {:.5}, Proximal: {:.5}",
//                           zone_id_str, zone_type_str, bar.symbol,
//                           live_zone_state.live_touches_this_cycle, historical_touches, entry_price_check_against_bar, proximal_line);

//                     // Log the touch event
//                     log_trade_condition_check(
//                         zone_id_str,
//                         &bar.symbol,
//                         "ZONE_TOUCHED",
//                         true,
//                         &format!(
//                             "Live touches: {}, Historical: {}, Price: {:.5}",
//                             live_zone_state.live_touches_this_cycle,
//                             historical_touches,
//                             entry_price_check_against_bar
//                         ),
//                     );

//                     if live_zone_state.live_touches_this_cycle == 1
//                         && historical_touches < historical_touch_limit
//                     {
//                         info!("[TRADE_CRITERIA] First live touch for zone {} & hist touches ({}) < limit ({}). Checking conditions...",
//                               zone_id_str, historical_touches, historical_touch_limit);

//                         // Log first touch condition
//                         log_trade_condition_check(
//                             zone_id_str,
//                             &bar.symbol,
//                             "FIRST_TOUCH",
//                             true,
//                             &format!(
//                                 "Live touches: 1, Historical: {} < limit: {}",
//                                 historical_touches, historical_touch_limit
//                             ),
//                         );

//                         let now_utc = Utc::now();
//                         let is_allowed_day = allowed_days.contains(&now_utc.weekday());

//                         // Log day check
//                         log_trade_condition_check(
//                             zone_id_str,
//                             &bar.symbol,
//                             "ALLOWED_DAY",
//                             is_allowed_day,
//                             &format!(
//                                 "Today: {:?}, Allowed: {:?}",
//                                 now_utc.weekday(),
//                                 allowed_days
//                             ),
//                         );

//                         if !is_allowed_day {
//                             info!(
//                                 "[TRADE_REJECT] Zone {}: Today ({:?}) not in allowed days ({:?}).",
//                                 zone_id_str,
//                                 now_utc.weekday(),
//                                 allowed_days
//                             );

//                             log_trade_rejection(
//                                 zone_id_str,
//                                 &bar.symbol,
//                                 "WRONG_DAY",
//                                 &format!(
//                                     "Today: {:?}, Allowed: {:?}",
//                                     now_utc.weekday(),
//                                     allowed_days
//                                 ),
//                             );

//                             live_zone_state.trade_attempted_this_cycle = true;
//                             // Update outside zone state and continue
//                             live_zone_state.is_outside_zone = !touched_this_bar;
//                             continue;
//                         }

//                         // Log successful trade attempt
//                         let side_str = if trade_side_for_api == 0 {
//                             "BUY"
//                         } else {
//                             "SELL"
//                         };
//                         log_trade_attempt(
//                             zone_id_str,
//                             &bar.symbol,
//                             zone_type_str,
//                             side_str,
//                             lot_size,
//                             proximal_line,
//                             entry_price_check_against_bar,
//                             historical_touches,
//                         );

//                         info!(
//                             "[TRADE_ATTEMPT] Placing trade for zone {} (Symbol: {}, Side: {})!",
//                             zone_id_str, bar.symbol, side_str
//                         );
//                         live_zone_state.trade_attempted_this_cycle = true;

//                         if let Some(s_id) = get_symbol_id_from_name_str(&bar.symbol) {
//                             // Log the API call
//                             log_trade_api_call(
//                                 zone_id_str,
//                                 &bar.symbol,
//                                 s_id,
//                                 trade_side_for_api,
//                                 lot_size,
//                             );

//                             let trade_params = json!({
//                                 "symbolId": s_id, "tradeSide": trade_side_for_api, "volume": lot_size
//                             });
//                             let order_url_clone = order_placement_url.to_string();
//                             let http_client_clone = Arc::clone(http_client);
//                             let symbol_clone = bar.symbol.clone();
//                             let zone_id_clone = zone_id_str.to_string();

//                             tokio::spawn(async move {
//                                 info!(
//                                     "[ORDER_SEND_TASK] Sending order for {}: {:?}",
//                                     symbol_clone, trade_params
//                                 );
//                                 match http_client_clone
//                                     .post(&order_url_clone)
//                                     .json(&trade_params)
//                                     .send()
//                                     .await
//                                 {
//                                     Ok(response) => {
//                                         let status = response.status();
//                                         let status_code = status.as_u16();
//                                         let text = response
//                                             .text()
//                                             .await
//                                             .unwrap_or_else(|_| "No response body".to_string());

//                                         if status.is_success() {
//                                             info!("[TRADE_API_SUCCESS] 🎉 Order placed for {}. Response: {}", symbol_clone, text);
//                                             // NEW: Play sound on successful trade
//                                             log_trade_api_response_with_sound(
//                                                 &zone_id_clone,
//                                                 &symbol_clone,
//                                                 true,
//                                                 Some(status_code),
//                                                 &text,
//                                                 trade_side_for_api,
//                                             );
//                                         } else {
//                                             error!("[TRADE_API_FAIL] ❌ Failed to place order for {}. Status: {}. Resp: {}", symbol_clone, status, text);
//                                             log_trade_api_response_with_sound(
//                                                 &zone_id_clone,
//                                                 &symbol_clone,
//                                                 false,
//                                                 Some(status_code),
//                                                 &text,
//                                                 trade_side_for_api,
//                                             );
//                                         }
//                                     }
//                                     Err(e) => {
//                                         error!(
//                                             "[TRADE_API_ERROR] ❌ HTTP Error for {}: {}",
//                                             symbol_clone, e
//                                         );
//                                         let error_msg = format!("HTTP Error: {}", e);
//                                         log_trade_api_response_with_sound(
//                                             &zone_id_clone,
//                                             &symbol_clone,
//                                             false,
//                                             None,
//                                             &error_msg,
//                                             trade_side_for_api,
//                                         );
//                                     }
//                                 }
//                             });
//                         } else {
//                             warn!(
//                                 "[TRADE_SKIP] Could not find symbolId for {} to place order.",
//                                 bar.symbol
//                             );
//                             log_trade_rejection(
//                                 zone_id_str,
//                                 &bar.symbol,
//                                 "NO_SYMBOL_ID",
//                                 "Symbol ID mapping not found",
//                             );
//                         }
//                     } else if live_zone_state.live_touches_this_cycle > 1 {
//                         log_trade_rejection(
//                             zone_id_str,
//                             &bar.symbol,
//                             "MULTIPLE_TOUCHES",
//                             &format!("Live touches: {}", live_zone_state.live_touches_this_cycle),
//                         );
//                         debug!(
//                             "[TRADE_SKIP] Zone {} not eligible: live_touches={}",
//                             zone_id_str, live_zone_state.live_touches_this_cycle
//                         );
//                     } else if historical_touches >= historical_touch_limit {
//                         log_trade_rejection(
//                             zone_id_str,
//                             &bar.symbol,
//                             "TOO_MANY_HIST_TOUCHES",
//                             &format!(
//                                 "Historical touches: {} >= limit: {}",
//                                 historical_touches, historical_touch_limit
//                             ),
//                         );
//                         debug!(
//                             "[TRADE_SKIP] Zone {} not eligible: hist_touches={} >= limit={}",
//                             zone_id_str, historical_touches, historical_touch_limit
//                         );
//                     }
//                 } else if touched_this_bar {
//                     write_debug("   📍 Touch detected but was already inside zone - not counting");
//                 } else {
//                     write_debug("   📍 No touch detected");
//                 }

//                 // CRITICAL FIX: Always update the outside zone state after processing (matches Generator logic)
//                 live_zone_state.is_outside_zone = !touched_this_bar;
//                 write_debug(&format!(
//                     "   Updated outside_zone status: {}",
//                     live_zone_state.is_outside_zone
//                 ));
//                 write_debug(&format!(
//                     "   Final zone active status: {}",
//                     live_zone_state.zone_data.is_active
//                 ));
//                 write_debug("---");
//             }
//         }
//     }
// }

// // Add these imports to the top of your zone_monitor_service.rs file
// use chrono::Local;
// use std::fs::OpenOptions;
// use std::io::Write;

// // Add this new lazy_static for the trade log file (add after existing lazy_static if you have any)
// use lazy_static::lazy_static;
// use std::sync::Mutex as StdMutex;

// lazy_static! {
//     static ref TRADE_LOG_FILE: StdMutex<std::io::BufWriter<std::fs::File>> = {
//         let file_path = "trade_execution_log.txt";
//         let file = OpenOptions::new()
//             .create(true)
//             .append(true)
//             .open(file_path)
//             .unwrap_or_else(|e| panic!("Failed to open or create trade log {}: {}", file_path, e));
//         StdMutex::new(std::io::BufWriter::new(file))
//     };
// }

// // Add these helper functions for trade logging
// fn log_trade_attempt(
//     zone_id: &str,
//     symbol: &str,
//     zone_type: &str,
//     side: &str,
//     lot_size: f64,
//     proximal_price: f64,
//     current_price: f64,
//     historical_touches: i64,
// ) {
//     let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
//     let log_line = format!(
//         "[{}] TRADE_ATTEMPT | Zone: {} | Symbol: {} | Type: {} | Side: {} | Size: {} | Proximal: {:.5} | CurrentPrice: {:.5} | HistTouches: {}\n",
//         timestamp, zone_id, symbol, zone_type, side, lot_size, proximal_price, current_price, historical_touches
//     );

//     if let Ok(mut writer) = TRADE_LOG_FILE.lock() {
//         if let Err(e) = writer.write_all(log_line.as_bytes()) {
//             error!("[TRADE_LOG_ERROR] Failed to write trade attempt: {}", e);
//         }
//         let _ = writer.flush();
//     }
// }

// fn log_trade_rejection(zone_id: &str, symbol: &str, reason: &str, details: &str) {
//     let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
//     let log_line = format!(
//         "[{}] TRADE_REJECTED | Zone: {} | Symbol: {} | Reason: {} | Details: {}\n",
//         timestamp, zone_id, symbol, reason, details
//     );

//     if let Ok(mut writer) = TRADE_LOG_FILE.lock() {
//         if let Err(e) = writer.write_all(log_line.as_bytes()) {
//             error!("[TRADE_LOG_ERROR] Failed to write trade rejection: {}", e);
//         }
//         let _ = writer.flush();
//     }
// }

// fn log_trade_api_call(zone_id: &str, symbol: &str, symbol_id: u32, trade_side: i32, volume: f64) {
//     let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
//     let log_line = format!(
//         "[{}] TRADE_API_CALL | Zone: {} | Symbol: {} | SymbolID: {} | Side: {} | Volume: {} | Status: SENDING\n",
//         timestamp, zone_id, symbol, symbol_id, if trade_side == 0 { "BUY" } else { "SELL" }, volume
//     );

//     if let Ok(mut writer) = TRADE_LOG_FILE.lock() {
//         if let Err(e) = writer.write_all(log_line.as_bytes()) {
//             error!("[TRADE_LOG_ERROR] Failed to write API call: {}", e);
//         }
//         let _ = writer.flush();
//     }
// }

// fn log_trade_api_response(
//     zone_id: &str,
//     symbol: &str,
//     success: bool,
//     status_code: Option<u16>,
//     response_body: &str,
// ) {
//     let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
//     let status = if success { "SUCCESS" } else { "FAILED" };
//     let code_str = status_code.map_or("N/A".to_string(), |c| c.to_string());

//     let log_line = format!(
//         "[{}] TRADE_API_RESPONSE | Zone: {} | Symbol: {} | Status: {} | Code: {} | Response: {}\n",
//         timestamp,
//         zone_id,
//         symbol,
//         status,
//         code_str,
//         response_body.chars().take(200).collect::<String>()
//     );

//     if let Ok(mut writer) = TRADE_LOG_FILE.lock() {
//         if let Err(e) = writer.write_all(log_line.as_bytes()) {
//             error!("[TRADE_LOG_ERROR] Failed to write API response: {}", e);
//         }
//         let _ = writer.flush();
//     }
// }

// fn log_trade_condition_check(
//     zone_id: &str,
//     symbol: &str,
//     condition: &str,
//     result: bool,
//     details: &str,
// ) {
//     let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
//     let result_str = if result { "PASS" } else { "FAIL" };

//     let log_line = format!(
//         "[{}] TRADE_CONDITION | Zone: {} | Symbol: {} | Check: {} | Result: {} | Details: {}\n",
//         timestamp, zone_id, symbol, condition, result_str, details
//     );

//     if let Ok(mut writer) = TRADE_LOG_FILE.lock() {
//         if let Err(e) = writer.write_all(log_line.as_bytes()) {
//             error!("[TRADE_LOG_ERROR] Failed to write condition check: {}", e);
//         }
//         let _ = writer.flush();
//     }
// }

// fn parse_allowed_days_from_strings(day_strings: &[String]) -> Vec<Weekday> {
//     day_strings
//         .iter()
//         .filter_map(|day_str| match day_str.to_lowercase().as_str() {
//             "mon" | "monday" => Some(Weekday::Mon),
//             "tue" | "tuesday" => Some(Weekday::Tue),
//             "wed" | "wednesday" => Some(Weekday::Wed),
//             "thu" | "thursday" => Some(Weekday::Thu),
//             "fri" | "friday" => Some(Weekday::Fri),
//             "sat" | "saturday" => Some(Weekday::Sat),
//             "sun" | "sunday" => Some(Weekday::Sun),
//             _ => {
//                 warn!("Invalid day string for strategy: {}", day_str);
//                 None
//             }
//         })
//         .collect()
// }

// fn get_symbol_id_from_name_str(symbol_name_sb: &str) -> Option<u32> {
//     match symbol_name_sb {
//         "EURUSD_SB" => Some(185),
//         "GBPUSD_SB" => Some(199),
//         "USDJPY_SB" => Some(226),
//         "USDCHF_SB" => Some(222),
//         "AUDUSD_SB" => Some(158),
//         "USDCAD_SB" => Some(221),
//         "NZDUSD_SB" => Some(211),
//         "EURGBP_SB" => Some(175),
//         "EURJPY_SB" => Some(177),
//         "EURCHF_SB" => Some(173),
//         "EURAUD_SB" => Some(171),
//         "EURCAD_SB" => Some(172),
//         "EURNZD_SB" => Some(180),
//         "GBPJPY_SB" => Some(192),
//         "GBPCHF_SB" => Some(191),
//         "GBPAUD_SB" => Some(189),
//         "GBPCAD_SB" => Some(190),
//         "GBPNZD_SB" => Some(195),
//         "AUDJPY_SB" => Some(155),
//         "AUDNZD_SB" => Some(156),
//         "AUDCAD_SB" => Some(153),
//         "NZDJPY_SB" => Some(210),
//         "CADJPY_SB" => Some(162),
//         "CHFJPY_SB" => Some(163),
//         "NAS100_SB" => Some(205),
//         "US500_SB" => Some(220),
//         _ => {
//             warn!("No symbolId mapping found for: {}", symbol_name_sb);
//             None
//         }
//     }
// }

// fn perform_enrichment_for_revalidation(
//     original_zone: &StoredZone,
//     subsequent_candles: &[CandleData],
// ) -> (bool, i64, Option<u64>, Option<String>, Option<f64>) {
//     // Returns: (new_is_active, new_total_touch_count, new_total_bars_active_segment, invalidation_time_if_any, new_strength_score)

//     if subsequent_candles.is_empty() {
//         debug!("[REVALIDATE_ENRICH] No subsequent candles for zone {:?}. Original active status: {}, touches: {:?}, bars_active: {:?}",
//             original_zone.zone_id, original_zone.is_active, original_zone.touch_count, original_zone.bars_active);
//         return (
//             original_zone.is_active,
//             original_zone.touch_count.unwrap_or(0),
//             original_zone.bars_active,
//             None,
//             original_zone.strength_score,
//         );
//     }

//     let zone_high = match original_zone.zone_high {
//         Some(zh) => zh,
//         None => {
//             warn!(
//                 "[REVALIDATE_ENRICH] Zone ID {:?} missing zone_high.",
//                 original_zone.zone_id
//             );
//             return (false, 0, None, None, Some(0.0));
//         }
//     };
//     let zone_low = match original_zone.zone_low {
//         Some(zl) => zl,
//         None => {
//             warn!(
//                 "[REVALIDATE_ENRICH] Zone ID {:?} missing zone_low.",
//                 original_zone.zone_id
//             );
//             return (false, 0, None, None, Some(0.0));
//         }
//     };
//     let zone_type_str = original_zone.zone_type.as_deref().unwrap_or("");
//     let is_supply = zone_type_str.eq_ignore_ascii_case("supply")
//         || zone_type_str.eq_ignore_ascii_case("supply_zone");

//     let (proximal_line, distal_line) = if is_supply {
//         (zone_low, zone_high)
//     } else {
//         (zone_high, zone_low)
//     };

//     let mut current_is_active = true;
//     let mut touches_in_this_reval_period: i64 = 0;
//     let mut is_outside_zone_for_touch_counting = true; // Assume price starts outside for this subsequent batch
//     let mut invalidation_time_found: Option<String> = None;
//     let mut bars_processed_in_reval: u64 = 0;

//     for candle in subsequent_candles {
//         bars_processed_in_reval += 1;

//         if (is_supply && candle.high > distal_line) || (!is_supply && candle.low < distal_line) {
//             current_is_active = false;
//             invalidation_time_found = Some(candle.time.clone());
//             debug!(
//                 "[REVALIDATE_ENRICH] Zone {:?} invalidated by candle: {} (H:{}, L:{}, D:{})",
//                 original_zone.zone_id.as_deref().unwrap_or("?"),
//                 candle.time,
//                 candle.high,
//                 candle.low,
//                 distal_line
//             );
//             break;
//         }

//         let mut interaction_occurred = false;
//         if (is_supply && candle.high >= proximal_line)
//             || (!is_supply && candle.low <= proximal_line)
//         {
//             interaction_occurred = true;
//         }

//         if interaction_occurred && is_outside_zone_for_touch_counting {
//             touches_in_this_reval_period += 1;
//         }
//         is_outside_zone_for_touch_counting = !interaction_occurred;
//     }

//     let new_total_touch_count =
//         original_zone.touch_count.unwrap_or(0) + touches_in_this_reval_period;
//     // This `new_bars_active` should represent the total bars the zone was active.
//     // If the original `bars_active` was from formation to `original_zone.end_time`,
//     // and it didn't invalidate in this reval, we add `bars_processed_in_reval`.
//     // If it *did* invalidate in this reval, `bars_processed_in_reval` is the count until invalidation.
//     let new_total_bars_active = if !current_is_active {
//         original_zone.bars_active.unwrap_or(0) + bars_processed_in_reval // Bars until this invalidation
//     } else {
//         original_zone.bars_active.unwrap_or(0) + subsequent_candles.len() as u64
//         // Still active, add all processed
//     };
//     let new_strength_score =
//         Some(100.0 - (new_total_touch_count as f64 * 10.0).max(0.0).min(100.0));

//     (
//         current_is_active,
//         new_total_touch_count,
//         Some(new_total_bars_active),
//         invalidation_time_found,
//         new_strength_score,
//     )
// }

// async fn update_zone_in_influxdb(
//     updated_zone: &StoredZone,
//     http_client: &HttpClient,
//     influx_host: &str,
//     influx_org: &str,
//     influx_token: &str,
//     zone_bucket: &str,
//     zone_measurement: &str,
// ) -> Result<(), String> {
//     if updated_zone.zone_id.is_none() || updated_zone.start_time.is_none() {
//         return Err("Zone ID or start_time missing for DB update".to_string());
//     }

//     let mut fields_vec: Vec<String> = Vec::new();
//     fields_vec.push(format!(
//         "zone_id=\"{}\"",
//         updated_zone.zone_id.as_ref().unwrap().escape_default()
//     ));
//     if let Some(v) = updated_zone.start_idx {
//         fields_vec.push(format!("start_idx={}i", v));
//     }
//     if let Some(v) = updated_zone.end_idx {
//         fields_vec.push(format!("end_idx={}i", v));
//     }
//     if let Some(v) = &updated_zone.end_time {
//         fields_vec.push(format!("end_time_rfc3339=\"{}\"", v.escape_default()));
//     }
//     if let Some(v) = updated_zone.zone_high {
//         fields_vec.push(format!("zone_high={}", v));
//     }
//     if let Some(v) = updated_zone.zone_low {
//         fields_vec.push(format!("zone_low={}", v));
//     }
//     if let Some(v) = updated_zone.fifty_percent_line {
//         fields_vec.push(format!("fifty_percent_line={}", v));
//     }
//     if let Some(v) = &updated_zone.detection_method {
//         fields_vec.push(format!("detection_method=\"{}\"", v.escape_default()));
//     }
//     if let Some(v) = updated_zone.extended {
//         fields_vec.push(format!("extended={}", v));
//     }
//     if let Some(v) = updated_zone.extension_percent {
//         fields_vec.push(format!("extension_percent={}", v));
//     }
//     fields_vec.push(format!(
//         "is_active={}i",
//         if updated_zone.is_active { 1 } else { 0 }
//     ));
//     if let Some(v) = updated_zone.bars_active {
//         fields_vec.push(format!("bars_active={}i", v));
//     }
//     if let Some(v) = updated_zone.touch_count {
//         fields_vec.push(format!("touch_count={}i", v));
//     }
//     if let Some(v) = updated_zone.strength_score {
//         fields_vec.push(format!("strength_score={}", v));
//     }
//     let candles_json_str = serde_json::to_string(&updated_zone.formation_candles)
//         .unwrap_or_else(|_| "[]".to_string())
//         .replace("\"", "\\\"");
//     fields_vec.push(format!("formation_candles_json=\"{}\"", candles_json_str));

//     if fields_vec.is_empty() {
//         return Err("No fields to update for zone".to_string());
//     }

//     let timestamp_nanos =
//         chrono::DateTime::parse_from_rfc3339(updated_zone.start_time.as_ref().unwrap())
//             .map_err(|e| format!("Failed to parse zone start_time: {}", e))?
//             .timestamp_nanos_opt()
//             .ok_or_else(|| "Failed to get nanos from start_time".to_string())?;

//     let symbol_tag = updated_zone
//         .symbol
//         .as_deref()
//         .unwrap_or("unknown_symbol")
//         .replace(" ", "\\ ")
//         .replace(",", "\\,")
//         .replace("=", "\\=");
//     let timeframe_tag = updated_zone
//         .timeframe
//         .as_deref()
//         .unwrap_or("unknown_tf")
//         .replace(" ", "\\ ")
//         .replace(",", "\\,")
//         .replace("=", "\\=");
//     let pattern_tag = updated_zone
//         .pattern
//         .as_deref()
//         .unwrap_or("unknown_pattern")
//         .replace(" ", "\\ ")
//         .replace(",", "\\,")
//         .replace("=", "\\=");
//     let zone_type_tag = updated_zone
//         .zone_type
//         .as_deref()
//         .unwrap_or("unknown_type")
//         .replace(" ", "\\ ")
//         .replace(",", "\\,")
//         .replace("=", "\\=");

//     let tags_str = format!(
//         "symbol={},timeframe={},pattern={},zone_type={}",
//         symbol_tag, timeframe_tag, pattern_tag, zone_type_tag
//     );

//     let line = format!(
//         "{measurement},{tags} {fields} {timestamp}",
//         measurement = zone_measurement,
//         tags = tags_str,
//         fields = fields_vec.join(","),
//         timestamp = timestamp_nanos
//     );

//     debug!("[DB_UPDATE] Line protocol for update: {}", line);
//     let write_url = format!(
//         "{}/api/v2/write?org={}&bucket={}&precision=ns",
//         influx_host, influx_org, zone_bucket
//     );

//     match http_client
//         .post(&write_url)
//         .bearer_auth(influx_token)
//         .header("Content-Type", "text/plain; charset=utf-8")
//         .body(line)
//         .send()
//         .await
//     {
//         Ok(response) => {
//             let status = response.status();
//             if status.is_success() {
//                 info!(
//                     "[DB_UPDATE] Successfully updated zone ID: {}",
//                     updated_zone.zone_id.as_deref().unwrap_or("N/A")
//                 );
//                 Ok(())
//             } else {
//                 let err_text = response.text().await.unwrap_or_else(|_| {
//                     format!(
//                         "Unknown error updating zone {}",
//                         updated_zone.zone_id.as_deref().unwrap_or("N/A")
//                     )
//                 });
//                 error!(
//                     "[DB_UPDATE] Failed to update zone in InfluxDB (status {}): {}",
//                     status, err_text
//                 );
//                 Err(format!(
//                     "Failed to update zone (status {}): {}",
//                     status, err_text
//                 ))
//             }
//         }
//         Err(e) => {
//             error!("[DB_UPDATE] HTTP error updating zone in InfluxDB: {}", e);
//             Err(format!("HTTP error updating zone: {}", e))
//         }
//     }
// }

// pub async fn revalidate_one_zone_activity_by_id(
//     zone_id_to_check: String,
//     http_client: &Arc<HttpClient>,
//     influx_host: &str,
//     influx_org: &str,
//     influx_token: &str,
//     zone_bucket: &str,
//     zone_measurement: &str,
//     candle_bucket: &str,
//     candle_measurement_for_candles: &str,
// ) -> Result<Option<StoredZone>, String> {
//     info!("[REVALIDATE] Revalidating zone ID: {}", zone_id_to_check);

//     let query_for_zone = format!(
//         r#"from(bucket: "{zone_bucket}") |> range(start: 0) |> filter(fn: (r) => r._measurement == "{zone_measurement}") |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value") |> filter(fn: (r) => r.zone_id == "{escaped_zone_id}") |> limit(n: 1)"#,
//         zone_bucket = zone_bucket,
//         zone_measurement = zone_measurement,
//         escaped_zone_id = zone_id_to_check.replace("\"", "\\\"") // Escape quotes in zone_id for the filter
//     );

//     debug!("[REVALIDATE] Querying for zone details: {}", query_for_zone);
//     let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);

//     let mut original_stored_zones: Vec<StoredZone> = Vec::new();
//     match http_client
//         .post(&query_url)
//         .bearer_auth(influx_token)
//         .header("Accept", "application/csv")
//         .header("Content-Type", "application/json")
//         .json(&json!({ "query": query_for_zone, "type": "flux" }))
//         .send()
//         .await
//     {
//         Ok(response) => {
//             let status = response.status();
//             if status.is_success() {
//                 let text = response
//                     .text()
//                     .await
//                     .map_err(|e| format!("Error reading response text: {}", e.to_string()))?;
//                 if text
//                     .lines()
//                     .skip_while(|l| l.starts_with('#') || l.is_empty())
//                     .count()
//                     > 1
//                 {
//                     let mut rdr = csv::ReaderBuilder::new()
//                         .has_headers(true)
//                         .flexible(true)
//                         .comment(Some(b'#'))
//                         .from_reader(text.as_bytes());
//                     for result in rdr.deserialize::<ZoneCsvRecord>() {
//                         match result {
//                             Ok(csv) => original_stored_zones.push(map_csv_to_stored_zone(csv)),
//                             Err(e) => {
//                                 return Err(format!(
//                                     "CSV deserialize error for zone {}: {}",
//                                     zone_id_to_check, e
//                                 ))
//                             }
//                         }
//                     }
//                 } else {
//                     warn!(
//                         "[REVALIDATE] Zone ID {} query successful but no data rows.",
//                         zone_id_to_check
//                     );
//                     return Ok(None);
//                 }
//             } else {
//                 let err_body = response
//                     .text()
//                     .await
//                     .unwrap_or_else(|_| "Failed to read error body".to_string());
//                 error!(
//                     "[REVALIDATE] Failed to fetch zone {} details (status {}): {}. Query: {}",
//                     zone_id_to_check, status, err_body, query_for_zone
//                 );
//                 return Err(format!(
//                     "Failed to fetch zone details (status {}): {}, {}",
//                     zone_id_to_check, status, err_body
//                 ));
//             }
//         }
//         Err(e) => {
//             error!(
//                 "[REVALIDATE] HTTP error fetching zone {}: {}. Query: {}",
//                 zone_id_to_check, e, query_for_zone
//             );
//             return Err(format!("HTTP error: {}", e));
//         }
//     }

//     if original_stored_zones.is_empty() {
//         warn!(
//             "[REVALIDATE] Zone ID {} not found after parsing.",
//             zone_id_to_check
//         );
//         return Ok(None);
//     }
//     let mut zone_to_revalidate = original_stored_zones.remove(0);

//     info!(
//         "[REVALIDATE] Found zone: ID: {:?}, Formed: {:?}, DB Active: {}, DB Touches: {:?}",
//         zone_to_revalidate.zone_id,
//         zone_to_revalidate.start_time,
//         zone_to_revalidate.is_active,
//         zone_to_revalidate.touch_count
//     );

//     let activity_check_start_time = match &zone_to_revalidate.end_time {
//         Some(et) if !et.is_empty() => et.clone(),
//         _ => zone_to_revalidate
//             .start_time
//             .clone()
//             .ok_or_else(|| "Zone missing start_time".to_string())?,
//     };
//     let activity_check_end_time = "now()".to_string();

//     let subsequent_candles: Vec<CandleData> = match fetch_candles_direct(
//         http_client.as_ref(),
//         influx_host,
//         influx_org,
//         influx_token,
//         candle_bucket,
//         candle_measurement_for_candles,
//         zone_to_revalidate.symbol.as_deref().unwrap_or_default(),
//         zone_to_revalidate.timeframe.as_deref().unwrap_or_default(),
//         &activity_check_start_time,
//         &activity_check_end_time,
//     )
//     .await
//     {
//         Ok(candles) => candles,
//         Err(e) => {
//             error!(
//                 "[REVALIDATE] Candle fetch error for {}: {}",
//                 zone_id_to_check, e
//             );
//             return Err(e);
//         }
//     };
//     info!(
//         "[REVALIDATE] Fetched {} subsequent candles for {} (from {} to {}).",
//         subsequent_candles.len(),
//         zone_id_to_check,
//         activity_check_start_time,
//         activity_check_end_time
//     );

//     let (
//         new_is_active,
//         new_total_touch_count,
//         new_total_bars_active_opt,
//         _new_invalidation_time_opt,
//         new_strength_score_opt,
//     ) = perform_enrichment_for_revalidation(&zone_to_revalidate, &subsequent_candles);

//     let mut updated_in_db = false;
//     if zone_to_revalidate.is_active != new_is_active {
//         info!(
//             "[REVALIDATE] ID {} active: {} -> {}",
//             zone_id_to_check, zone_to_revalidate.is_active, new_is_active
//         );
//         zone_to_revalidate.is_active = new_is_active;
//         updated_in_db = true;
//     }
//     if zone_to_revalidate.touch_count.unwrap_or(-1) != new_total_touch_count {
//         info!(
//             "[REVALIDATE] ID {} touches: {:?} -> {}",
//             zone_id_to_check, zone_to_revalidate.touch_count, new_total_touch_count
//         );
//         zone_to_revalidate.touch_count = Some(new_total_touch_count);
//         updated_in_db = true;
//     }
//     if let Some(new_bars) = new_total_bars_active_opt {
//         if zone_to_revalidate.bars_active != Some(new_bars) {
//             info!(
//                 "[REVALIDATE] ID {} bars_active: {:?} -> {}",
//                 zone_id_to_check, zone_to_revalidate.bars_active, new_bars
//             );
//             zone_to_revalidate.bars_active = Some(new_bars);
//             updated_in_db = true;
//         }
//     }
//     if let Some(new_score) = new_strength_score_opt {
//         if zone_to_revalidate.strength_score != Some(new_score) {
//             info!(
//                 "[REVALIDATE] ID {} strength: {:?} -> {}",
//                 zone_id_to_check, zone_to_revalidate.strength_score, new_score
//             );
//             zone_to_revalidate.strength_score = Some(new_score);
//             updated_in_db = true;
//         }
//     }
//     // TODO: update zone_to_revalidate.end_time and .invalidation_time if _new_invalidation_time_opt is Some

//     if updated_in_db {
//         info!(
//             "[REVALIDATE] Changes for {}. Updating DB.",
//             zone_id_to_check
//         );
//         if let Err(e) = update_zone_in_influxdb(
//             &zone_to_revalidate,
//             http_client.as_ref(),
//             influx_host,
//             influx_org,
//             influx_token,
//             zone_bucket,
//             zone_measurement,
//         )
//         .await
//         {
//             error!(
//                 "[REVALIDATE] DB update failed for {}: {}",
//                 zone_id_to_check, e
//             );
//             return Err(e);
//         }
//     } else {
//         info!(
//             "[REVALIDATE] No changes needed in DB for {}.",
//             zone_id_to_check
//         );
//     }

//     Ok(Some(zone_to_revalidate))
// }

// async fn check_proximity_alerts(
//     bar: &BarUpdateData,
//     zone_cache: &ActiveZoneCache,
// ) {
//     use crate::notification_manager::get_global_notification_manager;
    
//     let proximity_threshold_pips = std::env::var("PROXIMITY_THRESHOLD_PIPS")
//         .unwrap_or_else(|_| "15.0".to_string())
//         .parse::<f64>()
//         .unwrap_or(15.0);
    
//     info!("🔍 [PROXIMITY_DEBUG] Checking proximity for bar: {} @ {:.5} (timeframe: {})", 
//           bar.symbol, bar.close, bar.timeframe);
    
//     let cache_guard = zone_cache.lock().await;
//     let total_zones = cache_guard.len();
//     let mut zones_checked = 0;
//     let mut symbol_matches = 0;
//     let mut active_zones = 0;
//     let mut zones_with_proximity = 0;
    
//     for (zone_id, live_zone_state) in cache_guard.iter() {
//         zones_checked += 1;
//         let zone = &live_zone_state.zone_data;
        
//         // Debug: Log every zone we're checking
//         if zones_checked <= 3 {  // Only log first few to avoid spam
//             info!("🔍 [PROXIMITY_DEBUG] Zone {}: symbol={:?}, timeframe={:?}, active={}", 
//                   zone_id, zone.symbol, zone.timeframe, zone.is_active);
//         }
        
//         // Only check zones for this symbol and that are active
//         if let Some(zone_symbol) = &zone.symbol {
//             if zone_symbol == &bar.symbol {
//                 symbol_matches += 1;
//                 info!("🔍 [PROXIMITY_DEBUG] Symbol match found: {} (zone: {})", bar.symbol, zone_id);
                
//                 if zone.is_active {
//                     active_zones += 1;
//                     info!("🔍 [PROXIMITY_DEBUG] Zone is active: {}", zone_id);
                    
//                     if let (Some(zone_high), Some(zone_low), Some(zone_type)) = (
//                         zone.zone_high,
//                         zone.zone_low, 
//                         &zone.zone_type,
//                     ) {
//                         let is_supply = zone_type.to_lowercase().contains("supply");
//                         let proximal_line = if is_supply { zone_low } else { zone_high };
                        
//                         // Use bar.close as current price for proximity check
//                         let distance_pips = calculate_pips_distance_for_proximity(&bar.symbol, bar.close, proximal_line);
                        
//                         info!("🔍 [PROXIMITY_DEBUG] Zone {}: type={}, proximal={:.5}, distance={:.2} pips, threshold={:.1}", 
//                               zone_id, zone_type, proximal_line, distance_pips, proximity_threshold_pips);
                        
//                         if distance_pips <= proximity_threshold_pips {
//                             zones_with_proximity += 1;
//                             info!("🎯 [PROXIMITY_ALERT] TRIGGERED: {} {} zone {} - {:.1} pips away (close: {:.5})", 
//                                   bar.symbol, zone_type, zone_id, distance_pips, bar.close);
                            
//                             // Send proximity notification
//                             if let Some(notification_manager) = get_global_notification_manager() {
//                                 if let Some(zone_timeframe) = &zone.timeframe {
//                                     info!("📱 [PROXIMITY_ALERT] Sending notification for {} {} zone", bar.symbol, zone_type);
//                                     notification_manager.notify_proximity_alert(
//                                         &bar.symbol,
//                                         bar.close,
//                                         zone_type,
//                                         zone_high,
//                                         zone_low,
//                                         zone_id,
//                                         zone_timeframe,
//                                         distance_pips
//                                     ).await;
                                    
//                                     info!("✅ [PROXIMITY_ALERT] Notification sent for {} {} zone at {:.1} pips", 
//                                           bar.symbol, zone_type, distance_pips);
//                                 } else {
//                                     warn!("⚠️ [PROXIMITY_DEBUG] Zone missing timeframe: {}", zone_id);
//                                 }
//                             } else {
//                                 warn!("⚠️ [PROXIMITY_DEBUG] Notification manager not available");
//                             }
//                         } else {
//                             info!("🔍 [PROXIMITY_DEBUG] Zone {} not within threshold: {:.2} > {:.1} pips", 
//                                   zone_id, distance_pips, proximity_threshold_pips);
//                         }
//                     } else {
//                         warn!("⚠️ [PROXIMITY_DEBUG] Zone {} missing price data: high={:?}, low={:?}, type={:?}", 
//                               zone_id, zone.zone_high, zone.zone_low, zone.zone_type);
//                     }
//                 } else {
//                     info!("🔍 [PROXIMITY_DEBUG] Zone {} is inactive", zone_id);
//                 }
//             }
//         }
//     }
    
//     info!("📊 [PROXIMITY_SUMMARY] Bar: {}, Total zones: {}, Symbol matches: {}, Active: {}, Within proximity: {}", 
//           bar.symbol, total_zones, symbol_matches, active_zones, zones_with_proximity);
// }

// // Helper function (add if not already present)
// fn calculate_pips_distance_for_proximity(symbol: &str, price1: f64, price2: f64) -> f64 {
//     let pip_value = if symbol.contains("JPY") { 0.01 } else { 0.0001 };
//     (price1 - price2).abs() / pip_value
// }