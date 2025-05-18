// src/main.rs
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use csv::ReaderBuilder;
use dotenv::dotenv;
use serde::Deserialize;
use serde_json::json; // For InfluxDB query JSON and error responses
use std::env; // Need this for reading env vars

// --- Module Declarations ---
mod backtest;
mod detect;
mod errors;
mod get_zone_by_id;
mod influx_fetcher;
mod patterns;
pub mod trades;
pub mod trading;
mod types; // Make sure StoredZone is defined here and derives Serialize/Deserialize
mod zone_generator; // Make sure run_zone_generation accepts bool parameter

// --- Use necessary types ---
// Ensure StoredZone is accessible, adjust path if needed
use crate::types::{CandleData, StoredZone};

#[derive(serde::Deserialize, Debug, Clone, Default)]
pub struct ZoneCsvRecord {
    // --- Columns from pivot rowKey ---
    // These MUST match the column names produced by the pivot if they are part of rowKey
    #[serde(rename = "_time")]
    pub time: Option<String>,
    pub symbol: Option<String>,
    pub timeframe: Option<String>,
    pub pattern: Option<String>,
    pub zone_type: Option<String>,

    // --- Columns from pivot columnKey (original _field values) ---
    // Add ALL fields you expect after pivot. Use Option<String> for initial parsing.
    pub bars_active: Option<String>,
    pub detection_method: Option<String>,
    pub end_time_rfc3339: Option<String>,
    pub fifty_percent_line: Option<String>,
    pub formation_candles_json: Option<String>,
    pub is_active: Option<String>, // Will be "0" or "1"
    pub start_time_rfc3339: Option<String>,
    pub strength_score: Option<String>,
    pub touch_count: Option<String>,
    pub zone_high: Option<String>,
    pub zone_id: Option<String>, // zone_id is now a regular column
    pub zone_low: Option<String>,

    // --- Standard InfluxDB meta columns (often present in CSV, can be ignored) ---
    #[serde(default, rename = "_start")]
    pub _start: Option<String>,
    #[serde(default, rename = "_stop")]
    pub _stop: Option<String>,
    #[serde(default, rename = "result")]
    pub _result_influx: Option<String>, // Avoid conflict if StoredZone has 'result'
    #[serde(default)]
    pub table: Option<u32>, // Table ID is usually numeric
    #[serde(default, rename = "_measurement")]
    pub _measurement: Option<String>,
}

#[derive(serde::Serialize)]
struct LatestZonesResponse {
    zones: Vec<StoredZone>,
    query_executed: String,
    raw_response_sample: Option<String>,
}

fn map_csv_to_stored_zone(csv_record: ZoneCsvRecord) -> crate::types::StoredZone {
    // Use fully qualified StoredZone
    let parse_optional_f64 = |s: Option<String>| -> Option<f64> {
        s.and_then(|str_val| str_val.trim().parse::<f64>().ok())
    };
    let parse_optional_i64 = |s: Option<String>| -> Option<i64> {
        s.and_then(|str_val| str_val.trim().parse::<i64>().ok())
    };
    let parse_optional_u64 = |s: Option<String>| -> Option<u64> {
        s.and_then(|str_val| str_val.trim().parse::<u64>().ok())
    };
    let is_active_bool = csv_record
        .is_active
        .as_deref()
        .map_or(false, |s| s.trim() == "1");

    let formation_candles_vec: Vec<crate::types::CandleData> = csv_record
        .formation_candles_json
        .as_ref()
        .and_then(|json_str| match serde_json::from_str(json_str.trim()) {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!(
                    "[MAP_CSV] Failed to parse formation_candles_json string: {}. JSON: '{}'",
                    e,
                    json_str
                );
                None
            }
        })
        .unwrap_or_default();

    crate::types::StoredZone {
        zone_id: csv_record
            .zone_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        symbol: csv_record.symbol.map(|s| s.trim().to_string()),
        timeframe: csv_record.timeframe.map(|s| s.trim().to_string()),
        pattern: csv_record.pattern.map(|s| s.trim().to_string()),
        zone_type: csv_record.zone_type.map(|s| s.trim().to_string()),
        start_time: csv_record.time.map(|s| s.trim().to_string()), // Map Influx _time
        end_time: csv_record.end_time_rfc3339.map(|s| s.trim().to_string()),
        zone_high: parse_optional_f64(csv_record.zone_high),
        zone_low: parse_optional_f64(csv_record.zone_low),
        fifty_percent_line: parse_optional_f64(csv_record.fifty_percent_line),
        detection_method: csv_record.detection_method.map(|s| s.trim().to_string()),
        is_active: is_active_bool,
        bars_active: parse_optional_u64(csv_record.bars_active),
        touch_count: parse_optional_i64(csv_record.touch_count),
        strength_score: parse_optional_f64(csv_record.strength_score),
        formation_candles: formation_candles_vec,
        start_idx: None,
        end_idx: None,
        extended: None,
        extension_percent: None,
    }
}

