// src/main.rs - Updated to include real-time zone monitor
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
mod optimize_handler;
mod patterns;
pub mod trades;
pub mod trading;
mod types;
mod zone_detection;
mod zone_generator;
mod zone_lifecycle_updater;
mod zone_monitor_service;
mod zone_revalidator_util;
// NEW: Add real-time monitor modules
mod price_feed;
mod realtime_monitor;
mod websocket_server;

mod minimal_zone_cache;
use minimal_zone_cache::{test_minimal_cache, CacheSymbolConfig, MinimalZoneCache};

// --- Use necessary types ---
use crate::types::StoredZone;
use crate::zone_lifecycle_updater::run_zone_lifecycle_updater_service;
use crate::zone_monitor_service::ActiveZoneCache;
// NEW: Import real-time monitor types
use crate::price_feed::{PriceFeedBridge, PriceUpdate};
use crate::realtime_monitor::{MonitorConfig, RealTimeZoneMonitor, SymbolTimeframeConfig};
use crate::websocket_server::WebSocketServer;

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

// Create a simple query struct just for the UI endpoint
#[derive(serde::Deserialize, Debug)]
pub struct UIActiveZonesQuery {
    #[serde(default)]
    pub max_touch_count: Option<i64>,
}

// In main.rs
// pub async fn get_ui_active_zones_handler(
//     zone_cache: web::Data<ActiveZoneCache>,
//     query: web::Query<UIActiveZonesQuery>,
// ) -> impl Responder {
//     log::info!(
//         "[UI_HANDLER] Request for active zones for UI received. Query: {:?}",
//         query
//     );

//     let cache_guard = zone_cache.lock().await;
//     log::info!("[UI_HANDLER] Cache locked. Size: {}", cache_guard.len());

//     let mut zones_list: Vec<StoredZone> = Vec::new();

//     log::info!(
//         "[UI_HANDLER_CACHE_DUMP] --- START Cache Content for UI (Size: {}) ---",
//         cache_guard.len()
//     );
//     for (cache_key, live_state) in cache_guard.iter() {
//         let zone_from_cache = &live_state.zone_data;

//         log::info!(
//             "[UI_CACHE_READ_CHECK] CacheKey: {:?}, Zone ID: {:?}, Symbol: {:?}, TF: {:?}, Type: {:?}, \
//             CACHE_StartTime: {:?}, CACHE_ZoneHigh: {:?}, CACHE_ZoneLow: {:?}, CACHE_TouchCount: {:?}, \
//             CACHE_IsActive: {}, CACHE_StartIdx: {:?}, CACHE_EndIdx: {:?}",
//             cache_key,
//             zone_from_cache.zone_id,
//             zone_from_cache.symbol,
//             zone_from_cache.timeframe,
//             zone_from_cache.zone_type,
//             zone_from_cache.start_time,
//             zone_from_cache.zone_high,
//             zone_from_cache.zone_low,
//             zone_from_cache.touch_count,
//             zone_from_cache.is_active,
//             zone_from_cache.start_idx,
//             zone_from_cache.end_idx
//         );

//         if zone_from_cache
//             .zone_type
//             .as_deref()
//             .unwrap_or("")
//             .to_lowercase()
//             .contains("supply")
//             && zone_from_cache.zone_high.is_some()
//             && zone_from_cache.zone_high == Some(0.0)
//         {
//             log::error!(
//                 "[UI_CACHE_READ_ERROR_DETECTED] SUPPLY ZONE ID {:?} (CacheKey: {:?}) HAS ZONE_HIGH OF 0.0 WHEN READ FROM CACHE! StoredZone: {:?}",
//                 zone_from_cache.zone_id,
//                 cache_key,
//                 zone_from_cache // Log the whole struct
//             );
//         }
//         if zone_from_cache.touch_count.is_none() {
//             log::warn!(
//                 "[UI_CACHE_READ_WARN_DETECTED] ZONE ID {:?} (CacheKey: {:?}) HAS touch_count of None WHEN READ FROM CACHE! StoredZone: {:?}",
//                 zone_from_cache.zone_id,
//                 cache_key,
//                 zone_from_cache // Log the whole struct
//              );
//         }
//         zones_list.push(zone_from_cache.clone());
//     }
//     log::info!("[UI_HANDLER_CACHE_DUMP] --- END Cache Content for UI ---");

