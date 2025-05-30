// src/main.rs - Reorganized with candidates for extraction at bottom
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

mod backtest;
pub mod backtest_api;
mod cache_endpoints;
mod ctrader_integration;
mod detect;
mod errors;
mod get_zone_by_id;
mod influx_fetcher;
mod latest_zones_handler;
mod minimal_zone_cache;
mod multi_backtest_handler;
mod optimize_handler;
mod patterns;
mod price_feed;
mod realtime_monitor;
mod simple_price_websocket;
pub mod trades;
pub mod trading;
mod types;
mod websocket_server;
mod zone_detection;

// --- Use necessary types ---
use crate::cache_endpoints::{get_minimal_cache_zones_debug, test_cache_endpoint};
use crate::ctrader_integration::connect_to_ctrader_websocket;
use crate::minimal_zone_cache::{get_minimal_cache, CacheSymbolConfig, MinimalZoneCache};
use crate::price_feed::{PriceFeedBridge, PriceUpdate};
use crate::simple_price_websocket::{broadcast_price_update, SimplePriceWebSocketServer};
use crate::types::{
    BulkActiveZonesResponse, BulkResultData, BulkResultItem, ChartQuery, EnrichedZone,
};
// --- Global State ---
use std::sync::LazyLock;
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

// --- Simple API Handlers ---
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

// --- Main Application ---
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv::dotenv().ok();
    log4rs::init_file("log4rs.yaml", Default::default())
        .expect("Failed to initialize log4rs logging");

    log::info!("Starting Pattern Detector Application...");

    let shared_http_client = Arc::new(HttpClient::new());

    // Simple price WebSocket setup
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
                log::error!("Simple price WebSocket failed: {}", e);
            }
        });

        log::info!("Simple price WebSocket started on port {}", ws_port);
    }

    // Start price feed if enabled
    let enable_price_feed = env::var("ENABLE_CTRADER_PRICE_FEED")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        == "true";

    if enable_price_feed {
        // Clone the broadcaster outside the spawn to avoid Send issues
        let broadcaster_clone = {
            let guard = GLOBAL_PRICE_BROADCASTER.lock().unwrap();
            guard.as_ref().cloned()
        };

        tokio::spawn(async move {
            log::info!("Starting cTrader price feed integration");

            if let Some(broadcaster) = broadcaster_clone {
                if let Err(e) = connect_to_ctrader_websocket(&broadcaster).await {
                    log::error!("cTrader price feed failed: {}", e);
                }
            } else {
                log::error!("Price broadcaster not initialized");
            }
        });
    }

    // Test minimal cache
    use log::{error, info};
    info!("🚀 Testing minimal zone cache...");
    match get_minimal_cache().await {
        Ok(cache) => {
            let (total, supply, demand) = cache.get_stats();
            info!(
                "✅ Cache test complete: {} total zones ({} supply, {} demand)",
                total, supply, demand
            );
        }
        Err(e) => {
            error!("❌ Cache test failed: {}", e);
        }
    }

    // Server configuration
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    log::info!("Web server starting on http://{}:{}", host, port);
    print_server_info(&host, port);

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
            // Core endpoints
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            // Zone endpoints
            .route("/analyze", web::get().to(detect::detect_patterns))
            .route(
                "/active-zones",
                web::get().to(detect::get_active_zones_handler),
            )
            .route(
                "/bulk-multi-tf-active-zones",
                web::post().to(detect::get_bulk_multi_tf_active_zones_handler),
            )
            .route("/zone", web::get().to(get_zone_by_id::get_zone_by_id))
            .route(
                "/latest-formed-zones",
                web::get().to(latest_zones_handler::get_latest_formed_zones_handler),
            )
            .route(
                "/find-and-verify-zone",
                web::get().to(detect::find_and_verify_zone_handler),
            )
            // Backtest endpoints
            .route("/backtest", web::post().to(backtest::run_backtest))
            .route(
                "/multi-symbol-backtest",
                web::post().to(multi_backtest_handler::run_multi_symbol_backtest),
            )
            .route(
                "/optimize-parameters",
                web::post().to(optimize_handler::run_parameter_optimization),
            )
            .route(
                "/portfolio-meta-backtest",
                web::post().to(optimize_handler::run_portfolio_meta_optimized_backtest),
            )
            // Test/Debug endpoints
            .route(
                "/testDataRequest",
                web::get().to(influx_fetcher::test_data_request_handler),
            )
            .route("/test-cache", web::get().to(test_cache_endpoint))
            .route(
                "/debug/minimal-cache-zones",
                web::get().to(get_minimal_cache_zones_debug),
            )
    })
    .bind((host, port))?
    .run();

    // Wait for server to complete
    tokio::select! {
        result = server_handle => {
            log::info!("HTTP server ended: {:?}", result);
            result
        }
    }
}

// --- Utility Functions ---
fn print_server_info(host: &str, port: u16) {
    println!("---> Starting Pattern Detector Web Server <---");
    println!("---> Listening on: http://{}:{} <---", host, port);
    println!("Available endpoints:");
    println!("  GET  /health");
    println!("  GET  /echo?...");
    println!("  POST /generate-zones");
    println!("  GET  /analyze?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...");
    println!(
        "  GET  /active-zones?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=..."
    );
    println!("  POST /bulk-multi-tf-active-zones");
    println!("  POST /backtest");
    println!("  POST /multi-symbol-backtest");
    println!("  POST /optimize-parameters");
    println!("  POST /portfolio-meta-backtest");
    println!("  GET  /zone?zone_id=...");
    println!("  GET  /latest-formed-zones");
    println!("  GET  /test-cache");
    println!("  GET  /debug/minimal-cache-zones");
    println!("--------------------------------------------------");
}
