// src/main.rs - Clean version focused on real-time trading with reduced logging
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use log::{error, info, warn};
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
mod realtime_cache_updater;
mod realtime_monitor;
mod realtime_zone_monitor;
mod simple_price_websocket;
mod terminal_dashboard;
mod trade_decision_engine;
mod trade_event_logger; // Keep for real-time trade logging
pub mod trades;
pub mod trading;
mod types;
mod websocket_server;
mod zone_detection;

// Add these new module declarations at the top of main.rs after your existing modules:
mod notification_manager;
mod proximity_detector;
mod sound_notifier;
mod telegram_notifier;
// Add these imports after your existing use statements:
use crate::notification_manager::{
    get_global_notification_manager, set_global_notification_manager, NotificationManager,
};

// --- Use necessary types ---
use crate::cache_endpoints::{
    get_minimal_cache_zones_debug_with_shared_cache, test_cache_endpoint,
};
use crate::ctrader_integration::connect_to_ctrader_websocket;
use crate::minimal_zone_cache::{get_minimal_cache, MinimalZoneCache};
use crate::realtime_cache_updater::RealtimeCacheUpdater;
use crate::realtime_zone_monitor::NewRealTimeZoneMonitor;
use crate::simple_price_websocket::SimplePriceWebSocketServer;
use crate::terminal_dashboard::TerminalDashboard;
use crate::trade_decision_engine::TradeDecisionEngine;
use crate::trade_event_logger::TradeEventLogger;

// --- Global State ---
use std::sync::LazyLock;
static GLOBAL_PRICE_BROADCASTER: LazyLock<
    std::sync::Mutex<Option<tokio::sync::broadcast::Sender<String>>>,
> = LazyLock::new(|| std::sync::Mutex::new(None));

static GLOBAL_TRADE_ENGINE: LazyLock<
    std::sync::Mutex<Option<Arc<tokio::sync::Mutex<TradeDecisionEngine>>>>,
> = LazyLock::new(|| std::sync::Mutex::new(None));

static GLOBAL_TRADE_LOGGER: LazyLock<
    std::sync::Mutex<Option<Arc<tokio::sync::Mutex<TradeEventLogger>>>>,
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
pub fn get_global_trade_engine() -> Option<Arc<tokio::sync::Mutex<TradeDecisionEngine>>> {
    GLOBAL_TRADE_ENGINE.lock().unwrap().clone()
}

pub fn get_global_trade_logger() -> Option<Arc<tokio::sync::Mutex<TradeEventLogger>>> {
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

async fn get_current_prices_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let prices = monitor.get_current_prices_by_symbol().await;
        HttpResponse::Ok().json(serde_json::json!({
            "prices": prices,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "count": prices.len()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "error": "Zone monitor not available",
            "prices": {}
        }))
    }
}

async fn test_prices_handler() -> impl Responder {
    let mut test_prices = HashMap::new();
    test_prices.insert("EURUSD".to_string(), 1.1234);
    test_prices.insert("GBPUSD".to_string(), 1.2567);
    test_prices.insert("USDJPY".to_string(), 143.45);

    HttpResponse::Ok().json(serde_json::json!({
        "prices": test_prices,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "count": test_prices.len(),
        "note": "Test prices for WebSocket testing"
    }))
}

async fn get_trade_notifications_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let notifications = monitor.get_trade_notifications_from_cache().await;
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "count": notifications.len(),
            "notifications": notifications,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available",
            "notifications": []
        }))
    }
}

async fn clear_trade_notifications_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        monitor.clear_cache_notifications().await;
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Trade notifications cleared",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available"
        }))
    }
}

async fn get_validated_signals_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let signals = monitor.get_validated_signals().await;
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "count": signals.len(),
            "signals": signals,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available",
            "signals": []
        }))
    }
}

async fn clear_validated_signals_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        monitor.clear_validated_signals().await;
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Validated signals cleared",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available"
        }))
    }
}

