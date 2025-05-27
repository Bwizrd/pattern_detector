// src/main.rs
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use log;
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
mod optimize_handler;

// --- Use necessary types ---
use crate::types::StoredZone;
use crate::zone_monitor_service::ActiveZoneCache;

// --- API & Background Tasking Globals ---
static ZONE_GENERATION_QUEUED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(serde::Serialize)]
struct GeneratorResponse {
    status: String,
    message: String,
}

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
    zone_cache: web::Data<ActiveZoneCache>,
) -> impl Responder {
    log::info!("[UI_ACTIVE_ZONES_HANDLER] Request for active zones for UI received.");
    let cache_guard = zone_cache.lock().await;
    let zones_list: Vec<StoredZone> = cache_guard
        .values()
        .map(|live_state| live_state.zone_data.clone())
        .collect();
    log::info!(
        "[UI_ACTIVE_ZONES_HANDLER] Returning {} active zones from Zone Monitor's cache.",
        zones_list.len()
    );
    let monitor_symbols_for_display =
        env::var("ZONE_MONITOR_SYMBOLS").unwrap_or_else(|_| "Defaults used by monitor".to_string());
    let monitor_timeframes_for_display = env::var("ZONE_MONITOR_PATTERNTFS")
        .unwrap_or_else(|_| "Defaults used by monitor".to_string());
    let monitor_allowed_days_for_display = env::var("STRATEGY_ALLOWED_DAYS")
        .unwrap_or_else(|_| "Defaults used by monitor".to_string());
    HttpResponse::Ok().json(serde_json::json!({
        "tradeable_zones": zones_list,
        "monitoring_filters": {
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

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv::dotenv().ok();
    log4rs::init_file("log4rs.yaml", Default::default())
        .expect("Failed to initialize log4rs logging");

    log::info!("Starting Pattern Detector Application...");

    let shared_http_client = Arc::new(HttpClient::new());

    let run_generator_on_startup = env::var("RUN_GENERATOR_ON_STARTUP")
        .map(|val| val.trim().to_lowercase() == "true")
        .unwrap_or(false);
    if run_generator_on_startup {
        log::info!("RUN_GENERATOR_ON_STARTUP is true. Running FULL zone generation at startup...");
        if let Err(e) = zone_generator::run_zone_generation(false, None).await {
            log::error!("Startup zone generation failed: {}", e);
        } else {
            log::info!("Startup zone generation completed successfully.");
        }
    } else {
        log::info!("Skipping zone generation at startup (RUN_GENERATOR_ON_STARTUP is not 'true').");
    }

    let run_stuck_zone_deactivation_on_startup = env::var("RUN_STUCK_ZONE_DEACTIVATION_ON_STARTUP")
        .map(|val| val.trim().to_lowercase() == "true")
        .unwrap_or(false);

    if run_stuck_zone_deactivation_on_startup {
        log::info!("[MAIN_STARTUP_CLEANUP] RUN_STUCK_ZONE_DEACTIVATION_ON_STARTUP is true. Running deactivation task...");
        let client_for_cleanup = Arc::clone(&shared_http_client);
        tokio::spawn(async move {
            let influx_host = env::var("INFLUXDB_HOST").expect("INFLUXDB_HOST not set for startup cleanup");
            let influx_org = env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG not set for startup cleanup");
            let influx_token = env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN not set for startup cleanup");
            let zone_bucket = env::var("GENERATOR_WRITE_BUCKET").expect("GENERATOR_WRITE_BUCKET not set for startup cleanup");
            let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT").expect("GENERATOR_ZONE_MEASUREMENT not set for startup cleanup");
            let age_threshold_days = env::var("STUCK_ZONE_CLEANUP_AGE_DAYS")
                .unwrap_or_else(|_| "2".to_string())
                .parse::<u32>()
                .unwrap_or(2);

            match admin_handlers::deactivate_stuck_initial_zones(
                &client_for_cleanup,
                &influx_host, &influx_org, &influx_token,
                &zone_bucket, &zone_measurement,
                age_threshold_days,
            ).await {
                Ok(results) => {
                    log::info!("[MAIN_STARTUP_CLEANUP] Stuck zone deactivation completed. {} zones processed. Summary: {:?}", results.len(), results);
                }
                Err(e) => {
                    log::error!("[MAIN_STARTUP_CLEANUP] Stuck zone deactivation failed: {}", e);
                }
            }
        });
    } else {
        log::info!("[MAIN_STARTUP_CLEANUP] Skipping stuck zone deactivation at startup (RUN_STUCK_ZONE_DEACTIVATION_ON_STARTUP is not 'true').");
    }

    let active_zones_shared_cache: zone_monitor_service::ActiveZoneCache =
        Arc::new(Mutex::new(HashMap::new()));
    let cache_for_monitor = Arc::clone(&active_zones_shared_cache);
    tokio::spawn(zone_monitor_service::run_zone_monitor_service(
        cache_for_monitor,
    ));
    log::info!("[MAIN] Zone Monitor Service spawned.");

    let enable_stale_checker = env::var("STALE_ZONE_CHECKER_ON")
        .map(|val| val.trim().to_lowercase() == "true")
        .unwrap_or(false); 

    if enable_stale_checker {
        let http_client_for_stale_checker = Arc::clone(&shared_http_client);
        tokio::spawn(stale_zone_checker::run_stale_zone_check_service(
            http_client_for_stale_checker,
        ));
        log::info!("[MAIN] Stale Zone Check Service spawned.");
    } else {
        log::info!(
            "[MAIN] Stale Zone Check Service is DISABLED via STALE_ZONE_CHECKER_ON='{}'.",
            env::var("STALE_ZONE_CHECKER_ON").unwrap_or_else(|_| "not set or false".to_string())
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
    println!("  GET  /active-zones?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...");
    println!("  POST /bulk-multi-tf-active-zones (Body: [{{symbol, timeframes:[]}}], Query: ?start_time=...&end_time=...&pattern=...)");
    println!("  GET  /debug-bulk-zone?symbol=...&timeframe=...&start_time=...&end_time=...[&pattern=...]");
    println!("  POST /backtest");
    println!("  POST /multi-symbol-backtest");
    println!("  POST /optimize-parameters");
    println!("  POST /portfolio-meta-backtest");
    println!("  GET  /testDataRequest");
    println!("  GET  /zone?zone_id=...");
    println!("  GET  /latest-formed-zones");
    println!("  GET  /ui/active-zones");
    println!("  POST /admin/revalidate-zone (Body: {{\"zone_id\": \"...\"}})");
    println!("  POST /admin/deactivate-stuck-zones (Optional Body: {{\"age_threshold_days\": N}})"); // Added new endpoint
    println!("  GET  /echo?... (Example)");
    println!("  GET  /health");
    println!("  POST /generate-zones");
    println!("--------------------------------------------------");

    // --- Background task for API-triggered zone generation ---
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    {
        let tx_clone = tx.clone();
        tokio::task::spawn_local(async move { // spawn_local if not Send
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
    tokio::task::spawn_local(async move { // spawn_local if not Send
        while rx.recv().await.is_some() {
            log::info!("Running queued FULL zone generation task triggered by API...");
            if let Err(e) = zone_generator::run_zone_generation(false, None).await { // false for FULL run
                log::error!("Queued FULL zone generation failed: {}", e);
            } else {
                log::info!("Queued FULL zone generation completed successfully");
            }
        }
        log::info!("API FULL Zone generation listener task finished.");
    });
    // --- End API-triggered background task ---


    // --- Background task for periodic zone generation ---
    let enable_periodic_env =
        env::var("ENABLE_PERIODIC_ZONE_GENERATOR").unwrap_or_else(|_| "true".to_string());
    let enable_periodic = enable_periodic_env.trim().to_lowercase() == "true";

    if enable_periodic {
        let periodic_interval_secs = env::var("GENERATOR_PERIODIC_INTERVAL")
            .unwrap_or_else(|_| "60".to_string()) // Default to 60 seconds
            .parse::<u64>()
            .unwrap_or(60); 

        if periodic_interval_secs > 0 {
            log::info!("[MAIN] ENABLE_PERIODIC_ZONE_GENERATOR is true. Starting PERIODIC zone generation trigger every {} seconds.", periodic_interval_secs);
            tokio::task::spawn_local(async move { // spawn_local if not Send
                // Add an initial delay if desired, e.g., to let server fully start
                tokio::time::sleep(std::time::Duration::from_secs(15)).await; 
                log::info!("[MAIN] Initial delay complete. Starting periodic zone generation interval.");
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(periodic_interval_secs));
                loop {
                    interval.tick().await;
                    log::info!("[MAIN] Periodic zone generation triggered by timer...");
                    // Run zone generation with is_periodic_run = true
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
    // --- End periodic background task ---


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
            .app_data(web::Data::new(Arc::clone(&http_client_for_app_factory)))
            .route("/analyze", web::get().to(detect::detect_patterns))
            .route("/testDataRequest", web::get().to(influx_fetcher::test_data_request_handler))
            .route("/active-zones", web::get().to(detect::get_active_zones_handler))
            .route("/bulk-multi-tf-active-zones", web::post().to(detect::get_bulk_multi_tf_active_zones_handler))
            .route("/multi-symbol-backtest", web::post().to(multi_backtest_handler::run_multi_symbol_backtest))
            .route("/optimize-parameters", web::post().to(optimize_handler::run_parameter_optimization))
            .route("/portfolio-meta-backtest", web::post().to(optimize_handler::run_portfolio_meta_optimized_backtest))
            .route("/debug-bulk-zone", web::get().to(detect::debug_bulk_zones_handler))
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            .route("/backtest", web::post().to(backtest::run_backtest))
            .route("/generate-zones", web::post().to(queue_zone_generator_api))
            .route("/zone", web::get().to(get_zone_by_id::get_zone_by_id))
            .route("/latest-formed-zones", web::get().to(latest_zones_handler::get_latest_formed_zones_handler))
            .route("/ui/active-zones", web::get().to(get_ui_active_zones_handler))
            .route("/find-and-verify-zone", web::get().to(detect::find_and_verify_zone_handler))
            .route("/admin/revalidate-zone", web::post().to(admin_handlers::handle_revalidate_zone_request))
            .route( // New endpoint
                "/admin/deactivate-stuck-zones",
                web::post().to(admin_handlers::handle_deactivate_stuck_zones_request)
            )
    })
    .bind((host, port))?
    .run()
    .await
}