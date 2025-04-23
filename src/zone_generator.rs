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

        // Add tags (escape tag keys/values if they contain special chars like space, comma, equals)
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
        // Removed tag-based is_active

        // Add fields
        let mut fields = Vec::new();
        if let Some(v) = zone.zone_low {
            fields.push(format!("zone_low={}", v));
        }
        if let Some(v) = zone.zone_high {
            fields.push(format!("zone_high={}", v));
        }
        // String fields need to be double-quoted and internal quotes escaped
        if let Some(v) = &zone.start_time {
            fields.push(format!("start_time_rfc3339=\"{}\"", v.escape_default()));
        }
        if let Some(v) = &zone.end_time {
            fields.push(format!("end_time_rfc3339=\"{}\"", v.escape_default()));
        }
        if let Some(v) = &zone.detection_method {
            fields.push(format!("detection_method=\"{}\"", v.escape_default()));
        }
        if let Some(v) = zone.quality_score {
            fields.push(format!("quality_score={}", v));
        }
        if let Some(v) = &zone.strength {
            fields.push(format!("strength=\"{}\"", v.escape_default()));
        }
        if let Some(v) = zone.fifty_percent_line {
            fields.push(format!("fifty_percent_line={}", v));
        }

        // Add is_active as integer field (MOVED HERE)
        let is_active_val = if zone.is_active { 1 } else { 0 };
        fields.push(format!("is_active={}i", is_active_val));

        // --- ADD bars_active FIELD (as integer) ---
        if let Some(v) = zone.bars_active {
            fields.push(format!("bars_active={}i", v)); // Append 'i' for integer type in line protocol
        }
        // --- End Add bars_active ---

        // Serialize formation candles array into a JSON string field
        let candles_json = match serde_json::to_string(&zone.formation_candles) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to serialize formation candles: {}", e);
                "[]".to_string()
            }
        };
        // Escape double quotes within the JSON string for line protocol
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
    }

    // If no valid points were generated, return early
    if line_protocol_body.is_empty() {
        info!("No valid points generated for batch write.");
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
                    info!("Batch write successful.");
                    return Ok(()); // Success
                } else {
                    // Handle HTTP errors from InfluxDB
                    let status = resp.status();
                    let body = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Failed to read error body".to_string());
                    attempts += 1;
                    error!(
                        "InfluxDB write attempt {}/{} failed with status {}: {}",
                        attempts, max_attempts, status, body
                    );
                    if attempts >= max_attempts {
                        return Err(format!(
                            "InfluxDB write failed after {} attempts (status {}): {}",
                            max_attempts, status, body
                        )
                        .into());
                    }
                    // Exponential backoff delay before retrying
                    tokio::time::sleep(tokio::time::Duration::from_secs(2u64.pow(attempts))).await;
                }
            }
            Err(e) => {
                // Handle reqwest network errors
                attempts += 1;
                error!(
                    "HTTP request error during write attempt {}/{}: {}",
                    attempts, max_attempts, e
                );
                if attempts >= max_attempts {
                    return Err(Box::new(e)); // Return the reqwest error
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2u64.pow(attempts))).await;
            }
        }
    }
    // Should not be reached if retry logic is correct
    Err("Max write attempts exceeded (logic error)".into())
}

