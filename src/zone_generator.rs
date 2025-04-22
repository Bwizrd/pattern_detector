// src/zone_generator.rs
use crate::detect::{self, CandleData, ChartQuery, CoreError}; // Reuses detect module's core logic and types
use crate::types::StoredZone; // Uses the StoredZone struct which includes formation_candles
                              // Note: PatternRecognizer trait itself is not directly used here, as _fetch_and_detect_core handles recognizer creation
use csv::ReaderBuilder; // For parsing CSV responses from InfluxDB
use dotenv::dotenv;
use futures::future::join_all; // Used for waiting on tasks if concurrency is re-added later
use log::{debug, error, info, warn};
use reqwest::Client; // HTTP client for InfluxDB communication
use serde_json::json;
use std::env; // For reading environment variables
use std::error::Error; // For building JSON request bodies

type BoxedError = Box<dyn std::error::Error + Send + Sync>;

// --- Helper function fetch_distinct_tag_values using reqwest ---
// Fetches distinct values for a given tag from a measurement via InfluxDB HTTP API.
async fn fetch_distinct_tag_values(
    http_client: &Client,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    read_bucket: &str,
    measurement: &str,
    tag: &str,
) -> Result<Vec<String>, Box<dyn Error>> {
    info!(
        "Fetching distinct values via HTTP for tag '{}' in measurement '{}' (bucket '{}')...",
        tag, measurement, read_bucket
    );
    // Flux query to get distinct tag values and keep only the value column
    let flux_query = format!(
        r#" import "influxdata/influxdb/schema"
            schema.measurementTagValues(bucket: "{}", measurement: "{}", tag: "{}") |> keep(columns: ["_value"]) "#,
        read_bucket, measurement, tag
    );

    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);
    let mut values = Vec::new();

    // Send POST request to InfluxDB /api/v2/query endpoint
    let response = http_client
        .post(&query_url)
        .bearer_auth(influx_token)
        .header("Accept", "application/csv") // Request CSV format
        .header("Content-Type", "application/json")
        .json(&json!({
            "query": flux_query,
            "type": "flux"
        }))
        .send()
        .await?;

    // Check response status for success
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error body".to_string());
        error!(
            "InfluxDB distinct tag query failed with status {}: {}",
            status, body
        );
        return Err(format!("InfluxDB query failed (status {}): {}", status, body).into());
    }

    // Read the response body as text (CSV)
    let csv_data = response.text().await?;
    debug!("Raw CSV response for tag '{}':\n{}", tag, csv_data);

    // Handle potentially empty responses
    if csv_data.trim().is_empty() || !csv_data.contains('\n') {
        warn!(
            "Received empty or header-only response for distinct tag query: {}",
            tag
        );
        return Ok(values); // Return empty vector, not an error
    }

    // Parse the CSV data using the csv crate
    let mut rdr = ReaderBuilder::new()
        .has_headers(true) // Expect a header row
        .from_reader(csv_data.as_bytes()); // Read from the response bytes

    // Read headers to find the index of the "_value" column
    let headers = rdr.headers()?.clone();
    if let Some(value_idx) = headers.iter().position(|h| h == "_value") {
        // Iterate over CSV records
        for result in rdr.records() {
            match result {
                Ok(record) => {
                    // Extract the value from the correct column
                    if let Some(value) = record.get(value_idx) {
                        if !value.is_empty() {
                            values.push(value.trim().to_string());
                        }
                    } else {
                        // Should not happen if header exists, but good to check
                        warn!(
                            "Record missing expected '_value' column at index {}: {:?}",
                            value_idx, record
                        );
                    }
                }
                Err(e) => {
                    // Log CSV parsing errors but continue processing other rows
                    error!("Error reading CSV record for tag '{}': {}", tag, e);
                }
            }
        }
    } else {
        // If the required "_value" header is missing, report an error
        let headers_str = headers.iter().collect::<Vec<_>>().join(", ");
        error!("'_value' column not found in schema query result header for tag '{}'. Headers found: [{}]", tag, headers_str);
        return Err(format!("'_value' column not found for tag '{}'", tag).into());
    }

    info!("Found {} distinct values for tag '{}'.", values.len(), tag);
    Ok(values)
}

