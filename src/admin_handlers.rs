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