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

// --- BEGINNING OF ADDITIONS/MODIFICATIONS FOR RAW ZONE LOGGING ---
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::sync::Mutex;
use chrono::Local; // For current detection time
use lazy_static::lazy_static; // Ensure lazy_static is in Cargo.toml
// --- END OF ADDITIONS/MODIFICATIONS FOR RAW ZONE LOGGING ---

type BoxedError = Box<dyn std::error::Error + Send + Sync>;

// --- BEGINNING OF ADDITIONS FOR RAW ZONE LOGGING ---
lazy_static! {
    static ref RAW_ZONE_TXT_LOGGER: Mutex<BufWriter<File>> = {
        let file_path = "detected_raw_zones.txt"; // Log file in the current working directory
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)
            .unwrap_or_else(|e| panic!("Failed to open or create {}: {}", file_path, e));
        Mutex::new(BufWriter::new(file))
    };
}

// Helper function to log a raw detected zone to the text file
fn write_raw_zone_to_text_log(
    symbol: &str,
    timeframe: &str,
    raw_zone_type_key: &str, // "supply_zones" or "demand_zones" (from recognizer output)
    zone_json: &serde_json::Value,
) {
    let detection_timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let zone_type_for_log = if raw_zone_type_key.contains("supply") { "supply" } else { "demand" };

    // Extract key details from the raw zone_json as produced by the recognizer
    let zone_start_time = zone_json.get("start_time").and_then(|v| v.as_str()).unwrap_or("N/A");
    let zone_high = zone_json.get("zone_high").and_then(|v| v.as_f64()).map_or_else(|| "N/A".to_string(), |f| format!("{:.6}", f));
    let zone_low = zone_json.get("zone_low").and_then(|v| v.as_f64()).map_or_else(|| "N/A".to_string(), |f| format!("{:.6}", f));
    let start_idx_raw = zone_json.get("start_idx").and_then(|v| v.as_u64()).map_or_else(|| "N/A".to_string(), |u| u.to_string());
    let detection_method = zone_json.get("detection_method").and_then(|v| v.as_str()).unwrap_or("N/A");


    let log_message = format!(
        "DetectedAt: {} | Symbol: {} | TF: {} | Type: {} | ZoneStartTime: {} | ZoneHigh: {} | ZoneLow: {} | RawStartIdx: {} | Method: {}\n",
        detection_timestamp,
        symbol,
        timeframe,
        zone_type_for_log,
        zone_start_time,
        zone_high,
        zone_low,
        start_idx_raw,
        detection_method
    );

    match RAW_ZONE_TXT_LOGGER.lock() {
        Ok(mut writer) => {
            if let Err(e) = writer.write_all(log_message.as_bytes()) {
                log::error!("[RAW_ZONE_TXT_LOG] Failed to write: {}", e);
            }
            // Optional: Flush immediately if needed, though BufWriter handles it.
            // if let Err(e) = writer.flush() {
            //     log::error!("[RAW_ZONE_TXT_LOG] Failed to flush: {}", e);
            // }
        }
        Err(e) => {
            log::error!("[RAW_ZONE_TXT_LOG] Failed to acquire lock: {}", e);
        }
    }
}
// --- END OF ADDITIONS FOR RAW ZONE LOGGING ---


// --- Helper function fetch_distinct_tag_values using reqwest ---
// (This function is UNCHANGED from your version)
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
// (This function is UNCHANGED from your version)
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