// --- Helper to extract formation candles ---
// Extracts the specific candles (based on start/end index) that formed the zone.
fn extract_formation_candles(
    candles: &[CandleData],
    start_idx: Option<u64>,
    end_idx: Option<u64>,
) -> Vec<CandleData> {
    match (start_idx, end_idx) {
        (Some(start), Some(end)) => {
            let start_usize = start as usize;
            let end_usize = end as usize;

            if start_usize < candles.len() && end_usize < candles.len() && start_usize <= end_usize
            {
                let mut key_candles = Vec::new();

                // Always include the 50% candle (usually at start_idx)
                if let Some(candle) = candles.get(start_usize) {
                    key_candles.push(candle.clone());
                }

                // Always include the big bar candle (usually at start_idx + 1)
                if start_usize + 1 < candles.len() {
                    if let Some(candle) = candles.get(start_usize + 1) {
                        key_candles.push(candle.clone());
                    }
                }

                // For longer zones, include price extremes (highest/lowest)
                if end_usize > start_usize + 2 {
                    // Find highest and lowest candles in the range
                    let mut highest_idx = start_usize;
                    let mut lowest_idx = start_usize;
                    let mut highest_price = candles[start_usize].high;
                    let mut lowest_price = candles[start_usize].low;

                    for i in (start_usize + 1)..=end_usize {
                        if i < candles.len() {
                            if candles[i].high > highest_price {
                                highest_price = candles[i].high;
                                highest_idx = i;
                            }
                            if candles[i].low < lowest_price {
                                lowest_price = candles[i].low;
                                lowest_idx = i;
                            }
                        }
                    }

                    // Add highest and lowest candles if they're not already included
                    if highest_idx > start_usize + 1 && highest_idx != lowest_idx {
                        key_candles.push(candles[highest_idx].clone());
                    }
                    if lowest_idx > start_usize + 1 && lowest_idx != start_usize {
                        key_candles.push(candles[lowest_idx].clone());
                    }
                }

                // Always include the last candle in the zone
                if end_usize > start_usize + 1 {
                    if let Some(candle) = candles.get(end_usize) {
                        // Only add if not already added
                        if !key_candles.iter().any(|c| c.time == candle.time) {
                            key_candles.push(candle.clone());
                        }
                    }
                }

                // Ensure candles are sorted by time
                key_candles.sort_by(|a, b| a.time.cmp(&b.time));

                key_candles
            } else {
                warn!(
                    "Invalid start/end indices ({:?}, {:?}) for candle slice (len {})",
                    start_idx,
                    end_idx,
                    candles.len()
                );
                vec![]
            }
        }
        _ => {
            warn!("Missing start_idx or end_idx for extracting formation candles");
            vec![]
        }
    }
}