// --- API & Background Tasking Globals ---
static ZONE_GENERATION_QUEUED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(serde::Serialize)]
struct GeneratorResponse {
    status: String,
    message: String,
}

#[derive(serde::Serialize)]
struct RawDataResponse {
    // Renamed for clarity for this specific test
    query_executed: String,
    raw_data_csv: Option<String>, // Will hold the direct CSV output
    influx_status_code: u16,
    error_message: Option<String>, // For any errors
}

// --- Web Server Handlers ---
#[derive(serde::Serialize)]
struct EchoResponse {
    data: String,
    extra: String,
}
#[derive(serde::Deserialize)]
struct QueryParams {
    pair: String,
    timeframe: String,
    range: String,
    pattern: String,
}
async fn echo(query: web::Query<QueryParams>) -> impl Responder {
    /* ... */
    let response = EchoResponse {
        data: format!(
            "Pair: {}, Timeframe: {}, Range: {}, Pattern: {}",
            query.pair, query.timeframe, query.range, query.pattern
        ),
        extra: "Response from Rust".to_string(),
    };
    HttpResponse::Ok().json(response)
}
async fn health_check() -> impl Responder {
    HttpResponse::Ok().body("OK, Rust server is running on port 8080")
}

// API handler to queue zone generation (triggers a FULL historical run)
async fn queue_zone_generator_api() -> impl Responder {
    log::info!("FULL Zone generation requested via API");
    ZONE_GENERATION_QUEUED.store(true, std::sync::atomic::Ordering::SeqCst);
    HttpResponse::Accepted().json(GeneratorResponse {
        status: "queued".to_string(),
        message: "FULL Zone generation has been queued. Check server logs for progress."
            .to_string(),
    })
}

