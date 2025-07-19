// src/bin/zone_monitor/main.rs
// Clean, organized zone monitor entry point with notifications and CSV logging

mod active_order_manager;
mod booked_trades;
mod csv_logger;
mod db;
mod html;
mod notifications;
mod pending_order_manager;
mod proximity;
mod proximity_logger;
mod sound_notifier;
mod state;
mod strategy_manager;
mod telegram_notifier;
mod trade_rules;
mod trading_engine;
mod types;
mod websocket;
mod zone_state_manager;
mod trading_plan;
mod friday_closer;
mod deadman;

mod enriched_trades;
use chrono::Utc;
use uuid::Uuid;

// Add this import to your use statements
// use enriched_trades::{enriched_trades_by_date_api, enriched_trades_by_range_api};

// Add these imports to the use statements:

use axum::{
    extract::State, http::StatusCode, response::Html, routing::get, routing::post, Json, Router,
};
use booked_trades::{booked_trades_api, booked_trades_html};
use serde::{Deserialize, Serialize};
use state::MonitorState;
use std::fs;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use types::{ZoneAlert, ZoneCache};

use std::collections::HashMap;
use serde_json::{json, Value};
use axum::extract::{Path, Query};

use crate::db::order_db::EnrichedDeal;
use axum::extract::Json as AxumJson;
use tokio::fs::read_to_string;
use trading_plan::TradingPlan;


// Request/Response structs for manual trading
#[derive(Debug, Deserialize, Serialize)]
struct ManualTradeRequest {
    symbol_id: i32,
    trade_side: i32, // 1 = BUY, 2 = SELL
    volume: f64,
    entry_price: f64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
}

#[derive(Debug, Serialize)]
struct ManualTradeResponse {
    success: bool,
    message: String,
    order_id: Option<String>,
    error_code: Option<String>,
    request_sent: ManualTradeRequest,
}

// API endpoint handlers
async fn zones_api(State(state): State<MonitorState>) -> Json<ZoneCache> {
    let cache = state.get_zone_cache().await;
    Json(cache)
}

async fn alerts_api(State(state): State<MonitorState>) -> Json<Vec<ZoneAlert>> {
    let alerts = state.get_recent_alerts().await;
    Json(alerts)
}

async fn notifications_status_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let status = state.get_notification_status().await;
    Json(status)
}

async fn test_notification_api(
    State(state): State<MonitorState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let success = state.send_test_notification().await;

    let response = serde_json::json!({
        "success": success,
        "message": if success {
            "Test notification sent successfully"
        } else {
            "Failed to send test notification"
        }
    });

    let status_code = if success {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status_code, Json(response))
}

async fn trading_stats_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let stats = state.get_trading_stats().await;
    let config = state.get_trading_config().await;
    let active_trades = state.get_active_trades().await;

    let response = serde_json::json!({
        "stats": stats,
        "config": config,
        "active_trades_count": active_trades.len(),
        "active_trades": active_trades,
    });

    Json(response)
}

async fn trade_history_api(
    State(state): State<MonitorState>,
) -> Json<Vec<crate::trading_engine::Trade>> {
    let history = state.get_trade_history().await;
    Json(history)
}

async fn zone_stats_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let stats = state.get_zone_stats().await;
    let thresholds = state.get_thresholds().await;

    let response = serde_json::json!({
        "zone_stats": stats,
        "proximity_threshold_pips": thresholds.0,
        "trading_threshold_pips": thresholds.1,
    });

    Json(response)
}

async fn zone_interactions_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let interactions = state.get_zone_interactions().await;
    Json(serde_json::json!(interactions))
}

async fn cooldown_stats_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let stats = state.get_cooldown_stats().await;
    Json(stats)
}

async fn strategies_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let stats = state.strategy_manager.get_strategy_stats().await;
    Json(serde_json::json!(stats))
}

async fn strategies_by_symbol_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    match state.strategy_manager.load_strategies_from_file().await {
        Ok(strategies_file) => Json(serde_json::json!({
            "strategies_by_symbol": strategies_file.strategies_by_symbol,
            "total_strategies": strategies_file.total_strategies,
            "symbols_with_strategies": strategies_file.strategies_by_symbol.len()
        })),
        Err(_) => Json(serde_json::json!({
            "strategies_by_symbol": {},
            "total_strategies": 0,
            "symbols_with_strategies": 0
        })),
    }
}