//     let initial_count = zones_list.len();
//     log::info!(
//         "[UI_HANDLER] Extracted {} zones from cache before UI filtering.",
//         initial_count
//     );

//     // Apply touch count filtering if specified
//     if let Some(max_touches) = query.max_touch_count {
//         log::info!(
//             "[UI_HANDLER] Applying touch count filter for UI: max_touches = {}",
//             max_touches
//         );

//         zones_list.retain(|zone| {
//             let tc = zone.touch_count.unwrap_or(0); // Use historical touch_count from DB
//             let zid = zone.zone_id.as_deref().unwrap_or("unknown_id_for_filter");
//             let should_keep = tc <= max_touches;
//             if !should_keep {
//                 log::debug!("[UI_HANDLER] Filtering out zone {} (touches: {}) for UI due to max_touches: {}", zid, tc, max_touches);
//             }
//             should_keep
//         });

//         let filtered_count = zones_list.len();
//         log::info!(
//             "[UI_HANDLER] Touch count filtering for UI complete: {} -> {} zones (max_touches: {})",
//             initial_count,
//             filtered_count,
//             max_touches
//         );
//     } else {
//         log::info!(
//             "[UI_HANDLER] No touch count filtering applied for UI (max_touch_count is None)."
//         );
//     }

//     log::info!(
//         "[UI_HANDLER] Returning {} active zones to UI.",
//         zones_list.len()
//     );

//     let monitor_symbols_for_display =
//         env::var("ZONE_MONITOR_SYMBOLS").unwrap_or_else(|_| "Defaults used by monitor".to_string());
//     let monitor_timeframes_for_display = env::var("ZONE_MONITOR_PATTERNTFS")
//         .unwrap_or_else(|_| "Defaults used by monitor".to_string());
//     let monitor_allowed_days_for_display = env::var("STRATEGY_ALLOWED_DAYS")
//         .unwrap_or_else(|_| "Defaults used by monitor".to_string());