// --- InfluxDB Writing Logic using reqwest ---
// (This function is UNCHANGED from your version)
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
    // Return generic Box<dyn Error>
    if zones_to_store.is_empty() {
        info!(
            "No zones provided to write_zones_batch for {}/{}",
            symbol, timeframe
        ); // Added context
        return Ok(());
    }

    let mut line_protocol_body = String::new();

    // Iterate through zones and build the line protocol string
    for zone in zones_to_store {
        // Attempt to parse start time and convert to nanoseconds for timestamp
        let timestamp_nanos_str = zone
            .start_time
            .as_ref()
            .and_then(|st| chrono::DateTime::parse_from_rfc3339(st).ok())
            .and_then(|dt| dt.timestamp_nanos_opt())
            .map(|ns| ns.to_string())
            .unwrap_or_else(|| {
                warn!(
                    "Zone missing valid start_time for timestamp: {:?}. Skipping point.",
                    zone
                );
                "".to_string() // Return empty string if invalid
            });

        // Skip this data point if the timestamp is invalid
        if timestamp_nanos_str.is_empty() {
            continue;
        }

        // Determine zone type heuristically from detection method string
        let zone_type = if zone
            .detection_method
            .as_deref()
            .unwrap_or("")
            .contains("Bullish")
            || zone
                .detection_method
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("demand")
        {
            "demand"
        } else if zone
            .detection_method
            .as_deref()
            .unwrap_or("")
            .contains("Bearish")
            || zone
                .detection_method
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("supply")
        {
            "supply"
        } else {
            "unknown"
        };

        // Start line protocol: measurement,tag_set field_set timestamp
        line_protocol_body.push_str(measurement_name);

        // Add tags (escape tag keys/values) - WITHOUT is_active
        line_protocol_body.push_str(&format!(
            ",symbol={}",
            symbol
                .replace(' ', "\\ ")
                .replace(',', "\\,")
                .replace('=', "\\=")
        ));
        line_protocol_body.push_str(&format!(",timeframe={}", timeframe));
        line_protocol_body.push_str(&format!(
            ",pattern={}",
            pattern
                .replace(' ', "\\ ")
                .replace(',', "\\,")
                .replace('=', "\\=")
        ));
        line_protocol_body.push_str(&format!(",zone_type={}", zone_type));
        // *** REMOVED is_active TAG LINE ***
        // line_protocol_body.push_str(&format!(",is_active={}", zone.is_active));

        // Add fields
        let mut fields = Vec::new();
        // Add zone_id field first - IMPORTANT NEW CODE
        if let Some(id) = &zone.zone_id {
            fields.push(format!("zone_id=\"{}\"", id.escape_default()));
        } else {
            // DO NOT GENERATE A NEW ID HERE.
            error!("Critical Error: StoredZone is missing zone_id for symbol={}, timeframe={}. Skipping record. Zone details: {:?}", symbol, timeframe, zone);
            continue; // Skip this zone if ID is missing
        }

        if let Some(v) = zone.zone_low {
            fields.push(format!("zone_low={}", v));
        }
        if let Some(v) = zone.zone_high {
            fields.push(format!("zone_high={}", v));
        }
        if let Some(v) = &zone.start_time {
            fields.push(format!("start_time_rfc3339=\"{}\"", v.escape_default()));
        }
        if let Some(v) = &zone.end_time {
            fields.push(format!("end_time_rfc3339=\"{}\"", v.escape_default()));
        }
        if let Some(v) = &zone.detection_method {
            fields.push(format!("detection_method=\"{}\"", v.escape_default()));
        }
        if let Some(v) = zone.fifty_percent_line {
            fields.push(format!("fifty_percent_line={}", v));
        }
        if let Some(v) = zone.bars_active {
            fields.push(format!("bars_active={}i", v));
        }

        // *** ADD is_active as INTEGER FIELD ***
        let is_active_int = if zone.is_active { 1 } else { 0 };
        fields.push(format!("is_active={}i", is_active_int)); // Note the 'i' suffix

        fields.push(format!("touch_count={}i", zone.touch_count.unwrap_or(0)));
        // Add strength_score (defaulting to 100.0 if None for safety)
        fields.push(format!(
            "strength_score={}",
            zone.strength_score.unwrap_or(100.0)
        ));

        // Serialize formation candles array into a JSON string field
        let candles_json = match serde_json::to_string(&zone.formation_candles) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to serialize formation candles: {}", e);
                "[]".to_string()
            }
        };
        fields.push(format!(
            "formation_candles_json=\"{}\"",
            candles_json.replace('"', "\\\"")
        ));

        // Combine fields and add timestamp if any fields were added
        if !fields.is_empty() {
            line_protocol_body.push(' '); // Space before fields
            line_protocol_body.push_str(&fields.join(","));
            line_protocol_body.push(' '); // Space before timestamp
            line_protocol_body.push_str(&timestamp_nanos_str); // Use the calculated string
            line_protocol_body.push('\n'); // Newline signifies end of point
        } else {
            warn!("Skipping zone point with no fields: {:?}", zone);
        }
    } // End of loop through zones

    // If no valid points were generated, return early
    if line_protocol_body.is_empty() {
        info!(
            "No valid points generated for batch write for {}/{}.",
            symbol, timeframe
        ); // Added context
        return Ok(());
    }

    // Prepare and send the HTTP POST request to the InfluxDB write API
    let write_url = format!(
        "{}/api/v2/write?org={}&bucket={}&precision=ns",
        influx_host, influx_org, write_bucket
    );
    info!(
        "Writing batch of ~{} points ({} bytes) for {}/{} to bucket '{}'",
        line_protocol_body.lines().count(),
        line_protocol_body.len(),
        symbol,
        timeframe,
        write_bucket
    );

    // Retry logic for writing
    let mut attempts = 0;
    let max_attempts = 3;
    while attempts < max_attempts {
        let response = http_client
            .post(&write_url)
            .bearer_auth(influx_token)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(line_protocol_body.clone()) // Clone body for potential retry
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    info!("Batch write successful for {}/{}.", symbol, timeframe); // Added context
                    return Ok(()); // Success
                } else {
                    let status = resp.status();
                    let body = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Failed to read error body".to_string());
                    attempts += 1;
                    error!(
                        "InfluxDB write attempt {}/{} for {}/{} failed with status {}: {}",
                        attempts, max_attempts, symbol, timeframe, status, body
                    );
                    if attempts >= max_attempts {
                        return Err(format!(
                            "InfluxDB write failed after {} attempts for {}/{} (status {}): {}",
                            max_attempts, symbol, timeframe, status, body
                        )
                        .into());
                    }
                    tokio::time::sleep(tokio::time::Duration::from_secs(2u64.pow(attempts))).await;
                }
            }
            Err(e) => {
                attempts += 1;
                error!(
                    "HTTP request error during write attempt {}/{} for {}/{}: {}",
                    attempts, max_attempts, symbol, timeframe, e
                );
                if attempts >= max_attempts {
                    return Err(Box::new(e));
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2u64.pow(attempts))).await;
            }
        }
    }
    // Should not be reached if retry logic is correct
    Err(format!(
        "Max write attempts exceeded for {}/{} (logic error)",
        symbol, timeframe
    )
    .into())
}

