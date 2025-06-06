// src/main.rs - Clean version with backtest and debug endpoints only

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
use std::sync::Arc;

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

// --- Backtest Handlers ---

async fn run_multi_symbol_backtest(
    request_body: web::Json<serde_json::Value>,
) -> impl Responder {
    let backtest_params = request_body.into_inner();
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "message": "Multi-symbol backtest endpoint - needs implementation",
        "received_params": backtest_params,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "note": "Connect this to your multi-symbol backtest logic"
    }))
}

async fn run_parameter_optimization(
    request_body: web::Json<serde_json::Value>,
) -> impl Responder {
    let optimization_params = request_body.into_inner();
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success", 
        "message": "Parameter optimization endpoint - needs implementation",
        "received_params": optimization_params,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "note": "Connect this to your optimization logic"
    }))
}

async fn run_portfolio_meta_optimized_backtest(
    request_body: web::Json<serde_json::Value>,
) -> impl Responder {
    let portfolio_params = request_body.into_inner();
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "message": "Portfolio meta backtest endpoint - needs implementation", 
        "received_params": portfolio_params,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "note": "Connect this to your portfolio backtest logic"
    }))
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

    // Server configuration
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    info!("API Server starting on http://{}:{}", host, port);
    info!("📊 Endpoints: Core, Zone Analysis, Backtest, Debug");

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
            .route(
                "/multi-symbol-backtest",
                web::post().to(run_multi_symbol_backtest),
            )
            .route(
                "/optimize-parameters",
                web::post().to(run_parameter_optimization),
            )
            .route(
                "/portfolio-meta-backtest",
                web::post().to(run_portfolio_meta_optimized_backtest),
            )
            
            // === DEBUG ENDPOINTS ===
            .route("/debug/cache-stats", web::get().to(debug_cache_stats))
            .route("/debug/cache-symbols", web::get().to(debug_cache_symbols))
            .route("/debug/cache-timeframes", web::get().to(debug_cache_timeframes))
            .route("/debug/test-data", web::get().to(test_data_request_handler))
            .route("/debug/minimal-cache-zones", web::get().to(get_minimal_cache_zones_debug_with_shared_cache))
            
            // === CACHE ENDPOINTS ===
            .route("/test-cache", web::get().to(test_cache_endpoint))
            
            // === BASIC HANDLERS (for compatibility) ===
            .route("/current-prices", web::get().to(get_current_prices_handler))
            .route("/test-prices", web::get().to(test_prices_handler))
    })
    .bind((host, port))?
    .run();

    server_handle.await
}