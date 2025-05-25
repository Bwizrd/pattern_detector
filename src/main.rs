use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use log; // For direct log macro usage in main
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

// --- Module Declarations ---
mod admin_handlers;
mod backtest;
pub mod backtest_api;
mod detect;
mod errors;
mod get_zone_by_id;
mod influx_fetcher;
mod latest_zones_handler;
mod multi_backtest_handler;
mod patterns;
mod stale_zone_checker;
pub mod trades;
pub mod trading;
mod types;
mod zone_generator;
mod zone_monitor_service;
mod zone_revalidator_util;

// --- Use necessary types ---
use crate::types::StoredZone; // Used by map_csv_to_stored_zone
use crate::zone_monitor_service::ActiveZoneCache;
// --- API & Background Tasking Globals ---
static ZONE_GENERATION_QUEUED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(serde::Serialize)]
struct GeneratorResponse {
    status: String,
    message: String,
}

// --- Web Server Handlers defined in main.rs (for simple ones) ---
#[derive(serde::Serialize)]
struct EchoResponse {
    data: String,
    extra: String,
}

#[derive(Deserialize)]
struct QueryParams {
    pair: String,
    timeframe: String,
    range: String,
    pattern: String,
}

pub async fn get_ui_active_zones_handler(
    zone_cache: web::Data<ActiveZoneCache>, // Injects the shared cache
) -> impl Responder {
    log::info!("[UI_ACTIVE_ZONES_HANDLER] Request for active zones for UI received.");

    let cache_guard = zone_cache.lock().await;
    let zones_list: Vec<StoredZone> = cache_guard // Assuming LiveZoneState.zone_data is StoredZone
        .values()
        .map(|live_state| live_state.zone_data.clone())
        .collect();

    // The zones in zones_list are ALREADY the ones the Zone Monitor Service
    // deemed relevant based on its ZONE_MONITOR_SYMBOLS and ZONE_MONITOR_PATTERNTFS
    // when it populated its cache from InfluxDB (after filtering for is_active=true).

    log::info!(
        "[UI_ACTIVE_ZONES_HANDLER] Returning {} active zones from Zone Monitor's cache.",
        zones_list.len()
    );

    // For the UI to display what filters the *Zone Monitor* is using,
    // those env vars could be read once and stored, or simply understood by the user.
    // If you want to display them, this handler could read them.
    let monitor_symbols_for_display =
        env::var("ZONE_MONITOR_SYMBOLS").unwrap_or_else(|_| "Defaults used by monitor".to_string());
    let monitor_timeframes_for_display = env::var("ZONE_MONITOR_PATTERNTFS")
        .unwrap_or_else(|_| "Defaults used by monitor".to_string());
    let monitor_allowed_days_for_display = env::var("STRATEGY_ALLOWED_DAYS")
        .unwrap_or_else(|_| "Defaults used by monitor".to_string());

    HttpResponse::Ok().json(serde_json::json!({
        "tradeable_zones": zones_list, // These are the zones the monitor is actively tracking
        "monitoring_filters": { // Filters used by the Zone Monitor service itself
            "symbols": monitor_symbols_for_display,
            "patternTimeframes": monitor_timeframes_for_display,
            "allowedTradeDays": monitor_allowed_days_for_display
        }
    }))
}