async fn refresh_strategies_api(
    State(state): State<MonitorState>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.strategy_manager.refresh_strategies().await {
        Ok(count) => {
            let response = serde_json::json!({
                "success": true,
                "message": format!("Successfully refreshed {} strategies", count),
                "count": count
            });
            (StatusCode::OK, Json(response))
        }
        Err(e) => {
            let response = serde_json::json!({
                "success": false,
                "message": format!("Failed to refresh strategies: {}", e),
                "error": e
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(response))
        }
    }
}

async fn health_check(State(state): State<MonitorState>) -> (StatusCode, Json<serde_json::Value>) {
    let connected = state.get_connection_status().await;
    let cache = state.get_zone_cache().await;
    let prices = state.get_latest_prices().await;
    let alerts = state.get_recent_alerts().await;
    let notifications = state.get_notification_status().await;

    let status = serde_json::json!({
        "status": if connected { "healthy" } else { "disconnected" },
        "websocket_connected": connected,
        "total_zones": cache.total_zones,
        "active_symbols": cache.zones.len(),
        "live_prices": prices.len(),
        "recent_alerts": alerts.len(),
        "notifications": notifications,
        "cache_file": state.cache_file_path,
        "websocket_url": state.websocket_url,
        "timestamp": chrono::Utc::now()
    });

    let status_code = if connected {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status_code, Json(status))
}

async fn pending_orders_html() -> Html<String> {
    match fs::read_to_string("pending_orders_viewer.html") {
        Ok(content) => Html(content),
        Err(_) => Html("<h1>Error: pending_orders_viewer.html not found</h1>".to_string()),
    }
}

// Replace the old implementation of pending_orders_json with Redis-backed logic
async fn pending_orders_json(State(state): State<MonitorState>) -> (StatusCode, Json<serde_json::Value>) {
    let pending_order_manager = state.pending_order_manager.read().await;
    match pending_order_manager.get_all_pending_orders_with_lookup_status().await {
        Ok(orders_info) => (StatusCode::OK, Json(orders_info)),
        Err(e) => {
            let error = serde_json::json!({
                "error": e.to_string(),
                "message": "Failed to fetch pending orders from Redis"
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error))
        }
    }
}

async fn manual_trade_api(
    State(state): State<MonitorState>,
    Json(request): Json<ManualTradeRequest>,
) -> (StatusCode, Json<ManualTradeResponse>) {
    info!("🧪 Manual trade request: {:?}", request);

    let ctrader_api_url = std::env::var("CTRADER_API_BRIDGE_URL")
        .unwrap_or_else(|_| "http://localhost:8000".to_string());

    let api_request = serde_json::json!({
        "symbolId": request.symbol_id,
        "tradeSide": request.trade_side,
        "volume": request.volume,
        "entryPrice": request.entry_price,
        "stopLoss": request.stop_loss,
        "takeProfit": request.take_profit
    });

    info!("🧪 Sending to cTrader API: {}", api_request);

    let client = reqwest::Client::new();
    let api_url = format!("{}/placePendingOrder", ctrader_api_url);

    match client.post(&api_url).json(&api_request).send().await {
        Ok(response) => {
            let response_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read response".to_string());
            info!("🧪 cTrader API response: {}", response_text);

            match serde_json::from_str::<serde_json::Value>(&response_text) {
                Ok(json_response) => {
                    let success = json_response
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let message = json_response
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("No message")
                        .to_string();
                    let order_id = json_response
                        .get("orderId")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let error_code = json_response
                        .get("errorCode")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    // --- Store in Redis if successful ---
                    if success && order_id.is_some() {
                        let mut pending_order_manager = state.pending_order_manager.write().await;
                        let zone_id = Uuid::new_v4().to_string();
                        let pending_order = crate::pending_order_manager::PendingOrder {
                            zone_id,
                            symbol: symbol_id_to_name(request.symbol_id),
                            timeframe: "manual".to_string(),
                            zone_type: "manual".to_string(),
                            order_type: if request.trade_side == 1 { "BUY_LIMIT".to_string() } else { "SELL_LIMIT".to_string() },
                            entry_price: request.entry_price,
                            lot_size: request.volume as i32,
                            stop_loss: request.stop_loss.unwrap_or(0.0),
                            take_profit: request.take_profit.unwrap_or(0.0),
                            ctrader_order_id: order_id.clone(),
                            placed_at: chrono::Utc::now(),
                            status: "PENDING".to_string(),
                            zone_high: request.entry_price, // placeholder
                            zone_low: request.entry_price,  // placeholder
                            zone_strength: 0.0,             // placeholder
                            touch_count: 0,                 // placeholder
                            distance_when_placed: 0.0,      // placeholder
                        };
                        let _ = pending_order_manager.store_pending_order_redis(&pending_order).await;
                    }

                    let response = ManualTradeResponse {
                        success,
                        message,
                        order_id,
                        error_code,
                        request_sent: request,
                    };

                    if success {
                        (StatusCode::OK, Json(response))
                    } else {
                        (StatusCode::BAD_REQUEST, Json(response))
                    }
                }
                Err(e) => {
                    let response = ManualTradeResponse {
                        success: false,
                        message: format!(
                            "Failed to parse API response: {} - Raw: {}",
                            e, response_text
                        ),
                        order_id: None,
                        error_code: Some("PARSE_ERROR".to_string()),
                        request_sent: request,
                    };
                    (StatusCode::INTERNAL_SERVER_ERROR, Json(response))
                }
            }
        }
        Err(e) => {
            let response = ManualTradeResponse {
                success: false,
                message: format!("Failed to call cTrader API: {}", e),
                order_id: None,
                error_code: Some("API_ERROR".to_string()),
                request_sent: request,
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(response))
        }
    }
}

// Helper to map symbol_id to name (should match your SYMBOLS array)
fn symbol_id_to_name(symbol_id: i32) -> String {
    match symbol_id {
        185 => "EURUSD_SB".to_string(),
        199 => "GBPUSD_SB".to_string(),
        226 => "USDJPY_SB".to_string(),
        222 => "USDCHF_SB".to_string(),
        158 => "AUDUSD_SB".to_string(),
        221 => "USDCAD_SB".to_string(),
        211 => "NZDUSD_SB".to_string(),
        175 => "EURGBP_SB".to_string(),
        177 => "EURJPY_SB".to_string(),
        173 => "EURCHF_SB".to_string(),
        171 => "EURAUD_SB".to_string(),
        172 => "EURCAD_SB".to_string(),
        180 => "EURNZD_SB".to_string(),
        192 => "GBPJPY_SB".to_string(),
        191 => "GBPCHF_SB".to_string(),
        189 => "GBPAUD_SB".to_string(),
        190 => "GBPCAD_SB".to_string(),
        195 => "GBPNZD_SB".to_string(),
        155 => "AUDJPY_SB".to_string(),
        156 => "AUDNZD_SB".to_string(),
        153 => "AUDCAD_SB".to_string(),
        210 => "NZDJPY_SB".to_string(),
        162 => "CADJPY_SB".to_string(),
        163 => "CHFJPY_SB".to_string(),
        205 => "NAS100_SB".to_string(),
        220 => "US500_SB".to_string(),
        _ => format!("symbol_{}", symbol_id),
    }
}

// Initialize logging with both console and file output
fn init_logging() -> Result<(), Box<dyn std::error::Error>> {
    // Create logs directory if it doesn't exist
    std::fs::create_dir_all("logs")?;

    // Create a file appender with rotation
    let file_appender = tracing_appender::rolling::daily("logs", "zone_monitor");

    // Also create a timestamped log file for this session
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let session_log_path = format!("logs/zone_monitor_{}.log", timestamp);
    let session_file = std::fs::File::create(&session_log_path)?;

    // Configure the subscriber with multiple outputs
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stdout) // Console output
                .with_target(false)
                .with_level(true)
                .compact(),
        )
        .with(
            fmt::layer()
                .with_writer(file_appender) // Daily rotating file
                .with_target(true)
                .with_level(true)
                .with_ansi(false), // No color codes in file
        )
        .with(
            fmt::layer()
                .with_writer(session_file) // Session-specific file
                .with_target(true)
                .with_level(true)
                .with_ansi(false),
        )
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    println!("📋 Logging initialized:");
    println!("   📄 Daily logs: logs/zone_monitor.YYYY-MM-DD");
    println!("   📄 Session log: {}", session_log_path);
    println!("   📺 Console: enabled");

    Ok(())
}

