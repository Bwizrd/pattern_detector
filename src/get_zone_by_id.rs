// src/get_zone_by_id.rs
use actix_web::{web, get, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::env;
use crate::errors::ServiceError; // Import your custom error
use crate::types::StoredZone; // Import your StoredZone struct path
use log; // <<<--- ADD THIS IMPORT
use serde_json;
use csv; 
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
          |> filter(fn: (r) => r._field == "zone_id" and r._value == "{}")
          |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value")
          |> drop(columns: ["_start", "_stop", "_measurement"])
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
        let error_body = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        log::error!("InfluxDB query failed. Status: {}, Body: {}", status, error_body);
        return Err(ServiceError::InfluxQueryError(format!(
            "InfluxDB returned status {}: {}", status, error_body
        )));
    }

    let csv_data = response.text().await?;
    log::debug!("Received CSV data:\n{}", csv_data);

    // --- Parse CSV Data ---
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#')) // Ignore comments often included by InfluxDB
        .trim(csv::Trim::All) // Trim whitespace
        .from_reader(csv_data.as_bytes());

    let headers = reader.headers()?.clone(); // Get headers for mapping

    // Try to deserialize the *first* record found into StoredZone
    // Note: After pivot, headers should hopefully match struct fields more closely.
    // Direct deserialization might work now. If not, manual parsing needed.
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
            },
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