// src/main.rs
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use csv::ReaderBuilder;
use dotenv::dotenv;
use serde::Deserialize;
use serde_json::json; // For InfluxDB query JSON and error responses
use std::env; // Need this for reading env vars
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;

// --- Module Declarations ---
mod backtest;
pub mod backtest_api; // Make sure this is public
mod detect;
mod errors;
mod get_zone_by_id;
mod influx_fetcher;
mod latest_zones_handler;
mod multi_backtest_handler;
mod patterns;
pub mod trades;
pub mod trading;
mod types; // Make sure StoredZone is defined here and derives Serialize/Deserialize
mod zone_generator; // Make sure run_zone_generation accepts bool parameter
mod zone_monitor_service; 

// --- Use necessary types ---
// Ensure StoredZone is accessible, adjust path if needed
use crate::types::{CandleData, StoredZone};

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

// In-memory cache for active zones
// Key: zone_id (String), Value: EnrichedZone (from types.rs)
type ActiveZoneCache = Arc<Mutex<HashMap<String, crate::types::EnrichedZone>>>; // Or StoredZone

// --- Handler for UI to get active zones ---
// This function now lives in main.rs
pub async fn get_ui_active_zones_handler( // Renamed slightly to avoid confusion if you had another
    zone_cache: web::Data<ActiveZoneCache>,
) -> impl Responder {
    log::info!("[UI_ACTIVE_ZONES_HANDLER] Request for active zones for UI received.");
    let cache_guard = zone_cache.lock().await;
    let zones_list: Vec<crate::types::EnrichedZone> = cache_guard.values().cloned().collect();
    // Consider sorting if needed: zones_list.sort_by_key(|z| z.start_time.clone());
    log::info!("[UI_ACTIVE_ZONES_HANDLER] Returning {} active zones.", zones_list.len());
    HttpResponse::Ok().json(zones_list)
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

    let active_zones_shared_cache: ActiveZoneCache =
        Arc::new(Mutex::new(HashMap::new()));

    // Spawn the Zone Monitor Service, passing a clone of the cache's Arc
    let cache_for_monitor = Arc::clone(&active_zones_shared_cache);
    tokio::spawn(zone_monitor_service::run_zone_monitor_service(cache_for_monitor)); // Pass the cache
    log::info!("[MAIN] Zone Monitor Service spawned.");

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

    let enable_periodic_env =
        env::var("ENABLE_PERIODIC_ZONE_GENERATOR").unwrap_or_else(|_| "true".to_string()); // Default to "true" if not set
    let enable_periodic = enable_periodic_env.trim().to_lowercase() == "true";

    if enable_periodic {
        // <<<<< CHECK THE FLAG HERE
        let periodic_interval_secs = env::var("GENERATOR_PERIODIC_INTERVAL")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .unwrap_or(60);

        if periodic_interval_secs > 0 {
            log::info!(
                "[MAIN] ENABLE_PERIODIC_ZONE_GENERATOR is true. Starting PERIODIC zone generation trigger every {} seconds.",
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
                "[MAIN] Periodic zone generation is configured to be enabled but GENERATOR_PERIODIC_INTERVAL ('{}') is <= 0. Not starting timer.",
                periodic_interval_secs
            );
        }
    } else {
        log::info!(
            "[MAIN] Periodic zone generation is DISABLED via ENABLE_PERIODIC_ZONE_GENERATOR='{}'.",
            enable_periodic_env
        );
    }
    // --- End Periodic Trigger ---
     let active_zones_cache_for_monitor_and_actix: zone_monitor_service::ActiveZoneCache =
        Arc::new(Mutex::new(HashMap::new()));

    // Clone the Arc for the monitor service
    let cache_for_monitor = Arc::clone(&active_zones_cache_for_monitor_and_actix);
    tokio::spawn(zone_monitor_service::run_zone_monitor_service(cache_for_monitor)); // Modify run_zone_monitor_service to accept the cache
    log::info!("[MAIN] Zone Monitor Service spawned.");

    // --- Configure and Start HTTP Server ---
    log::info!("Configuring HTTP server...");
    let server = HttpServer::new(move || {
        let cors = Cors::default()
            // .allowed_origin("http://localhost:4200") // Adjust as needed
            .allow_any_origin()
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
            .app_data(web::Data::new(Arc::clone(&active_zones_shared_cache))) 
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
                "/multi-symbol-backtest",
                web::post().to(multi_backtest_handler::run_multi_symbol_backtest),
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
                web::get().to(latest_zones_handler::get_latest_formed_zones_handler),
            )
            .route("/ui/active-zones", web::get().to(get_ui_active_zones_handler))
            .route(
                "/find-and-verify-zone",
                web::get().to(detect::find_and_verify_zone_handler),
            )
    })
    .bind((host, port))?;

    log::info!("Starting Actix HTTP server event loop...");
    server.run().await // Await server termination
}
