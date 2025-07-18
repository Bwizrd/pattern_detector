// src/main.rs - Fixed version with proper multi-symbol backtest integration

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

// Module declarations are now in the api module - no need to declare them here

// Standard library imports
use std::env;
use std::sync::Arc;

// External crate imports
use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use actix_web::http::StatusCode;
use actix_web::web::Json;
use actix_web::HttpResponse as ActixHttpResponse;
use actix_web::Result as ActixResult;
use dotenv::dotenv;
use log::{error, info};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::fs;
use std::collections::HashMap;

// Use tokio::sync::Mutex consistently throughout
use tokio::sync::Mutex;

// Import what you need from your modules
use crate::api::cache_endpoints::{
    get_minimal_cache_zones_debug_with_shared_cache, test_cache_endpoint,
};
use crate::cache::minimal_zone_cache::{get_minimal_cache, MinimalZoneCache};
use crate::cache::realtime_cache_updater::RealtimeCacheUpdater;
use crate::cache::zone_cache_writer::ZoneCacheWriter;

// Import API handlers
use crate::api::detect::{
    detect_patterns, find_and_verify_zone_handler, get_active_zones_handler,
    get_bulk_multi_tf_active_zones_handler,
};
use crate::trading::backtest::backtest::run_backtest;

// Import handlers from the separate file
use crate::handlers::*;

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

// --- Trading Settings Handler ---
async fn trading_settings_handler() -> impl Responder {
    let mut settings = HashMap::new();

    dotenv().ok();

    for (key, value) in std::env::vars() {
        // Include all trading-related settings
        if key.contains("TRADING") 
            || key.contains("STRATEGY") 
            || key.contains("LOT_SIZE")
            || key.contains("TP_PIPS")
            || key.contains("SL_PIPS")
            || key.contains("SCANNER_")
            || key.contains("PROXIMITY_")
            || key.contains("INTENSIVE_ALERT")
            || key.contains("LIMIT_ORDER")
            || key.contains("NOTIFICATIONS_")
            || key.contains("TELEGRAM_")
            || key.contains("ENABLE_SOUND")
            || key.contains("CTRADER_")
            || key == "HOST"
            || key == "PORT"
            || key.contains("ENABLE_REALTIME")
            || key.contains("CACHE_UPDATE")
            || key.contains("ENABLE_TERMINAL")
        {
            settings.insert(key, value);
        }
    }

    HttpResponse::Ok().json(settings)
}

// --- Debug Handlers ---

async fn test_data_request_handler(
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let params = query.into_inner();
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "message": "Test data request endpoint",
        "received_params": params,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "note": "Connect this to your data fetching logic (InfluxDB, etc.)"
    }))
}

