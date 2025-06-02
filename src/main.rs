// src/main.rs - Clean imports and integration
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use log::{error, info};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;

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
mod realtime_cache_updater;
mod realtime_monitor; // Keep existing for websocket_server.rs compatibility
mod realtime_zone_monitor; // ← ADD NEW MODULE
mod simple_price_websocket;
pub mod trades;
pub mod trading;
mod types;
mod websocket_server;
mod zone_detection;
mod terminal_dashboard;
mod trade_decision_engine;

// --- Use necessary types ---
use crate::cache_endpoints::{
    get_minimal_cache_zones_debug_with_shared_cache, test_cache_endpoint,
};
use crate::ctrader_integration::connect_to_ctrader_websocket;
use crate::minimal_zone_cache::{get_minimal_cache, MinimalZoneCache};
use crate::realtime_cache_updater::RealtimeCacheUpdater;
use crate::realtime_zone_monitor::NewRealTimeZoneMonitor; // ← ADD NEW IMPORT
use crate::simple_price_websocket::SimplePriceWebSocketServer;
use crate::terminal_dashboard::TerminalDashboard;

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

// ADD PRICES HANDLER
async fn get_current_prices_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    log::info!("Current prices endpoint called");
    
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        log::info!("Zone monitor available, getting prices");
        let prices = monitor.get_current_prices_by_symbol().await;
        log::info!("Retrieved {} prices", prices.len());
        
        HttpResponse::Ok().json(serde_json::json!({
            "prices": prices,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "count": prices.len()
        }))
    } else {
        log::warn!("Zone monitor not available");
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "error": "Zone monitor not available",
            "prices": {},
            "message": "Check if ENABLE_REALTIME_MONITOR=true"
        }))
    }
}

// TEST PRICES HANDLER
async fn test_prices_handler() -> impl Responder {
    let mut test_prices = HashMap::new();
    test_prices.insert("EURUSD".to_string(), 1.1234);
    test_prices.insert("GBPUSD".to_string(), 1.2567);
    test_prices.insert("USDJPY".to_string(), 143.45);
    
    HttpResponse::Ok().json(serde_json::json!({
        "prices": test_prices,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "count": test_prices.len(),
        "note": "Test prices - replace with real monitor data"
    }))
}

// NEW: TRADE NOTIFICATIONS HANDLERS
async fn get_trade_notifications_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    log::info!("Trade notifications endpoint called");
    
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let notifications = monitor.get_trade_notifications_from_cache().await;
        log::info!("Retrieved {} trade notifications", notifications.len());
        
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "count": notifications.len(),
            "notifications": notifications,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        log::warn!("Zone monitor not available for trade notifications");
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available",
            "notifications": [],
            "message": "Check if ENABLE_REALTIME_MONITOR=true"
        }))
    }
}

async fn clear_trade_notifications_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    log::info!("Clear trade notifications endpoint called");
    
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        monitor.clear_cache_notifications().await;
        log::info!("Trade notifications cleared");
        
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Trade notifications cleared",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        log::warn!("Zone monitor not available for clearing notifications");
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available",
            "message": "Check if ENABLE_REALTIME_MONITOR=true"
        }))
    }
}