async fn test_deal_enrichment_api(
    State(state): State<MonitorState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let pending_order_manager = state.pending_order_manager.read().await;

    match pending_order_manager.enrich_and_save_deals().await {
        Ok(enriched_count) => {
            let response = serde_json::json!({
                "success": true,
                "message": format!("Processed {} deals", enriched_count),
                "enriched_count": enriched_count
            });
            (StatusCode::OK, Json(response))
        }
        Err(e) => {
            let response = serde_json::json!({
                "success": false,
                "message": format!("Deal enrichment failed: {}", e),
                "error": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(response))
        }
    }
}

async fn redis_stats_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let stats = state.get_comprehensive_redis_stats().await;
    Json(stats)
}

async fn debug_redis_keys(State(state): State<MonitorState>) -> Json<Value> {
    let stats = state.get_comprehensive_redis_stats().await;
    Json(json!({
        "redis_stats": stats
    }))
}

async fn debug_pending_order(
    State(state): State<MonitorState>,
    Path(zone_id): Path<String>,
) -> Json<Value> {
    let redis_stats = state.get_pending_order_redis_stats().await;
    Json(json!({
        "zone_id": zone_id,
        "redis_stats": redis_stats
    }))
}

async fn debug_all_pending_orders(State(state): State<MonitorState>) -> Json<Value> {
    let pending_order_manager = state.pending_order_manager.read().await;
    
    match pending_order_manager.get_all_pending_orders_with_lookup_status().await {
        Ok(orders_info) => Json(orders_info),
        Err(e) => Json(json!({
            "error": e.to_string(),
            "redis_stats": state.get_pending_order_redis_stats().await
        }))
    }
}