async fn get_latest_formed_zones() -> impl Responder {
    log::info!("[API_LATEST_ZONES] Request received.");

    let influx_host = match env::var("INFLUXDB_HOST") {
        Ok(v) => v,
        Err(_) => {
            log::error!("INFLUXDB_HOST not set");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let influx_org = match env::var("INFLUXDB_ORG") {
        Ok(v) => v,
        Err(_) => {
            log::error!("INFLUXDB_ORG not set");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let influx_token = match env::var("INFLUXDB_TOKEN") {
        Ok(v) => v,
        Err(_) => {
            log::error!("INFLUXDB_TOKEN not set");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let zone_bucket = match env::var("GENERATOR_WRITE_BUCKET") {
        Ok(v) => v,
        Err(_) => {
            log::error!("GENERATOR_WRITE_BUCKET not set");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let zone_measurement = match env::var("GENERATOR_ZONE_MEASUREMENT") {
        Ok(v) => v,
        Err(_) => {
            log::error!("GENERATOR_ZONE_MEASUREMENT not set");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let lookback_minutes_str =
        env::var("LATEST_ZONES_LOOKBACK_MINUTES").unwrap_or_else(|_| "750".to_string()); // Keep wide for test
    let lookback_minutes = match lookback_minutes_str.parse::<i64>() {
        Ok(val) if val > 0 => val,
        _ => {
            log::warn!(
                "[API_LATEST_ZONES] Invalid LATEST_ZONES_LOOKBACK_MINUTES '{}', defaulting to 750.",
                lookback_minutes_str
            );
            750
        }
    };
    log::info!(
        "[API_LATEST_ZONES] Using lookback_minutes = {} for Flux query range.",
        lookback_minutes
    );

    // --- FLUX QUERY MATCHING SUCCESSFUL UI PIVOT TEST ---
    let flux_query = format!(
        r#"
       from(bucket: "{}")
        |> range(start: -{}m)
        |> filter(fn: (r) => r._measurement == "{}")
        // Filter out fields not needed in the API response
        |> filter(fn: (r) => r._field != "formation_candles_json" and r._field != "detection_method") 
        
        |> pivot(
            rowKey:["_time", "symbol", "timeframe", "pattern", "zone_type"],
            columnKey: ["_field"],
            valueColumn: "_value"
           )
        |> group() // Crucial for global sort
        |> sort(columns: ["_time"], desc: true) 
        |> limit(n: 100) // API might want a smaller limit than the Python script
        |> yield(name: "pivoted_api_zones")
    "#,
        zone_bucket, lookback_minutes, zone_measurement
    );

    log::debug!(
        "[API_LATEST_ZONES] Executing PIVOTED Flux query: {}",
        flux_query
    );

    let client = reqwest::Client::new();
    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);

    match client
        .post(&query_url)
        .bearer_auth(influx_token)
        .header("Accept", "application/csv") // Request CSV
        .header("Content-Type", "application/json")
        .json(&json!({ "query": flux_query, "type": "flux" }))
        .send()
        .await
    {
        Ok(response) => {
            let response_status = response.status();
            log::info!(
                "[API_LATEST_ZONES] Received response status: {}",
                response_status
            );
            let response_text = match response.text().await {
                Ok(t) => t,
                Err(e) => {
                    log::error!("[API_LATEST_ZONES] Failed to read response text: {}", e);
                    return HttpResponse::InternalServerError()
                        .json(json!({"error": "Failed to read InfluxDB response"}));
                }
            };
            log::debug!(
                "[API_LATEST_ZONES] InfluxDB raw CSV response (Status: {}): [{}]",
                response_status,
                response_text
            );

            if response_status.is_success() {
                // Check if the response text actually contains data rows beyond annotations/headers
                // A pivoted result should have at least one data line if successful.
                // The header line itself means data_line_count will be 1 if only header is present.
                // If actual data is present, count will be >= 2.
                let data_line_count = response_text
                    .lines()
                    .skip_while(|l| l.starts_with('#') || l.is_empty())
                    .count();

                if data_line_count <= 1 {
                    log::info!("[API_LATEST_ZONES] InfluxDB returned empty or header-only CSV response for PIVOTED query.");
                    HttpResponse::Ok().json(LatestZonesResponse {
                        zones: vec![],
                        query_executed: flux_query,
                        raw_response_sample: Some(response_text),
                    })
                } else {
                    let mut reader = ReaderBuilder::new()
                        .has_headers(true)
                        .flexible(true)
                        .comment(Some(b'#'))
                        .from_reader(response_text.as_bytes());

                    let mut zones: Vec<crate::types::StoredZone> = Vec::new();
                    let mut parsing_errors = Vec::new();

                    for result in reader.deserialize::<ZoneCsvRecord>() {
                        match result {
                            Ok(csv_record) => {
                                log::debug!(
                                    "[API_LATEST_ZONES] Successfully deserialized CSV record: {:?}",
                                    csv_record
                                );
                                zones.push(map_csv_to_stored_zone(csv_record));
                            }
                            Err(e) => {
                                let error_msg = format!(
                                    "[API_LATEST_ZONES] Failed to deserialize CSV record: {}",
                                    e
                                );
                                log::error!("{}", error_msg);
                                parsing_errors.push(error_msg);
                            }
                        }
                    }

                    if !parsing_errors.is_empty() {
                        log::warn!(
                            "[API_LATEST_ZONES] Encountered {} errors during CSV parsing.",
                            parsing_errors.len()
                        );
                    }

                    log::info!(
                        "[API_LATEST_ZONES] Successfully parsed CSV. Found {} zones.",
                        zones.len()
                    );
                    HttpResponse::Ok().json(LatestZonesResponse {
                        zones,
                        query_executed: flux_query,
                        raw_response_sample: Some(response_text.chars().take(2000).collect()),
                    })
                }
            } else {
                log::error!(
                    "[API_LATEST_ZONES] InfluxDB query failed with status {}. Body: {}",
                    response_status,
                    response_text
                );
                HttpResponse::InternalServerError().json(json!({ "error": format!("InfluxDB query error (status {})", response_status), "details": response_text, "query_executed": flux_query }))
            }
        }
        Err(e) => {
            log::error!("[API_LATEST_ZONES] Request to InfluxDB failed: {}", e);
            HttpResponse::InternalServerError().json(json!({ "error": "Failed to send request to InfluxDB", "details": e.to_string(), "query_executed": flux_query }))
        }
    }
}

// --- Main Function ---
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Use ? for cleaner error handling on init
    dotenv::dotenv().ok(); // Load .env file, ignore error if not found
    log4rs::init_file("log4rs.yaml", Default::default())
        .expect("Failed to initialize log4rs logging");

    log::info!("TEST LOG FROM MAIN: Logger initialized.");
    log::info!("Starting Pattern Detector Application...");

    // --- Check if Startup Generation is Enabled (Calls run_zone_generation with is_periodic_run = false) ---
    let run_on_startup = env::var("RUN_GENERATOR_ON_STARTUP")
        .map(|val| val.trim().to_lowercase() == "true")
        .unwrap_or(false);

    if run_on_startup {
        log::info!("RUN_GENERATOR_ON_STARTUP is true. Running FULL zone generation at startup...");
        // Pass false for is_periodic_run
        if let Err(e) = zone_generator::run_zone_generation(false, None).await {
            log::error!("Startup zone generation failed: {}", e);
        } else {
            log::info!("Startup zone generation completed successfully.");
        }
    } else {
        log::info!("Skipping zone generation at startup (RUN_GENERATOR_ON_STARTUP is not 'true').");
    }
    // --- End Startup Generation Check ---

    // --- Setup Web Server Variables ---
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    log::info!("Web server starting on http://{}:{}", host, port);
    println!("---> Starting Pattern Detector Web Server <---");
    println!("---> Listening on: http://{}:{} <---", host, port);
    // ... (println! for endpoints - remember to add /latest-formed-zones) ...
    println!("Available endpoints:");
    println!("  GET  /analyze?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...");
    println!(
        "  GET  /active-zones?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=..."
    );
    println!("  POST /bulk-multi-tf-active-zones (Body: [{{symbol, timeframes:[]}}], Query: ?start_time=...&end_time=...&pattern=...)");
    println!("  GET  /debug-bulk-zone?symbol=...&timeframe=...&start_time=...&end_time=...[&pattern=...]");
    println!("  POST /backtest");
    println!("  GET  /testDataRequest");
    println!("  GET  /zone?zone_id=...");
    println!("  GET  /latest-formed-zones"); // Added
    println!("  GET  /echo?...");
    println!("  GET  /health");
    println!("  POST /generate-zones");
    println!("--------------------------------------------------");

    // --- Background Task Setup for API trigger (Calls run_zone_generation with is_periodic_run = false) ---
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    {
        let tx_clone = tx.clone();
        tokio::task::spawn_local(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if ZONE_GENERATION_QUEUED.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    log::info!("Background task detected queued FULL zone generation request via API flag.");
                    if let Err(e) = tx_clone.send(()).await {
                        log::error!("Failed to send signal for queued zone generation: {}", e);
                    }
                }
            }
        });
    }
    tokio::task::spawn_local(async move {
        // Listener task
        while rx.recv().await.is_some() {
            log::info!("Running queued FULL zone generation task triggered by API...");
            // Pass false for is_periodic_run
            if let Err(e) = zone_generator::run_zone_generation(false, None).await {
                log::error!("Queued FULL zone generation failed: {}", e);
            } else {
                log::info!("Queued FULL zone generation completed successfully");
            }
        }
        log::info!("API FULL Zone generation listener task finished.");
    });
    // --- End API Background Task Setup ---

    // --- NEW Periodic Trigger (Calls run_zone_generation with is_periodic_run = true) ---
    let periodic_interval_secs = env::var("GENERATOR_PERIODIC_INTERVAL")
        .unwrap_or_else(|_| "60".to_string())
        .parse::<u64>()
        .unwrap_or(60);

    if periodic_interval_secs > 0 {
        log::info!(
            "[MAIN] Starting PERIODIC zone generation trigger every {} seconds.",
            periodic_interval_secs
        );
        tokio::task::spawn_local(async move {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await; // Initial delay
            log::info!(
                "[MAIN] Initial delay complete. Starting periodic zone generation interval."
            );
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(periodic_interval_secs));
            loop {
                interval.tick().await;
                log::info!("[MAIN] Periodic zone generation triggered by timer...");
                // Pass true for is_periodic_run
                if let Err(e) = zone_generator::run_zone_generation(true, None).await {
                    log::error!("[MAIN] Periodic zone generation run failed: {}", e);
                } else {
                    log::info!("[MAIN] Periodic zone generation run cycle completed.");
                }
            }
        });
    } else {
        log::info!(
            "[MAIN] Periodic zone generation is disabled (GENERATOR_PERIODIC_INTERVAL <= 0)."
        );
    }
    // --- End Periodic Trigger ---

    // --- Configure and Start HTTP Server ---
    log::info!("Configuring HTTP server...");
    let server = HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin("http://localhost:4200") // Adjust as needed
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE"])
            .allowed_headers(vec![
                actix_web::http::header::AUTHORIZATION,
                actix_web::http::header::ACCEPT,
                actix_web::http::header::CONTENT_TYPE,
            ])
            .max_age(3600);
        App::new()
            .wrap(Logger::default()) // Actix logger
            .wrap(cors)
            // --- Routes ---
            .route("/analyze", web::get().to(detect::detect_patterns))
            .route(
                "/testDataRequest",
                web::get().to(influx_fetcher::test_data_request_handler),
            )
            .route(
                "/active-zones",
                web::get().to(detect::get_active_zones_handler),
            )
            .route(
                "/bulk-multi-tf-active-zones",
                web::post().to(detect::get_bulk_multi_tf_active_zones_handler),
            )
            .route(
                "/debug-bulk-zone",
                web::get().to(detect::debug_bulk_zones_handler),
            )
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            .route("/backtest", web::post().to(backtest::run_backtest))
            .route("/generate-zones", web::post().to(queue_zone_generator_api)) // API trigger remains (for FULL run)
            .route("/zone", web::get().to(get_zone_by_id::get_zone_by_id))
            // --- Add the new route ---
            .route(
                "/latest-formed-zones",
                web::get().to(get_latest_formed_zones),
            )
    })
    .bind((host, port))?;

    log::info!("Starting Actix HTTP server event loop...");
    server.run().await // Await server termination
}
