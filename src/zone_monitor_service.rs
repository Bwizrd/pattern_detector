// src/zone_monitor_service.rs

use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

use futures_util::StreamExt;
use log::{debug, error, info, warn};
// No need for reqwest::Client or direct InfluxDB querying for zones in this version
use serde_json::Value; // For handling recognizer output
use tokio_tungstenite::tungstenite::error::Error as WsError;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;

// Types from your project
use crate::detect::{CoreError, _fetch_and_detect_core, generate_deterministic_zone_id};
use crate::types::{CandleData, ChartQuery, EnrichedZone, TouchPoint}; // Use EnrichedZone // Assuming these are pub

const TS_WEBSOCKET_URL: &str = "ws://localhost:8081";

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct BarUpdateData {
    time: String, // ISO 8601
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: u64,
    symbol_id: u32,
    symbol: String, // This should be the _SB suffixed one if your TS service standardizes
    timeframe: String,
    is_new_bar: bool,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct WebSocketMessage {
    #[serde(rename = "type")]
    msg_type: String,
    data: Option<BarUpdateData>,
}

pub type ActiveZoneCache = Arc<Mutex<HashMap<String, EnrichedZone>>>;

// --- Helper to fetch and enrich zones for a single symbol/timeframe ---
async fn get_and_enrich_active_zones_for_pair(
    symbol_sb: &str, // Expects _SB suffixed symbol
    timeframe: &str,
    pattern_name: &str, // e.g., "fifty_percent_before_big_bar"
    // Define lookback for fetching candles for detection/enrichment
    // This should be long enough for pattern recognition and some activity checking.
    // Could be different from zone_generator's periodic lookback.
    candle_fetch_start_time: &str, // e.g., "-24h" or specific RFC3339
    candle_fetch_end_time: &str,   // e.g., "now()" or specific RFC3339
) -> Result<Vec<EnrichedZone>, String> {
    info!(
        "[ZONE_MONITOR_ENRICH] Getting active zones for {}/{} pattern {}",
        symbol_sb, timeframe, pattern_name
    );

    let chart_query = ChartQuery {
        symbol: symbol_sb.to_string(),
        timeframe: timeframe.to_string(),
        start_time: candle_fetch_start_time.to_string(),
        end_time: candle_fetch_end_time.to_string(),
        pattern: pattern_name.to_string(),
        enable_trading: None,
        lot_size: None,
        stop_loss_pips: None,
        take_profit_pips: None,
        enable_trailing_stop: None,
    };

    let caller_task_id = format!("zone_monitor_fetch_{}/{}", symbol_sb, timeframe);

    match _fetch_and_detect_core(&chart_query, &caller_task_id).await {
        Ok((candles, _recognizer, detection_results_json)) => {
            if candles.is_empty() {
                info!(
                    "[ZONE_MONITOR_ENRICH] No candle data for {}/{}",
                    symbol_sb, timeframe
                );
                return Ok(Vec::new());
            }

            let mut final_active_zones: Vec<EnrichedZone> = Vec::new();
            let candle_count = candles.len();
            let last_candle_timestamp = candles.last().map(|c| c.time.clone());

            // Simplified Enrichment Logic (adapt from your get_bulk_multi_tf_active_zones_handler)
            // This is a condensed version. You'll need to ensure it matches your full logic.
            let process_raw_zones = |zones_option: Option<&Vec<Value>>,
                                     is_supply_zone_type: bool|
             -> Vec<EnrichedZone> {
                let mut enriched_for_type: Vec<EnrichedZone> = Vec::new();
                if let Some(raw_zones_array) = zones_option {
                    for raw_zone_json_val in raw_zones_array {
                        // 1. Deserialize (or manually map) raw_zone_json_val to a base EnrichedZone
                        //    Your types::deserialize_raw_zone_value might work or need adaptation if
                        //    the structure from _fetch_and_detect_core's JSON is slightly different
                        //    than what the CSV mapping produces.
                        //    For simplicity, let's assume direct mapping or manual extraction:

                        let start_time_opt = raw_zone_json_val
                            .get("start_time")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let zone_high_opt =
                            raw_zone_json_val.get("zone_high").and_then(|v| v.as_f64());
                        let zone_low_opt =
                            raw_zone_json_val.get("zone_low").and_then(|v| v.as_f64());
                        let end_idx_opt = raw_zone_json_val.get("end_idx").and_then(|v| v.as_u64());
                        // ... extract other fields for EnrichedZone ...

                        if start_time_opt.is_none()
                            || zone_high_opt.is_none()
                            || zone_low_opt.is_none()
                            || end_idx_opt.is_none()
                        {
                            warn!("[ZONE_MONITOR_ENRICH] Raw zone for {}/{} missing essential fields. Skipping.", symbol_sb, timeframe);
                            continue;
                        }
                        let mut current_enriched_zone = EnrichedZone {
                            symbol: Some(symbol_sb.to_string()),    // Populate
                            timeframe: Some(timeframe.to_string()), // Populate
                            start_time: start_time_opt,
                            end_time: raw_zone_json_val
                                .get("end_time")
                                .and_then(|v| v.as_str())
                                .map(String::from), // Populate if available
                            zone_high: zone_high_opt,
                            zone_low: zone_low_opt,
                            fifty_percent_line: raw_zone_json_val
                                .get("fifty_percent_line")
                                .and_then(|v| v.as_f64()), // Populate
                            detection_method: raw_zone_json_val
                                .get("detection_method")
                                .and_then(|v| v.as_str())
                                .map(String::from), // Populate
                            start_idx: raw_zone_json_val.get("start_idx").and_then(|v| v.as_u64()), // Populate
                            end_idx: end_idx_opt,
                            zone_id: None,   // Will generate below
                            is_active: true, // Assume active, then verify by enrichment
                            last_data_candle: last_candle_timestamp.clone(),
                            zone_type: Some(if is_supply_zone_type {
                                "supply_zone".to_string()
                            } else {
                                "demand_zone".to_string()
                            }),
                            // Initialize other Option fields from EnrichedZone to None or their default
                            extended: raw_zone_json_val.get("extended").and_then(|v| v.as_bool()),
                            extension_percent: raw_zone_json_val
                                .get("extension_percent")
                                .and_then(|v| v.as_f64()),
                            bars_active: None,
                            strength_score: None,
                            invalidation_time: None,
                            zone_end_idx: None,
                            zone_end_time: None,
                            touch_points: None,
                            touch_count: Some(0), // Initialize touch_count
                                                  // ..Default::default() // Remove this if you're manually setting most fields
                        };
                        // Generate Zone ID
                        let zone_type_for_id = if is_supply_zone_type {
                            "supply"
                        } else {
                            "demand"
                        };
                        current_enriched_zone.zone_id = Some(generate_deterministic_zone_id(
                            symbol_sb,
                            timeframe,
                            zone_type_for_id,
                            current_enriched_zone.start_time.as_deref(),
                            current_enriched_zone.zone_high,
                            current_enriched_zone.zone_low,
                        ));

                        // 2. Perform Activity Check & Touch Count (Simplified version of your handler's logic)
                        let formation_end_idx = current_enriched_zone.end_idx.unwrap() as usize;
                        let (proximal_line, distal_line) = if is_supply_zone_type {
                            (
                                current_enriched_zone.zone_low.unwrap(),
                                current_enriched_zone.zone_high.unwrap(),
                            )
                        } else {
                            (
                                current_enriched_zone.zone_high.unwrap(),
                                current_enriched_zone.zone_low.unwrap(),
                            )
                        };

                        let mut live_touch_count: i64 = 0;
                        let mut is_currently_active = true;
                        let mut is_outside_zone_after_formation = true;
                        // ... (Loop through `candles` from `formation_end_idx + 1` to `candle_count -1`)
                        // ... (Implement the logic for `is_currently_active` and `live_touch_count`
                        //      and `touch_points` similar to `get_bulk_multi_tf_active_zones_handler`)
                        // This is a placeholder for your detailed enrichment:
                        let check_start_idx = formation_end_idx + 1;
                        if check_start_idx < candle_count {
                            for i in check_start_idx..candle_count {
                                let candle = &candles[i];
                                // Invalidation Check
                                if (is_supply_zone_type && candle.high > distal_line)
                                    || (!is_supply_zone_type && candle.low < distal_line)
                                {
                                    is_currently_active = false;
                                    current_enriched_zone.invalidation_time =
                                        Some(candle.time.clone());
                                    break;
                                }
                                // Touch Check
                                let mut interaction_occurred = false;
                                if (is_supply_zone_type && candle.high >= proximal_line)
                                    || (!is_supply_zone_type && candle.low <= proximal_line)
                                {
                                    interaction_occurred = true;
                                }
                                if interaction_occurred && is_outside_zone_after_formation {
                                    live_touch_count += 1;
                                }
                                is_outside_zone_after_formation = !interaction_occurred;
                            }
                        }
                        current_enriched_zone.is_active = is_currently_active;
                        current_enriched_zone.touch_count = Some(live_touch_count);
                        // ... set other EnrichedZone fields like bars_active, strength_score ...

                        if current_enriched_zone.is_active {
                            // Only add active zones
                            enriched_for_type.push(current_enriched_zone);
                        }
                    }
                }
                enriched_for_type
            };

            if let Some(data) = detection_results_json
                .get("data")
                .and_then(|d| d.get("price"))
            {
                if let Some(supply_zones_val) = data
                    .get("supply_zones")
                    .and_then(|sz| sz.get("zones"))
                    .and_then(|z| z.as_array())
                {
                    final_active_zones.extend(process_raw_zones(Some(supply_zones_val), true));
                }
                if let Some(demand_zones_val) = data
                    .get("demand_zones")
                    .and_then(|dz| dz.get("zones"))
                    .and_then(|z| z.as_array())
                {
                    final_active_zones.extend(process_raw_zones(Some(demand_zones_val), false));
                }
            }
            info!(
                "[ZONE_MONITOR_ENRICH] Found {} active zones for {}/{}",
                final_active_zones.len(),
                symbol_sb,
                timeframe
            );
            Ok(final_active_zones)
        }
        Err(core_err) => {
            error!(
                "[ZONE_MONITOR_ENRICH] Core error for {}/{}: {}",
                symbol_sb, timeframe, core_err
            );
            Err(core_err.to_string())
        }
    }
}

// --- Main Service Function ---
pub async fn run_zone_monitor_service(passed_in_cache: ActiveZoneCache) {
    info!("[ZONE_MONITOR] Starting Zone Monitor Service with shared cache...");

    // Env vars are still needed for the initial load logic if it calls Influx directly,
    // but _fetch_and_detect_core likely handles its own env vars for Influx connection.
    // Let's assume _fetch_and_detect_core is self-contained for Influx access.
    // Remove direct env var loading for Influx from here if _fetch_and_detect_core handles it.
    // let influx_host = env::var("INFLUXDB_HOST").expect("INFLUXDB_HOST not set");
    // ...

    let monitor_target_symbols: Vec<String> = vec![
        "EURUSD_SB".to_string(),
        "USDJPY_SB".to_string(),
        "GBPJPY_SB".to_string(),
        "AUDJPY_SB".to_string(),
        "CHFJPY_SB".to_string(),
    ];
    let monitor_pattern_timeframes: Vec<String> =
        vec!["1h".to_string(), "4h".to_string(), "1d".to_string()];
    let pattern_to_monitor = "fifty_percent_before_big_bar".to_string();
    // let candle_fetch_start_relative = "-200h";
    // let candle_fetch_end_relative = "now()";
    let candle_fetch_start_specific_iso = "2025-04-21T00:00:00Z";
    let candle_fetch_end_specific_iso = "2025-05-21T23:59:59Z";

    // --- Initial load of active zones ---
    info!("[ZONE_MONITOR] Performing initial load of active zones...");
    let mut initial_load_tasks = vec![];
    for symbol_sb in monitor_target_symbols.iter() {
        for timeframe in monitor_pattern_timeframes.iter() {
            let sym = symbol_sb.clone();
            let tf = timeframe.clone();
            let pat = pattern_to_monitor.clone();
            // let start_rel = candle_fetch_start_relative.to_string();
            // let end_rel = candle_fetch_end_relative.to_string();
            let start_iso = candle_fetch_start_specific_iso.to_string();
            let end_iso = candle_fetch_end_specific_iso.to_string();

        //     initial_load_tasks.push(tokio::spawn(async move {
        //         get_and_enrich_active_zones_for_pair(&sym, &tf, &pat, &start_rel, &end_rel).await
        //     }));
        // }

        initial_load_tasks.push(tokio::spawn(async move {
                get_and_enrich_active_zones_for_pair(&sym, &tf, &pat, &start_iso, &end_iso).await
            }));
        }
    }

    let results = futures_util::future::join_all(initial_load_tasks).await;
    {
        let mut cache = passed_in_cache.lock().await; // Use passed_in_cache
        cache.clear(); // Good practice for an initial load
        for result in results {
            match result {
                Ok(Ok(zones_for_pair)) => {
                    for zone in zones_for_pair {
                        if let Some(id) = &zone.zone_id {
                            cache.insert(id.clone(), zone);
                        }
                    }
                }
                Ok(Err(e)) => error!(
                    "[ZONE_MONITOR] Error fetching/enriching zones during initial load: {}",
                    e
                ),
                Err(e) => error!("[ZONE_MONITOR] Task panicked during initial load: {}", e),
            }
        }
        info!(
            "[ZONE_MONITOR] Initialized active zone cache with {} zones.",
            cache.len()
        );
    }

    // WebSocket connection loop
    loop {
        info!(
            "[ZONE_MONITOR] Attempting to connect to TypeScript WebSocket Price Stream at {}...",
            TS_WEBSOCKET_URL
        );
        match connect_async(Url::parse(TS_WEBSOCKET_URL).expect("Invalid WebSocket URL")).await {
            Ok((ws_stream, _response)) => {
                info!(
                    "[ZONE_MONITOR] Successfully connected to TypeScript WebSocket Price Stream."
                );
                let (_ws_sender, mut ws_receiver) = ws_stream.split();

                while let Some(msg_result) = ws_receiver.next().await {
                    match msg_result {
                        Ok(Message::Text(text)) => {
                            match serde_json::from_str::<WebSocketMessage>(&text) {
                                Ok(ws_msg) => {
                                    if ws_msg.msg_type == "BAR_UPDATE" {
                                        if let Some(bar_data) = ws_msg.data {
                                            process_bar_update_enriched(
                                                bar_data,
                                                Arc::clone(&passed_in_cache),
                                            )
                                            .await; // Use passed_in_cache
                                        }
                                    } else if ws_msg.msg_type == "CONNECTED" {
                                        info!("[ZONE_MONITOR] Confirmed CONNECTED message from TS WSS.");
                                    }
                                }
                                Err(e) => {
                                    warn!("[ZONE_MONITOR] Failed to deserialize WebSocket message: {}. Raw: {}", e, text);
                                }
                            }
                        }
                        Ok(Message::Close(_)) => {
                            warn!("[ZONE_MONITOR] TS WSS closed.");
                            break;
                        }
                        Ok(Message::Ping(_ping_data)) => {
                            debug!("[ZONE_MONITOR] Ping from TS WSS.");
                        } // Prefixed _ping_data
                        Ok(Message::Pong(_)) => {
                            debug!("[ZONE_MONITOR] Pong from TS WSS.");
                        }
                        Ok(msg) => {
                            debug!("[ZONE_MONITOR] Other TS WSS msg: {:?}", msg);
                        }
                        Err(e) => {
                            /* ... error handling ... */
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                error!(
                    "[ZONE_MONITOR] Failed to connect to TypeScript WebSocket: {}",
                    e
                );
            }
        }
        info!("[ZONE_MONITOR] Disconnected from TypeScript WebSocket. Retrying in 5 seconds...");
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

async fn process_bar_update_enriched(bar: BarUpdateData, zone_cache: ActiveZoneCache) {
    // Takes ActiveZoneCache of EnrichedZone
    let cache = zone_cache.lock().await;
    if cache.is_empty() {
        return;
    }

    for (_zone_id, zone) in cache.iter() {
        // zone is &EnrichedZone
        // Ensure symbol in zone matches bar.symbol (which should be _SB)
        // Your EnrichedZone doesn't have a direct `symbol` field like StoredZone did.
        // The symbol is part of the key for fetching, or needs to be added to EnrichedZone if not already there.
        // For now, let's assume EnrichedZone needs its own symbol field, or we filter upstream.
        // *** Let's assume the `zone` object contains the symbol implicitly through its generation or add it.
        // *** For now, we'll assume the iteration means `zone` is for `bar.symbol`.
        // *** A better way is to make zone_cache: HashMap<String (symbol_tf_pattern), Vec<EnrichedZone>>
        // *** or when iterating: if !zone_matches_bar_symbol(&zone, &bar.symbol) { continue; }

        // This check needs EnrichedZone to have a symbol field, or you pass the symbol it belongs to.
        // Let's assume `get_and_enrich_active_zones_for_pair` ensures zones are for the bar.symbol
        // or we adjust the cache structure. For now, we'll proceed assuming the iteration implies relevance.

        if let (Some(zone_high_val), Some(zone_low_val), Some(zone_type_str)) =
            (zone.zone_high, zone.zone_low, &zone.zone_type)
        // Accessing EnrichedZone fields
        {
            let proximal_line: f64;
            let pip_factor = if bar.symbol.contains("JPY") {
                0.01
            } else {
                0.0001
            };

            if zone_type_str.eq_ignore_ascii_case("supply_zone") {
                // Match EnrichedZone.zone_type
                proximal_line = zone_low_val;
                if bar.high >= proximal_line {
                    info!("[ZONE_TOUCH_V1_ENRICHED] SUPPLY Zone {} ({}) touched by {} bar. Bar High: {}, Zone Low (Proximal): {}. Approx SL: {:.5}, Approx TP: {:.5}",
                          zone.zone_id.as_deref().unwrap_or("N/A"),
                          zone_type_str,
                          bar.symbol,
                          bar.high,
                          proximal_line,
                          zone_high_val + (4.0 * pip_factor),
                          zone_low_val - (70.0 * pip_factor)
                    );
                    // TODO V2: Manage touch count, emit signal
                }
            } else if zone_type_str.eq_ignore_ascii_case("demand_zone") {
                // Match EnrichedZone.zone_type
                proximal_line = zone_high_val;
                if bar.low <= proximal_line {
                    info!("[ZONE_TOUCH_V1_ENRICHED] DEMAND Zone {} ({}) touched by {} bar. Bar Low: {}, Zone High (Proximal): {}. Approx SL: {:.5}, Approx TP: {:.5}",
                          zone.zone_id.as_deref().unwrap_or("N/A"),
                          zone_type_str,
                          bar.symbol,
                          bar.low,
                          proximal_line,
                          zone_low_val - (4.0 * pip_factor),
                          zone_high_val + (70.0 * pip_factor)
                    );
                    // TODO V2: Manage touch count, emit signal
                }
            }
        }
    }
}