// --- Main Processing Function for the Generator with Concurrency ---
// Orchestrates the entire process with concurrent tasks
pub async fn run_zone_generation(is_periodic_run: bool) -> Result<(), Box<dyn Error>> {
    // At the start of your run_zone_generation function:
    let run_type = if is_periodic_run { "PERIODIC" } else { "FULL" };
    info!("Starting {} zone generation process...", run_type); // MODIFIED: Clarified run_type in log
    // dotenv().ok(); // MODIFIED: dotenv is called in main.rs, assuming it's already loaded
    
    // MODIFIED: Retained your original ENV var logging loop
    info!("Reading environment variables for generator run...");
    for var_name in [
        "INFLUXDB_HOST", "INFLUXDB_ORG", "INFLUXDB_TOKEN", "INFLUXDB_BUCKET",
        "GENERATOR_WRITE_BUCKET", "GENERATOR_TRENDBAR_MEASUREMENT", "GENERATOR_ZONE_MEASUREMENT",
        "GENERATOR_START_TIME", "GENERATOR_END_TIME", "GENERATOR_PATTERNS",
        "GENERATOR_PERIODIC_LOOKBACK", "GENERATOR_CHUNK_SIZE", "GENERATOR_EXCLUDE_TIMEFRAMES", // Added some common ones to your list
    ] {
        match env::var(var_name) {
            Ok(val) => info!("  [ENV] {} = {}", var_name, val),
            Err(_) => warn!("  [ENV] {} is not set (will use default or fail if required)", var_name),
        }
    }

    // --- Read ALL settings from Environment Variables ---
    let host = env::var("INFLUXDB_HOST")?;
    let org = env::var("INFLUXDB_ORG")?;
    let token = env::var("INFLUXDB_TOKEN")?;
    let read_bucket = env::var("INFLUXDB_BUCKET")?;
    let write_bucket = env::var("GENERATOR_WRITE_BUCKET")?;
    let trendbar_measurement = env::var("GENERATOR_TRENDBAR_MEASUREMENT")?;
    let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT")?;

    let patterns_str = env::var("GENERATOR_PATTERNS")?;
    let patterns: Vec<String> = patterns_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let chunk_size = env::var("GENERATOR_CHUNK_SIZE")
        .map(|s| s.parse::<usize>().unwrap_or(4))
        .unwrap_or(4);

    // --- Determine Time Window Based on Run Type ---
    // MODIFIED: These query_start_time and query_end_time are Strings and live long enough
    let (query_start_time, query_end_time) = if is_periodic_run {
        let lookback_duration = env::var("GENERATOR_PERIODIC_LOOKBACK")
                                    .unwrap_or_else(|_| "2h".to_string());
        info!("[ZONE_GENERATOR_{}] Using periodic lookback: {}", run_type, lookback_duration);
        (format!("-{}", lookback_duration), "now()".to_string())
    } else {
        // These are only used for full runs, not the query_start_time/query_end_time for periodic.
        let start_time_full = env::var("GENERATOR_START_TIME")?; 
        let end_time_full = env::var("GENERATOR_END_TIME")?;
        info!("[ZONE_GENERATOR_{}] Using fixed times: Start='{}', End='{}'", run_type, start_time_full, end_time_full);
        (start_time_full, end_time_full)
    };

    if patterns.is_empty() {
        return Err("No patterns specified in GENERATOR_PATTERNS env var.".into());
    }
    info!("Generator Config for this run:"); // MODIFIED: Clarified log for "this run"
    info!("  Read Bucket: {}", read_bucket);
    info!("  Write Bucket: {}", write_bucket);
    info!("  Trendbar Measurement: {}", trendbar_measurement);
    info!("  Zone Measurement: {}", zone_measurement);
    // MODIFIED: Log the actual query_start/end_time being used for candle fetching
    info!("  Query Start Time for candle fetch: {}", query_start_time);
    info!("  Query End Time for candle fetch: {}", query_end_time);
    info!("  Patterns: {:?}", patterns);
    info!("  Parallel Chunk Size: {}", chunk_size);

    let http_client = Client::new();

    let discovered_symbols = fetch_distinct_tag_values(
        &http_client, &host, &org, &token, &read_bucket, &trendbar_measurement, "symbol",
    ).await?;
    if discovered_symbols.is_empty() {
        return Err("No symbols found in trendbar measurement.".into());
    }

    let all_timeframes = fetch_distinct_tag_values(
        &http_client, &host, &org, &token, &read_bucket, &trendbar_measurement, "timeframe",
    ).await?;

    let excluded_timeframes_str = env::var("GENERATOR_EXCLUDE_TIMEFRAMES").unwrap_or_default();
    let excluded_timeframes: Vec<String> = excluded_timeframes_str
        .split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    info!("Excluded timeframes from ENV: {:?}", excluded_timeframes); // MODIFIED: Log actual excluded TFs

    let allowed_timeframes: Vec<String> = all_timeframes
        .into_iter()
        .filter(|tf| {
            if excluded_timeframes.contains(tf) { return false; }
            matches!(tf.as_str(), "1m" | "5m" | "15m" | "30m" | "1h" | "4h" | "1d")
        })
        .collect();
    if allowed_timeframes.is_empty() {
        return Err("No supported timeframes (1m-1d, excluding GENERATOR_EXCLUDE_TIMEFRAMES) found after filtering.".into()); // MODIFIED: Clarified error
    }
    info!("Will process symbols: {:?}", discovered_symbols);
    info!("Will process timeframes: {:?}", allowed_timeframes);

    let mut all_combinations = Vec::new();
    for symbol in discovered_symbols.iter() {
        for timeframe in allowed_timeframes.iter() {
            for pattern_name in patterns.iter() {
                all_combinations.push((symbol.clone(), timeframe.clone(), pattern_name.clone()));
            }
        }
    }

    let total_combinations = all_combinations.len();
    info!("Total combinations to process: {}", total_combinations);

    let mut total_processed = 0;
    let mut total_errors = 0;

    for (chunk_idx, chunk) in all_combinations.chunks(chunk_size).enumerate() {
        info!(
            "[ZONE_GENERATOR_{}] Processing chunk {}/{} ({} combinations)",
            run_type, chunk_idx + 1, (total_combinations + chunk_size - 1) / chunk_size, chunk.len()
        );
        let mut futures = Vec::new();
        for (symbol, timeframe, pattern_name) in chunk {
            // --- LIFETIME FIX APPLIED HERE ---
            // Pass references to the query_start_time and query_end_time Strings
            // which are defined in the outer scope of run_zone_generation and live long enough.
            let future = process_symbol_timeframe_pattern(
                symbol, timeframe, pattern_name,
                &query_start_time, // Pass as &str
                &query_end_time,   // Pass as &str
                &write_bucket, &zone_measurement,
                &http_client, &host, &org, &token,
            );
            // --- END OF LIFETIME FIX ---
            futures.push(future);
        }

        let results = futures::future::join_all(futures).await;
        for (i, result) in results.into_iter().enumerate() {
            let (symbol, timeframe, pattern_name) = &chunk[i];
            let task_id = format!("[ZONE_GENERATOR_{}] {}/{}/{}", run_type, symbol, timeframe, pattern_name);
            match result {
                Ok(_) => info!("[Task {}] Processing finished successfully.", task_id),
                Err(e) => {
                    error!("[Task {}] Error: {}", task_id, e);
                    total_errors += 1;
                }
            }
            total_processed += 1;
        }
        info!(
            "Progress: {}/{} combinations processed ({:.1}%)",
            total_processed, total_combinations,
            (total_processed as f64 / total_combinations as f64) * 100.0
        );
    }
    info!("[ZONE_GENERATOR_{}] Finished {} run.", run_type, run_type);
    if total_errors > 0 {
        error!("Zone generation process finished with {} errors.", total_errors);
        Err(format!("{} generation tasks failed.", total_errors).into())
    } else {
        info!("Zone generation process finished successfully.");
        Ok(())
    }
}

