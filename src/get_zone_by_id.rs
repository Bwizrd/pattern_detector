// src/get_zone_by_id.rs
use crate::detect::CandleData;
use crate::errors::ServiceError; // Import your custom error
use crate::types::StoredZone; // Import your StoredZone struct path and CandleData
use actix_web::{get, web, HttpResponse, Responder};
use csv;
use log; // <<<--- ADD THIS IMPORT
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json;
use std::env;
// --- Query Parameters ---
#[derive(Deserialize, Debug)]
pub struct ZoneQuery {
    zone_id: String,
}

// --- InfluxDB Config (Load once, maybe in AppState) ---
struct InfluxConfig {
    host: String,
    org: String,
    token: String,
    zone_bucket: String,
    zone_measurement: String,
}

impl InfluxConfig {
    fn load() -> Result<Self, ServiceError> {
        Ok(Self {
            host: env::var("INFLUXDB_HOST")?,
            org: env::var("INFLUXDB_ORG")?,
            token: env::var("INFLUXDB_TOKEN")?,
            // Use the bucket where ZONES are written
            zone_bucket: env::var("GENERATOR_WRITE_BUCKET")?,
            zone_measurement: env::var("GENERATOR_ZONE_MEASUREMENT")?,
        })
    }
}

// #[get("/zone")] // Define the route and method
pub async fn get_zone_by_id(
    query: web::Query<ZoneQuery>,
    // http_client: web::Data<Client>, // Get client from AppState if shared
) -> Result<HttpResponse, ServiceError> {
    log::info!("Received request for zone_id: {}", query.zone_id);

    // Load config (Consider loading once and storing in AppState)
    let config = InfluxConfig::load()?;

    // Create HTTP client (Consider creating once and storing in AppState)
    let http_client = Client::new();

    // --- Construct the Flux Query ---
    let flux_query = format!(
        r#"from(bucket: "{}")
          |> range(start: 0)
          |> filter(fn: (r) => r._measurement == "{}")
          |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value")
          |> filter(fn: (r) => r.zone_id == "{}")
          |> limit(n: 1)
        "#,
        config.zone_bucket, config.zone_measurement, query.zone_id
    );

    log::debug!("Executing Flux query: {}", flux_query);

    let query_url = format!("{}/api/v2/query?org={}", config.host, config.org);

    // --- Send Request to InfluxDB ---
    let response = http_client
        .post(&query_url)
        .bearer_auth(&config.token)
        .header("Accept", "application/csv") // Request CSV format
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ // Use serde_json::json! macro
            "query": flux_query,
            "type": "flux"
        }))
        .send()
        .await?;

    // --- Handle Response ---
    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error body".to_string());
        log::error!(
            "InfluxDB query failed. Status: {}, Body: {}",
            status,
            error_body
        );
        return Err(ServiceError::InfluxQueryError(format!(
            "InfluxDB returned status {}: {}",
            status, error_body
        )));
    }

    let csv_data = response.text().await?;
    log::debug!("Received CSV data:\n{}", csv_data);

    // Add more detailed logging for row-by-row analysis
    for (i, line) in csv_data.lines().enumerate() {
        log::debug!("CSV row {}: {}", i, line);
    }

    // --- REPLACE EVERYTHING BELOW THIS LINE WITH THE NEW CODE ---

    // Instead of direct deserialization, do a manual mapping
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(csv_data.as_bytes());

    let headers = match reader.headers() {
        Ok(h) => {
            let hclone = h.clone();
            log::debug!("CSV headers: {:?}", hclone);
            log::debug!("Header count: {}", hclone.len());
            for (i, h) in hclone.iter().enumerate() {
                log::debug!("  Header {}: '{}'", i, h);
            }
            hclone
        }
        Err(e) => {
            log::error!("Failed to read CSV headers: {}", e);
            return Err(ServiceError::CsvError(e));
        }
    };

    // Create a record reader
    let mut record_reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(csv_data.as_bytes());
    let _ = record_reader.headers(); // Skip header row

    if let Some(record_result) = record_reader.records().next() {
        match record_result {
            Ok(record) => {
                // Create a new zone and populate it manually
                let mut zone = StoredZone::default();

                // Map fields from CSV to zone struct using header positions
                for (i, header) in headers.iter().enumerate() {
                    if i >= record.len() {
                        continue; // Skip if record doesn't have enough fields
                    }

                    let value = &record[i];
                    match header {
                        "zone_id" => zone.zone_id = Some(value.to_string()),
                        "symbol" => zone.symbol = Some(value.to_string()),
                        "timeframe" => zone.timeframe = Some(value.to_string()),
                        "zone_type" => zone.zone_type = Some(value.to_string()),
                        "start_time_rfc3339" => zone.start_time = Some(value.to_string()),
                        "end_time_rfc3339" => zone.end_time = Some(value.to_string()),
                        "zone_high" => zone.zone_high = value.parse::<f64>().ok(),
                        "zone_low" => zone.zone_low = value.parse::<f64>().ok(),
                        "fifty_percent_line" => zone.fifty_percent_line = value.parse::<f64>().ok(),
                        "detection_method" => zone.detection_method = Some(value.to_string()),
                        "is_active" => {
                            zone.is_active = match value {
                                "1" | "true" | "TRUE" | "True" => true,
                                _ => false,
                            }
                        }
                        "formation_candles_json" => {
                            // Parse the JSON array of candles
                            if let Ok(candles) = serde_json::from_str::<Vec<CandleData>>(value) {
                                zone.formation_candles = candles;
                            } else {
                                log::warn!("Failed to parse formation_candles_json: {}", value);
                            }
                        }
                        "strength_score" => zone.strength_score = value.parse::<f64>().ok(),
                        "touch_count" => zone.touch_count = value.parse::<i64>().ok(),
                        "bars_active" => zone.bars_active = value.parse::<u64>().ok(),
                        // Skip other fields that don't map directly
                        _ => {}
                    }
                }

                // Ensure zone_id from the query parameter is set if missing
                if zone.zone_id.is_none() {
                    zone.zone_id = Some(query.zone_id.clone());
                }

                log::info!("Successfully fetched and parsed zone: {:?}", zone.zone_id);
                Ok(HttpResponse::Ok().json(zone)) // Return 200 OK with JSON body
            }
            Err(e) => {
                log::error!("Failed to read CSV record: {}", e);
                Err(ServiceError::CsvError(e))
            }
        }
    } else {
        log::warn!("No records found for zone_id: {}", query.zone_id);
        Err(ServiceError::NotFound) // Return 404 Not Found
    }
}
