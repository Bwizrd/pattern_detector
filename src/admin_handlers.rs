// src/admin_handlers.rs
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
use reqwest::Client as HttpClient;
use std::env;
use crate::zone_revalidator_util::revalidate_one_zone_activity_by_id;
// use crate::types::StoredZone; // Import StoredZone if not already
use log::{info, error};
use serde_json::json;

#[derive(serde::Deserialize, Debug)]
pub struct RevalidateZoneRequest {
    pub zone_id: String,
}

// This attribute macro defines the route when using .service() in main.rs
// Or if using .route() in main.rs, this attribute is optional but good for clarity.
pub async fn handle_revalidate_zone_request(
    req_body: web::Json<RevalidateZoneRequest>,
    http_client_data: web::Data<Arc<HttpClient>>,
) -> impl Responder {
    let zone_id_to_revalidate = req_body.into_inner().zone_id; // Renamed for clarity
    info!("[ADMIN_HANDLER] Received request to revalidate zone: {}", zone_id_to_revalidate);

    let http_client_arc = http_client_data.into_inner();

    let influx_host = match env::var("INFLUXDB_HOST") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: INFLUXDB_HOST not set"})) };
    let influx_org = match env::var("INFLUXDB_ORG") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: INFLUXDB_ORG not set"})) };
    let influx_token = match env::var("INFLUXDB_TOKEN") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: INFLUXDB_TOKEN not set"})) };
    let zone_bucket = match env::var("GENERATOR_WRITE_BUCKET") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: GENERATOR_WRITE_BUCKET not set"})) };
    let zone_measurement = match env::var("GENERATOR_ZONE_MEASUREMENT") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: GENERATOR_ZONE_MEASUREMENT not set"})) };
    let candle_bucket = match env::var("INFLUXDB_BUCKET") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: INFLUXDB_BUCKET for candles not set"})) };
    let candle_measurement_for_candles = match env::var("GENERATOR_TRENDBAR_MEASUREMENT") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: GENERATOR_TRENDBAR_MEASUREMENT not set"})) };

    match revalidate_one_zone_activity_by_id(
        zone_id_to_revalidate.clone(), // Clone zone_id for use in error messages
        &http_client_arc,
        &influx_host, &influx_org, &influx_token,
        &zone_bucket, &zone_measurement,
        &candle_bucket, &candle_measurement_for_candles
    ).await {
        Ok(Some(revalidated_zone_data)) => { // `revalidated_zone_data` is the StoredZone
            HttpResponse::Ok().json(json!({
                "message": "Zone revalidated successfully.",
                "revalidated_zone": revalidated_zone_data // <<< SERIALIZE THE ENTIRE StoredZone OBJECT
            }))
        }
        Ok(None) => HttpResponse::NotFound().json(json!({
            "error": "Zone not found or no update deemed necessary by revalidator.",
            "zone_id": zone_id_to_revalidate
        })),
        Err(e) => {
            error!("[ADMIN_HANDLER] Error revalidating zone {}: {}", zone_id_to_revalidate, e);
            HttpResponse::InternalServerError().json(json!({
                "error": "Failed to revalidate zone.",
                "zone_id": zone_id_to_revalidate,
                "details": e
            }))
        }
    }
}

// Add these imports to your existing imports
use csv::ReaderBuilder as CsvReaderBuilder;
use serde::{Deserialize, Serialize};
use log::warn;

// Add these structs after your existing RevalidateZoneRequest struct

#[derive(Deserialize, Debug, Clone)]
struct ZoneToDeactivateInfo {
    #[serde(rename = "_time")]
    start_time: String,
    symbol: String,
    timeframe: String,
    pattern: Option<String>,
    zone_type: Option<String>,
    zone_id: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct DeactivatedZoneInfo {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub start_time: String,
    pub status: String,
}

#[derive(Deserialize, Debug)]
pub struct DeactivateStuckZonesRequest {
    pub age_threshold_days: Option<u32>,
}

// Add this function after your existing handle_revalidate_zone_request function

pub async fn deactivate_stuck_initial_zones(
    http_client: &Arc<HttpClient>,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    zone_bucket: &str,
    zone_measurement: &str,
    age_threshold_days: u32,
) -> Result<Vec<DeactivatedZoneInfo>, String> {
    info!("[DEACTIVATE_STUCK_ZONES] Starting process for zones older than {} days.", age_threshold_days);

    let age_cutoff_time_flux = format!("experimental.subDuration(d: {}d, from: now())", age_threshold_days);

    let flux_query = format!(
        r#"
        import "influxdata/influxdb/schema"
        age_cutoff = {age_cutoff_time_flux_val}
        from(bucket: "{db_bucket}")
          |> range(start: -180d) 
          |> filter(fn: (r) => r._measurement == "{db_measurement}")
          |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value")
          |> filter(fn: (r) => r.is_active == 1)
          |> filter(fn: (r) => r.bars_active == 0)
          |> filter(fn: (r) => (not exists r.start_idx or not exists r.end_idx))
          |> filter(fn: (r) => r._time < age_cutoff)
          |> keep(columns: ["_time", "symbol", "timeframe", "pattern", "zone_type", "zone_id"])
        "#,
        age_cutoff_time_flux_val = age_cutoff_time_flux,
        db_bucket = zone_bucket,
        db_measurement = zone_measurement
    );

    info!("[DEACTIVATE_STUCK_ZONES] Querying for zones to deactivate: {}", flux_query);
    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);
    