// --- Trading Control Handlers ---
async fn get_trading_status() -> impl Responder {
    if let Some(engine_arc) = get_global_trade_engine() {
        let engine_guard = engine_arc.lock().await;
        let rules = engine_guard.get_rules();
        let daily_signals = engine_guard.get_daily_signal_count();

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "trading_enabled": rules.trading_enabled,
            "daily_signal_count": daily_signals,
            "max_daily_signals": rules.max_daily_signals,
            "allowed_symbols": rules.allowed_symbols,
            "allowed_timeframes": rules.allowed_timeframes,
            "min_zone_strength": rules.min_zone_strength,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine not available"
        }))
    }
}

async fn get_validated_signals_from_engine() -> impl Responder {
    if let Some(engine_arc) = get_global_trade_engine() {
        let engine_guard = engine_arc.lock().await;
        let signals = engine_guard.get_validated_signals();

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "count": signals.len(),
            "signals": signals,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine not available"
        }))
    }
}

async fn clear_validated_signals_from_engine() -> impl Responder {
    if let Some(engine_arc) = get_global_trade_engine() {
        let mut engine_guard = engine_arc.lock().await;
        engine_guard.clear_signals();

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Validated signals cleared from engine",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine not available"
        }))
    }
}

async fn manual_trade_test(
    request_body: web::Json<crate::minimal_zone_cache::TradeNotification>,
) -> impl Responder {
    let notification = request_body.into_inner();

    if let (Some(engine_arc), Some(logger_arc)) =
        (get_global_trade_engine(), get_global_trade_logger())
    {
        let mut engine_guard = engine_arc.lock().await;
        let logger_guard = logger_arc.lock().await;

        // Log the incoming notification
        logger_guard.log_notification_received(&notification).await;

        // Process through trade decision engine
        match engine_guard
            .process_notification_with_reason(notification.clone())
            .await
        {
            Ok(validated_signal) => {
                logger_guard.log_signal_validated(&validated_signal).await;
                info!(
                    "📈 Manual test - Trade validated: {} {} @ {:.5}",
                    validated_signal.action, validated_signal.symbol, validated_signal.price
                );

                if let Some(notification_manager) = get_global_notification_manager() {
                    info!("📢 Sending notifications for validated manual test signal");
                    notification_manager
                        .notify_trade_signal(&validated_signal)
                        .await;
                } else {
                    warn!("📢 Notification manager not available for manual test signal");
                }

                HttpResponse::Ok().json(serde_json::json!({
                    "status": "success",
                    "result": "validated",
                    "signal": validated_signal,
                    "message": "Trade notification successfully validated",
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
            Err(reason) => {
                logger_guard
                    .log_signal_rejected(&notification, reason.clone())
                    .await;
                warn!(
                    "📈 Manual test - Trade rejected: {} {} @ {:.5} - {}",
                    notification.action, notification.symbol, notification.price, reason
                );

                HttpResponse::Ok().json(serde_json::json!({
                    "status": "success",
                    "result": "rejected",
                    "reason": reason,
                    "notification": notification,
                    "message": "Trade notification was rejected",
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
        }
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine or logger not available"
        }))
    }
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
            // Process any trade notifications from the zone monitor
            // Note: You'll need to implement a way to get notifications from your zone monitor
            // This is a placeholder showing how it would work once you have that mechanism

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
        Option<Arc<tokio::sync::Mutex<TradeDecisionEngine>>>,
        Option<Arc<tokio::sync::Mutex<TradeEventLogger>>>,
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

    // Test notifications on startup (optional)
    tokio::spawn({
        let manager = Arc::clone(&notification_manager);
        async move {
            // Wait a bit for everything to initialize
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            manager.notify_startup().await;
        }
    });

    // Initialize trade event logger for real-time logging
    let trade_logger = match TradeEventLogger::new() {
        Ok(logger) => {
            info!("Trade logger initialized");
            let logger_arc = Arc::new(tokio::sync::Mutex::new(logger));
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

    // Initialize trade decision engine (fix the constructor call)
    let trading_enabled = env::var("TRADING_ENABLED")
        .unwrap_or_else(|_| "false".to_string())
        .trim()
        .to_lowercase()
        == "true";

    let trade_engine = if trading_enabled {
        let engine = TradeDecisionEngine::new_with_cache(Arc::clone(&cache));
        info!("Trade engine initialized");
        let engine_arc = Arc::new(tokio::sync::Mutex::new(engine));
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
    dotenv::dotenv().ok();
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
    // Update this line to capture the notification manager:
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

    // Start price feed
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
    }

    // Start trade notification processing
    tokio::spawn(async move {
        process_trade_notifications().await;
    });

    // Start terminal dashboard if enabled
    if let Some(ref monitor) = zone_monitor {
        let enable_terminal_dashboard = env::var("ENABLE_TERMINAL_DASHBOARD")
            .unwrap_or_else(|_| "false".to_string())
            .trim()
            .to_lowercase()
            == "true";

        if enable_terminal_dashboard {
            let dashboard = TerminalDashboard::new(Arc::clone(monitor));
            tokio::spawn(async move {
                dashboard.start().await;
            });
            info!("Terminal dashboard started");
        }
    }

    // Server configuration
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    info!("Server starting on http://{}:{}", host, port);

    let http_client_for_app_factory = Arc::clone(&shared_http_client);
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
            .app_data(web::Data::new(zone_monitor_for_app.clone()))
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
            // Debug endpoints
            .route(
                "/testDataRequest",
                web::get().to(influx_fetcher::test_data_request_handler),
            )
            .route("/test-cache", web::get().to(test_cache_endpoint))
            .route(
                "/debug/minimal-cache-zones",
                web::get().to(get_minimal_cache_zones_debug_with_shared_cache),
            )
            // Price endpoints (for dashboard and WebSocket testing)
            .route("/current-prices", web::get().to(get_current_prices_handler))
            .route("/test-prices", web::get().to(test_prices_handler)) // Keep for WebSocket testing
            // Trade notification endpoints (for dashboard)
            .route(
                "/trade-notifications",
                web::get().to(get_trade_notifications_handler),
            )
            .route(
                "/trade-notifications/clear",
                web::post().to(clear_trade_notifications_handler),
            )
            .route(
                "/validated-signals",
                web::get().to(get_validated_signals_handler),
            )
            .route(
                "/validated-signals/clear",
                web::post().to(clear_validated_signals_handler),
            )
            // Real-time trading control endpoints
            .route("/trading/status", web::get().to(get_trading_status))
            .route(
                "/trading/validated-signals",
                web::get().to(get_validated_signals_from_engine),
            )
            .route(
                "/trading/validated-signals/clear",
                web::post().to(clear_validated_signals_from_engine),
            )
            .route(
                "/trading/reload-rules",
                web::post().to(reload_trading_rules),
            )
            .route("/trading/test", web::post().to(manual_trade_test))
            // Add these new routes to your HttpServer::new closure:
            .route(
                "/notifications/test",
                web::post().to(test_notifications_handler),
            )
            .route(
                "/notifications/daily-summary",
                web::post().to(send_daily_summary_handler),
            )
            .route(
                "/proximity/test",
                web::post().to(test_proximity_alert_handler),
            )
            .route(
                "/proximity/status",
                web::get().to(get_proximity_status_handler),
            )
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

async fn reload_trading_rules() -> impl Responder {
    if let Some(engine_arc) = get_global_trade_engine() {
        let mut engine_guard = engine_arc.lock().await;
        engine_guard.reload_rules();

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Trading rules reloaded from environment",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine not available"
        }))
    }
}

// Add this new API endpoint for testing notifications:
async fn test_notifications_handler() -> impl Responder {
    if let Some(manager) = get_global_notification_manager() {
        let (telegram_ok, sound_ok) = manager.test_notifications().await;

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "telegram": telegram_ok,
            "sound": sound_ok,
            "message": format!("Telegram: {}, Sound: {}",
                if telegram_ok { "✅" } else { "❌" },
                if sound_ok { "✅" } else { "❌" }
            ),
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Notification manager not available"
        }))
    }
}

// Add this endpoint to send daily summary:
async fn send_daily_summary_handler() -> impl Responder {
    if let (Some(manager), Some(engine_arc)) =
        (get_global_notification_manager(), get_global_trade_engine())
    {
        let engine_guard = engine_arc.lock().await;
        let signals_today = engine_guard.get_daily_signal_count();
        let max_signals = engine_guard.get_rules().max_daily_signals;
        drop(engine_guard);

        manager.send_daily_summary(signals_today, max_signals).await;

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Daily summary sent",
            "signals_today": signals_today,
            "max_signals": max_signals,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Notification manager or trade engine not available"
        }))
    }
}

