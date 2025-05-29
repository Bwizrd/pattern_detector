// src/zone_generator.rs
use crate::detect::enrich_zones_with_activity;
use crate::detect::{self, CandleData, ChartQuery, CoreError}; // Reuses detect module's core logic and types
use crate::types::StoredZone; // Uses the StoredZone struct which includes formation_candles

use csv::ReaderBuilder; // For parsing CSV responses from InfluxDB
use dotenv::dotenv; // For loading environment variables
use log::{debug, error, info, warn, trace};
use reqwest::Client; // HTTP client for InfluxDB communication
use serde_json::json;
use std::env; // For reading environment variables
use std::error::Error; // For building JSON request bodies
use std::collections::HashSet;

// --- BEGINNING OF ADDITIONS/MODIFICATIONS FOR RAW ZONE LOGGING ---
use chrono::Local; // For current detection time
use lazy_static::lazy_static;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::sync::Mutex; // Ensure lazy_static is in Cargo.toml
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
    let zone_type_for_log = if raw_zone_type_key.contains("supply") {
        "supply"
    } else {
        "demand"
    };

    // Extract key details from the raw zone_json as produced by the recognizer
    let zone_start_time = zone_json
        .get("start_time")
        .and_then(|v| v.as_str())
        .unwrap_or("N/A");
    let zone_high = zone_json
        .get("zone_high")
        .and_then(|v| v.as_f64())
        .map_or_else(|| "N/A".to_string(), |f| format!("{:.6}", f));
    let zone_low = zone_json
        .get("zone_low")
        .and_then(|v| v.as_f64())
        .map_or_else(|| "N/A".to_string(), |f| format!("{:.6}", f));
    let start_idx_raw = zone_json
        .get("start_idx")
        .and_then(|v| v.as_u64())
        .map_or_else(|| "N/A".to_string(), |u| u.to_string());
    let detection_method = zone_json
        .get("detection_method")
        .and_then(|v| v.as_str())
        .unwrap_or("N/A");

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
async fn fetch_distinct_tag_values(
    http_client: &Client,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    read_bucket: &str,
    measurement: &str,
    tag: &str,
) -> Result<Vec<String>, BoxedError> {
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

// --- MODIFIED: InfluxDB Writing Logic with start_idx and end_idx support ---
async fn write_zones_batch(
    http_client: &Client,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    write_bucket: &str,
    measurement_name: &str,
    symbol: &str,
    timeframe: &str,
    pattern: &str, // This is the pattern_name
    zones_to_store: &[StoredZone],
) -> Result<(), BoxedError> {
    if zones_to_store.is_empty() {
        info!("[GENERATOR_WRITE] No zones provided to write_zones_batch for {}/{}/{}", symbol, timeframe, pattern);
        return Ok(());
    }
    info!("[GENERATOR_WRITE] Preparing to write {} zones for {}/{}/{}", zones_to_store.len(), symbol, timeframe, pattern);

    let mut line_protocol_body = String::new();

    for zone in zones_to_store {
        let timestamp_nanos_str = zone.start_time.as_ref()
            .and_then(|st| chrono::DateTime::parse_from_rfc3339(st).ok())
            .and_then(|dt| dt.timestamp_nanos_opt()).map(|ns| ns.to_string())
            .unwrap_or_else(|| {
                warn!("[GENERATOR_WRITE] Zone missing valid start_time for timestamp: {:?}. Skipping point.", zone.zone_id);
                "".to_string()
            });

        if timestamp_nanos_str.is_empty() {
            continue;
        }

        let zone_type_for_tag = zone.zone_type.as_deref().unwrap_or_else(|| {
            if zone.detection_method.as_deref().unwrap_or("").to_lowercase().contains("supply") ||
               zone.detection_method.as_deref().unwrap_or("").to_lowercase().contains("bearish") { "supply" } else { "demand" }
        }).replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=");

        line_protocol_body.push_str(measurement_name);
        line_protocol_body.push_str(&format!(",symbol={}", symbol.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        line_protocol_body.push_str(&format!(",timeframe={}", timeframe.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        line_protocol_body.push_str(&format!(",pattern={}", pattern.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        line_protocol_body.push_str(&format!(",zone_type={}", zone_type_for_tag));

        let mut fields = Vec::new();
        if let Some(id) = &zone.zone_id {
            fields.push(format!("zone_id=\"{}\"", id.escape_default()));
        } else {
            error!("[GENERATOR_WRITE] Critical: StoredZone missing zone_id for symbol={}, timeframe={}. Skipping write. Zone details: {:?}", symbol, timeframe, zone);
            continue;
        }

        // --- FIXED: ENSURE start_idx AND end_idx ARE CORRECTLY WRITTEN ---
        if let Some(v) = zone.start_idx {
            fields.push(format!("start_idx={}i", v));
        }
        if let Some(v) = zone.end_idx {
            fields.push(format!("end_idx={}i", v));
        }
        // --- END OF FIXED SECTION ---

        if let Some(v) = zone.zone_low { fields.push(format!("zone_low={}", v)); }
        if let Some(v) = zone.zone_high { fields.push(format!("zone_high={}", v)); }
        if let Some(v) = &zone.start_time { fields.push(format!("start_time_rfc3339=\"{}\"", v.escape_default())); }
        if let Some(v) = &zone.end_time { fields.push(format!("end_time_rfc3339=\"{}\"", v.escape_default())); }
        if let Some(v) = &zone.detection_method { fields.push(format!("detection_method=\"{}\"", v.escape_default())); }
        if let Some(v) = zone.fifty_percent_line { fields.push(format!("fifty_percent_line={}", v)); }
        
        // is_active and bars_active are crucial for lifecycle
        fields.push(format!("is_active={}i", if zone.is_active { 1 } else { 0 }));
        if let Some(v) = zone.bars_active { // Only write if Some, generator should give Some(0) for new
            fields.push(format!("bars_active={}i", v));
        } else { // Should not happen if enricher always sets it
            warn!("[GENERATOR_WRITE] Zone {:?} missing bars_active, writing as 0i", zone.zone_id);
            fields.push("bars_active=0i".to_string());
        }
        
        fields.push(format!("touch_count={}i", zone.touch_count.unwrap_or(0)));
        fields.push(format!("strength_score={}", zone.strength_score.unwrap_or(100.0)));
        if let Some(v) = zone.extended { fields.push(format!("extended={}", v)); }
        if let Some(v) = zone.extension_percent { fields.push(format!("extension_percent={}", v)); }
        
        let candles_json = match serde_json::to_string(&zone.formation_candles) {
            Ok(s) => s, Err(e) => { error!("[GENERATOR_WRITE] Failed to serialize formation_candles for zone {:?}: {}", zone.zone_id, e); "[]".to_string() }
        };
        fields.push(format!("formation_candles_json=\"{}\"", candles_json.replace('"', "\\\"")));

        // Your debug log for specific zones can remain here if useful
        if let Some(ref zid) = zone.zone_id {
            if zid == "287ede7e9d142ade" || (zone.symbol.as_deref() == Some("AUDJPY_SB") && zone.timeframe.as_deref() == Some("1h") && zone.start_time.as_deref() == Some("2025-05-26T21:00:00Z") && zone.zone_id.as_deref() == Some("578ac4cac3bf62b6")) {
                 log::error!(
                    "[GENERATOR_WRITE_FIELDS_DEBUG] For target zone ID {:?}, fields collected for InfluxDB line: {:?}",
                    zone.zone_id, fields
                );
            }
        }

        if !fields.is_empty() {
            line_protocol_body.push(' '); line_protocol_body.push_str(&fields.join(","));
            line_protocol_body.push(' '); line_protocol_body.push_str(&timestamp_nanos_str);
            line_protocol_body.push('\n');
        } else {
            warn!("[GENERATOR_WRITE] Skipping zone point with no fields (should not happen if zone_id is present): {:?}", zone.zone_id);
        }
    }

    if line_protocol_body.is_empty() {
        info!("[GENERATOR_WRITE] No valid points generated for batch write for {}/{}/{}", symbol, timeframe, pattern);
        return Ok(());
    }

    let write_url = format!("{}/api/v2/write?org={}&bucket={}&precision=ns", influx_host, influx_org, write_bucket);
    info!("[GENERATOR_WRITE] Writing batch of ~{} points ({} bytes) for {}/{}/{} to bucket '{}'",
        line_protocol_body.lines().count(), line_protocol_body.len(), symbol, timeframe, pattern, write_bucket);
    
    // Retry logic for HTTP POST
    let mut attempts = 0; 
    let max_attempts = 3;
    while attempts < max_attempts {
        let response_result = http_client.post(&write_url)
            .bearer_auth(influx_token)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(line_protocol_body.clone())
            .send()
            .await;
        
        match response_result {
            Ok(resp) => {
                if resp.status().is_success() { 
                    info!("[GENERATOR_WRITE] Batch write successful for {}/{}/{}", symbol, timeframe, pattern); 
                    return Ok(()); 
                } else {
                    let status = resp.status(); 
                    let body = resp.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
                    attempts += 1; 
                    error!("[GENERATOR_WRITE] InfluxDB write attempt {}/{} for {}/{}/{} failed with status {}: {}", attempts, max_attempts, symbol, timeframe, pattern, status, body);
                    if attempts >= max_attempts { 
                        return Err(format!("InfluxDB write failed after {} attempts for {}/{}/{} (status {}): {}", max_attempts, symbol, timeframe, pattern, status, body).into()); 
                    }
                    tokio::time::sleep(tokio::time::Duration::from_secs(2u64.pow(attempts))).await;
                }
            }
            Err(e) => {
                attempts += 1; 
                error!("[GENERATOR_WRITE] HTTP request error during write attempt {}/{} for {}/{}/{}: {}", attempts, max_attempts, symbol, timeframe, pattern, e);
                if attempts >= max_attempts { 
                    return Err(format!("HTTP request failed after {} attempts: {}", max_attempts, e).into()); 
                } 
                tokio::time::sleep(tokio::time::Duration::from_secs(2u64.pow(attempts))).await;
            }
        }
    }
    Err(format!("Max write attempts exceeded for {}/{}/{} (logic error in retry loop)", symbol, timeframe, pattern).into())
}

// --- NEW: Helper function to fetch zone_ids in range ---
async fn fetch_zone_ids_in_range(
    http_client: &Client,
    host: &str,
    org: &str,
    token: &str,
    bucket: &str,         // This should be the bucket where zones are written (e.g., GENERATOR_WRITE_BUCKET)
    measurement: &str,    // The measurement name for zones
    start_range: &str,    // e.g., "-90d" or an RFC3339 timestamp from generator's window
    end_range: &str,      // e.g., "now()" or an RFC3339 timestamp from generator's window
) -> Result<HashSet<String>, BoxedError> {
    // This query targets the `zone_id` FIELD.
    let flux_query = format!(
        r#"from(bucket: "{}")
           |> range(start: {}, stop: {})
           |> filter(fn: (r) => r._measurement == "{}")
           |> filter(fn: (r) => r._field == "zone_id")
           |> keep(columns: ["_value"])
           |> distinct(column: "_value")
           |> yield(name: "distinct_zone_ids")"#,
        bucket, start_range, end_range, measurement
    );

    debug!("[GENERATOR_FETCH_IDS] Querying for existing zone IDs in range {} to {}. Query: {}", start_range, end_range, flux_query);
    let query_url = format!("{}/api/v2/query?org={}", host, org);

    let response = http_client
        .post(&query_url)
        .bearer_auth(token)
        .header("Accept", "application/csv")
        .header("Content-Type", "application/json")
        .json(&json!({ "query": flux_query, "type": "flux" }))
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let err_body = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        error!("[GENERATOR_FETCH_IDS] Failed to fetch zone IDs (status {}): {}", status, err_body);
        return Err(format!("InfluxDB query for zone_ids failed (status {}): {}", status, err_body).into());
    }

    let csv_text = response.text().await?;
    let mut ids = HashSet::new();

    // Check if there's more than just a header or empty lines.
    if csv_text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).count() > 1 {
        let mut rdr = ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .comment(Some(b'#'))
            .from_reader(csv_text.as_bytes());

        let headers = match rdr.headers() {
            Ok(h) => h.clone(),
            Err(e) => {
                warn!("[GENERATOR_FETCH_IDS] Failed to read CSV headers for zone IDs: {}. CSV text was: '{}'", e, csv_text.chars().take(500).collect::<String>());
                return Ok(ids);
            }
        };
        
        if let Some(value_col_idx) = headers.iter().position(|h_val| h_val == "_value") {
            for result in rdr.records() {
                match result {
                    Ok(record) => {
                        if let Some(id_val_str) = record.get(value_col_idx) {
                            if !id_val_str.is_empty() {
                                ids.insert(id_val_str.trim().to_string());
                            }
                        }
                    }
                    Err(e) => {
                        warn!("[GENERATOR_FETCH_IDS] CSV record error while parsing distinct zone IDs: {}", e);
                    }
                }
            }
        } else {
            info!("[GENERATOR_FETCH_IDS] '_value' column not found in distinct zone_id query CSV result. This might be normal if no zone_ids were found matching the criteria. Headers: {:?}. CSV (first 500 chars): '{}'", headers, csv_text.chars().take(500).collect::<String>());
        }
    } else {
        info!("[GENERATOR_FETCH_IDS] CSV response for distinct zone_id query contained no data rows. Assuming no existing IDs found.");
    }

    info!("[GENERATOR_FETCH_IDS] Pre-fetched {} distinct existing zone IDs in the given range.", ids.len());
    Ok(ids)
}

// --- MODIFIED: Main orchestrator with existing zone ID pre-fetching ---
pub async fn run_zone_generation(
    is_periodic_run: bool,
    target_zone_measurement_override: Option<&str>,
) -> Result<(), BoxedError> {
    let run_type = if is_periodic_run { "PERIODIC" } else { "FULL" };
    log::info!("[GENERATOR_RUN] Starting {} zone generation process...", run_type);

    // --- Environment Variable Reading & Initial Setup ---
    log::info!("[GENERATOR_RUN] Reading environment variables for generator run...");
    for var_name in [
        "INFLUXDB_HOST", "INFLUXDB_ORG", "INFLUXDB_TOKEN", "INFLUXDB_BUCKET",
        "GENERATOR_WRITE_BUCKET", "GENERATOR_TRENDBAR_MEASUREMENT", "GENERATOR_ZONE_MEASUREMENT",
        "GENERATOR_START_TIME", "GENERATOR_END_TIME", "GENERATOR_PATTERNS",
        "GENERATOR_PERIODIC_LOOKBACK", "GENERATOR_CHUNK_SIZE", "GENERATOR_EXCLUDE_TIMEFRAMES",
    ] {
        match env::var(var_name) {
            Ok(val) => info!("[GENERATOR_RUN_ENV] {} = {}", var_name, val),
            Err(_) => warn!("[GENERATOR_RUN_ENV] {} is not set (will use default or fail if required by subsequent logic)", var_name),
        }
    }

    let host = env::var("INFLUXDB_HOST").map_err(|e| format!("INFLUXDB_HOST: {}", e))?;
    let org = env::var("INFLUXDB_ORG").map_err(|e| format!("INFLUXDB_ORG: {}", e))?;
    let token = env::var("INFLUXDB_TOKEN").map_err(|e| format!("INFLUXDB_TOKEN: {}", e))?;
    let read_bucket = env::var("INFLUXDB_BUCKET").map_err(|e| format!("INFLUXDB_BUCKET: {}", e))?;
    let write_bucket = env::var("GENERATOR_WRITE_BUCKET").map_err(|e| format!("GENERATOR_WRITE_BUCKET: {}", e))?;
    let trendbar_measurement = env::var("GENERATOR_TRENDBAR_MEASUREMENT").map_err(|e| format!("GENERATOR_TRENDBAR_MEASUREMENT: {}", e))?;
    let patterns_str = env::var("GENERATOR_PATTERNS").map_err(|e| format!("GENERATOR_PATTERNS: {}", e))?;
    let patterns: Vec<String> = patterns_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if patterns.is_empty() { return Err("No patterns specified in GENERATOR_PATTERNS env var.".into()); }
    
    let chunk_size_str = env::var("GENERATOR_CHUNK_SIZE").unwrap_or_else(|_| "4".to_string());
    let chunk_size = chunk_size_str.parse::<usize>().map_err(|e| format!("Invalid GENERATOR_CHUNK_SIZE '{}': {}", chunk_size_str, e))?;

    let zone_measurement_to_use = match target_zone_measurement_override {
        Some(name) => { info!("[GENERATOR_RUN] Using overridden target zone measurement: '{}'", name); name.to_string() }
        None => env::var("GENERATOR_ZONE_MEASUREMENT").map_err(|e| format!("GENERATOR_ZONE_MEASUREMENT: {}", e))?,
    };

    let (query_start_time, query_end_time) = if is_periodic_run {
        let lookback = env::var("GENERATOR_PERIODIC_LOOKBACK").unwrap_or_else(|_| "2h".to_string());
        info!("[GENERATOR_RUN_{}] Using periodic lookback: {}", run_type, lookback); 
        (format!("-{}", lookback), "now()".to_string())
    } else {
        let start_f = env::var("GENERATOR_START_TIME").map_err(|e| format!("GENERATOR_START_TIME: {}", e))?;
        let end_f = env::var("GENERATOR_END_TIME").map_err(|e| format!("GENERATOR_END_TIME: {}", e))?;
        info!("[GENERATOR_RUN_{}] Using fixed times: Start='{}', End='{}'", run_type, start_f, end_f); 
        (start_f, end_f)
    };

    let http_client = Client::new();

    // --- NEW: Pre-fetch existing zone IDs for duplicate checking ---
    let existing_zone_ids_in_processing_range: HashSet<String> = if !is_periodic_run {
        info!("[GENERATOR_RUN] FULL RUN: Pre-fetching existing zone IDs in range {} to {} for duplicate checking.", query_start_time, query_end_time);
        fetch_zone_ids_in_range(&http_client, &host, &org, &token, &write_bucket, &zone_measurement_to_use, &query_start_time, &query_end_time)
            .await.unwrap_or_else(|e| {
                error!("[GENERATOR_RUN] Failed to pre-fetch existing zone IDs for full run: {}. Proceeding without duplicate check.", e);
                HashSet::new()
        })
    } else {
        let periodic_prefetch_start = "-24h";
        info!("[GENERATOR_RUN] PERIODIC RUN: Pre-fetching existing zone IDs in range {} to now() for duplicate checking.", periodic_prefetch_start);
        fetch_zone_ids_in_range(&http_client, &host, &org, &token, &write_bucket, &zone_measurement_to_use, periodic_prefetch_start, "now()")
            .await.unwrap_or_else(|e| {
                error!("[GENERATOR_RUN] Failed to pre-fetch existing zone IDs for periodic run: {}. Proceeding without duplicate check.", e);
                HashSet::new()
        })
    };
    info!("[GENERATOR_RUN] Pre-fetched {} existing zone IDs for duplicate checking in current scope.", existing_zone_ids_in_processing_range.len());

    // --- Symbol and Timeframe Discovery ---
    let discovered_symbols_from_fetch = fetch_distinct_tag_values(&http_client, &host, &org, &token, &read_bucket, &trendbar_measurement, "symbol").await?;
    if discovered_symbols_from_fetch.is_empty() { 
        info!("[GENERATOR_RUN] No symbols found in trendbar data. Not proceeding."); 
        return Ok(()); 
    }
    log::info!("[GENERATOR_RUN] Initially discovered {} symbols: {:?}", discovered_symbols_from_fetch.len(), discovered_symbols_from_fetch);
    
    let filtered_discovered_symbols: Vec<String> = discovered_symbols_from_fetch.into_iter().filter(|s| s.ends_with("_SB")).collect();
    if filtered_discovered_symbols.is_empty() { 
        log::warn!("[GENERATOR_RUN] After filtering, no _SB symbols remain. Not proceeding."); 
        return Ok(()); 
    }
    log::info!("[GENERATOR_RUN] Will process {} _SB symbols: {:?}", filtered_discovered_symbols.len(), filtered_discovered_symbols);
    
    let all_timeframes_from_fetch = fetch_distinct_tag_values(&http_client, &host, &org, &token, &read_bucket, &trendbar_measurement, "timeframe").await?;
    let excluded_timeframes_str = env::var("GENERATOR_EXCLUDE_TIMEFRAMES").unwrap_or_default();
    let excluded_timeframes: Vec<String> = excluded_timeframes_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    log::info!("[GENERATOR_RUN] Excluded timeframes from ENV: {:?}", excluded_timeframes);
    
    let allowed_timeframes: Vec<String> = all_timeframes_from_fetch.into_iter().filter(|tf| {
        !excluded_timeframes.contains(tf) && matches!(tf.as_str(), "1m" | "5m" | "15m" | "30m" | "1h" | "4h" | "1d")
    }).collect();
    if allowed_timeframes.is_empty() { 
        return Err("No supported timeframes found after filtering.".into()); 
    }
    log::info!("[GENERATOR_RUN] Will process symbols: {:?}", filtered_discovered_symbols);
    log::info!("[GENERATOR_RUN] Will process timeframes: {:?}", allowed_timeframes);

    // --- Processing with concurrency ---
    let mut all_combinations = Vec::new();
    for symbol_val in filtered_discovered_symbols.iter() {
        for timeframe_val in allowed_timeframes.iter() {
            for pattern_name_val in patterns.iter() {
                all_combinations.push((symbol_val.clone(), timeframe_val.clone(), pattern_name_val.clone()));
            }
        }
    }

    let total_combinations = all_combinations.len();
    info!("[GENERATOR_RUN] Total symbol/timeframe/pattern combinations to process: {}", total_combinations);
    let mut total_processed_combinations = 0;
    let mut total_errors_in_processing = 0;

    for (chunk_idx, chunk) in all_combinations.chunks(chunk_size).enumerate() {
        info!("[GENERATOR_RUN_{}] Processing chunk {}/{} ({} combinations)",
            run_type, chunk_idx + 1, (total_combinations + chunk_size - 1) / chunk_size, chunk.len());
        
        let mut futures = Vec::new();
        for (symbol_str, timeframe_str, pattern_name_str) in chunk {
            let q_start_for_task = query_start_time.clone();
            let q_end_for_task = query_end_time.clone();
            let wb_for_task = write_bucket.clone();
            let tzm_for_task = zone_measurement_to_use.clone();
            let client_for_task = http_client.clone();
            let h_for_task = host.clone();
            let o_for_task = org.clone();
            let t_for_task = token.clone();
            let existing_ids_for_task = existing_zone_ids_in_processing_range.clone(); // Pass existing IDs
            let sym_owned = symbol_str.to_string();
            let tf_owned = timeframe_str.to_string();
            let pat_owned = pattern_name_str.to_string();

            let future = async move {
                // Call the renamed function with existing_zone_ids parameter
                process_symbol_timeframe_pattern_and_collect_ids(
                    &sym_owned, &tf_owned, &pat_owned,
                    &q_start_for_task, &q_end_for_task,
                    &wb_for_task, &tzm_for_task,
                    &client_for_task,
                    &h_for_task, &o_for_task, &t_for_task,
                    &existing_ids_for_task, // Pass existing IDs
                ).await
            };
            futures.push(future);
        }

        let task_results = futures::future::join_all(futures).await;
        for (i, result_item) in task_results.into_iter().enumerate() {
            let (symbol, timeframe, pattern_name) = &chunk[i];
            match result_item {
                Ok(newly_written_ids) => {
                    info!("[GENERATOR_TASK_RESULT] {}_{}_{}: Success. Created {} new zones.", 
                          symbol, timeframe, pattern_name, newly_written_ids.len());
                }
                Err(e) => {
                    error!("[GENERATOR_TASK_ERROR] {}_{}_{}: Task processing failed: {}", 
                           symbol, timeframe, pattern_name, e);
                    total_errors_in_processing += 1;
                }
            }
            total_processed_combinations += 1;
        }
        info!("[GENERATOR_RUN] Chunk {}/{} processed. Overall progress: {}/{} ({:.1}%)",
            chunk_idx + 1, (total_combinations + chunk_size - 1) / chunk_size,
            total_processed_combinations, total_combinations,
            (total_processed_combinations as f64 / total_combinations as f64) * 100.0
        );
    }

    info!("[GENERATOR_RUN_{}] Finished {} run. Total errors during processing: {}", run_type, run_type, total_errors_in_processing);
    if total_errors_in_processing > 0 {
        Err(format!("Zone generation finished with {} task errors.", total_errors_in_processing).into())
    } else {
        info!("[GENERATOR_RUN] Zone generation process completed successfully with no task errors.");
        Ok(())
    }
}

// --- RENAMED AND MODIFIED: Process function with duplicate checking ---
async fn process_symbol_timeframe_pattern_and_collect_ids(
    symbol: &str,
    timeframe: &str,
    pattern_name: &str,
    fetch_start_time: &str,
    fetch_end_time: &str,
    write_bucket: &str,
    target_zone_measurement: &str,
    http_client: &Client,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    existing_zone_ids: &HashSet<String>, // NEW PARAMETER
) -> Result<Vec<String>, String> { // CHANGED RETURN TYPE

    log::info!(
        "[PSP_COLLECT_IDS] Processing {}/{} for pattern '{}'. Will check against {} existing IDs.",
        symbol, timeframe, pattern_name, existing_zone_ids.len()
    );

    let chart_query = ChartQuery {
        symbol: symbol.to_string(),
        timeframe: timeframe.to_string(),
        start_time: fetch_start_time.to_string(),
        end_time: fetch_end_time.to_string(),
        pattern: pattern_name.to_string(),
        enable_trading: None, 
        lot_size: None, 
        stop_loss_pips: None, 
        take_profit_pips: None, 
        enable_trailing_stop: None,
        max_touch_count: None,
    };

    // Call _fetch_and_detect_core and map its error to String
    let (all_fetched_candles, _recognizer, recognizer_output_value) = 
        detect::_fetch_and_detect_core(&chart_query, &format!("generator_{}/{}", symbol, timeframe))
            .await
            .map_err(|e: CoreError| {
                let err_msg = format!("Core fetch/detect error for {}/{}: {}", symbol, timeframe, e);
                error!("[PSP_COLLECT_IDS] {}", err_msg);
                err_msg
            })?;

    let candle_count_total = all_fetched_candles.len();
    if candle_count_total == 0 {
        info!("[PSP_COLLECT_IDS] No candle data for {}/{}/{}. Skipping.", symbol, timeframe, pattern_name);
        return Ok(Vec::new());
    }
    info!("[PSP_COLLECT_IDS] Fetched {} candles for {}/{}.", candle_count_total, symbol, timeframe);

    let mut new_zones_to_write_to_db: Vec<StoredZone> = Vec::new();
    let price_data = recognizer_output_value.get("data").and_then(|d| d.get("price"));

    if price_data.is_none() {
        warn!("[PSP_COLLECT_IDS] 'data.price' missing in recognizer output for {}/{}/{}. No zones to process.",
            symbol, timeframe, pattern_name);
        return Ok(Vec::new());
    }

    let last_candle_timestamp = all_fetched_candles.last().map(|c| c.time.clone());

    for zone_kind_key in ["supply_zones", "demand_zones"] {
        let is_supply = zone_kind_key == "supply_zones";
        if let Some(zones_array_val) = price_data
            .and_then(|p| p.get(zone_kind_key))
            .and_then(|zk| zk.get("zones"))
            .and_then(|z| z.as_array())
        {
            if zones_array_val.is_empty() {
                trace!("[PSP_COLLECT_IDS] No raw {} found for {}/{}", zone_kind_key, symbol, timeframe);
                continue;
            }
            debug!("[PSP_COLLECT_IDS] Processing {} raw {} from recognizer for {}/{}.",
                zones_array_val.len(), zone_kind_key, symbol, timeframe);

            for raw_zone_json in zones_array_val {
                write_raw_zone_to_text_log(symbol, timeframe, zone_kind_key, raw_zone_json);
            }

            // enrich_zones_with_activity should return Result<Vec<EnrichedZone>, String>
            let enriched_zones_vec = enrich_zones_with_activity(
                Some(zones_array_val), is_supply, &all_fetched_candles,
                candle_count_total, last_candle_timestamp.clone(),
                symbol, timeframe,
            ).map_err(|e_str| {
                error!("[PSP_COLLECT_IDS] Enrichment failed for {} {}/{}: {}", zone_kind_key, symbol, timeframe, e_str);
                e_str
            })?;

            for enriched_zone_item in enriched_zones_vec {
                if let Some(ref zone_id) = enriched_zone_item.zone_id {
                    debug!("[GENERATOR_DEBUG] Zone ID: {}", zone_id);
                    debug!("[GENERATOR_DEBUG]   Symbol/TF: {}/{}", symbol, timeframe);
                    debug!("[GENERATOR_DEBUG]   Active: {}", enriched_zone_item.is_active);
                    debug!("[GENERATOR_DEBUG]   Touch Count: {:?}", enriched_zone_item.touch_count);
                    debug!("[GENERATOR_DEBUG]   Bars Active: {:?}", enriched_zone_item.bars_active);
                    debug!("[GENERATOR_DEBUG]   Start Time: {:?}", enriched_zone_item.start_time);
                    debug!("[GENERATOR_DEBUG]   End Time: {:?}", enriched_zone_item.end_time);
                    debug!("[GENERATOR_DEBUG]   Start Idx: {:?}", enriched_zone_item.start_idx);
                    debug!("[GENERATOR_DEBUG]   End Idx: {:?}", enriched_zone_item.end_idx);
                    debug!("[GENERATOR_DEBUG]   Zone Type: {:?}", enriched_zone_item.zone_type);
                    debug!("[GENERATOR_DEBUG]   Invalidation Time: {:?}", enriched_zone_item.invalidation_time);
                    debug!("[GENERATOR_DEBUG]   ---");
                }
                // --- CHECK IF ZONE ALREADY EXISTS ---
                if let Some(ref current_zone_id) = enriched_zone_item.zone_id {
                    if existing_zone_ids.contains(current_zone_id) {
                        trace!("[PSP_COLLECT_IDS] Zone ID {} (from {}/{}) already exists (found in pre-fetched set). Skipping initial write by generator.",
                            current_zone_id, symbol, timeframe);
                        continue; // Skip this zone, it's not new
                    }
                } else {
                    warn!("[PSP_COLLECT_IDS] Enriched zone for {}/{} is missing a zone_id. Cannot check for existence or write.", symbol, timeframe);
                    continue;
                }

                // If we are here, the zone is new
                let stored_zone = StoredZone {
                    zone_id: enriched_zone_item.zone_id.clone(),
                    start_time: enriched_zone_item.start_time,
                    end_time: enriched_zone_item.end_time,
                    zone_high: enriched_zone_item.zone_high,
                    zone_low: enriched_zone_item.zone_low,
                    fifty_percent_line: enriched_zone_item.fifty_percent_line,
                    detection_method: enriched_zone_item.detection_method,
                    start_idx: enriched_zone_item.start_idx,
                    end_idx: enriched_zone_item.end_idx,
                    is_active: enriched_zone_item.is_active,
                    bars_active: enriched_zone_item.bars_active,
                    touch_count: enriched_zone_item.touch_count,
                    strength_score: enriched_zone_item.strength_score,
                    formation_candles: extract_formation_candles(
                        &all_fetched_candles,
                        enriched_zone_item.start_idx,
                        enriched_zone_item.end_idx,
                    ),
                    extended: enriched_zone_item.extended,
                    extension_percent: enriched_zone_item.extension_percent,
                    symbol: Some(symbol.to_string()),
                    timeframe: Some(timeframe.to_string()),
                    zone_type: enriched_zone_item.zone_type,
                    pattern: Some(pattern_name.to_string()),
                };

                // Debug log for specific target zones
                if let Some(ref zid) = stored_zone.zone_id {
                     if zid == "287ede7e9d142ade" || (stored_zone.symbol.as_deref() == Some("AUDJPY_SB") && stored_zone.timeframe.as_deref() == Some("1h") && stored_zone.start_time.as_deref() == Some("2025-05-26T21:00:00Z") && stored_zone.zone_id.as_deref() == Some("578ac4cac3bf62b6")) {
                        log::error!(
                            "[GENERATOR_PRE_WRITE_NEW_ZONE_DEBUG] Target NEW zone candidate for {}/{}. ID: {:?}, start_idx: {:?}, end_idx: {:?}, bars_active: {:?}, is_active: {}",
                            symbol, timeframe,
                            stored_zone.zone_id, stored_zone.start_idx, stored_zone.end_idx,
                            stored_zone.bars_active, stored_zone.is_active
                        );
                    }
                }
                new_zones_to_write_to_db.push(stored_zone);
            }
        }
    }

    let mut newly_written_zone_ids: Vec<String> = Vec::new();
    if !new_zones_to_write_to_db.is_empty() {
        info!("[PSP_COLLECT_IDS] Attempting to write {} NEW zones for {}/{}/{} to DB.",
            new_zones_to_write_to_db.len(), symbol, timeframe, pattern_name);

        // write_zones_batch should return Result<(), BoxedError>
        write_zones_batch(
            http_client, influx_host, influx_org, influx_token,
            write_bucket, target_zone_measurement,
            symbol, timeframe, pattern_name,
            &new_zones_to_write_to_db,
        ).await.map_err(|e| {
            let err_msg = format!("Failed to write new zones batch for {}/{}/{}: {}", symbol, timeframe, pattern_name, e);
            error!("[PSP_COLLECT_IDS] {}", err_msg);
            err_msg
        })?;

        // If write was successful, collect the IDs
        for zone in new_zones_to_write_to_db {
            if let Some(id) = zone.zone_id {
                newly_written_zone_ids.push(id);
            }
        }
        info!("[PSP_COLLECT_IDS] Successfully wrote {} NEW zones for {}/{}/{}.",
            newly_written_zone_ids.len(), symbol, timeframe, pattern_name);
    } else {
        info!("[PSP_COLLECT_IDS] No NEW zones to store for {}/{}/{}.", symbol, timeframe, pattern_name);
    }

    Ok(newly_written_zone_ids)
}