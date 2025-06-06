// src/main.rs - Complete version with all routes

// Module declarations
mod api;
mod cache;
mod data;
mod errors;
mod notifications;
mod realtime;
mod trading;
mod types;
mod zones;
mod handlers; // Add the handlers module

// Standard library imports
use std::env;
use std::sync::Arc;
use std::sync::LazyLock;

// External crate imports
use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use actix_web::middleware::Logger;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use log::{warn, info, error};
use dotenv::dotenv;

// Use tokio::sync::Mutex consistently throughout
use tokio::sync::Mutex;

// Import what you need from your modules
use crate::api::cache_endpoints::{
    get_minimal_cache_zones_debug_with_shared_cache, test_cache_endpoint,
};
use crate::cache::minimal_zone_cache::{get_minimal_cache, MinimalZoneCache};
use crate::realtime::realtime_zone_monitor::NewRealTimeZoneMonitor;
use crate::realtime::simple_price_websocket::SimplePriceWebSocketServer;
use crate::trading::trade_decision_engine::TradeDecisionEngine;
use crate::trading::trade_event_logger::TradeEventLogger;
use crate::notifications::notification_manager::{
    set_global_notification_manager, NotificationManager,
};
use crate::cache::realtime_cache_updater::RealtimeCacheUpdater;
use crate::data::ctrader_integration::connect_to_ctrader_websocket;

// Import API handlers
use crate::api::detect::{
    detect_patterns, 
    get_active_zones_handler, 
    get_bulk_multi_tf_active_zones_handler,
    find_and_verify_zone_handler
};
use crate::trading::backtest::backtest::run_backtest;

// Import handlers from the separate file
use crate::handlers::*;

// --- Global State ---
static GLOBAL_PRICE_BROADCASTER: LazyLock<
    std::sync::Mutex<Option<tokio::sync::broadcast::Sender<String>>>,
> = LazyLock::new(|| std::sync::Mutex::new(None));

static GLOBAL_TRADE_ENGINE: LazyLock<
    std::sync::Mutex<Option<Arc<Mutex<TradeDecisionEngine>>>>,
> = LazyLock::new(|| std::sync::Mutex::new(None));

static GLOBAL_TRADE_LOGGER: LazyLock<
    std::sync::Mutex<Option<Arc<Mutex<TradeEventLogger>>>>,
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

// --- Helper Functions ---
pub fn get_global_trade_engine() -> Option<Arc<Mutex<TradeDecisionEngine>>> {
    GLOBAL_TRADE_ENGINE.lock().unwrap().clone()
}

pub fn get_global_trade_logger() -> Option<Arc<Mutex<TradeEventLogger>>> {
    GLOBAL_TRADE_LOGGER.lock().unwrap().clone()
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

// --- Real-time Trade Processing ---
async fn process_trade_notifications() {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));

    loop {
        interval.tick().await;

        // Get trade engine and logger
        let (trade_engine, trade_logger) = {
            let engine = get_global_trade_engine();
            let logger = get_global_trade_logger();
            (engine, logger)
        };

        if let (Some(engine_arc), Some(logger_arc)) = (trade_engine, trade_logger) {
            // For now, just handle periodic maintenance
            {
                let mut engine_guard = engine_arc.lock().await;
                engine_guard.cleanup_old_triggers();
            }
        }

        // Check for daily rollover in logger
        if let Some(logger_arc) = get_global_trade_logger() {
            let mut logger_guard = logger_arc.lock().await;
            if let Err(e) = logger_guard.force_rollover_check().await {
                error!("Trade logger rollover failed: {}", e);
            }
        }
    }
}

// --- Cache Setup Function ---
async fn setup_cache_system() -> Result<
    (
        Arc<Mutex<MinimalZoneCache>>,
        Option<Arc<NewRealTimeZoneMonitor>>,
        Option<Arc<Mutex<TradeDecisionEngine>>>,
        Option<Arc<Mutex<TradeEventLogger>>>,
        Arc<NotificationManager>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    info!("Setting up trading system...");

    let cache = match get_minimal_cache().await {
        Ok(cache) => {
            let (total, supply, demand) = cache.get_stats();
            info!(
                "Cache loaded: {} zones ({} supply, {} demand)",
                total, supply, demand
            );
            Arc::new(Mutex::new(cache))
        }
        Err(e) => {
            error!("Cache load failed: {}", e);
            return Err(e);
        }
    };

    // Initialize notification manager
    let notification_manager = Arc::new(NotificationManager::new());
    set_global_notification_manager(Arc::clone(&notification_manager));
    info!("📢 Notification manager initialized");

    // Test notifications on startup
    tokio::spawn({
        let manager = Arc::clone(&notification_manager);
        async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            manager.notify_startup().await;
        }
    });

    // Initialize trade event logger
    let trade_logger = match TradeEventLogger::new() {
        Ok(logger) => {
            info!("Trade logger initialized");
            let logger_arc = Arc::new(Mutex::new(logger));
            *GLOBAL_TRADE_LOGGER.lock().unwrap() = Some(Arc::clone(&logger_arc));
            Some(logger_arc)
        }
        Err(e) => {
            error!("Trade logger initialization failed: {}", e);
            None
        }
    };

    // Set up real-time zone monitor
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

        if let Err(e) = monitor.sync_with_cache().await {
            error!("Zone monitor sync failed: {}", e);
        } else {
            info!("Zone monitor initialized");
        }

        Some(monitor)
    } else {
        warn!("Zone monitor disabled");
        None
    };

    // Initialize trade decision engine
    let trading_enabled = env::var("TRADING_ENABLED")
        .unwrap_or_else(|_| "false".to_string())
        .trim()
        .to_lowercase()
        == "true";

    let trade_engine = if trading_enabled {
        let engine = TradeDecisionEngine::new_with_cache(Arc::clone(&cache));
        info!("Trade engine initialized");
        let engine_arc = Arc::new(Mutex::new(engine));
        *GLOBAL_TRADE_ENGINE.lock().unwrap() = Some(Arc::clone(&engine_arc));
        Some(engine_arc)
    } else {
        info!("Trading disabled");
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

        let mut cache_updater =
            RealtimeCacheUpdater::new(Arc::clone(&cache)).with_interval(cache_update_interval);

        if let Some(ref monitor) = zone_monitor {
            cache_updater = cache_updater.with_zone_monitor(Arc::clone(monitor));
        }

        if let Err(e) = cache_updater.start().await {
            error!("Cache updater failed: {}", e);
        } else {
            info!("Cache updater started");
        }
    }

    Ok((
        cache,
        zone_monitor,
        trade_engine,
        trade_logger,
        notification_manager,
    ))
}