//     HttpResponse::Ok().json(serde_json::json!({
//         "tradeable_zones": zones_list,
//         "monitoring_filters": {
//             "symbols": monitor_symbols_for_display,
//             "patternTimeframes": monitor_timeframes_for_display,
//             "allowedTradeDays": monitor_allowed_days_for_display,
//             "applied_max_touch_count": query.max_touch_count,
//             "total_zones_before_filter": initial_count, // Count from cache before UI filter
//             "zones_returned_after_filter": zones_list.len() // Count after UI filter
//         }
//     }))
// }

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
            let influx_host =
                env::var("INFLUXDB_HOST").expect("INFLUXDB_HOST not set for startup cleanup");
            let influx_org =
                env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG not set for startup cleanup");
            let influx_token =
                env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN not set for startup cleanup");
            let zone_bucket = env::var("GENERATOR_WRITE_BUCKET")
                .expect("GENERATOR_WRITE_BUCKET not set for startup cleanup");
            let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT")
                .expect("GENERATOR_ZONE_MEASUREMENT not set for startup cleanup");
            let age_threshold_days = env::var("STUCK_ZONE_CLEANUP_AGE_DAYS")
                .unwrap_or_else(|_| "2".to_string())
                .parse::<u32>()
                .unwrap_or(2);

            match admin_handlers::deactivate_stuck_initial_zones(
                &client_for_cleanup,
                &influx_host,
                &influx_org,
                &influx_token,
                &zone_bucket,
                &zone_measurement,
                age_threshold_days,
            )
            .await
            {
                Ok(results) => {
                    log::info!("[MAIN_STARTUP_CLEANUP] Stuck zone deactivation completed. {} zones processed. Summary: {:?}", results.len(), results);
                }
                Err(e) => {
                    log::error!(
                        "[MAIN_STARTUP_CLEANUP] Stuck zone deactivation failed: {}",
                        e
                    );
                }
            }
        });
    } else {
        log::info!("[MAIN_STARTUP_CLEANUP] Skipping stuck zone deactivation at startup (RUN_STUCK_ZONE_DEACTIVATION_ON_STARTUP is not 'true').");
    }

    let active_zones_shared_cache: zone_monitor_service::ActiveZoneCache =
        Arc::new(Mutex::new(HashMap::new())); // Cache is still needed for UI even if monitor is off

    let enable_zone_monitor_env =
        env::var("ENABLE_ZONE_MONITOR_SERVICE").unwrap_or_else(|_| "true".to_string()); // Default to true if not set
    let enable_zone_monitor = enable_zone_monitor_env.trim().to_lowercase() == "true";

    if enable_zone_monitor {
        let cache_for_monitor = Arc::clone(&active_zones_shared_cache);
        tokio::spawn(zone_monitor_service::run_zone_monitor_service(
            cache_for_monitor,
        ));
        log::info!("[MAIN] Zone Monitor Service SPAWNED (ENABLE_ZONE_MONITOR_SERVICE=true).");
    } else {
        log::info!(
            "[MAIN] Zone Monitor Service is DISABLED via ENABLE_ZONE_MONITOR_SERVICE='{}'.",
            enable_zone_monitor_env
        );
    }

    // ==================== NEW: REAL-TIME ZONE MONITOR SETUP ====================

    let enable_realtime_monitor_env =
        env::var("ENABLE_REALTIME_MONITOR").unwrap_or_else(|_| "false".to_string()); // Default to false for now
    let enable_realtime_monitor = enable_realtime_monitor_env.trim().to_lowercase() == "true";

    let mut realtime_monitor_handle = None;
    let mut websocket_server_handle = None;
    let mut price_feed_handle = None;

    if enable_realtime_monitor {
        log::info!("🚀 [MAIN] Real-time Zone Monitor is ENABLED. Starting components...");

        // Configuration for real-time monitor
        let realtime_config = MonitorConfig {
            refresh_interval_secs: env::var("REALTIME_REFRESH_INTERVAL_SECS")
                .unwrap_or_else(|_| "120".to_string())
                .parse::<u64>()
                .unwrap_or(120),
            price_tolerance_pips: env::var("REALTIME_PRICE_TOLERANCE_PIPS")
                .unwrap_or_else(|_| "0.5".to_string())
                .parse::<f64>()
                .unwrap_or(0.5),
            max_events_per_second: 100,
            monitored_symbols: vec![
                SymbolTimeframeConfig {
                    symbol: "EURUSD".to_string(),
                    timeframes: vec!["1h".to_string(), "4h".to_string(), "1d".to_string()],
                },
                SymbolTimeframeConfig {
                    symbol: "GBPUSD".to_string(),
                    timeframes: vec!["1h".to_string(), "4h".to_string(), "1d".to_string()],
                },
                SymbolTimeframeConfig {
                    symbol: "USDJPY".to_string(),
                    timeframes: vec!["1h".to_string(), "4h".to_string(), "1d".to_string()],
                },
                SymbolTimeframeConfig {
                    symbol: "USDCHF".to_string(),
                    timeframes: vec!["1h".to_string(), "4h".to_string(), "1d".to_string()],
                },
                SymbolTimeframeConfig {
                    symbol: "AUDUSD".to_string(),
                    timeframes: vec!["1h".to_string(), "4h".to_string(), "1d".to_string()],
                },
            ],
        };

        // Create channels for price updates
        let (price_sender, price_receiver) = tokio::sync::mpsc::channel::<PriceUpdate>(1000);

        // Create real-time zone monitor
        match RealTimeZoneMonitor::new(realtime_config.clone(), price_receiver) {
            Ok((zone_monitor, zone_event_receiver)) => {
                log::info!("✅ [MAIN] Real-time Zone Monitor created successfully");

                // Create price feed bridge
                let price_bridge = Arc::new(PriceFeedBridge::new(price_sender.clone()));

                // Wrap zone monitor in Arc<Mutex> for sharing
                let zone_monitor_shared = Arc::new(tokio::sync::Mutex::new(zone_monitor));

                // Start WebSocket server for clients
                let ws_port = env::var("REALTIME_WS_PORT")
                    .unwrap_or_else(|_| "8082".to_string())
                    .parse::<u16>()
                    .unwrap_or(8082);

                let ws_addr = format!("127.0.0.1:{}", ws_port);

                // Pass the zone monitor reference to WebSocket server
                let mut ws_server = WebSocketServer::new(
                    zone_event_receiver,
                    Some(Arc::clone(&zone_monitor_shared)),
                );

                websocket_server_handle = Some(tokio::spawn(async move {
                    match ws_addr.parse() {
                        Ok(addr) => {
                            log::info!("📡 [MAIN] Starting WebSocket server on {}", addr);
                            if let Err(e) = ws_server.start(addr).await {
                                log::error!("❌ [MAIN] WebSocket server failed: {}", e);
                            }
                        }
                        Err(e) => {
                            log::error!("❌ [MAIN] Invalid WebSocket address '{}': {}", ws_addr, e);
                        }
                    }
                }));

                // Start real-time monitor
                let monitor_clone = Arc::clone(&zone_monitor_shared);

                realtime_monitor_handle = Some(tokio::spawn(async move {
                    let mut monitor = monitor_clone.lock().await;
                    log::info!("🔍 [MAIN] Starting Real-time Zone Monitor");
                    if let Err(e) = monitor.start().await {
                        log::error!("❌ [MAIN] Real-time Zone Monitor failed: {}", e);
                    }
                }));

                // Start price feed integration (connect to your TypeScript WebSocket)
                let enable_price_feed = env::var("ENABLE_CTRADER_PRICE_FEED")
                    .unwrap_or_else(|_| "true".to_string())
                    .trim()
                    .to_lowercase()
                    == "true";

                if enable_price_feed {
                    let price_bridge_clone = Arc::clone(&price_bridge);
                    price_feed_handle = Some(tokio::spawn(async move {
                        log::info!("💹 [MAIN] Starting cTrader price feed integration");
                        if let Err(e) = connect_to_ctrader_websocket(price_bridge_clone).await {
                            log::error!("❌ [MAIN] cTrader price feed failed: {}", e);
                        }
                    }));
                } else {
                    log::info!("⏸️  [MAIN] cTrader price feed integration is DISABLED");
                }

                log::info!("🎯 [MAIN] Real-time Zone Monitor components started:");
                log::info!("   📡 WebSocket server: ws://127.0.0.1:{}", ws_port);
                log::info!(
                    "   🔍 Zone refresh interval: {} seconds",
                    realtime_config.refresh_interval_secs
                );
                log::info!(
                    "   💹 Price feed: {}",
                    if enable_price_feed {
                        "ENABLED"
                    } else {
                        "DISABLED"
                    }
                );
            }
            Err(e) => {
                log::error!("❌ [MAIN] Failed to create Real-time Zone Monitor: {}", e);
            }
        }
    } else {
        log::info!(
            "⏸️  [MAIN] Real-time Zone Monitor is DISABLED via ENABLE_REALTIME_MONITOR='{}'",
            enable_realtime_monitor_env
        );
    }

    // ==================== END: REAL-TIME ZONE MONITOR SETUP ====================

    let enable_stale_checker = env::var("STALE_ZONE_CHECKER_ON")
        .map(|val| val.trim().to_lowercase() == "true")
        .unwrap_or(false);

    if enable_stale_checker {
        let http_client_for_stale_checker = Arc::clone(&shared_http_client);
        tokio::spawn(run_zone_lifecycle_updater_service(
            http_client_for_stale_checker.clone(),
        ));
        log::info!("[MAIN] Stale Zone Check Service spawned.");
    } else {
        log::info!(
            "[MAIN] Stale Zone Check Service is DISABLED via STALE_ZONE_CHECKER_ON='{}'.",
            env::var("STALE_ZONE_CHECKER_ON").unwrap_or_else(|_| "not set or false".to_string())
        );
    }
    use log::{info, error,};

    info!("🚀 Testing minimal zone cache...");
    if let Err(e) = test_minimal_cache().await {
        error!("❌ Cache test failed: {}", e);
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
    println!("  POST /multi-symbol-backtest");
    println!("  POST /optimize-parameters");
    println!("  POST /portfolio-meta-backtest");
    println!("  GET  /testDataRequest");
    println!("  GET  /zone?zone_id=...");
    println!("  GET  /latest-formed-zones");
    println!("  GET  /ui/active-zones");
    println!("  POST /admin/revalidate-zone (Body: {{\"zone_id\": \"...\"}})");
    println!("  POST /admin/deactivate-stuck-zones (Optional Body: {{\"age_threshold_days\": N}})");
    println!("  GET  /echo?... (Example)");
    println!("  GET  /health");
    println!("  POST /generate-zones");
    if enable_realtime_monitor {
        println!(
            "  📡 Real-time WebSocket: ws://127.0.0.1:{}",
            env::var("REALTIME_WS_PORT").unwrap_or_else(|_| "8082".to_string())
        );
    }
    println!("--------------------------------------------------");

    // --- Background task for API-triggered zone generation ---
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

    // --- Background task for periodic zone generation ---
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
            .app_data(web::Data::new(Arc::clone(&active_zones_shared_cache)))
            .app_data(web::Data::new(Arc::clone(&http_client_for_app_factory)))
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
                "/optimize-parameters",
                web::post().to(optimize_handler::run_parameter_optimization),
            )
            .route(
                "/portfolio-meta-backtest",
                web::post().to(optimize_handler::run_portfolio_meta_optimized_backtest),
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
            // .route(
            //     "/ui/active-zones",
            //     web::get().to(get_ui_active_zones_handler),
            // )
            .route(
                "/find-and-verify-zone",
                web::get().to(detect::find_and_verify_zone_handler),
            )
            .route(
                "/admin/revalidate-zone",
                web::post().to(admin_handlers::handle_revalidate_zone_request),
            )
            .route(
                "/admin/deactivate-stuck-zones",
                web::post().to(admin_handlers::handle_deactivate_stuck_zones_request),
            )
            .route("/test-cache", web::get().to(test_cache_endpoint))
    })
    .bind((host, port))?
    .run();

    // Wait for server or any background task to complete
    tokio::select! {
        result = server_handle => {
            log::info!("HTTP server ended: {:?}", result);
            result
        }
        result = async {
            if let Some(handle) = realtime_monitor_handle {
                handle.await
            } else {
                std::future::pending().await
            }
        } => {
            log::info!("Real-time monitor ended: {:?}", result);
            Ok(())
        }
        result = async {
            if let Some(handle) = websocket_server_handle {
                handle.await
            } else {
                std::future::pending().await
            }
        } => {
            log::info!("WebSocket server ended: {:?}", result);
            Ok(())
        }
        result = async {
            if let Some(handle) = price_feed_handle {
                handle.await
            } else {
                std::future::pending().await
            }
        } => {
            log::info!("Price feed ended: {:?}", result);
            Ok(())
        }
    }
}