async fn echo(query: web::Query<QueryParams>) -> impl Responder {
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
    dotenv::dotenv().ok();
    log4rs::init_file("log4rs.yaml", Default::default())
        .expect("Failed to initialize log4rs logging");

    log::info!("Starting Pattern Detector Application...");

    let run_on_startup = env::var("RUN_GENERATOR_ON_STARTUP")
        .map(|val| val.trim().to_lowercase() == "true")
        .unwrap_or(false);
    if run_on_startup {
        log::info!("RUN_GENERATOR_ON_STARTUP is true. Running FULL zone generation at startup...");
        if let Err(e) = zone_generator::run_zone_generation(false, None).await {
            log::error!("Startup zone generation failed: {}", e);
        } else {
            log::info!("Startup zone generation completed successfully.");
        }
    } else {
        log::info!("Skipping zone generation at startup (RUN_GENERATOR_ON_STARTUP is not 'true').");
    }

    let active_zones_shared_cache: zone_monitor_service::ActiveZoneCache =
        Arc::new(Mutex::new(HashMap::new()));
    let cache_for_monitor = Arc::clone(&active_zones_shared_cache);
    tokio::spawn(zone_monitor_service::run_zone_monitor_service(
        cache_for_monitor,
    ));
    log::info!("[MAIN] Zone Monitor Service spawned.");

    let shared_http_client = Arc::new(HttpClient::new());

    let enable_stale_checker = env::var("STALE_ZONE_CHECKER_ON")
        .map(|val| val.trim().to_lowercase() == "true")
        .unwrap_or(true); // Default to true if not set

    if enable_stale_checker {
        let http_client_for_stale_checker = Arc::clone(&shared_http_client);
        tokio::spawn(stale_zone_checker::run_stale_zone_check_service(
            http_client_for_stale_checker,
        ));
        log::info!("[MAIN] Stale Zone Check Service spawned.");
    } else {
        log::info!(
            "[MAIN] Stale Zone Check Service is DISABLED via STALE_ZONE_CHECKER_ON='{}'.",
            env::var("STALE_ZONE_CHECKER_ON").unwrap_or_else(|_| "not set".to_string())
        );
    }

    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    log::info!("Web server starting on http://{}:{}", host, port);
    println!("---> Starting Pattern Detector Web Server <---");
    println!("---> Listening on: http://{}:{} <---", host, port);
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
    println!("  GET  /latest-formed-zones");
    println!("  GET  /ui/active-zones");
    println!("  POST /admin/revalidate-zone (Body: {{\"zone_id\": \"...\"}})");
    println!("  GET  /echo?... (Example)");
    println!("  GET  /health");
    println!("  POST /generate-zones");
    println!("--------------------------------------------------");

    // Background task for API triggered zone generation
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
        while rx.recv().await.is_some() {
            log::info!("Running queued FULL zone generation task triggered by API...");
            if let Err(e) = zone_generator::run_zone_generation(false, None).await {
                log::error!("Queued FULL zone generation failed: {}", e);
            } else {
                log::info!("Queued FULL zone generation completed successfully");
            }
        }
        log::info!("API FULL Zone generation listener task finished.");
    });

    // Periodic Zone Generator Trigger
    let enable_periodic_env =
        env::var("ENABLE_PERIODIC_ZONE_GENERATOR").unwrap_or_else(|_| "true".to_string());
    let enable_periodic = enable_periodic_env.trim().to_lowercase() == "true";
    if enable_periodic {
        let periodic_interval_secs = env::var("GENERATOR_PERIODIC_INTERVAL")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .unwrap_or(60);
        if periodic_interval_secs > 0 {
            log::info!("[MAIN] ENABLE_PERIODIC_ZONE_GENERATOR is true. Starting PERIODIC zone generation trigger every {} seconds.", periodic_interval_secs);
            tokio::task::spawn_local(async move {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                log::info!(
                    "[MAIN] Initial delay complete. Starting periodic zone generation interval."
                );
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(periodic_interval_secs));
                loop {
                    interval.tick().await;
                    log::info!("[MAIN] Periodic zone generation triggered by timer...");
                    if let Err(e) = zone_generator::run_zone_generation(true, None).await {
                        log::error!("[MAIN] Periodic zone generation run failed: {}", e);
                    } else {
                        log::info!("[MAIN] Periodic zone generation run cycle completed.");
                    }
                }
            });
        } else {
            log::info!("[MAIN] Periodic zone generation is configured to be enabled but GENERATOR_PERIODIC_INTERVAL ('{}') is <= 0. Not starting timer.", periodic_interval_secs);
        }
    } else {
        log::info!(
            "[MAIN] Periodic zone generation is DISABLED via ENABLE_PERIODIC_ZONE_GENERATOR='{}'.",
            enable_periodic_env
        );
    }

    // Clone shared_http_client again for the App factory closure
    let http_client_for_app_factory = Arc::clone(&shared_http_client);

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE"])
            .allowed_headers(vec![
                actix_web::http::header::AUTHORIZATION,
                actix_web::http::header::ACCEPT,
                actix_web::http::header::CONTENT_TYPE,
            ])
            .max_age(3600);
        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            .app_data(web::Data::new(Arc::clone(&active_zones_shared_cache)))
            .app_data(web::Data::new(Arc::clone(&http_client_for_app_factory))) // Share HttpClient
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
            .route("/generate-zones", web::post().to(queue_zone_generator_api))
            .route("/zone", web::get().to(get_zone_by_id::get_zone_by_id))
            .route(
                "/latest-formed-zones",
                web::get().to(latest_zones_handler::get_latest_formed_zones_handler),
            )
            .route(
                "/ui/active-zones",
                web::get().to(get_ui_active_zones_handler),
            )
            .route(
                "/find-and-verify-zone",
                web::get().to(detect::find_and_verify_zone_handler),
            )
            .route(
                // Correctly define the route for the admin handler
                "/admin/revalidate-zone",
                web::post().to(admin_handlers::handle_revalidate_zone_request),
            )
    })
    .bind((host, port))?
    .run()
    .await
}
