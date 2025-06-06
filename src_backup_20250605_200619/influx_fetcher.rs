// src/influx_fetcher.rs

use crate::api::detect::CandleData; // Assuming CandleData is defined in src/detect.rs
use log::{info, error, warn};
use dotenv::dotenv;
use reqwest;
use csv::ReaderBuilder;
use std::io::Cursor;
use std::error::Error; // Use standard Error trait
use actix_web::{HttpResponse, Responder}; // Needed for the test handler

// --- Core function to fetch and parse data ---
// Returns Vec<CandleData> on success, or an Error
pub async fn query_influxdb_hardcoded() -> Result<Vec<CandleData>, Box<dyn Error>> {
    info!("[influx_fetcher::query_influxdb_hardcoded] Starting hardcoded query...");
    dotenv().ok();

    // --- Hardcoded Parameters ---
    let start_time = "2025-04-03T16:00:00Z";
    let end_time = "2025-04-04T16:00:00Z";
    let symbol = "EURUSD_SB";
    let timeframe = "30m";
    // --- End Hardcoded Parameters ---

    // --- Database Connection Details (from env) ---
    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG")?; // Propagate error if missing
    let token = std::env::var("INFLUXDB_TOKEN")?; // Propagate error if missing
    let bucket = std::env::var("INFLUXDB_BUCKET")?; // Propagate error if missing

    // --- Build Flux Query ---
    let flux_query = format!(
         r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r["_measurement"] == "trendbar") |> filter(fn: (r) => r["symbol"] == "{}") |> filter(fn: (r) => r["timeframe"] == "{}") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
        bucket, start_time, end_time, symbol, timeframe // Use hardcoded values
    );
    info!("[influx_fetcher::query_influxdb_hardcoded] Sending Query:\n{}", flux_query);

    // --- Execute Query ---
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response_text = client.post(&url).bearer_auth(&token).json(&serde_json::json!({"query": flux_query, "type": "flux"})).send().await?.text().await?;
    info!("[influx_fetcher::query_influxdb_hardcoded] Response length: {}", response_text.len());

    // --- Parse Response ---
    let mut candles = Vec::<CandleData>::new();
    if !response_text.trim().is_empty() {
        let cursor = Cursor::new(response_text.as_bytes());
        let mut rdr = ReaderBuilder::new().has_headers(true).flexible(true).from_reader(cursor);
        let headers = rdr.headers()?.clone(); // Propagate CSV errors
        let get_idx = |name: &str| headers.iter().position(|h| h == name);

        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) =
            (get_idx("_time"), get_idx("open"), get_idx("high"), get_idx("low"), get_idx("close"), get_idx("volume"))
        {
            info!("[influx_fetcher::query_influxdb_hardcoded] Found required headers.");
            for result in rdr.records() {
                 match result {
                     Ok(record) => {
                         let parse_f64 = |idx: usize| record.get(idx).and_then(|v| v.parse::<f64>().ok());
                         let parse_u32 = |idx: usize| record.get(idx).and_then(|v| v.parse::<u32>().ok());
                         if record.len() > t_idx && record.len() > o_idx && record.len() > h_idx && record.len() > l_idx && record.len() > c_idx && record.len() > v_idx {
                             let time = record.get(t_idx).unwrap_or("").to_string();
                             if !time.is_empty() { candles.push(CandleData { time, open: parse_f64(o_idx).unwrap_or(0.0), high: parse_f64(h_idx).unwrap_or(0.0), low: parse_f64(l_idx).unwrap_or(0.0), close: parse_f64(c_idx).unwrap_or(0.0), volume: parse_u32(v_idx).unwrap_or(0) }); }
                         }
                     },
                     Err(e) => warn!("[influx_fetcher::query_influxdb_hardcoded] CSV record parse error: {}", e), // Log error but continue
                 }
             }
        } else {
            // Header mismatch is a more critical error
            let error_msg = "CSV header mismatch in InfluxDB response.";
            error!("[influx_fetcher::query_influxdb_hardcoded] {}", error_msg);
            return Err(error_msg.into()); // Convert &str to Box<dyn Error>
        }
    } else {
        info!("[influx_fetcher::query_influxdb_hardcoded] InfluxDB response text was empty.");
    }

    info!("[influx_fetcher::query_influxdb_hardcoded] Parsed {} candles.", candles.len());
    Ok(candles)
}

// --- Function to call the query function and log result (simulates direct call) ---
pub async fn log_hardcoded_query_results() {
    info!("[influx_fetcher::log_hardcoded_query_results] Attempting direct call...");
    match query_influxdb_hardcoded().await {
        Ok(candles) => {
            info!("[influx_fetcher::log_hardcoded_query_results] Direct call SUCCESS: Received {} candles.", candles.len());
            // Optionally log first few candles if needed for deeper debug:
            // for (i, candle) in candles.iter().take(3).enumerate() {
            //     info!("  Direct Call - Candle {}: Time={}, Close={}", i, candle.time, candle.close);
            // }
        }
        Err(e) => {
            error!("[influx_fetcher::log_hardcoded_query_results] Direct call FAILED: {}", e);
        }
    }
}

// --- Actix Handler for the test endpoint ---
// Calls the same query function and logs result. Returns simple HTTP response.
pub async fn test_data_request_handler() -> impl Responder {
     info!("[influx_fetcher::test_data_request_handler] Endpoint /testDataRequest hit.");
     match query_influxdb_hardcoded().await {
         Ok(candles) => {
             info!("[influx_fetcher::test_data_request_handler] Endpoint call SUCCESS: Received {} candles.", candles.len());
             // Optionally log candles here too if needed
             HttpResponse::Ok().body(format!("Test query successful via endpoint. Received {} candles. Check server logs.", candles.len()))
         }
         Err(e) => {
             error!("[influx_fetcher::test_data_request_handler] Endpoint call FAILED: {}", e);
             HttpResponse::InternalServerError().body(format!("Test query failed via endpoint: {}. Check server logs.", e))
         }
     }
}