// --- write_zones_batch: is_active as integer FIELD ---
async fn write_zones_batch(
    http_client: &Client,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    write_bucket: &str,
    measurement_name: &str,
    symbol: &str,
    timeframe: &str,
    pattern: &str,
    zones_to_store: &[StoredZone],
) -> Result<(), BoxedError> {
    if zones_to_store.is_empty() { info!("No zones provided to write_zones_batch."); return Ok(()); }

    let mut line_protocol_body = String::new();
    info!("[WRITE_INIT] {}/{}: Preparing to build line protocol for {} zones. Initial body length: {}",
          symbol, timeframe, zones_to_store.len(), line_protocol_body.len());
    let mut loop_active_count = 0;
    let mut loop_inactive_count = 0;

    for (index, zone) in zones_to_store.iter().enumerate() {
        let initial_len = line_protocol_body.len();

        let timestamp_nanos_str = zone.start_time.as_ref()
            .and_then(|st| chrono::DateTime::parse_from_rfc3339(st).ok())
            .and_then(|dt| dt.timestamp_nanos_opt()).map(|ns| ns.to_string())
            .unwrap_or_else(|| { warn!("Zone missing valid start_time... Skipping."); "".to_string() });

        if timestamp_nanos_str.is_empty() { warn!("[WRITE_GROWTH] Index: {}, SKIPPED (invalid timestamp)", index); continue; }

        let zone_type = if zone.detection_method.as_deref().unwrap_or("").contains("Bullish") || zone.detection_method.as_deref().unwrap_or("").to_lowercase().contains("demand") { "demand" }
                        else if zone.detection_method.as_deref().unwrap_or("").contains("Bearish") || zone.detection_method.as_deref().unwrap_or("").to_lowercase().contains("supply") { "supply" }
                        else { "unknown" };

        debug!("[WRITE_LOOP_CHECK] Index: {}, StartTime: {:?}, IsActive Field: {}", index, zone.start_time, zone.is_active);
        if zone.is_active { loop_active_count += 1; } else { loop_inactive_count += 1; }

        line_protocol_body.push_str(measurement_name);
        // Add tags (NO is_active tag here)
        line_protocol_body.push_str(&format!(",symbol={}", symbol.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        line_protocol_body.push_str(&format!(",timeframe={}", timeframe));
        line_protocol_body.push_str(&format!(",pattern={}", pattern.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        line_protocol_body.push_str(&format!(",zone_type={}", zone_type));
        // REMOVED: line_protocol_body.push_str(&format!(",is_active={}", ...));

        // Add fields
        let mut fields = Vec::new();
        if let Some(v) = zone.zone_low { fields.push(format!("zone_low={}", v)); }
        if let Some(v) = zone.zone_high { fields.push(format!("zone_high={}", v)); }
        if let Some(v) = &zone.start_time { fields.push(format!("start_time_rfc3339=\"{}\"", v.escape_default())); }
        if let Some(v) = &zone.end_time { fields.push(format!("end_time_rfc3339=\"{}\"", v.escape_default())); }
        if let Some(v) = &zone.detection_method { fields.push(format!("detection_method=\"{}\"", v.escape_default())); }
        if let Some(v) = zone.quality_score { fields.push(format!("quality_score={}", v)); }
        if let Some(v) = &zone.strength { fields.push(format!("strength=\"{}\"", v.escape_default())); }
        if let Some(v) = zone.fifty_percent_line { fields.push(format!("fifty_percent_line={}", v)); }
        if let Some(v) = zone.bars_active { fields.push(format!("bars_active={}i", v)); }

        // *** ADD is_active as INTEGER FIELD ***
        let is_active_int = if zone.is_active { 1 } else { 0 };
        fields.push(format!("is_active={}i", is_active_int));
        // *** END ADD ***

        // Add back the candles JSON field
        let candles_json = match serde_json::to_string(&zone.formation_candles) {
            Ok(s) => s, Err(e) => { error!("Failed to serialize formation candles: {}", e); "[]".to_string() }
        };
        fields.push(format!("formation_candles_json=\"{}\"", candles_json.replace('"', "\\\"")));

        if !fields.is_empty() {
            line_protocol_body.push(' ');
            line_protocol_body.push_str(&fields.join(","));
            line_protocol_body.push(' ');
            line_protocol_body.push_str(&timestamp_nanos_str);
            line_protocol_body.push('\n');

            let added_len = line_protocol_body.len() - initial_len;
            debug!("[WRITE_GROWTH] Index: {}, Active: {}, Added Length: {}, Current Total Length: {}", index, zone.is_active, added_len, line_protocol_body.len());
        } else {
            warn!("Skipping zone point with no fields: {:?}", zone);
            debug!("[WRITE_GROWTH] Index: {}, SKIPPED (no fields), Current Total Length: {}", index, line_protocol_body.len());
            line_protocol_body.truncate(initial_len);
        }
    }

    info!("[WRITE_LOOP_SUMMARY] {}/{}: Loop finished. Processed {} zones. Internal counts - Active: {}, Inactive: {}", symbol, timeframe, zones_to_store.len(), loop_active_count, loop_inactive_count);

    if line_protocol_body.is_empty() { info!("No valid points generated for batch write after processing loop."); return Ok(()); }

    let lines: Vec<&str> = line_protocol_body.trim_end().split('\n').collect();
    let line_count = lines.len();
    // PRE-SEND check no longer checks is_active tag
    info!("[WRITE_ZONES_BATCH] {}/{}: PRE-SEND CHECK: Built line protocol body with {} total lines.", symbol, timeframe, line_count);

    let write_url = format!("{}/api/v2/write?org={}&bucket={}&precision=ns", influx_host, influx_org, write_bucket);
    info!("Writing batch of ~{} points ({} bytes) for {}/{} to bucket '{}'", line_protocol_body.lines().count(), line_protocol_body.len(), symbol, timeframe, write_bucket);

    let mut attempts = 0;
    let max_attempts = 3;
    while attempts < max_attempts {
        let response = http_client.post(&write_url).bearer_auth(influx_token).header("Content-Type", "text/plain; charset=utf-8").body(line_protocol_body.clone()).send().await;
        match response {
            Ok(resp) => {
                if resp.status().is_success() { info!("Batch write successful."); return Ok(()); }
                else {
                    let status = resp.status(); let body = resp.text().await.unwrap_or_else(|_| "Failed to read error body".to_string()); attempts += 1;
                    error!("InfluxDB write attempt {}/{} failed with status {}: {}", attempts, max_attempts, status, body);
                    if attempts >= max_attempts { return Err(format!("InfluxDB write failed after {} attempts (status {}): {}", max_attempts, status, body).into()); }
                    tokio::time::sleep(tokio::time::Duration::from_secs(2u64.pow(attempts))).await;
                }
            }
            Err(e) => {
                attempts += 1; error!("HTTP request error during write attempt {}/{}: {}", attempts, max_attempts, e);
                if attempts >= max_attempts { return Err(Box::new(e)); }
                tokio::time::sleep(tokio::time::Duration::from_secs(2u64.pow(attempts))).await;
            }
        }
    }
    Err("Max write attempts exceeded (logic error)".into())
}

// --- Main Processing Function for the Generator with Concurrency ---
// Orchestrates the entire process with concurrent tasks
pub async fn run_zone_generation() -> Result<(), Box<dyn Error>> {
    info!("!!! Starting zone generation process (HARDCODED for single symbol/timeframe) !!!");
    dotenv().ok();

    // --- Read Env Vars needed for the single task ---
    info!("Reading environment variables for single task...");
    let host = env::var("INFLUXDB_HOST")?;
    let org = env::var("INFLUXDB_ORG")?;
    let token = env::var("INFLUXDB_TOKEN")?;
    let write_bucket = env::var("GENERATOR_WRITE_BUCKET")?;
    let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT")?;
    let start_time = env::var("GENERATOR_START_TIME")?;
    let end_time = env::var("GENERATOR_END_TIME")?;
    let patterns_str = env::var("GENERATOR_PATTERNS").expect("GENERATOR_PATTERNS must be set");

    // --- Parse the first pattern from the env var ---
    let patterns: Vec<String> = patterns_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if patterns.is_empty() {
        error!("No patterns specified in GENERATOR_PATTERNS env var.");
        return Err("No patterns specified in GENERATOR_PATTERNS env var.".into());
    }
    let target_pattern = patterns[0].clone(); // Use the first pattern found
                                              // --- End Reading Env Vars ---

    // **** DEFINE HARDCODED VALUES ****
    let target_symbol = "EURUSD".to_string(); // Base symbol, _SB suffix is handled in _fetch_and_detect_core if needed
    let target_timeframe = "5m".to_string();
    // **** END HARDCODED VALUES ****

    // --- Log Configuration for the single task ---
    info!("-> Generator Config from Env (for single task):");
    info!("   InfluxDB Host: {}", host); // Log host for verification
    info!("   InfluxDB Org: {}", org);
    info!("   Write Bucket: {}", write_bucket);
    info!("   Zone Measurement: {}", zone_measurement);
    info!("   Start Time: {}", start_time);
    info!("   End Time: {}", end_time);
    info!("-> Hardcoded Task:");
    info!("   Symbol: {}", target_symbol);
    info!("   Timeframe: {}", target_timeframe);
    info!("   Pattern: {}", target_pattern);

    // --- Create a single reusable reqwest client ---
    let http_client = Client::new();

    // --- Directly Call the Processing Function ---
    info!(
        "[Direct Call] Processing the single task: {}/{} with pattern {}",
        target_symbol, target_timeframe, target_pattern
    );

    // Await the single future directly
    let result = process_symbol_timeframe_pattern(
        &target_symbol,    // Pass reference
        &target_timeframe, // Pass reference
        &target_pattern,   // Pass reference
        &start_time,       // Pass reference
        &end_time,         // Pass reference
        &write_bucket,     // Pass reference
        &zone_measurement, // Pass reference
        &http_client,      // Pass reference
        &host,             // Pass reference
        &org,              // Pass reference
        &token,            // Pass reference
    )
    .await; // Await the single future

    // --- Process the single result ---
    match result {
        Ok(_) => {
            info!(
                "[Direct Call] Processing for {}/{} finished successfully.",
                target_symbol, target_timeframe
            );
            info!("!!! Zone generation process finished successfully (single task mode). !!!");
            Ok(()) // Return success
        }
        Err(e) => {
            error!(
                "[Direct Call] Error processing single task {}/{}: {}",
                target_symbol, target_timeframe, e
            );
            error!("!!! Zone generation process finished WITH ERROR (single task mode). !!!");
            Err(e) // Propagate the specific error
        }
    }
}

// --- Inner processing function for a single symbol/timeframe/pattern ---
// Fetches candle data, runs detection, enriches zones, and writes to InfluxDB.
async fn process_symbol_timeframe_pattern(
    symbol: &str,
    timeframe: &str,
    pattern_name: &str,
    start_time: &str,
    end_time: &str,
    write_bucket: &str,
    zone_measurement: &str,
    http_client: &Client,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
) -> Result<(), BoxedError> {
    // Add Send + Sync bounds to ensure thread safety

    // Prepare query parameters for the detection function
    let query = ChartQuery {
        symbol: symbol.to_string(),
        timeframe: timeframe.to_string(),
        start_time: start_time.to_string(),
        end_time: end_time.to_string(),
        pattern: pattern_name.to_string(),
        enable_trading: None,
        lot_size: None,
        stop_loss_pips: None,
        take_profit_pips: None,
        enable_trailing_stop: None,
    };

    // Call the shared detection function from detect.rs
    // Process the result immediately to avoid moving non-Send types across await points
    let fetch_result =
        detect::_fetch_and_detect_core(&query, &format!("generator_{}/{}", symbol, timeframe))
            .await;

    // Process the result of the detection core function - immediately process data to avoid Send issues
    let (candles, detection_results) = match fetch_result {
        Ok((c, _recognizer, d)) => (c, d), // Extract candles and JSON results, drop non-Send recognizer
        Err(e) => {
            // Handle errors from detection core
            match e {
                CoreError::Request(rq_err) => {
                    warn!(
                        "InfluxDB request failed for {}/{}: {}. Skipping.",
                        symbol, timeframe, rq_err
                    );
                    return Ok(()); // Treat request errors as skippable for this combo
                }
                CoreError::Config(cfg_err)
                    if cfg_err.contains("Pattern") && cfg_err.contains("not supported") =>
                {
                    warn!(
                        "Pattern '{}' not supported. Skipping combo {}/{}/{}.",
                        pattern_name, symbol, timeframe, pattern_name
                    );
                    return Ok(()); // Skip unsupported patterns gracefully
                }
                CoreError::Config(cfg_err) if cfg_err.contains("No data") => {
                    info!(
                        // Log as info, not error, as empty data is expected sometimes
                        "No candle data returned by query for {}/{}/{}. Skipping.",
                        symbol, timeframe, pattern_name
                    );
                    return Ok(()); // Skip processing if no data fetched
                }
                CoreError::Config(cfg_err) if cfg_err.contains("CSV header mismatch") => {
                    error!(
                        "CSV header mismatch for {}/{}: {}",
                        symbol, timeframe, cfg_err
                    );
                    // This is a more critical error, return it
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        cfg_err,
                    )) as BoxedError);
                }
                CoreError::Csv(csv_err) => {
                    error!(
                        "CSV parsing error for {}/{}: {}",
                        symbol, timeframe, csv_err
                    );
                    // This is a more critical error, return it
                    return Err(Box::new(csv_err) as BoxedError);
                }
                // REMOVED CoreError::Json arm as it doesn't exist in the definition
                _ => {
                    // Catch-all for other CoreErrors or wrapped errors
                    error!(
                        "Core fetch/detect error for {}/{}: {}",
                        symbol, timeframe, e
                    );
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    )) as BoxedError);
                }
            }
        }
    };

    // If no candle data was retrieved (handled above, but good safety check)
    if candles.is_empty() {
        info!(
            "No candle data available for {}/{}/{}. Skipping zone processing.",
            symbol, timeframe, pattern_name
        );
        return Ok(());
    }
    info!(
        "Fetched {} candles for {}/{}.",
        candles.len(),
        symbol,
        timeframe
    );
    info!(
        "Detection complete for {}/{}/{}.",
        symbol, timeframe, pattern_name
    );

    // **** ADD THIS LOG ****
    if let Ok(results_str) = serde_json::to_string_pretty(&detection_results) {
        info!(
            "[RAW_DETECTION_RESULTS] {}/{}: \n{}",
            symbol, timeframe, results_str
        );
    } else {
        warn!(
            "[RAW_DETECTION_RESULTS] {}/{}: Failed to serialize detection results.",
            symbol, timeframe
        );
    }
    // **** END ADDED LOG ****

    // Prepare to store processed zones
    let mut zones_to_store: Vec<StoredZone> = Vec::new();
    // Navigate the JSON structure returned by the detector
    let price_data = detection_results.get("data").and_then(|d| d.get("price"));

    let mut total_zones = 0;
    let mut active_zones = 0;
    let mut inactive_zones = 0;

    // Process both supply and demand zones found in the results
    for (zone_type_key, is_supply_flag) in [("supply_zones", true), ("demand_zones", false)] {
        if let Some(zones_array) = price_data
            .and_then(|p| p.get(zone_type_key))
            .and_then(|zt| zt.get("zones"))
            .and_then(|z| z.as_array())
        {
            let zone_count = zones_array.len();
            total_zones += zone_count;
            debug!(
                "Processing {} raw {} zones for {}/{}.",
                zone_count, zone_type_key, symbol, timeframe
            );

            // Track zone counts by active status
            let mut active_count = 0;
            let mut inactive_count = 0;
            let mut deser_failed = 0;

            // Iterate through each raw zone detected
            for zone_json in zones_array {
                // Attempt to deserialize the raw JSON into the StoredZone struct
                match serde_json::from_value::<StoredZone>(zone_json.clone()) {
                    Ok(mut base_zone) => {
                        // Ensure required fields for processing are present
                        if base_zone.start_idx.is_none() || base_zone.end_idx.is_none() {
                            warn!(
                                "Skipping zone due to missing start_idx or end_idx: {:?}",
                                zone_json
                            );
                            deser_failed += 1;
                            continue; // Skip this zone if essential indices are missing
                        }

                        // Determine if the zone is still active based on ALL candles fetched for this run
                        base_zone.is_active =
                            detect::is_zone_still_active(zone_json, &candles, is_supply_flag);

                        // Count active vs inactive
                        if base_zone.is_active {
                            active_count += 1;
                            active_zones += 1;
                        } else {
                            inactive_count += 1;
                            inactive_zones += 1;
                        }

                        // Extract the specific candles that formed this zone
                        // NOTE: Ensure extract_formation_candles is defined elsewhere in this file or imported
                        base_zone.formation_candles = extract_formation_candles(
                            &candles,
                            base_zone.start_idx,
                            base_zone.end_idx,
                        );

                        // --- Calculate and assign bars_active ---
                        // The number of bars forming the zone is end_idx - start_idx + 1 (inclusive)
                        // We already checked start_idx and end_idx are Some above
                        let start = base_zone.start_idx.unwrap(); // Safe to unwrap here
                        let end = base_zone.end_idx.unwrap(); // Safe to unwrap here

                        if end >= start {
                            // Basic sanity check
                            base_zone.bars_active = Some(end - start + 1);
                            debug!(
                                "Calculated bars_active: {} for zone starting at idx {}",
                                base_zone.bars_active.unwrap_or(0), // Use unwrap_or for logging clarity
                                start
                            );
                        } else {
                            warn!(
                                "Invalid indices for bars_active calculation: start={}, end={}. Setting bars_active to None.",
                                start, end
                            );
                            // Reset to None if calculation is invalid
                            base_zone.bars_active = None;
                        }
                        // --- End calculation ---
                        let was_active = base_zone.is_active;
                        // Add the fully processed zone to the list
                        zones_to_store.push(base_zone);
                        // **** ADD THIS LOG ****
                        debug!(
                            "[PUSH_CHECK] Pushed zone (active: {}). zones_to_store size is now: {}",
                            was_active,
                            zones_to_store.len()
                        );
                        // **** END ADDED LOG ****
                    }
                    Err(e) => {
                        // Log errors if deserialization fails (e.g., structure mismatch)
                        error!(
                            "Failed to deserialize zone into StoredZone for {}/{}: {}. JSON: {}",
                            symbol, timeframe, e, zone_json
                        );
                        deser_failed += 1;
                    }
                }
            }

            info!(
                "[ZONE_GENERATOR] {}/{}/{}: {} {} zones - {} active, {} inactive, {} failed deserialization",
                symbol, timeframe, zone_type_key, zone_count, zone_type_key, active_count, inactive_count, deser_failed
            );
        } else {
            debug!(
                "No '{}' key found or it's not an array in detection results for {}/{}/{}.",
                zone_type_key, symbol, timeframe, pattern_name
            );
        }
    }

    info!(
        "[ZONE_GENERATOR] {}/{}: TOTAL ZONES: {} - {} active, {} inactive, {} stored",
        symbol,
        timeframe,
        total_zones,
        active_zones,
        inactive_zones,
        zones_to_store.len()
    );

    // **** ADD THIS LOGGING ****
    let final_stored_count = zones_to_store.len();
    let final_active_stored_count = zones_to_store.iter().filter(|z| z.is_active).count();
    let final_inactive_stored_count = final_stored_count - final_active_stored_count;
    info!("[ZONE_GENERATOR] {}/{}: PRE-WRITE CHECK: zones_to_store contains {} total zones ({} active, {} inactive).",
       symbol, timeframe, final_stored_count, final_active_stored_count, final_inactive_stored_count);

    // Add detailed check for duplicates based on start_time (example)
    use std::collections::HashMap;
    let mut start_time_counts = HashMap::new();
    for zone in &zones_to_store {
        if let Some(st) = &zone.start_time {
            *start_time_counts.entry(st.clone()).or_insert(0) += 1;
        }
    }
    let duplicates: Vec<_> = start_time_counts
        .iter()
        .filter(|(_, &count)| count > 1)
        .collect();
    if !duplicates.is_empty() {
        warn!("[ZONE_GENERATOR] {}/{}: PRE-WRITE CHECK: Found duplicate start_times in zones_to_store: {:?}",
           symbol, timeframe, duplicates);
    } else {
        info!("[ZONE_GENERATOR] {}/{}: PRE-WRITE CHECK: No duplicate start_times found in zones_to_store.",
           symbol, timeframe);
    }
    // **** END ADDED LOGGING ****

    // If any zones were processed, write them to InfluxDB
    if !zones_to_store.is_empty() {
        info!(
            "Storing {} processed zones for {}/{}/{}/{}/{}/{}.",
            final_stored_count,
            zones_to_store.len(),
            symbol,
            timeframe,
            pattern_name,
            final_active_stored_count,
            final_inactive_stored_count
        );
        // Call the batch write function, passing necessary details
        // The write_zones_batch function already handles the 'bars_active' field correctly
        // NOTE: Ensure write_zones_batch is defined elsewhere in this file or imported
        match write_zones_batch(
            http_client,
            influx_host,
            influx_org,
            influx_token,
            write_bucket,
            zone_measurement,
            symbol,
            timeframe,
            pattern_name,
            &zones_to_store,
        )
        .await
        {
            Ok(_) => info!(
                "Successfully wrote zones for {}/{}/{}",
                symbol, timeframe, pattern_name
            ),
            Err(e) => {
                error!(
                    "Failed to write zones batch for {}/{}/{}: {}",
                    symbol, timeframe, pattern_name, e
                );
                // Decide if a write error should stop the entire process or just this combo
                return Err(e); // Propagate write errors
            }
        }
    } else {
        info!(
            "No zones detected or processed to store for {}/{}/{}.",
            symbol, timeframe, pattern_name
        );
    }

    Ok(()) // Indicate successful processing for this combination
}