async fn debug_deal_enrichment(
    State(state): State<MonitorState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let start_date = params.get("startDate").cloned()
        .unwrap_or_else(|| "2024-12-06".to_string());
    
    Json(json!({
        "start_date": start_date,
        "message": "Use POST /api/test/deal-enrichment for testing"
    }))
}

// Add this endpoint to main.rs
async fn debug_simulate_pending_order(State(state): State<MonitorState>) -> Json<Value> {
    // Create a mock pending order
    let mock_order = serde_json::json!({
        "zone_id": "test_zone_123",
        "symbol": "EURUSD",
        "timeframe": "1h",
        "zone_type": "supply_zone",
        "order_type": "SELL_LIMIT",
        "entry_price": 1.0500,
        "lot_size": 1000,
        "stop_loss": 1.0520,
        "take_profit": 1.0480,
        "ctrader_order_id": "test_order_999",
        "placed_at": chrono::Utc::now().to_rfc3339(),
        "status": "PENDING",
        "zone_high": 1.0520,
        "zone_low": 1.0500,
        "zone_strength": 85.0,
        "touch_count": 3,
        "distance_when_placed": 15.5
    });

    // Test the storage process using your existing Redis connection
    let pending_order_manager = state.pending_order_manager.read().await;
    
    // We'll call the Redis storage directly to test both keys get created
    match pending_order_manager.test_redis_storage(&mock_order).await {
        Ok(result) => Json(result),
        Err(e) => Json(json!({
            "error": e.to_string(),
            "mock_order": mock_order
        }))
    }
}

async fn fix_missing_lookups(State(state): State<MonitorState>) -> Json<Value> {
    let pending_order_manager = state.pending_order_manager.read().await;
    match pending_order_manager.create_missing_lookup_keys().await {
        Ok(count) => Json(json!({
            "success": true,
            "created_lookup_keys": count,
            "message": format!("Created {} missing lookup keys", count)
        })),
        Err(e) => Json(json!({
            "success": false,
            "error": e.to_string()
        }))
    }
}

// --- Trade Enricher endpoints ---
async fn trade_enricher_html() -> Html<String> {
    match read_to_string("web/trade_enricher.html").await {
        Ok(contents) => Html(contents),
        Err(_) => Html("<h1>404 Not Found</h1><p>trade_enricher.html not found.</p>".to_string()),
    }
}