async fn test_proximity_alert_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    // This is a test endpoint to manually trigger a proximity alert
    if let Some(notification_manager) = get_global_notification_manager() {
        // Send a test proximity alert
        notification_manager
            .notify_proximity_alert(
                "EURUSD",
                1.12340,
                "supply",
                1.12400,
                1.12300,
                "test_zone_123",
                "4h",
                12.5,
            )
            .await;

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Test proximity alert sent",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Notification manager not available"
        }))
    }
}

// Add this endpoint to get proximity detector status:
async fn get_proximity_status_handler() -> impl Responder {
    let enabled = std::env::var("ENABLE_PROXIMITY_ALERTS")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        == "true";

    let threshold_pips = std::env::var("PROXIMITY_THRESHOLD_PIPS")
        .unwrap_or_else(|_| "15.0".to_string())
        .parse::<f64>()
        .unwrap_or(15.0);

    let cooldown_minutes = std::env::var("PROXIMITY_ALERT_COOLDOWN_MINUTES")
        .unwrap_or_else(|_| "30".to_string())
        .parse::<i64>()
        .unwrap_or(30);

    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "proximity_alerts": {
            "enabled": enabled,
            "threshold_pips": threshold_pips,
            "cooldown_minutes": cooldown_minutes
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

async fn get_proximity_distances_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let proximity_distances = monitor.get_proximity_distances_debug().await;
        
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "total_zones": proximity_distances.len(),
            "zones": proximity_distances,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available"
        }))
    }
}