// --- Inner processing function for a single symbol/timeframe/pattern ---
// Fetches candle data, runs detection, enriches zones, and writes to InfluxDB.
async fn process_symbol_timeframe_pattern(
    symbol: &str,
    timeframe: &str,
    pattern_name: &str,
    // MODIFIED: These are now &str, referencing the longer-lived Strings from run_zone_generation
    fetch_start_time: &str,
    fetch_end_time: &str,
    write_bucket: &str,
    zone_measurement: &str,
    http_client: &Client,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
) -> Result<(), BoxedError> {

    // ChartQuery takes owned Strings, so we convert the borrowed &str here.
    let query = ChartQuery {
        symbol: symbol.to_string(),
        timeframe: timeframe.to_string(),
        start_time: fetch_start_time.to_string(),
        end_time: fetch_end_time.to_string(),
        pattern: pattern_name.to_string(),
        enable_trading: None, lot_size: None, stop_loss_pips: None,
        take_profit_pips: None, enable_trailing_stop: None,
    };

    let fetch_result =
        detect::_fetch_and_detect_core(&query, &format!("generator_{}/{}", symbol, timeframe))
            .await;

    let (candles, detection_results) = match fetch_result {
        Ok((c, _recognizer, d)) => (c, d),
        Err(e) => {
            // MODIFIED: Consistent logging prefix and error handling structure
            return match e {
                CoreError::Request(rq_err) => {
                    warn!("[GENERATOR] InfluxDB request failed for {}/{}: {}. Skipping.", symbol, timeframe, rq_err);
                    Ok(()) // Skippable
                }
                CoreError::Config(cfg_err) if cfg_err.contains("Pattern") && cfg_err.contains("not supported") => {
                    warn!("[GENERATOR] Pattern '{}' not supported. Skipping combo {}/{}/{}.", pattern_name, symbol, timeframe, pattern_name);
                    Ok(()) // Skippable
                }
                CoreError::Config(cfg_err) if cfg_err.contains("No data") => {
                    info!("[GENERATOR] No candle data returned by query for {}/{}/{}. Skipping.", symbol, timeframe, pattern_name);
                    Ok(()) // Skippable
                }
                CoreError::Config(cfg_err) if cfg_err.contains("CSV header mismatch") => {
                    error!("[GENERATOR] CSV header mismatch for {}/{}: {}", symbol, timeframe, cfg_err);
                    Err(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, cfg_err)) as BoxedError)
                }
                CoreError::Csv(csv_err) => {
                    error!("[GENERATOR] CSV parsing error for {}/{}: {}", symbol, timeframe, csv_err);
                    Err(Box::new(csv_err) as BoxedError)
                }
                _ => {
                    error!("[GENERATOR] Core fetch/detect error for {}/{}: {}", symbol, timeframe, e);
                    Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())) as BoxedError)
                }
            };
        }
    };

    let candle_count = candles.len();

    if candle_count == 0 {
        info!("[GENERATOR] No candle data available for {}/{}/{}. Skipping zone processing.", symbol, timeframe, pattern_name);
        return Ok(());
    }

    info!("[GENERATOR] Fetched {} candles for {}/{}.", candle_count, symbol, timeframe);
    info!("[GENERATOR] Detection complete for {}/{}/{}.", symbol, timeframe, pattern_name);

    let mut zones_to_store: Vec<StoredZone> = Vec::new();
    let price_data = detection_results.get("data").and_then(|d| d.get("price"));

    // MODIFIED: Counters for more detailed summary logging
    let mut total_zones_from_recognizer_field = 0;
    if let Some(val) = detection_results.get("total_detected").and_then(|v| v.as_u64()) {
        total_zones_from_recognizer_field = val;
    }
    let mut actual_zones_iterated_from_arrays = 0;


    for (zone_type_key, is_supply_flag) in [("supply_zones", true), ("demand_zones", false)] {
        if let Some(zones_array) = price_data
            .and_then(|p| p.get(zone_type_key))
            .and_then(|zt| zt.get("zones"))
            .and_then(|z| z.as_array())
        {
            debug!("[GENERATOR] Processing {} raw {} zones from recognizer output for {}/{}.", zones_array.len(), zone_type_key, symbol, timeframe);

            for zone_json in zones_array {
                actual_zones_iterated_from_arrays += 1; // Count each zone we attempt to process

                // --- ADDITION: Call to log raw zone to text file ---
                write_raw_zone_to_text_log(symbol, timeframe, zone_type_key, zone_json);
                // --- END OF ADDITION ---

                // Your existing StoredZone conversion and enrichment logic
                match serde_json::from_value::<StoredZone>(zone_json.clone()) {
                    Ok(mut base_zone) => {
                        if base_zone.zone_id.is_none() {
                            let raw_type_str = zone_json.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            let zone_type_for_id = if raw_type_str.contains("supply") { "supply" } else { "demand" };
                            let symbol_for_id = if symbol.ends_with("_SB") { symbol.to_string() } else { format!("{}_SB", symbol) };
                            let zone_id = detect::generate_deterministic_zone_id(
                                &symbol_for_id, timeframe, zone_type_for_id,
                                base_zone.start_time.as_deref(), base_zone.zone_high, base_zone.zone_low,
                            );
                            base_zone.zone_id = Some(zone_id);
                        }
                        if base_zone.start_idx.is_none() || base_zone.end_idx.is_none() {
                            warn!("[GENERATOR] Skipping StoredZone conversion due to missing start/end_idx: ZoneID {:?}", base_zone.zone_id);
                            continue;
                        }
                        base_zone.is_active = detect::is_zone_still_active(zone_json, &candles, is_supply_flag);
                        base_zone.formation_candles = extract_formation_candles(&candles, base_zone.start_idx, base_zone.end_idx);
                        let start_idx_val = base_zone.start_idx.unwrap();
                        let end_idx_val = base_zone.end_idx.unwrap();
                        if end_idx_val >= start_idx_val {
                            base_zone.bars_active = Some(end_idx_val - start_idx_val + 1);
                        } else {
                            warn!("[GENERATOR] Invalid indices for bars_active: start={}, end={}. Setting to None. ZoneID: {:?}", start_idx_val, end_idx_val, base_zone.zone_id);
                            base_zone.bars_active = None;
                        }
                        let calculated_touches = detect::calculate_touches_within_range(zone_json, &candles, is_supply_flag, candle_count);
                        base_zone.touch_count = Some(calculated_touches);
                        base_zone.strength_score = Some(100.0); // Default score
                        zones_to_store.push(base_zone);
                    }
                    Err(e) => {
                        error!("[GENERATOR] Failed to deserialize zone_json into StoredZone for {}/{}: {}. JSON: {}", symbol, timeframe, e, zone_json);
                    }
                }
            }
        } else {
            debug!("[GENERATOR] No '{}' key or not an array in detection_results for {}/{}/{}.", zone_type_key, symbol, timeframe, pattern_name);
        }
    }

    // MODIFIED: More detailed summary log
    info!(
        "[GENERATOR_SUMMARY] {}/{}/{}: RecognizerOutputTotalDetected={}, ActualZonesIteratedFromArray={}, FinalZonesPreparedForDB={}",
        symbol, timeframe, pattern_name,
        total_zones_from_recognizer_field,
        actual_zones_iterated_from_arrays,
        zones_to_store.len()
    );

    if !zones_to_store.is_empty() {
        info!("[GENERATOR] Storing {} processed zones for {}/{}/{}.", zones_to_store.len(), symbol, timeframe, pattern_name);
        match write_zones_batch(
            http_client, influx_host, influx_org, influx_token,
            write_bucket, zone_measurement, symbol, timeframe, pattern_name, &zones_to_store,
        ).await {
            Ok(_) => info!("[GENERATOR] Successfully wrote zones for {}/{}/{}", symbol, timeframe, pattern_name),
            Err(e) => {
                error!("[GENERATOR] Failed to write zones batch for {}/{}/{}: {}", symbol, timeframe, pattern_name, e);
                return Err(e);
            }
        }
    } else {
        info!("[GENERATOR] No zones to store for {}/{}/{}.", symbol, timeframe, pattern_name);
    }

    Ok(())
}