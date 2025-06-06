// src/latest_zones_handler.rs

use actix_web::{web, HttpResponse, Responder};
use serde::Serialize;
use std::env;
use reqwest;
use serde_json::json;
use csv::ReaderBuilder;
use chrono::{DateTime, Utc, Duration}; // Duration might be needed

// Import necessary types from your project structure
use crate::types::{map_csv_to_stored_zone, StoredZone, ZoneCsvRecord};
use std::collections::HashMap; 

// --- Structs specific to this handler's response ---
#[derive(Serialize)]
pub struct LatestZonesSummary { // Made pub so it can be part of the public API response struct
    query_start_time_utc: String,
    query_end_time_utc: String,
    total_zones_in_response: usize,
    total_demand_zones_in_response: usize,
    total_supply_zones_in_response: usize,
     zones_per_timeframe: HashMap<String, usize>,
}

#[derive(Serialize)]
pub struct LatestZonesApiResponse { // Made pub for use in main.rs routing
    pub summary: LatestZonesSummary,
    pub zones: Vec<StoredZone>,
    pub query_executed: String,
}


// --- The Handler Function ---
pub async fn get_latest_formed_zones_handler() -> impl Responder { // Renamed slightly to avoid conflict if main still has it
    log::info!("[LATEST_ZONES_HANDLER] Request received.");

    // --- Environment Variable Fetching ---
    let influx_host = match env::var("INFLUXDB_HOST") {
        Ok(v) => v,
        Err(_) => {
            log::error!("[LATEST_ZONES_HANDLER] INFLUXDB_HOST not set");
            // For handlers in separate files, it's good to return a proper HttpResponse error
            return HttpResponse::InternalServerError().json(json!({"error": "Server configuration error: INFLUXDB_HOST not set"}));
        }
    };
    let influx_org = match env::var("INFLUXDB_ORG") {
        Ok(v) => v,
        Err(_) => {
            log::error!("[LATEST_ZONES_HANDLER] INFLUXDB_ORG not set");
            return HttpResponse::InternalServerError().json(json!({"error": "Server configuration error: INFLUXDB_ORG not set"}));
        }
    };
    let influx_token = match env::var("INFLUXDB_TOKEN") {
        Ok(v) => v,
        Err(_) => {
            log::error!("[LATEST_ZONES_HANDLER] INFLUXDB_TOKEN not set");
            return HttpResponse::InternalServerError().json(json!({"error": "Server configuration error: INFLUXDB_TOKEN not set"}));
        }
    };
    let zone_bucket = match env::var("GENERATOR_WRITE_BUCKET") {
        Ok(v) => v,
        Err(_) => {
            log::error!("[LATEST_ZONES_HANDLER] GENERATOR_WRITE_BUCKET not set");
            return HttpResponse::InternalServerError().json(json!({"error": "Server configuration error: GENERATOR_WRITE_BUCKET not set"}));
        }
    };
    let zone_measurement = match env::var("GENERATOR_ZONE_MEASUREMENT") {
        Ok(v) => v,
        Err(_) => {
            log::error!("[LATEST_ZONES_HANDLER] GENERATOR_ZONE_MEASUREMENT not set");
            return HttpResponse::InternalServerError().json(json!({"error": "Server configuration error: GENERATOR_ZONE_MEASUREMENT not set"}));
        }
    };

    let lookback_minutes_str =
        env::var("LATEST_ZONES_LOOKBACK_MINUTES").unwrap_or_else(|_| "120".to_string());
    let lookback_minutes = match lookback_minutes_str.parse::<i64>() {
        Ok(val) if val > 0 => val,
        _ => {
            log::warn!(
                "[LATEST_ZONES_HANDLER] Invalid LATEST_ZONES_LOOKBACK_MINUTES '{}', defaulting to 120.",
                lookback_minutes_str
            );
            120
        }
    };
    log::info!(
        "[LATEST_ZONES_HANDLER] Using lookback_minutes = {} for Flux query range.",
        lookback_minutes
    );

    // --- Calculate actual query start and end times for summary ---
    let query_actual_end_time_utc: DateTime<Utc> = Utc::now();
    let query_actual_start_time_utc: DateTime<Utc> = query_actual_end_time_utc - Duration::minutes(lookback_minutes);

    // --- FLUX QUERY ---
    let flux_query = format!(
        r#"
       from(bucket: "{}")
        |> range(start: -{}m)
        |> filter(fn: (r) => r._measurement == "{}")
        |> filter(fn: (r) => r._field != "formation_candles_json" and r._field != "detection_method") 
        |> pivot(
            rowKey:["_time", "symbol", "timeframe", "pattern", "zone_type"],
            columnKey: ["_field"],
            valueColumn: "_value"
           )
        |> group() 
        |> sort(columns: ["_time"], desc: true) 
        |> limit(n: 100) 
        |> yield(name: "pivoted_api_zones")
    "#,
        zone_bucket, lookback_minutes, zone_measurement
    );

    log::debug!(
        "[LATEST_ZONES_HANDLER] Executing PIVOTED Flux query: {}",
        flux_query
    );

    let client = reqwest::Client::new();
    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);

    match client
        .post(&query_url)
        .bearer_auth(influx_token)
        .header("Accept", "application/csv")
        .header("Content-Type", "application/json")
        .json(&json!({ "query": flux_query, "type": "flux" }))
        .send()
        .await
    {
        Ok(response) => {
            let response_status = response.status();
            let response_text = match response.text().await {
                Ok(t) => t,
                Err(e) => {
                    log::error!("[LATEST_ZONES_HANDLER] Failed to read response text: {}", e);
                    let summary = LatestZonesSummary {
                        query_start_time_utc: query_actual_start_time_utc.to_rfc3339(),
                        query_end_time_utc: query_actual_end_time_utc.to_rfc3339(),
                        total_zones_in_response: 0,
                        total_demand_zones_in_response: 0,
                        total_supply_zones_in_response: 0,
                        zones_per_timeframe: HashMap::new(),
                    };
                    return HttpResponse::InternalServerError().json(LatestZonesApiResponse {
                        summary,
                        zones: vec![],
                        query_executed: flux_query,
                    });
                }
            };
            
            // log::debug!( ... raw response ... ); // Keep if needed for debugging

            if response_status.is_success() {
                let data_line_count = response_text.lines().skip_while(|l| l.starts_with('#') || l.is_empty()).count();

                let mut zones: Vec<StoredZone> = Vec::new();
                if data_line_count > 1 {
                    let mut reader = ReaderBuilder::new().has_headers(true).flexible(true).comment(Some(b'#')).from_reader(response_text.as_bytes());
                    for result in reader.deserialize::<ZoneCsvRecord>() { // Assumes ZoneCsvRecord is accessible
                        match result {
                            Ok(csv_record) => zones.push(map_csv_to_stored_zone(csv_record)), // Assumes map_csv_to_stored_zone is accessible
                            Err(e) => log::error!("[LATEST_ZONES_HANDLER] Failed to deserialize CSV record: {}", e),
                        }
                    }
                }

                // --- Calculate summary statistics ---
                let total_zones_in_response = zones.len();
                let mut total_demand_zones_in_response = 0;
                let mut total_supply_zones_in_response = 0;
                let mut zones_per_timeframe_map: HashMap<String, usize> = HashMap::new();

                for zone_item in &zones {
                    if let Some(zone_type_str) = &zone_item.zone_type {
                        if zone_type_str.eq_ignore_ascii_case("demand") {
                            total_demand_zones_in_response += 1;
                        } else if zone_type_str.eq_ignore_ascii_case("supply") {
                            total_supply_zones_in_response += 1;
                        }
                    }
                     if let Some(tf_str) = &zone_item.timeframe {
                        *zones_per_timeframe_map.entry(tf_str.clone()).or_insert(0) += 1;
                    }
                }
                
                let summary = LatestZonesSummary {
                    query_start_time_utc: query_actual_start_time_utc.to_rfc3339(),
                    query_end_time_utc: query_actual_end_time_utc.to_rfc3339(),
                    total_zones_in_response,
                    total_demand_zones_in_response,
                    total_supply_zones_in_response,
                    zones_per_timeframe: zones_per_timeframe_map,
                };
                
                HttpResponse::Ok().json(LatestZonesApiResponse {
                    summary,
                    zones,
                    query_executed: flux_query,
                })

            } else {
                log::error!(
                    "[LATEST_ZONES_HANDLER] InfluxDB query failed with status {}. Body: {}",
                    response_status,
                    response_text
                );
                 let summary = LatestZonesSummary {
                    query_start_time_utc: query_actual_start_time_utc.to_rfc3339(),
                    query_end_time_utc: query_actual_end_time_utc.to_rfc3339(),
                    total_zones_in_response: 0,
                    total_demand_zones_in_response: 0,
                    total_supply_zones_in_response: 0,
                    zones_per_timeframe: HashMap::new(),
                };
                HttpResponse::InternalServerError().json(LatestZonesApiResponse {
                    summary,
                    zones: vec![],
                    query_executed: flux_query,
                })
            }
        }
        Err(e) => {
            log::error!("[LATEST_ZONES_HANDLER] Request to InfluxDB failed: {}", e);
             let summary = LatestZonesSummary {
                query_start_time_utc: query_actual_start_time_utc.to_rfc3339(),
                query_end_time_utc: query_actual_end_time_utc.to_rfc3339(),
                total_zones_in_response: 0,
                total_demand_zones_in_response: 0,
                total_supply_zones_in_response: 0,
                zones_per_timeframe: HashMap::new(),
            };
            HttpResponse::InternalServerError().json(LatestZonesApiResponse {
                summary,
                zones: vec![],
                query_executed: flux_query,
            })
        }
    }
}