// --- Main Processing Function for the Generator with Concurrency ---
// Orchestrates the entire process with concurrent tasks
pub async fn run_zone_generation() -> Result<(), Box<dyn Error>> {
    // At the start of your run_zone_generation function:
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
            "Processing chunk {}/{} ({} combinations)",
            chunk_idx + 1,
            (total_combinations + chunk_size - 1) / chunk_size,
            chunk.len()
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
            let task_id = format!("{}/{}/{}", symbol, timeframe, pattern_name);

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

    let fetch_result =
        detect::_fetch_and_detect_core(&query, &format!("generator_{}/{}", symbol, timeframe))
            .await;

    let (candles, detection_results) = match fetch_result {
        Ok((c, _, d)) => (c, d),
        Err(e) => match e {
            CoreError::Request(rq_err) => {
                warn!(
                    "InfluxDB request failed for {}/{}: {}. Skipping.",
                    symbol, timeframe, rq_err
                );
                return Ok(());
            }
            CoreError::Config(cfg_err)
                if cfg_err.contains("Pattern") && cfg_err.contains("not supported") =>
            {
                warn!("Pattern '{}' not supported. Skipping...", pattern_name);
                return Ok(());
            }
            CoreError::Config(cfg_err) if cfg_err.contains("No data") => {
                info!(
                    "No candle data for {}/{}/{}. Skipping.",
                    symbol, timeframe, pattern_name
                );
                return Ok(());
            }
            CoreError::Config(cfg_err) if cfg_err.contains("CSV header mismatch") => {
                error!(
                    "CSV header mismatch for {}/{}: {}",
                    symbol, timeframe, cfg_err
                );
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
                return Err(Box::new(csv_err) as BoxedError);
            }
            _ => {
                error!(
                    "Core fetch/detect error for {}/{}: {}",
                    symbol, timeframe, e
                );
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )) as BoxedError);
            }
        },
    };

    if candles.is_empty() {
        info!(
            "No candle data for {}/{}/{}. Skipping.",
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

    // Log RAW JSON
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

    let mut zones_to_store: Vec<StoredZone> = Vec::new();
    let price_data = detection_results.get("data").and_then(|d| d.get("price"));
    let mut total_zones = 0;
    let mut active_zones = 0;
    let mut inactive_zones = 0;

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
            let mut active_count = 0;
            let mut inactive_count = 0;
            let mut deser_failed = 0;

            for zone_json in zones_array {
                // **** Extract start_idx HERE for logging ****
                let current_start_idx = zone_json.get("start_idx").and_then(|v| v.as_u64());

                match serde_json::from_value::<StoredZone>(zone_json.clone()) {
                    Ok(mut base_zone) => {
                        if base_zone.start_idx.is_none() || base_zone.end_idx.is_none() {
                            warn!("Skipping zone missing indices...");
                            deser_failed += 1;
                            continue;
                        }

                        // Check activity
                        base_zone.is_active =
                            detect::is_zone_still_active(zone_json, &candles, is_supply_flag);

                        // **** Log activity check result ****
                        info!("[GENERATOR_ACTIVITY_CHECK] {}/{}: Zone start_idx {:?}, is_active result: {}",
                              symbol, timeframe,
                              current_start_idx, // Use variable defined above
                              base_zone.is_active);
                        // **** END Log ****

                        if base_zone.is_active {
                            active_count += 1;
                            active_zones += 1;
                        } else {
                            inactive_count += 1;
                            inactive_zones += 1;
                        }

                        base_zone.formation_candles = extract_formation_candles(
                            &candles,
                            base_zone.start_idx,
                            base_zone.end_idx,
                        );
                        if let (Some(start), Some(end)) = (base_zone.start_idx, base_zone.end_idx) {
                            if end >= start {
                                base_zone.bars_active = Some(end - start + 1); /* debug log */
                            } else {                                /* warn log */
                                base_zone.bars_active = None;
                            }
                        } else {
                            base_zone.bars_active = None;
                        }

                        let was_active = base_zone.is_active;
                        zones_to_store.push(base_zone);
                        debug!(
                            "[PUSH_CHECK] Pushed zone (active: {}). zones_to_store size is now: {}",
                            was_active,
                            zones_to_store.len()
                        );
                    }
                    Err(e) => {
                        error!(
                            "Failed to deserialize zone for {}/{}: {}. JSON: {}",
                            symbol, timeframe, e, zone_json
                        );
                        deser_failed += 1;
                    }
                }
            }
            info!("[ZONE_GENERATOR] {}/{}/{}: {} {} zones - {} active, {} inactive, {} failed deserialization", symbol, timeframe, zone_type_key, zone_count, zone_type_key, active_count, inactive_count, deser_failed);
        } else {
            debug!(
                "No '{}' key found or not an array for {}/{}/{}",
                zone_type_key, symbol, timeframe, pattern_name
            );
        }
    }

    info!(
        "[ZONE_GENERATOR] {}/{}: TOTAL ZONES: {} - {} active, {} inactive, {} to be stored",
        symbol,
        timeframe,
        total_zones,
        active_zones,
        inactive_zones,
        zones_to_store.len()
    );

    // Log active start times before write
    let active_zone_start_times: Vec<String> = zones_to_store
        .iter()
        .filter(|z| z.is_active)
        .filter_map(|z| z.start_time.clone())
        .collect();
    info!(
        "[GENERATOR_ACTIVE_LIST] {}/{}: Found {} active zones with start_times: {:?}",
        symbol,
        timeframe,
        active_zone_start_times.len(),
        active_zone_start_times
    );

    // PRE-WRITE check log
    let final_stored_count = zones_to_store.len();
    let final_active_stored_count = zones_to_store.iter().filter(|z| z.is_active).count();
    let final_inactive_stored_count = final_stored_count - final_active_stored_count;
    info!("[ZONE_GENERATOR] {}/{}: PRE-WRITE CHECK: zones_to_store contains {} total zones ({} active, {} inactive).", symbol, timeframe, final_stored_count, final_active_stored_count, final_inactive_stored_count);

    // Call write_zones_batch
    if !zones_to_store.is_empty() {
        info!(
            "Storing {} processed zones for {}/{}/{}.",
            final_stored_count, symbol, timeframe, pattern_name
        );
        match write_zones_batch(
            // Use the correct version (writing tag is_active="true" or integer field is_active=1i)
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
                return Err(e);
            }
        }
    } else {
        info!(
            "No zones detected or processed to store for {}/{}/{}.",
            symbol, timeframe, pattern_name
        );
    }

    Ok(())
}