async fn api_broker_deals(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let pending_order_manager = state.pending_order_manager.read().await;
    match pending_order_manager.fetch_broker_deals().await {
        Ok(deals) => Json(serde_json::json!({ "deals": deals })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

async fn api_enrich_deal(Path(deal_id): Path<String>, State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let pending_order_manager = state.pending_order_manager.read().await;
    match pending_order_manager.fetch_broker_deals().await {
        Ok(deals) => {
            if let Some(deal) = deals.iter().find(|d| d.deal_id.to_string() == deal_id) {
                match pending_order_manager.convert_broker_deal_to_enriched(deal).await {
                    Ok(mut enriched) => {
                        if let Some(order_id) = &deal.order_id {
                            let order_id_str = order_id.to_string();
                            if let Some(pending_order) = pending_order_manager.get_pending_order_for_enrichment(&order_id_str).await {
                                // Enrich with zone data
                                enriched.zone_id = Some(pending_order.zone_id);
                                enriched.zone_type = Some(pending_order.zone_type);
                                enriched.zone_strength = Some(pending_order.zone_strength);
                                enriched.zone_high = Some(pending_order.zone_high);
                                enriched.zone_low = Some(pending_order.zone_low);
                                enriched.touch_count = Some(pending_order.touch_count);
                                enriched.timeframe = Some(pending_order.timeframe);
                                enriched.distance_when_placed = Some(pending_order.distance_when_placed);
                                enriched.original_entry_price = Some(pending_order.entry_price);
                                enriched.stop_loss = Some(pending_order.stop_loss);
                                enriched.take_profit = Some(pending_order.take_profit);
                                // Calculate slippage
                                let pip_value = pending_order_manager.get_pip_value(&enriched.symbol);
                                let slippage = (deal.exit_price - pending_order.entry_price).abs() / pip_value;
                                enriched.slippage_pips = Some(slippage);
                            }
                        }
                        Json(serde_json::to_value(&enriched).unwrap_or_else(|_| serde_json::json!({ "error": "Serialization failed" })))
                    }
                    Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
                }
            } else {
                Json(serde_json::json!({ "error": "Deal not found" }))
            }
        }
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

async fn api_save_enriched_deal(Path(_deal_id): Path<String>, State(state): State<MonitorState>, AxumJson(payload): AxumJson<EnrichedDeal>) -> (StatusCode, Json<serde_json::Value>) {
    let order_db = &state.order_database;
    match order_db.write_enriched_deal(&payload).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "success": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "error": e.to_string() }))),
    }
}