// ==================== CTRADER WEBSOCKET INTEGRATION ====================

async fn connect_to_ctrader_websocket(
    price_bridge: Arc<PriceFeedBridge>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let ctrader_ws_url =
        env::var("CTRADER_WS_URL").unwrap_or_else(|_| "ws://localhost:8081".to_string());

    log::info!(
        "🔌 [PRICE_FEED] Connecting to cTrader WebSocket at {}",
        ctrader_ws_url
    );

    let (ws_stream, _) = connect_async(&ctrader_ws_url).await?;
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Subscribe to all symbols we're monitoring
    let symbols_to_subscribe = vec![
        (185, "EURUSD_SB"),
        (199, "GBPUSD_SB"),
        (226, "USDJPY_SB"),
        (222, "USDCHF_SB"),
        (158, "AUDUSD_SB"),
        (221, "USDCAD_SB"),
        (211, "NZDUSD_SB"),
        (175, "EURGBP_SB"),
        (177, "EURJPY_SB"),
        (173, "EURCHF_SB"),
        // Add more symbols as needed from your CURRENCIES_MAP
    ];

    let timeframes = vec!["1h", "4h", "1d"];

    // Send subscription requests
    for (symbol_id, symbol_name) in &symbols_to_subscribe {
        for timeframe in &timeframes {
            let subscribe_msg = json!({
                "type": "SUBSCRIBE",
                "symbolId": symbol_id,
                "timeframe": timeframe
            });

            if let Err(e) = ws_sender
                .send(Message::Text(subscribe_msg.to_string()))
                .await
            {
                log::error!(
                    "❌ [PRICE_FEED] Failed to send subscription for {}/{}: {}",
                    symbol_name,
                    timeframe,
                    e
                );
            } else {
                log::debug!(
                    "📡 [PRICE_FEED] Subscribed to {}/{} ({})",
                    symbol_name,
                    timeframe,
                    symbol_id
                );
            }

            // Small delay between subscriptions to avoid overwhelming the server
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    log::info!("✅ [PRICE_FEED] Sent all subscription requests to cTrader WebSocket");

    // Process incoming messages
    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(e) = process_ctrader_message(&price_bridge, &text).await {
                    log::warn!("⚠️  [PRICE_FEED] Error processing cTrader message: {}", e);
                }
            }
            Ok(Message::Close(_)) => {
                log::warn!("🔌 [PRICE_FEED] cTrader WebSocket connection closed");
                break;
            }
            Ok(Message::Ping(payload)) => {
                if let Err(e) = ws_sender.send(Message::Pong(payload)).await {
                    log::error!("❌ [PRICE_FEED] Failed to send pong: {}", e);
                }
            }
            Err(e) => {
                log::error!("❌ [PRICE_FEED] cTrader WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    log::warn!("🔌 [PRICE_FEED] cTrader WebSocket connection ended");
    Ok(())
}

async fn process_ctrader_message(
    price_bridge: &Arc<PriceFeedBridge>,
    message: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data: serde_json::Value = serde_json::from_str(message)?;

    match data.get("type").and_then(|t| t.as_str()) {
        Some("BAR_UPDATE") => {
            if let Some(bar_data) = data.get("data") {
                let symbol_id = bar_data["symbolId"].as_u64().unwrap_or(0) as u32;
                let timeframe = bar_data["timeframe"].as_str().unwrap_or("");
                let close = bar_data["close"].as_f64().unwrap_or(0.0);
                let is_new_bar = bar_data["isNewBar"].as_bool().unwrap_or(false);

                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                price_bridge
                    .handle_price_update(symbol_id, timeframe, close, timestamp, is_new_bar)
                    .await?;

                if is_new_bar {
                    log::debug!(
                        "📊 [PRICE_FEED] New bar: {} {} @ {:.5}",
                        symbol_id,
                        timeframe,
                        close
                    );
                }
            }
        }
        Some("SUBSCRIPTION_CONFIRMED") => {
            if let (Some(symbol_id), Some(timeframe)) = (
                data.get("symbolId").and_then(|s| s.as_u64()),
                data.get("timeframe").and_then(|t| t.as_str()),
            ) {
                log::debug!(
                    "✅ [PRICE_FEED] Subscription confirmed: {}/{}",
                    symbol_id,
                    timeframe
                );
            }
        }
        Some("CONNECTED") => {
            log::info!("🔗 [PRICE_FEED] Connected to cTrader WebSocket");
        }
        Some("ERROR") => {
            log::error!("❌ [PRICE_FEED] cTrader error: {:?}", data);
        }
        _ => {
            log::trace!(
                "📨 [PRICE_FEED] Unknown message type: {:?}",
                data.get("type")
            );
        }
    }

    Ok(())
}
use crate::types::{BulkActiveZonesResponse, BulkResultItem, BulkResultData, ChartQuery, EnrichedZone};
// Add this endpoint to test your cache
async fn test_cache_endpoint() -> impl Responder {
    log::info!("[TEST_CACHE_ENDPOINT] Received request for /test-cache (100% identical response test mode)");

    // Define the symbols and timeframes for the cache to load.
    // MUST MATCH THE EXACT POSTMAN BODY for the 100% identical test
    let symbols_for_cache_config = vec![
        CacheSymbolConfig {
            symbol: "EURUSD".to_string(), 
            timeframes: vec!["1h".to_string(), "4h".to_string()],
        },
        // If your Postman request includes more symbols/timeframes, add them here identically.
    ];

    // Log the config being used for the cache
    log::debug!("[TEST_CACHE_ENDPOINT] Initializing cache with config: {:?}", symbols_for_cache_config);

    let mut cache = match MinimalZoneCache::new(symbols_for_cache_config.clone()) {
        Ok(c) => c,
        Err(e) => {
            log::error!("[TEST_CACHE_ENDPOINT] Failed to create MinimalZoneCache: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to create cache: {}", e)
            }));
        }
    };

    // MinimalZoneCache::refresh_zones() will now use the hardcoded dates internally.
    if let Err(e) = cache.refresh_zones().await {
        log::error!("[TEST_CACHE_ENDPOINT] Failed to refresh zones in MinimalZoneCache: {}", e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to refresh zones: {}", e)
        }));
    }
    log::info!("[TEST_CACHE_ENDPOINT] Cache refresh complete. Total zones in cache: {}", cache.get_all_zones().len());


    let all_enriched_zones_from_cache: Vec<EnrichedZone> = cache.get_all_zones();
    log::debug!("[TEST_CACHE_ENDPOINT] Retrieved {} zones from cache. First few: {:?}",
        all_enriched_zones_from_cache.len(),
        all_enriched_zones_from_cache.iter().take(2).collect::<Vec<_>>()
    );


    // Group zones by symbol and then by timeframe
    // The symbol in EnrichedZone should now be "EURUSD" (not "EURUSD_SB") 
    // because we set ZoneDetectionRequest.symbol to the non-SB version.
    let mut grouped_by_symbol_tf: HashMap<String, HashMap<String, (Vec<EnrichedZone>, Vec<EnrichedZone>)>> = HashMap::new();

    for zone in all_enriched_zones_from_cache {
        let symbol_key = zone.symbol.clone().unwrap_or_else(|| {
            log::warn!("[TEST_CACHE_ENDPOINT] Zone found with no symbol: {:?}", zone.zone_id);
            "UNKNOWN_SYMBOL".to_string()
        });
        let timeframe_key = zone.timeframe.clone().unwrap_or_else(|| {
            log::warn!("[TEST_CACHE_ENDPOINT] Zone found with no timeframe: {:?}", zone.zone_id);
            "UNKNOWN_TIMEFRAME".to_string()
        });

        // Log the zone being grouped
        log::trace!("[TEST_CACHE_ENDPOINT] Grouping zone: ID {:?}, Symbol '{}', TF '{}', Type {:?}", 
            zone.zone_id, symbol_key, timeframe_key, zone.zone_type);

        let symbol_entry = grouped_by_symbol_tf.entry(symbol_key).or_default();
        let timeframe_entry = symbol_entry.entry(timeframe_key).or_insert_with(|| (Vec::new(), Vec::new()));

        if zone.zone_type.as_deref().unwrap_or("").contains("supply") {
            timeframe_entry.0.push(zone);
        } else if zone.zone_type.as_deref().unwrap_or("").contains("demand") {
            timeframe_entry.1.push(zone);
        } else {
            log::warn!("[TEST_CACHE_ENDPOINT] Zone {:?} has unknown type: {:?}", zone.zone_id, zone.zone_type);
        }
    }
    log::debug!("[TEST_CACHE_ENDPOINT] Grouped zones map: {:?}", grouped_by_symbol_tf.keys());


    // Construct BulkResultItems, ensuring the order matches symbols_for_cache_config
    let mut bulk_results: Vec<BulkResultItem> = Vec::new();

    for config_item in &symbols_for_cache_config { // Iterate based on input config to try and match order
        let symbol_for_item = &config_item.symbol; // This is "EURUSD"

        // Check if this symbol was actually processed and has entries in the grouped map
        if let Some(timeframe_map_for_symbol) = grouped_by_symbol_tf.get(symbol_for_item) {
            for timeframe_for_item in &config_item.timeframes { // Iterate based on input config
                if let Some((supply_zones, demand_zones)) = timeframe_map_for_symbol.get(timeframe_for_item) {
                    log::debug!("[TEST_CACHE_ENDPOINT] For {}/{}: Found {} supply, {} demand zones in grouped map.",
                        symbol_for_item, timeframe_for_item, supply_zones.len(), demand_zones.len());
                    bulk_results.push(BulkResultItem {
                        symbol: symbol_for_item.clone(),
                        timeframe: timeframe_for_item.clone(),
                        status: "Success".to_string(),
                        data: Some(BulkResultData {
                            supply_zones: supply_zones.clone(),
                            demand_zones: demand_zones.clone(),
                        }),
                        error_message: None,
                    });
                } else {
                    // This case means the symbol was found, but this specific timeframe for it wasn't (e.g. no zones)
                    log::warn!("[TEST_CACHE_ENDPOINT] For {}/{}: No data found in grouped map (expected timeframe missing). Adding empty entry.",
                        symbol_for_item, timeframe_for_item);
                    bulk_results.push(BulkResultItem {
                        symbol: symbol_for_item.clone(),
                        timeframe: timeframe_for_item.clone(),
                        status: "Success".to_string(), 
                        data: Some(BulkResultData {
                            supply_zones: Vec::new(),
                            demand_zones: Vec::new(),
                        }),
                        error_message: None, // Or a message like "No zones found for this timeframe"
                    });
                }
            }
        } else {
            // This case means the entire symbol from config wasn't found in grouped data
            // (e.g., fetch failed for all its timeframes, or no zones at all for that symbol)
            log::warn!("[TEST_CACHE_ENDPOINT] Symbol {} from config not found in grouped_by_symbol_tf. Adding empty entries for its timeframes.", symbol_for_item);
            for timeframe_for_item in &config_item.timeframes {
                bulk_results.push(BulkResultItem {
                    symbol: symbol_for_item.clone(),
                    timeframe: timeframe_for_item.clone(),
                    status: "Success".to_string(), // Or "No Data" if more appropriate
                    data: Some(BulkResultData {
                        supply_zones: Vec::new(),
                        demand_zones: Vec::new(),
                    }),
                    error_message: None, // Or Some("No data found for this symbol.".to_string())
                });
            }
        }
    }
    log::debug!("[TEST_CACHE_ENDPOINT] Constructed {} bulk_results items.", bulk_results.len());


    // Construct the ChartQuery for the query_params field.
    // MUST MATCH THE EXACT POSTMAN QUERY PARAMETERS for the 100% identical test
    let query_params_for_response = ChartQuery {
        start_time: "2025-05-22T00:00:00Z".to_string(),
        end_time: "2025-05-29T23:59:59Z".to_string(),
        symbol: "DUMMY".to_string(),   // As in your Postman example's query params
        timeframe: "DUMMY".to_string(),// As in your Postman example's query params
        pattern: "fifty_percent_before_big_bar".to_string(), // As in your Postman
        enable_trading: None,
        lot_size: None,
        stop_loss_pips: None,
        take_profit_pips: None,
        enable_trailing_stop: None,
        max_touch_count: None, // Postman query has no max_touch_count
    };
    log::debug!("[TEST_CACHE_ENDPOINT] Using query_params_for_response: {:?}", query_params_for_response);


    let response_payload = BulkActiveZonesResponse {
        results: bulk_results,
        query_params: Some(query_params_for_response),
        symbols: HashMap::new(), // As per your Postman example response (empty for this type of bulk request)
    };

    log::info!("[TEST_CACHE_ENDPOINT] Sending response for /test-cache.");
    HttpResponse::Ok().json(response_payload)
}
