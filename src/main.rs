// src/main.rs - Simplified version for cache and API only

// Module declarations
mod api;
mod cache;
mod data;
mod errors;
mod handlers;
mod notifications;
mod realtime;
mod trading;
mod types;
mod zones;

// Standard library imports
use std::env;
use std::sync::{Arc, LazyLock};

// External crate imports
use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use dotenv::dotenv;
use log::{error, info};
use reqwest::Client as HttpClient;
use serde::Deserialize;

// Use tokio::sync::Mutex consistently throughout
use tokio::sync::Mutex;

// Import what you need from your modules
use crate::api::cache_endpoints::{
    get_minimal_cache_zones_debug_with_shared_cache, test_cache_endpoint,
};
use crate::cache::minimal_zone_cache::{get_minimal_cache, MinimalZoneCache};
use crate::cache::realtime_cache_updater::RealtimeCacheUpdater;
use crate::cache::zone_cache_writer::ZoneCacheWriter;
use crate::data::ctrader_integration::connect_to_ctrader_websocket;
use crate::realtime::simple_price_websocket::SimplePriceWebSocketServer;

// Import API handlers
use crate::api::detect::{
    detect_patterns, find_and_verify_zone_handler, get_active_zones_handler,
    get_bulk_multi_tf_active_zones_handler,
};
use crate::trading::backtest::backtest::run_backtest;

// Import handlers from the separate file
use crate::handlers::*;

// --- Global State ---
static GLOBAL_PRICE_BROADCASTER: LazyLock<
    std::sync::Mutex<Option<tokio::sync::broadcast::Sender<String>>>,
> = LazyLock::new(|| std::sync::Mutex::new(None));

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

// --- Core API Handlers ---
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

// --- Cache Setup Function ---
async fn setup_cache_system(
) -> Result<Arc<Mutex<MinimalZoneCache>>, Box<dyn std::error::Error + Send + Sync>> {
    info!("Setting up cache system...");

    let cache = match get_minimal_cache().await {
        Ok(cache) => {
            let (total, supply, demand) = cache.get_stats();
            info!(
                "Cache loaded: {} zones ({} supply, {} demand)",
                total, supply, demand
            );

            // Write initial zones to JSON for monitor
            let cache_writer = ZoneCacheWriter::new("shared_zones.json".to_string());
            if let Err(e) = cache_writer.write_zones_from_cache(&cache).await {
                error!("Failed to write initial zone cache: {}", e);
            } else {
                info!("Initial zone cache written to shared_zones.json");
            }

            Arc::new(Mutex::new(cache))
        }
        Err(e) => {
            error!("Cache load failed: {}", e);
            return Err(e);
        }
    };

    // Start real-time cache updates
    let enable_realtime_cache = env::var("ENABLE_REALTIME_CACHE")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        == "true";

    if enable_realtime_cache {
        let cache_update_interval = env::var("CACHE_UPDATE_INTERVAL_SECONDS")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .unwrap_or(60);

        let cache_updater =
            RealtimeCacheUpdater::new(Arc::clone(&cache)).with_interval(cache_update_interval);

        if let Err(e) = cache_updater.start().await {
            error!("Cache updater failed: {}", e);
        } else {
            info!(
                "Cache updater started (updates every {} seconds)",
                cache_update_interval
            );
        }
    }

    Ok(cache)
}

// --- Main Application ---
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    log4rs::init_file("log4rs.yaml", Default::default())
        .expect("Failed to initialize log4rs logging");

    info!("Starting Trading API Server...");

    // Setup the cache system
    let shared_cache = match setup_cache_system().await {
        Ok(cache) => cache,
        Err(e) => {
            error!("Failed to setup cache system: {}", e);
            std::process::exit(1);
        }
    };

    let shared_http_client = Arc::new(HttpClient::new());

    let enable_simple_price_ws = env::var("ENABLE_SIMPLE_PRICE_WS")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        == "true";

    if enable_simple_price_ws {
        let (ws_server, price_broadcaster) = SimplePriceWebSocketServer::new();
        let ws_port = env::var("PRICE_WS_PORT")
            .unwrap_or_else(|_| "8083".to_string())
            .parse::<u16>()
            .unwrap_or(8083);
        let ws_addr = format!("127.0.0.1:{}", ws_port);

        *GLOBAL_PRICE_BROADCASTER.lock().unwrap() = Some(price_broadcaster);

        tokio::spawn(async move {
            if let Err(e) = ws_server.start(ws_addr).await {
                error!("Price WebSocket failed: {}", e);
            }
        });

        info!("Price WebSocket started on port {}", ws_port);
    }

    // // Start price feed connection
    let enable_price_feed = env::var("ENABLE_CTRADER_PRICE_FEED")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        == "true";

    if enable_price_feed {
        let broadcaster_clone = {
            let guard = GLOBAL_PRICE_BROADCASTER.lock().unwrap();
            guard.as_ref().cloned()
        };

        tokio::spawn(async move {
            if let Some(broadcaster) = broadcaster_clone {
                if let Err(e) = connect_to_ctrader_websocket(&broadcaster, None).await {
                    error!("Price feed failed: {}", e);
                }
            }
        });
        info!("Price feed connection started");
    }

    // Server configuration
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    info!("API Server starting on http://{}:{}", host, port);

    let http_client_for_app_factory = Arc::clone(&shared_http_client);

    // Start the HTTP server
    let server_handle = HttpServer::new(move || {
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
            .app_data(web::Data::new(Arc::clone(&http_client_for_app_factory)))
            .app_data(web::Data::new(Arc::clone(&shared_cache)))
            // Core endpoints
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            // Zone endpoints
            .route("/analyze", web::get().to(detect_patterns))
            .route("/active-zones", web::get().to(get_active_zones_handler))
            .route(
                "/bulk-multi-tf-active-zones",
                web::post().to(get_bulk_multi_tf_active_zones_handler),
            )
            .route(
                "/zone",
                web::get().to(crate::api::get_zone_by_id::get_zone_by_id),
            )
            .route(
                "/latest-formed-zones",
                web::get().to(crate::api::latest_zones_handler::get_latest_formed_zones_handler),
            )
            .route(
                "/find-and-verify-zone",
                web::get().to(find_and_verify_zone_handler),
            )
            // Backtest endpoints
            .route("/backtest", web::post().to(run_backtest))
            // Debug endpoints
            .route("/test-cache", web::get().to(test_cache_endpoint))
            .route(
                "/debug/minimal-cache-zones",
                web::get().to(get_minimal_cache_zones_debug_with_shared_cache),
            )
            // Basic handlers for compatibility (these can return simple responses now)
            .route("/current-prices", web::get().to(get_current_prices_handler))
            .route("/test-prices", web::get().to(test_prices_handler))
    })
    .bind((host, port))?
    .run();

    server_handle.await
}