// --- Main Application ---
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    log4rs::init_file("log4rs.yaml", Default::default())
        .expect("Failed to initialize log4rs logging");

    info!("Starting Trading Application...");

    let shared_http_client = Arc::new(HttpClient::new());

    // Price WebSocket setup
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

    // Setup main trading system
    let (shared_cache, zone_monitor, _trade_engine, _trade_logger, _notification_manager) =
        match setup_cache_system().await {
            Ok(result) => {
                info!("Trading system initialized");
                result
            }
            Err(e) => {
                error!("Trading system setup failed: {}", e);
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
            }
        };

    // Start price feed connection
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

        let zone_monitor_clone = zone_monitor.clone();

        tokio::spawn(async move {
            if let Some(broadcaster) = broadcaster_clone {
                if let Err(e) = connect_to_ctrader_websocket(&broadcaster, zone_monitor_clone).await
                {
                    error!("Price feed failed: {}", e);
                }
            }
        });
        info!("Price feed connection started");
    }

    // Start trade notification processing
    tokio::spawn(async move {
        process_trade_notifications().await;
    });

    // Server configuration
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    info!("Server starting on http://{}:{}", host, port);

    let http_client_for_app_factory = Arc::clone(&shared_http_client);
    let zone_monitor_for_app = zone_monitor.clone();

    // Start the HTTP server with ALL routes
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
            .app_data(web::Data::new(zone_monitor_for_app.clone()))
            // Core endpoints
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            // Zone endpoints
            .route("/analyze", web::get().to(detect_patterns))
            .route("/active-zones", web::get().to(get_active_zones_handler))
            .route("/bulk-multi-tf-active-zones", web::post().to(get_bulk_multi_tf_active_zones_handler))
            .route("/zone", web::get().to(crate::api::get_zone_by_id::get_zone_by_id))
            .route("/latest-formed-zones", web::get().to(crate::api::latest_zones_handler::get_latest_formed_zones_handler))
            .route("/find-and-verify-zone", web::get().to(find_and_verify_zone_handler))
            // Backtest endpoints
            .route("/backtest", web::post().to(run_backtest))
            // Comment out routes that don't have handlers yet
            // .route("/multi-symbol-backtest", web::post().to(crate::api::multi_backtest_handler::multi_backtest_handler))
            // .route("/optimize-parameters", web::post().to(crate::api::optimize_handler::optimize_handler))
            // .route("/portfolio-meta-backtest", web::post().to(crate::api::optimize_handler::optimize_single_symbol_handler))
            // Debug endpoints
            // .route("/testDataRequest", web::get().to(crate::data::influx_fetcher::fetch_influx_data))
            .route("/test-cache", web::get().to(test_cache_endpoint))
            .route("/debug/minimal-cache-zones", web::get().to(get_minimal_cache_zones_debug_with_shared_cache))
            // Price endpoints (for dashboard and WebSocket testing)
            .route("/current-prices", web::get().to(get_current_prices_handler))
            .route("/test-prices", web::get().to(test_prices_handler))
            // Trade notification endpoints (for dashboard)
            .route("/trade-notifications", web::get().to(get_trade_notifications_handler))
            .route("/trade-notifications/clear", web::post().to(clear_trade_notifications_handler))
            .route("/validated-signals", web::get().to(get_validated_signals_handler))
            .route("/validated-signals/clear", web::post().to(clear_validated_signals_handler))
            // Real-time trading control endpoints
            .route("/trading/status", web::get().to(get_trading_status))
            .route("/trading/validated-signals", web::get().to(get_validated_signals_from_engine))
            .route("/trading/validated-signals/clear", web::post().to(clear_validated_signals_from_engine))
            .route("/trading/reload-rules", web::post().to(reload_trading_rules))
            .route("/trading/test", web::post().to(manual_trade_test))
            // Notification endpoints
            .route("/notifications/test", web::post().to(test_notifications_handler))
            .route("/notifications/daily-summary", web::post().to(send_daily_summary_handler))
            // Proximity endpoints
            .route("/proximity/test", web::post().to(test_proximity_alert_handler))
            .route("/proximity/status", web::get().to(get_proximity_status_handler))
            .route("/proximity/distances", web::get().to(get_proximity_distances_handler))
            .route("/proximity/check-distance", web::get().to(check_proximity_distance_handler))
            .route("/proximity/trigger-check", web::post().to(trigger_proximity_check_handler))
            .route("/debug/proximity-distances", web::get().to(get_proximity_distances_handler))
            .route("/debug/zones-within-proximity", web::get().to(get_zones_within_proximity_handler))
    })
    .bind((host, port))?
    .run();

    server_handle.await
}