async fn get_zones_within_proximity_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let close_zones = monitor.get_zones_within_proximity().await;
        
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "zones_within_proximity": close_zones.len(),
            "zones": close_zones,
            "message": if close_zones.is_empty() { 
                "No zones within proximity threshold" 
            } else { 
                "Found zones within proximity threshold" 
            },
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available"
        }))
    }
}

// Endpoint to manually check proximity for a specific symbol and price
async fn check_proximity_distance_handler(
    query: web::Query<ProximityCheckQuery>,
) -> impl Responder {
    let symbol = &query.symbol;
    let current_price = query.price;
    
    // Get proximity threshold from environment
    let proximity_threshold_pips = std::env::var("PROXIMITY_THRESHOLD_PIPS")
        .unwrap_or_else(|_| "15.0".to_string())
        .parse::<f64>()
        .unwrap_or(15.0);
    
    // This is a simplified distance calculation - for full functionality,
    // you'd need to check against actual zones from your cache
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "symbol": symbol,
        "current_price": current_price,
        "proximity_threshold_pips": proximity_threshold_pips,
        "message": "For full zone distance checking, zones need to be loaded from cache",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "note": "This endpoint needs zone cache access to calculate actual distances"
    }))
}

// Query struct for proximity checking
#[derive(serde::Deserialize)]
struct ProximityCheckQuery {
    symbol: String,
    price: f64,
}

// Endpoint to manually trigger proximity check for all zones
async fn trigger_proximity_check_handler() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "message": "Manual proximity check would require integration with your zone monitor",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "suggestion": "Check your zone monitor logs for proximity detection activity"
    }))
}