// src/get_zone_by_id.rs
use crate::errors::ServiceError; // Import your custom error
use crate::types::StoredZone; // Import your StoredZone struct path
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
    // let flux_query = format!(
    //     r#"from(bucket: "{}")
    //       |> range(start: 0)
    //       |> filter(fn: (r) => r._measurement == "{}")
    //       |> filter(fn: (r) => r._field == "zone_id" and r._value == "{}")
    //       |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value")
    //       |> drop(columns: ["_start", "_stop", "_measurement"])
    //     "#,
    //     config.zone_bucket, config.zone_measurement, query.zone_id
    // );

    let flux_query = format!(
        r#"from(bucket: "{}")
          |> range(start: 0)
          |> filter(fn: (r) => r._measurement == "{}")
          |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value")
          |> filter(fn: (r) => r.zone_id == "{}")
          |> limit(n: 1)
        "#,
        config.zone_bucket,
        config.zone_measurement,
        query.zone_id
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
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(csv_data.as_bytes());

    match reader.headers() {
        Ok(headers) => {
            log::debug!("CSV headers: {:?}", headers);
            log::debug!("Header count: {}", headers.len());
            for (i, h) in headers.iter().enumerate() {
                log::debug!("  Header {}: '{}'", i, h);
            }
        }
        Err(e) => log::error!("Failed to read CSV headers: {}", e),
    };

    // Create a second reader since we consumed the first one
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(csv_data.as_bytes());

    // Skip the header row since we already processed it
    if let Ok(_) = reader.headers() {
        log::debug!("Successfully skipped headers for record parsing");
    }

    // Collect records into a vector and log details
    let records: Result<Vec<_>, _> = reader.records().collect();
    match &records {
        Ok(recs) => {
            log::debug!("Parsed {} CSV records", recs.len());
            for (i, record) in recs.iter().enumerate() {
                log::debug!("Record {}: field count={}", i, record.len());
                for (j, field) in record.iter().enumerate() {
                    log::debug!("  Field {}[{}]: '{}'", i, j, field);
                }
            }
        }
        Err(e) => log::error!("Failed to collect CSV records: {}", e),
    };

    // Continue with your existing code, but with more detailed logging
    // of the StoredZone struct before attempting deserialization
    log::debug!("StoredZone struct fields: zone_id, start_idx, end_idx, start_time, end_time, zone_high, zone_low, fifty_percent_line, detection_method, extended, extension_percent, is_active, formation_candles");

    // Now try to deserialize with a fresh reader
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(csv_data.as_bytes());

    let headers = match reader.headers() {
        Ok(h) => {
            let hclone = h.clone();
            log::debug!("Headers for deserialization: {:?}", hclone);
            hclone
        }
        Err(e) => {
            log::error!("Failed to get headers for deserialization: {}", e);
            return Err(ServiceError::CsvError(e));
        }
    };
    let mut records = reader.deserialize::<StoredZone>();

    if let Some(result) = records.next() {
        match result {
            Ok(mut zone) => {
                // Ensure zone_id from the query parameter is set if missing from DB data
                if zone.zone_id.is_none() {
                    zone.zone_id = Some(query.zone_id.clone());
                }
                log::info!("Successfully fetched and parsed zone: {:?}", zone.zone_id);
                Ok(HttpResponse::Ok().json(zone)) // Return 200 OK with JSON body
            }
            Err(e) => {
                log::error!("Failed to deserialize InfluxDB record: {}", e);
                Err(ServiceError::CsvError(e)) // Return specific CSV error
            }
        }
    } else {
        log::warn!("No records found for zone_id: {}", query.zone_id);
        Err(ServiceError::NotFound) // Return 404 Not Found
    }
}
