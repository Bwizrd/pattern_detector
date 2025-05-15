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

// --- InfluxDB Writing Logic using reqwest ---
// Writes a batch of StoredZone data points using InfluxDB Line Protocol via HTTP API.
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
    info!("Starting zone generation process...");
    dotenv().ok();
    info!("Reading environment variables...");
    for var_name in [
        "INFLUXDB_HOST",
        "INFLUXDB_ORG",
        "INFLUXDB_TOKEN",
        "INFLUXDB_BUCKET",
        "GENERATOR_WRITE_BUCKET",
        "GENERATOR_TRENDBAR_MEASUREMENT",
        "GENERATOR_ZONE_MEASUREMENT",
        "GENERATOR_START_TIME",
        "GENERATOR_END_TIME",
        "GENERATOR_PATTERNS",
    ] {
        match env::var(var_name) {
            Ok(val) => info!("  {} = {}", var_name, val),
            Err(_) => error!("  {} is not set!", var_name),
        }
    }
    // dotenv is called in main.rs

    // --- Read ALL settings from Environment Variables ---
    let host = env::var("INFLUXDB_HOST")?;
    let org = env::var("INFLUXDB_ORG")?;
    let token = env::var("INFLUXDB_TOKEN")?;
    let read_bucket = env::var("INFLUXDB_BUCKET")?; // Bucket for reading candles
    let write_bucket = env::var("GENERATOR_WRITE_BUCKET")?; // Bucket for writing zones
    let trendbar_measurement = env::var("GENERATOR_TRENDBAR_MEASUREMENT")?; // Measurement with candles
    let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT")?; // Target measurement for zones

    // Start and end time values - could be relative (like -90d) or absolute RFC3339 timestamps
    let start_time = env::var("GENERATOR_START_TIME")?; // Start of analysis period (can be relative)
    let end_time = env::var("GENERATOR_END_TIME")?; // End of analysis period (can be relative)

    let patterns_str = env::var("GENERATOR_PATTERNS")?; // Comma-separated list of patterns
    let patterns: Vec<String> = patterns_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Get parallel processing size from env var or use a reasonable default
    let chunk_size = env::var("GENERATOR_CHUNK_SIZE")
        .map(|s| s.parse::<usize>().unwrap_or(4))
        .unwrap_or(4); // Default: process 4 combinations at once
                       // --- End Reading Env Vars ---

                       // --- Determine Time Window Based on Run Type ---
    let (query_start_time, query_end_time) = if is_periodic_run {
        // Periodic Run: Use rolling window relative to now()
        let lookback_duration = env::var("GENERATOR_PERIODIC_LOOKBACK") // New env var
                                    .unwrap_or_else(|_| "2h".to_string());
        info!("[ZONE_GENERATOR_PERIODIC] Using lookback: {}", lookback_duration);
        (format!("-{}", lookback_duration), "now()".to_string())
    } else {
        // Full/Historical Run: Use fixed times from environment
        let start_time = env::var("GENERATOR_START_TIME")?;
        let end_time = env::var("GENERATOR_END_TIME")?;
        info!("[ZONE_GENERATOR_FULL] Using fixed times: Start='{}', End='{}'", start_time, end_time);
        (start_time, end_time)
    };

    if patterns.is_empty() {
        return Err("No patterns specified in GENERATOR_PATTERNS env var.".into());
    }
    info!("Generator Config from Env:");
    info!("  Read Bucket: {}", read_bucket);
    info!("  Write Bucket: {}", write_bucket);
    info!("  Trendbar Measurement: {}", trendbar_measurement);
    info!("  Zone Measurement: {}", zone_measurement);
    info!("  Start Time: {}", start_time);
    info!("  End Time: {}", end_time);
    info!("  Patterns: {:?}", patterns);
    info!("  Parallel Chunk Size: {}", chunk_size);

    // Create a single reusable reqwest client
    let http_client = Client::new();

    // 1. Discover Symbols from the source measurement
    let discovered_symbols = fetch_distinct_tag_values(
        &http_client,
        &host,
        &org,
        &token,
        &read_bucket,
        &trendbar_measurement,
        "symbol",
    )
    .await?;
    if discovered_symbols.is_empty() {
        return Err("No symbols found in trendbar measurement.".into());
    }

    // 2. Discover Timeframes and filter for supported ones

    let all_timeframes = fetch_distinct_tag_values(
        &http_client,
        &host,
        &org,
        &token,
        &read_bucket,
        &trendbar_measurement,
        "timeframe",
    )
    .await?;

    let excluded_timeframes_str = env::var("GENERATOR_EXCLUDE_TIMEFRAMES").unwrap_or_default();
    let excluded_timeframes: Vec<String> = excluded_timeframes_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    info!("Excluded timeframes: {:?}", excluded_timeframes);

    let allowed_timeframes: Vec<String> = all_timeframes
        .into_iter()
        .filter(|tf| {
            // First, check if it's in the excluded list
            if excluded_timeframes.contains(tf) {
                return false;
            }
            // Then apply the original supported timeframe filter
            matches!(
                tf.as_str(),
                "1m" | "5m" | "15m" | "30m" | "1h" | "4h" | "1d"
            )
        })
        .collect();
    if allowed_timeframes.is_empty() {
        return Err("No supported timeframes (1m-1d) found in trendbar measurement.".into());
    }
    info!("Will process symbols: {:?}", discovered_symbols);
    info!("Will process timeframes: {:?}", allowed_timeframes);

    // --- Create a list of all combinations ---
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

    // --- Process combinations in chunks (for parallel processing) ---
    let mut total_processed = 0;
    let mut total_errors = 0;

    // Process in chunks to allow for some parallelism
    for (chunk_idx, chunk) in all_combinations.chunks(chunk_size).enumerate() {
        info!(
            "[ZONE_GENERATOR_{}] Processing chunk {}/{} ({} combinations)",
            run_type, chunk_idx + 1, (total_combinations + chunk_size - 1) / chunk_size, chunk.len()
        );

        let mut futures = Vec::new();

        // Create a future for each combination in this chunk
        for (symbol, timeframe, pattern_name) in chunk {
            let future = process_symbol_timeframe_pattern(
                symbol,
                timeframe,
                pattern_name,
                &start_time,
                &end_time,
                &write_bucket,
                &zone_measurement,
                &http_client,
                &host,
                &org,
                &token,
            );
            futures.push(future);
        }

        // Process this chunk of futures in parallel
        let results = futures::future::join_all(futures).await;

        // Process results from this chunk
        for (i, result) in results.into_iter().enumerate() {
            let (symbol, timeframe, pattern_name) = &chunk[i];
            let task_id = format!("[ZONE_GENERATOR_{}] {}/{}/{}", run_type, symbol, timeframe, pattern_name);

            match result {
                Ok(_) => {
                    info!("[Task {}] Processing finished successfully.", task_id);
                }
                Err(e) => {
                    error!("[Task {}] Error: {}", task_id, e);
                    total_errors += 1;
                }
            }

            total_processed += 1;
        }

        // Log progress after each chunk
        info!(
            "Progress: {}/{} combinations processed ({:.1}%)",
            total_processed,
            total_combinations,
            (total_processed as f64 / total_combinations as f64) * 100.0
        );
    }
    info!("[ZONE_GENERATOR_{}] Finished {} run.", run_type, run_type);
    // Report final status based on errors
    if total_errors > 0 {
        error!(
            "Zone generation process finished with {} errors.",
            total_errors
        );
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
            // --- RESTORED FULL ERROR HANDLING ---
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
            // --- END RESTORED ERROR HANDLING ---
        }
    };

    // --- Get candle count early ---
    let candle_count = candles.len();

    // If no candle data was retrieved (handled above, but good safety check)
    if candle_count == 0 { // Use candle_count here
        info!(
            "No candle data available for {}/{}/{}. Skipping zone processing.",
            symbol, timeframe, pattern_name
        );
        return Ok(());
    }

    info!(
        "Fetched {} candles for {}/{}.",
        candle_count, // Use candle_count
        symbol,
        timeframe
    );
    info!(
        "Detection complete for {}/{}/{}.",
        symbol, timeframe, pattern_name
    );

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
                        // --- Zone ID Generation Block ---
                        if base_zone.zone_id.is_none() {
                            // Calculate type for ID using .contains()
                            let raw_type_str =
                                zone_json.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            let zone_type_for_id = if raw_type_str.contains("supply") {
                                "supply"
                            } else {
                                "demand"
                            };

                            // Log inputs before ID generation
                            info!("[GENERATOR_ID_CHECK] Symbol: {}, TF: {}, TypeForID: '{}', StartTime: {:?}, High: {:?}, Low: {:?}, RawJsonType: {:?}",
                                  symbol,
                                  timeframe,
                                  zone_type_for_id,
                                  base_zone.start_time.as_deref(),
                                  base_zone.zone_high,
                                  base_zone.zone_low,
                                  zone_json.get("type").and_then(|v| v.as_str()));

                            // Generate ID using the public function from detect module
                            let symbol_for_id = if symbol.ends_with("_SB") { symbol.to_string() } else { format!("{}_SB", symbol) }; // Ensure suffix
                            let zone_id = detect::generate_deterministic_zone_id(
                                &symbol_for_id, // Use suffixed symbol
                                timeframe,
                                zone_type_for_id, // Use calculated type
                                base_zone.start_time.as_deref(),
                                base_zone.zone_high,
                                base_zone.zone_low,
                            );
                            info!("[GENERATOR_ID_CHECK] -> GeneratedID: {}", zone_id); // Log result
                            base_zone.zone_id = Some(zone_id);
                        }
                        // --- End Zone ID Generation ---


                        // --- Essential Index Check ---
                        // Ensure required fields for further processing are present
                        if base_zone.start_idx.is_none() || base_zone.end_idx.is_none() {
                            warn!(
                                "Skipping zone processing (after ID gen) due to missing start_idx or end_idx: ZoneID {:?}",
                                base_zone.zone_id
                            );
                            deser_failed += 1; // Count as failure for processing, though ID might be generated
                            continue; // Skip further processing for this zone
                        }
                        // --- End Index Check ---


                        // --- is_active Calculation ---
                        // Determine if the zone is still active based on ALL candles fetched for this run
                        // Use public function from detect module
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
                        // --- End is_active Calculation ---


                        // --- Formation Candles Extraction ---
                        base_zone.formation_candles = extract_formation_candles(
                            &candles,
                            base_zone.start_idx, // These are Some() due to check above
                            base_zone.end_idx,   // These are Some() due to check above
                        );
                        // --- End Formation Candles ---


                        // --- bars_active Calculation ---
                        // Indices are guaranteed Some() here from the check above
                        let start_idx_val = base_zone.start_idx.unwrap();
                        let end_idx_val = base_zone.end_idx.unwrap();

                        if end_idx_val >= start_idx_val {
                            base_zone.bars_active = Some(end_idx_val - start_idx_val + 1);
                             // debug!( /* ... */ ); // Optional debug log
                        } else {
                            warn!(
                                "Invalid indices for bars_active calculation: start={}, end={}. Setting bars_active to None.",
                                start_idx_val, end_idx_val
                            );
                            base_zone.bars_active = None;
                        }
                        // --- End bars_active Calculation ---


                        // --- Touch Count Calculation ---
                        // Log candle range available for touch calculation relative to zone end time
                        let first_candle_time = candles.first().map(|c| c.time.as_str());
                        let last_candle_time = candles.last().map(|c| c.time.as_str());
                        let zone_end_time_str = zone_json.get("end_time").and_then(|v| v.as_str()); // Get end time as string
                        info!("[GENERATOR_TOUCH_CHECK] ZoneID: {:?}, Zone End Time: {:?}. Processing {} candles from {:?} to {:?}",
                              base_zone.zone_id, // Log the ID for correlation
                              zone_end_time_str,
                              candle_count,
                              first_candle_time,
                              last_candle_time);

                        // Call the public function from detect module
                        let calculated_touches = detect::calculate_touches_within_range(
                            zone_json,      // Pass raw JSON for reliable indices/levels
                            &candles,       // Pass the candle slice fetched for THIS generator run
                            is_supply_flag, // Pass the flag indicating zone type context
                            candle_count    // Pass candle count
                        );
                        info!("[GENERATOR_TOUCH_CHECK] ZoneID: {:?} -> Calculated Touches: {}", base_zone.zone_id, calculated_touches); // Log result
                        base_zone.touch_count = Some(calculated_touches); // Store the calculated value
                        // --- End Touch Count Calculation ---


                        // --- Strength Score Initialization ---
                        base_zone.strength_score = Some(100.0); // Initialize score
                        // --- End Strength Score Initialization ---


                        // Add the fully processed zone to the list
                        zones_to_store.push(base_zone);

                    } // End Ok arm
                    Err(e) => {
                        // Log errors if deserialization fails
                        error!(
                            "Failed to deserialize zone into StoredZone for {}/{}: {}. JSON: {}",
                            symbol, timeframe, e, zone_json
                        );
                        deser_failed += 1;
                    }
                } // End match serde_json
            } // End loop for zone_json

            // Log summary for this zone type
            info!(
                "[ZONE_GENERATOR] {}/{}/{}: {} {} zones - {} active, {} inactive, {} failed deserialization",
                symbol, timeframe, zone_type_key, zone_count, zone_type_key, active_count, inactive_count, deser_failed
            );

        } else { // End if let Some(zones_array)
            debug!(
                "No '{}' key found or it's not an array in detection results for {}/{}/{}.",
                zone_type_key, symbol, timeframe, pattern_name
            );
        }
    } // End loop for (zone_type_key, is_supply_flag)

    // Log overall summary for the symbol/timeframe
    info!(
        "[ZONE_GENERATOR] {}/{}: TOTAL ZONES PROCESSED: {} - {} active, {} inactive, {} stored",
        symbol,
        timeframe,
        total_zones,
        active_zones,
        inactive_zones,
        zones_to_store.len()
    );

    // --- Write to InfluxDB ---
    if !zones_to_store.is_empty() {
        info!(
            "Storing {} processed zones for {}/{}/{}.",
            zones_to_store.len(),
            symbol,
            timeframe,
            pattern_name
        );
        // Call the batch write function
        match write_zones_batch(
            http_client,
            influx_host,
            influx_org,
            influx_token,
            write_bucket,
            zone_measurement,
            symbol, // Pass original symbol or suffixed one depending on write_zones_batch needs
            timeframe,
            pattern_name,
            &zones_to_store, // Pass zones containing calculated touch_count
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
                return Err(e); // Propagate write errors
            }
        }
    } else {
        info!(
            "No zones detected or processed to store for {}/{}/{}.",
            symbol, timeframe, pattern_name
        );
    }
    // --- End Write to InfluxDB ---

    Ok(()) // Indicate successful processing for this combination
}