async fn debug_cache_stats(
    shared_cache: web::Data<Arc<Mutex<MinimalZoneCache>>>,
) -> impl Responder {
    let cache = shared_cache.lock().await;
    let (total, supply, demand) = cache.get_stats();
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "cache_stats": {
            "total_zones": total,
            "supply_zones": supply,
            "demand_zones": demand
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

async fn debug_cache_symbols(
    shared_cache: web::Data<Arc<Mutex<MinimalZoneCache>>>,
) -> impl Responder {
    let cache = shared_cache.lock().await;
    let all_zones = cache.get_all_zones();
    
    let mut symbol_counts = std::collections::HashMap::new();
    for zone in all_zones {
        if let Some(symbol) = &zone.symbol {
            *symbol_counts.entry(symbol.clone()).or_insert(0) += 1;
        }
    }
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "symbols": symbol_counts,
        "total_symbols": symbol_counts.len(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

async fn debug_cache_timeframes(
    shared_cache: web::Data<Arc<Mutex<MinimalZoneCache>>>,
) -> impl Responder {
    let cache = shared_cache.lock().await;
    let all_zones = cache.get_all_zones();
    
    let mut timeframe_counts = std::collections::HashMap::new();
    for zone in all_zones {
        if let Some(timeframe) = &zone.timeframe {
            *timeframe_counts.entry(timeframe.clone()).or_insert(0) += 1;
        }
    }
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "timeframes": timeframe_counts,
        "total_timeframes": timeframe_counts.len(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

// --- Combined Backtest HTML Handler ---
async fn combined_backtest_html() -> ActixResult<ActixHttpResponse> {
    match fs::read_to_string("combined_backtest_ui.html") {
        Ok(content) => Ok(ActixHttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(content)),
        Err(_) => Ok(ActixHttpResponse::NotFound()
            .content_type("text/html; charset=utf-8")
            .body("<h1>Error: combined_backtest_ui.html not found</h1>")),
    }
}

// --- Trading Settings HTML Handler ---
async fn trading_settings_html() -> ActixResult<ActixHttpResponse> {
    match fs::read_to_string("trading_settings.html") {
        Ok(content) => Ok(ActixHttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(content)),
        Err(_) => Ok(ActixHttpResponse::NotFound()
            .content_type("text/html; charset=utf-8")
            .body("<h1>Error: trading_settings.html not found</h1>")),
    }
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

    info!("Starting Trading API Server (Clean Version)...");

    // Setup the cache system
    let shared_cache = match setup_cache_system().await {
        Ok(cache) => cache,
        Err(e) => {
            error!("Failed to setup cache system: {}", e);
            std::process::exit(1);
        }
    };

    let shared_http_client = Arc::new(HttpClient::new());

    // Initialize live price manager for mobile dashboard
    let price_manager = Arc::new(crate::api::mobile_dashboard::LivePriceManager::new());
    
    // Start WebSocket price feed for mobile dashboard
    crate::api::mobile_dashboard::start_price_websocket(Arc::clone(&price_manager)).await;
    info!("📱 Mobile dashboard price feed started");

    // Server configuration
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    info!("API Server starting on http://{}:{}", host, port);
    info!("📊 Endpoints: Core, Zone Analysis, Backtest, Debug");

    let http_client_for_app_factory = Arc::clone(&shared_http_client);
    let price_manager_for_app_factory = Arc::clone(&price_manager);

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
            .app_data(web::Data::new(Arc::clone(&price_manager_for_app_factory)))
            
            // === CORE ENDPOINTS ===
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            
            // === ZONE ANALYSIS ENDPOINTS ===
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
            
            // === BACKTEST ENDPOINTS ===
            .route("/backtest", web::post().to(run_backtest))
            // FIXED: Use the proper api module handlers
            .route(
                "/multi-symbol-backtest",
                web::post().to(crate::api::multi_backtest_handler::run_multi_symbol_backtest),
            )
            .route(
                "/combined-backtest",
                web::post().to(crate::api::combined_backtest_handler::run_combined_backtest),
            )
            .route(
                "/optimize-parameters",
                web::post().to(crate::api::optimize_handler::run_parameter_optimization),
            )
            .route(
                "/portfolio-meta-backtest",
                web::post().to(crate::api::optimize_handler::run_portfolio_meta_optimized_backtest),
            )
            
            // === DEBUG ENDPOINTS ===
            .route("/debug/cache-stats", web::get().to(debug_cache_stats))
            .route("/debug/cache-symbols", web::get().to(debug_cache_symbols))
            .route("/debug/cache-timeframes", web::get().to(debug_cache_timeframes))
            .route("/debug/test-data", web::get().to(test_data_request_handler))
            .route("/debug/minimal-cache-zones", web::get().to(get_minimal_cache_zones_debug_with_shared_cache))
            
            // === TRADING TEST ENDPOINTS ===
            .route("/test-trade", web::post().to(crate::api::test_trade_handler::trigger_test_trade))
            
            // === CACHE ENDPOINTS ===
            .route("/test-cache", web::get().to(test_cache_endpoint))
            
            // === MOBILE DASHBOARD ===
            .route("/dashboard", web::get().to(crate::api::mobile_dashboard::serve_dashboard))
            .route("/api/dashboard-data", web::get().to(crate::api::mobile_dashboard::get_dashboard_data))
            
// === COMBINED BACKTEST UI ===
            .route("/combined-backtest-ui", web::get().to(combined_backtest_html))
            
// === TRADING SETTINGS HANDLER ===
            .route("/api/settings", web::get().to(trading_settings_handler))
            .route("/trading-settings", web::get().to(trading_settings_html))

            // === BASIC HANDLERS (for compatibility) ===
            .route("/current-prices", web::get().to(get_current_prices_handler))
            .route("/test-prices", web::get().to(test_prices_handler))
    })
    .bind((host, port))?
    .run();

    server_handle.await
}