    let response = match http_client
        .post(&query_url)
        .bearer_auth(influx_token)
        .header("Accept", "application/csv")
        .header("Content-Type", "application/json")
        .json(&json!({ "query": flux_query, "type": "flux" }))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => return Err(format!("HTTP request to InfluxDB for query failed: {}", e)),
    };

    let query_status = response.status(); 
    if !query_status.is_success() {
        let err_body = response.text().await.unwrap_or_else(|_| "Failed to read error body for query".to_string());
        error!("[DEACTIVATE_STUCK_ZONES] InfluxDB query failed (status {}): {}", query_status, err_body);
        return Err(format!("InfluxDB query failed (status {}): {}", query_status, err_body));
    }

    let csv_text = match response.text().await {
        Ok(text) => text,
        Err(e) => return Err(format!("Failed to read InfluxDB query response text: {}", e)),
    };

    let mut zones_found_to_deactivate: Vec<ZoneToDeactivateInfo> = Vec::new();
    if csv_text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).count() > 1 {
        let mut rdr = CsvReaderBuilder::new()
            .has_headers(true)
            .flexible(true) 
            .comment(Some(b'#'))
            .from_reader(csv_text.as_bytes());

        for result in rdr.deserialize::<ZoneToDeactivateInfo>() {
            match result {
                Ok(record) => zones_found_to_deactivate.push(record),
                Err(e) => warn!("[DEACTIVATE_STUCK_ZONES] CSV deserialize error for a zone: {}. Row might be skipped.", e),
            }
        }
    } else {
        info!("[DEACTIVATE_STUCK_ZONES] No data rows in CSV response matching criteria for deactivation.");
        return Ok(Vec::new());
    }

    if zones_found_to_deactivate.is_empty() {
        info!("[DEACTIVATE_STUCK_ZONES] No zones found matching criteria after parsing attempt.");
        return Ok(Vec::new());
    }

    info!("[DEACTIVATE_STUCK_ZONES] Found {} zones to attempt deactivation.", zones_found_to_deactivate.len());

    let mut deactivated_results: Vec<DeactivatedZoneInfo> = Vec::new();
    let write_url = format!("{}/api/v2/write?org={}&bucket={}&precision=ns", influx_host, influx_org, zone_bucket);

    for zone_info in zones_found_to_deactivate {
        let original_timestamp_nanos = match chrono::DateTime::parse_from_rfc3339(&zone_info.start_time) {
            Ok(dt) => match dt.timestamp_nanos_opt() {
                Some(nanos) => nanos.to_string(),
                None => {
                    warn!("[DEACTIVATE_STUCK_ZONES] Could not get nanos from timestamp for zone_id: {}. Skipping.", zone_info.zone_id);
                    deactivated_results.push(DeactivatedZoneInfo {
                        zone_id: zone_info.zone_id.clone(), symbol: zone_info.symbol.clone(),
                        timeframe: zone_info.timeframe.clone(), start_time: zone_info.start_time.clone(),
                        status: "Error: Invalid original timestamp".to_string(),
                    });
                    continue;
                }
            },
            Err(e) => {
                warn!("[DEACTIVATE_STUCK_ZONES] Could not parse original timestamp {} for zone_id: {}: {}. Skipping.", zone_info.start_time, zone_info.zone_id, e);
                deactivated_results.push(DeactivatedZoneInfo {
                    zone_id: zone_info.zone_id.clone(), symbol: zone_info.symbol.clone(),
                    timeframe: zone_info.timeframe.clone(), start_time: zone_info.start_time.clone(),
                    status: "Error: Could not parse original timestamp".to_string(),
                });
                continue;
            }
        };

        let mut tags: Vec<String> = Vec::new();
        tags.push(format!("symbol={}", zone_info.symbol.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        tags.push(format!("timeframe={}", zone_info.timeframe.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        if let Some(p) = &zone_info.pattern {
            tags.push(format!("pattern={}", p.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        }
        if let Some(zt) = &zone_info.zone_type {
            tags.push(format!("zone_type={}", zt.replace(' ', "\\ ").replace(',', "\\,").replace('=', "\\=")));
        }
        let tags_str = tags.join(",");

        let line = format!(
            "{measurement},{tags_from_query} zone_id=\"{zone_id_val}\",is_active=0i {timestamp}",
            measurement = zone_measurement,
            tags_from_query = tags_str,
            zone_id_val = zone_info.zone_id.escape_default(),
            timestamp = original_timestamp_nanos
        );

        info!("[DEACTIVATE_STUCK_ZONES] Attempting to deactivate zone_id: {}. Line: {}", zone_info.zone_id, line);

        let attempt_status = match http_client
            .post(&write_url)
            .bearer_auth(influx_token)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(line)
            .send()
            .await
        {
            Ok(update_response) => {
                let update_status = update_response.status();
                if update_status.is_success() {
                    info!("[DEACTIVATE_STUCK_ZONES] Successfully sent deactivation for zone_id: {}", zone_info.zone_id);
                    "Deactivated".to_string()
                } else {
                    let err_text = update_response.text().await.unwrap_or_else(|_| "Failed to read error body for update".to_string());
                    error!("[DEACTIVATE_STUCK_ZONES] Failed to deactivate zone_id {}. Status: {}. Resp: {}", zone_info.zone_id, update_status, err_text);
                    format!("Error: Update failed (status {})", update_status)
                }
            }
            Err(e) => {
                error!("[DEACTIVATE_STUCK_ZONES] HTTP Error sending deactivation for zone_id {}: {}", zone_info.zone_id, e);
                format!("Error: HTTP send error - {}", e)
            }
        };

        deactivated_results.push(DeactivatedZoneInfo {
            zone_id: zone_info.zone_id,
            symbol: zone_info.symbol,
            timeframe: zone_info.timeframe,
            start_time: zone_info.start_time,
            status: attempt_status,
        });
    }

    Ok(deactivated_results)
}

pub async fn handle_deactivate_stuck_zones_request(
    req_body_optional: Option<web::Json<DeactivateStuckZonesRequest>>,
    http_client_data: web::Data<Arc<HttpClient>>,
) -> impl Responder {
    info!("[ADMIN_HANDLER] Received request to deactivate stuck initial zones.");

    let http_client_arc = http_client_data.into_inner();

    let influx_host = match env::var("INFLUXDB_HOST") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: INFLUXDB_HOST not set"})) };
    let influx_org = match env::var("INFLUXDB_ORG") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: INFLUXDB_ORG not set"})) };
    let influx_token = match env::var("INFLUXDB_TOKEN") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: INFLUXDB_TOKEN not set"})) };
    let zone_bucket = match env::var("GENERATOR_WRITE_BUCKET") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: GENERATOR_WRITE_BUCKET not set"})) };
    let zone_measurement = match env::var("GENERATOR_ZONE_MEASUREMENT") { Ok(v) => v, Err(_) => return HttpResponse::InternalServerError().json(json!({"error": "Server Config: GENERATOR_ZONE_MEASUREMENT not set"})) };
    
    let default_age_days = env::var("STUCK_ZONE_CLEANUP_AGE_DAYS")
        .unwrap_or_else(|_| "2".to_string())
        .parse::<u32>()
        .unwrap_or(2);

    let age_days = match req_body_optional {
        Some(req_body) => req_body.into_inner().age_threshold_days.unwrap_or(default_age_days),
        None => default_age_days,
    };

    match deactivate_stuck_initial_zones(
        &http_client_arc,
        &influx_host, &influx_org, &influx_token,
        &zone_bucket, &zone_measurement,
        age_days,
    ).await {
        Ok(results) => {
            HttpResponse::Ok().json(json!({
                "message": format!("Attempted deactivation for {} zones.", results.len()),
                "deactivated_zones_summary": results
            }))
        }
        Err(e) => {
            error!("[ADMIN_HANDLER] Error during stuck zone deactivation process: {}", e);
            HttpResponse::InternalServerError().json(json!({
                "error": "Failed to process stuck zone deactivation.",
                "details": e
            }))
        }
    }
}