async fn api_db_deal(Path(deal_id): Path<String>, State(state): State<MonitorState>) -> Json<serde_json::Value> {
    match state.order_database.as_ref().get_enriched_deal_by_id(&deal_id).await {
        Ok(Some(deal)) => Json(serde_json::to_value(&deal).unwrap_or_else(|_| serde_json::json!({ "error": "Serialization failed" }))),
        Ok(None) => Json(serde_json::json!({ "error": "Deal not found" })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

// GET /deadmanStatus
async fn deadman_status() -> Json<serde_json::Value> {
    Json(json!({ "status": deadman::get_deadman_status() }))
}

// POST /resetDeadman
async fn reset_deadman() -> Json<serde_json::Value> {
    let _ = deadman::set_deadman_status("on");
    Json(json!({ "status": "on" }))
}

// POST /emergencyCloseAll
async fn emergency_close_all() -> Json<serde_json::Value> {
    let api_url = std::env::var("CTRADER_API_BRIDGE_URL").unwrap_or_else(|_| "http://localhost:8000".to_string());
    let endpoint = format!("{}/closeAllPositionsAndOrders", api_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let result = client.post(&endpoint).send().await;
    let _ = deadman::set_deadman_status("off");
    match result {
        Ok(resp) if resp.status().is_success() => Json(json!({ "result": "success" })),
        Ok(resp) => Json(json!({ "result": "error", "status": resp.status().as_u16() })),
        Err(e) => Json(json!({ "result": "error", "error": e.to_string() })),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file from current directory (root of project)
    if let Err(e) = dotenv::dotenv() {
        println!("Warning: Could not load .env file: {}", e);
    }

    // Initialize logging with file output
    if let Err(e) = init_logging() {
        eprintln!("Failed to initialize logging: {}", e);
        // Fall back to simple console logging
        tracing_subscriber::fmt()
            .with_target(false)
            .with_level(true)
            .init();
    }

    info!("🚀 Starting Zone Monitor Dashboard with Notifications and CSV Logging...");

    // Configuration
    let cache_file_path =
        std::env::var("ZONE_CACHE_FILE").unwrap_or_else(|_| "shared_zones.json".to_string());

    let monitor_port = std::env::var("MONITOR_PORT")
        .unwrap_or_else(|_| "3003".to_string())
        .parse::<u16>()
        .unwrap_or(3003);

    info!("⚙️  Configuration:");
    info!("   📄 Cache file: {}", cache_file_path);
    info!("   🌐 Monitor port: {}", monitor_port);
    info!(
        "   📢 Notifications enabled: {}",
        std::env::var("NOTIFICATIONS_ENABLED").unwrap_or_else(|_| "false".to_string())
    );
    info!(
        "   🔗 Webhook URL: {}",
        std::env::var("WEBHOOK_URL").unwrap_or_else(|_| "Not configured".to_string())
    );
    info!("   📊 CSV logging: logs/proximity_YYYY-MM-DD.csv & logs/trading_signal_YYYY-MM-DD.csv");

    let trading_plan_enabled = std::env::var("TRADING_PLAN_ENABLED").unwrap_or_else(|_| "false".to_string()) == "true";
    let trading_plan = if trading_plan_enabled {
        let plan = TradingPlan::load_from_file("trading_plan.json");
        match &plan {
            Some(plan) => {
                info!("\n🚦 TRADING PLAN ENABLED! Only these setups will be traded:");
                for setup in plan.best_combinations.values() {
                    info!("   - {} {} | SL: {} | TP: {}", setup.symbol, setup.timeframe, setup.stop_loss, setup.take_profit);
                }
            },
            None => {
                info!("⚠️  TRADING PLAN ENABLED but trading_plan.json could not be loaded or parsed!");
            }
        }
        plan
    } else {
        info!("\n🔄 Trading plan disabled. Using default trading rules.");
        None
    };
    // Initialize monitor state
    let state = MonitorState::new(cache_file_path, trading_plan);

    // Initialize all systems
    if let Err(e) = state.initialize().await {
        tracing::error!("❌ Failed to initialize monitor: {}", e);
        std::process::exit(1);
    }

    let cors = CorsLayer::new()
        .allow_origin(Any) // or .allow_origin("http://localhost:5500".parse::<HeaderValue>().unwrap()) for a specific origin
        .allow_methods(Any) // or specify methods like [Method::GET, Method::POST]
        .allow_headers(Any);

    // Build web server with new notification endpoints
    let app = Router::new()
        .route("/", get(html::status_page))
        .route("/closest", get(html::closest_zones_page))
        .route("/pending-orders", get(pending_orders_html))
        .route("/api/zones", get(zones_api))
        .route("/api/alerts", get(alerts_api))
        .route("/api/pending-orders", get(pending_orders_json))
        .route("/api/notifications/status", get(notifications_status_api))
        .route("/api/notifications/test", post(test_notification_api))
        .route("/api/notifications/cooldowns", get(cooldown_stats_api))
        .route("/api/zones/stats", get(zone_stats_api))
        .route("/api/zones/interactions", get(zone_interactions_api))
        .route("/api/trading/stats", get(trading_stats_api))
        .route("/api/trading/history", get(trade_history_api))
        .route("/api/trading/manual", post(manual_trade_api))
        .route("/api/strategies", get(strategies_api))
        .route("/api/strategies/by-symbol", get(strategies_by_symbol_api))
        .route("/api/strategies/refresh", post(refresh_strategies_api))
        .route("/health", get(health_check))
        .route("/booked-trades", get(booked_trades_html))
        .route("/api/booked-trades", get(booked_trades_api))
        // ADD THESE TWO NEW ROUTES:
        // .route(
        //     "/api/enriched-trades/:date",
        //     get(enriched_trades_by_date_api),
        // )
        // .route("/api/enriched-trades", get(enriched_trades_by_range_api))
        .route("/api/test/deal-enrichment", post(test_deal_enrichment_api))
        .route("/api/redis/stats", get(redis_stats_api))
        .route("/debug/redis/keys", get(debug_redis_keys))
        .route("/debug/pending-order/:zone_id", get(debug_pending_order))
        .route("/debug/pending-orders", get(debug_all_pending_orders))
        .route("/debug/deal-enrichment", get(debug_deal_enrichment))
        .route("/debug/simulate-pending-order", post(debug_simulate_pending_order))
        .route("/debug/fix-missing-lookups", post(fix_missing_lookups))
        .route("/trade-enricher", get(trade_enricher_html))
        .route("/api/broker_deals", get(api_broker_deals))
        .route("/api/enrich_deal/:deal_id", get(api_enrich_deal))
        .route("/api/save_enriched_deal/:deal_id", post(api_save_enriched_deal))
        .route("/api/db_deal/:deal_id", get(api_db_deal))
        .route("/deadmanStatus", get(deadman_status))
        .route("/resetDeadman", post(reset_deadman))
        .route("/emergencyCloseAll", post(emergency_close_all))
        .layer(cors)
        .with_state(state);

    let api_url = std::env::var("CTRADER_API_BRIDGE_URL").unwrap_or_else(|_| "http://localhost:8000".to_string());
    let api_url_clone = api_url.clone();
    tokio::spawn(async move {
        friday_closer::friday_close_all_positions_task(&api_url_clone).await;
    });
    tokio::spawn(friday_closer::sunday_reset_deadman_task());

    info!(
        "🌐 Zone Monitor Dashboard starting on http://localhost:{}",
        monitor_port
    );
    info!("🔗 Available endpoints:");
    info!("   📊 Dashboard: http://localhost:{}", monitor_port);
    info!(
        "   📋 Pending Orders: http://localhost:{}/pending-orders",
        monitor_port
    );
    info!(
        "   📈 Zones API: http://localhost:{}/api/zones",
        monitor_port
    );
    info!(
        "   🚨 Alerts API: http://localhost:{}/api/alerts",
        monitor_port
    );
    info!(
        "   📋 Pending Orders API: http://localhost:{}/api/pending-orders",
        monitor_port
    );
    info!(
        "   📢 Notifications Status: http://localhost:{}/api/notifications/status",
        monitor_port
    );
    info!(
        "   🧪 Test Notification: POST http://localhost:{}/api/notifications/test",
        monitor_port
    );
    info!(
        "   ⏱️ Cooldown Stats: http://localhost:{}/api/notifications/cooldowns",
        monitor_port
    );
    info!(
        "   📊 Zone Stats: http://localhost:{}/api/zones/stats",
        monitor_port
    );
    info!(
        "   🔄 Zone Interactions: http://localhost:{}/api/zones/interactions",
        monitor_port
    );
    info!(
        "   💰 Trading Stats: http://localhost:{}/api/trading/stats",
        monitor_port
    );
    info!(
        "   📈 Trade History: http://localhost:{}/api/trading/history",
        monitor_port
    );
    info!(
        "   🧪 Manual Trade: POST http://localhost:{}/api/trading/manual",
        monitor_port
    );
    info!(
        "   ❤️  Health Check: http://localhost:{}/health",
        monitor_port
    );
    info!(
        "   💰 Deadman Status: http://localhost:{}/deadmanStatus",
        monitor_port
    );
    info!(
        "   🔄 Reset Deadman: POST http://localhost:{}/resetDeadman",
        monitor_port
    );
    info!(
        "   💥 Emergency Close All: POST http://localhost:{}/emergencyCloseAll",
        monitor_port
    );
    info!("✅ Zone Monitor with Notifications and CSV Logging ready!");
    info!(
        "   💰 Booked Trades: http://localhost:{}/booked-trades",
        monitor_port
    );
    info!(
        "   💰 Booked Trades API: http://localhost:{}/api/booked-trades",
        monitor_port
    );
    info!(
        "   🔄 Retroactive Enrichment: POST http://localhost:{}/api/enriched-trades/retroactive-fix",
        monitor_port
    );

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", monitor_port)).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