// --- Cache Setup Function ---
async fn setup_cache_system() -> Result<
    (
        Arc<Mutex<MinimalZoneCache>>,
        Option<Arc<NewRealTimeZoneMonitor>>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    info!("🚀 [MAIN] Setting up zone cache system...");

    let cache = match get_minimal_cache().await {
        Ok(cache) => {
            let (total, supply, demand) = cache.get_stats();
            info!(
                "✅ [MAIN] Initial cache load complete: {} total zones ({} supply, {} demand)",
                total, supply, demand
            );
            Arc::new(Mutex::new(cache))
        }
        Err(e) => {
            error!("❌ [MAIN] Initial cache load failed: {}", e);
            return Err(e);
        }
    };

    // Set up real-time zone monitor if enabled
    let enable_realtime_monitor = env::var("ENABLE_REALTIME_MONITOR")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        == "true";

    let zone_monitor = if enable_realtime_monitor {
        let event_capacity = env::var("ZONE_EVENT_CAPACITY")
            .unwrap_or_else(|_| "1000".to_string())
            .parse::<usize>()
            .unwrap_or(1000);

        let (monitor, _event_receiver) =
            NewRealTimeZoneMonitor::new(Arc::clone(&cache), event_capacity);

        let monitor = Arc::new(monitor);

        // Initial sync with cache
        if let Err(e) = monitor.sync_with_cache().await {
            error!("❌ [MAIN] Initial zone monitor sync failed: {}", e);
        } else {
            info!("✅ [MAIN] Real-time zone monitor initialized and synced");
        }

        Some(monitor)
    } else {
        info!("⏸️ [MAIN] Real-time zone monitor disabled via ENABLE_REALTIME_MONITOR");
        None
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

        info!(
            "🔄 [MAIN] Starting real-time cache updater (interval: {}s)",
            cache_update_interval
        );

        let mut cache_updater =
            RealtimeCacheUpdater::new(Arc::clone(&cache)).with_interval(cache_update_interval);

        // Add zone monitor to cache updater if available
        if let Some(ref monitor) = zone_monitor {
            cache_updater = cache_updater.with_zone_monitor(Arc::clone(monitor));
            info!("🔗 [MAIN] Cache updater linked with zone monitor");
        }

        if let Err(e) = cache_updater.start().await {
            error!("❌ [MAIN] Failed to start real-time cache updater: {}", e);
            info!("⚠️ [MAIN] Continuing with static cache only");
        } else {
            info!("✅ [MAIN] Real-time cache updater started successfully");
        }
    } else {
        info!("⏸️ [MAIN] Real-time cache updates disabled via ENABLE_REALTIME_CACHE");
    }

    Ok((cache, zone_monitor))
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

    // Setup cache system with zone monitor
    let (shared_cache, zone_monitor) = match setup_cache_system().await {
        Ok(cache) => {
            log::info!("✅ [MAIN] Cache system initialized successfully");
            cache
        }
        Err(e) => {
            log::error!("❌ [MAIN] Failed to setup cache system: {}", e);
            return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
        }
    };

    // Start price feed if enabled
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

        let zone_monitor_clone = zone_monitor.clone(); // Clone the zone monitor

        tokio::spawn(async move {
            log::info!("🚀 [MAIN] Starting cTrader price feed integration with zone monitor");

            if let Some(broadcaster) = broadcaster_clone {
                if let Err(e) = connect_to_ctrader_websocket(&broadcaster, zone_monitor_clone).await
                {
                    log::error!("❌ [MAIN] cTrader price feed failed: {}", e);
                }
            } else {
                log::error!("❌ [MAIN] Price broadcaster not initialized");
            }
        });
    }

    // Start terminal dashboard if enabled
    if let Some(ref monitor) = zone_monitor {
        let enable_terminal_dashboard = env::var("ENABLE_TERMINAL_DASHBOARD")
            .unwrap_or_else(|_| "false".to_string())
            .trim()
            .to_lowercase()
            == "true";

        if enable_terminal_dashboard {
            info!("🎯 [MAIN] Starting terminal dashboard");

            let dashboard = TerminalDashboard::new(Arc::clone(monitor));
            tokio::spawn(async move {
                dashboard.start().await;
            });
        } else {
            info!("📊 [MAIN] Terminal dashboard disabled. Set ENABLE_TERMINAL_DASHBOARD=true to enable");
        }
    }

    // Server configuration
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    log::info!("Web server starting on http://{}:{}", host, port);
    print_server_info(&host, port);

    let http_client_for_app_factory = Arc::clone(&shared_http_client);
    
    // Clone zone_monitor for the HTTP server
    let zone_monitor_for_app = zone_monitor.clone();

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
            .app_data(web::Data::new(zone_monitor_for_app.clone())) // FIXED: now properly available
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
                web::get().to(get_minimal_cache_zones_debug_with_shared_cache),
            )
            // PRICE ENDPOINTS
            .route("/current-prices", web::get().to(get_current_prices_handler))
            .route("/test-prices", web::get().to(test_prices_handler))
            // NEW: TRADE NOTIFICATION ENDPOINTS
            .route("/trade-notifications", web::get().to(get_trade_notifications_handler))
            .route("/trade-notifications/clear", web::post().to(clear_trade_notifications_handler))
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
    println!("  GET  /current-prices");
    println!("  GET  /test-prices");
    println!("  GET  /trade-notifications");
    println!("  POST /trade-notifications/clear");
    println!("--------------------------------------------------");
}