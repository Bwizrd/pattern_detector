// src/main.rs - Complete file with trade logging and tracking integration
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use log::{error, info};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
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
mod trade_event_logger;
mod trade_tracker;
pub mod trades;
pub mod trading;
mod types;
mod websocket_server;
mod zone_detection;

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
use crate::trade_event_logger::TradeEventLogger;
use crate::trade_tracker::{TradeTracker, TradeTrackingRequest};

// --- Global State ---
use std::sync::LazyLock;
static GLOBAL_PRICE_BROADCASTER: LazyLock<
    std::sync::Mutex<Option<tokio::sync::broadcast::Sender<String>>>,
> = LazyLock::new(|| std::sync::Mutex::new(None));

static GLOBAL_TRADE_LOGGER: LazyLock<
    std::sync::Mutex<Option<Arc<tokio::sync::Mutex<TradeEventLogger>>>>,
> = LazyLock::new(|| std::sync::Mutex::new(None));

static GLOBAL_TRADE_TRACKER: LazyLock<
    std::sync::Mutex<Option<Arc<tokio::sync::Mutex<TradeTracker>>>>,
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
pub fn get_global_trade_logger() -> Option<Arc<tokio::sync::Mutex<TradeEventLogger>>> {
    GLOBAL_TRADE_LOGGER.lock().unwrap().clone()
}

pub fn get_global_trade_tracker() -> Option<Arc<tokio::sync::Mutex<TradeTracker>>> {
    GLOBAL_TRADE_TRACKER.lock().unwrap().clone()
}

// --- API Handlers ---
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

async fn get_validated_signals_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    log::info!("Validated signals endpoint called");

    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let signals = monitor.get_validated_signals().await;
        log::info!("Retrieved {} validated signals", signals.len());

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "count": signals.len(),
            "signals": signals,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        log::warn!("Zone monitor not available for validated signals");
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available",
            "signals": [],
            "message": "Check if ENABLE_REALTIME_MONITOR=true"
        }))
    }
}

async fn clear_validated_signals_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    log::info!("Clear validated signals endpoint called");

    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        monitor.clear_validated_signals().await;
        log::info!("Validated signals cleared");

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Validated signals cleared",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        log::warn!("Zone monitor not available for clearing validated signals");
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available",
            "message": "Check if ENABLE_REALTIME_MONITOR=true"
        }))
    }
}

// --- Trade Logging Handlers ---
async fn get_trade_log_info() -> impl Responder {
    if let Some(logger_arc) = get_global_trade_logger() {
        let logger_guard = logger_arc.lock().await;
        let log_path = logger_guard.get_log_file_path();

        let metadata = match fs::metadata(log_path) {
            Ok(meta) => Some(serde_json::json!({
                "size_bytes": meta.len(),
                "created": meta.created().ok().map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
                "modified": meta.modified().ok().map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())
            })),
            Err(_) => None,
        };

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "log_file_path": log_path.display().to_string(),
            "historical_analysis_complete": logger_guard.has_completed_historical_analysis(),
            "metadata": metadata,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade logger not available",
            "message": "Trade logging system not initialized"
        }))
    }
}

async fn get_recent_trade_logs(query: web::Query<HashMap<String, String>>) -> impl Responder {
    let lines_to_read = query
        .get("lines")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100);

    if let Some(logger_arc) = get_global_trade_logger() {
        let logger_guard = logger_arc.lock().await;
        let log_path = logger_guard.get_log_file_path();

        match fs::read_to_string(log_path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let recent_lines: Vec<&str> = lines
                    .iter()
                    .rev()
                    .take(lines_to_read)
                    .rev()
                    .copied()
                    .collect();

                let mut parsed_entries = Vec::new();
                let mut parse_errors = 0;

                for line in recent_lines {
                    match serde_json::from_str::<serde_json::Value>(line) {
                        Ok(json) => parsed_entries.push(json),
                        Err(_) => parse_errors += 1,
                    }
                }

                HttpResponse::Ok().json(serde_json::json!({
                    "status": "success",
                    "total_entries": parsed_entries.len(),
                    "parse_errors": parse_errors,
                    "requested_lines": lines_to_read,
                    "entries": parsed_entries,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
            Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "error": "Failed to read log file",
                "message": e.to_string()
            })),
        }
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade logger not available"
        }))
    }
}

async fn get_trade_log_stats() -> impl Responder {
    if let Some(logger_arc) = get_global_trade_logger() {
        let logger_guard = logger_arc.lock().await;
        let log_path = logger_guard.get_log_file_path();

        match fs::read_to_string(log_path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let total_entries = lines.len();

                let mut event_type_counts = HashMap::new();
                let mut symbol_counts = HashMap::new();
                let mut timeframe_counts = HashMap::new();
                let mut parse_errors = 0;

                for line in lines {
                    match serde_json::from_str::<serde_json::Value>(line) {
                        Ok(entry) => {
                            if let Some(event_type) = entry.get("event_type").and_then(|v| v.as_str()) {
                                *event_type_counts.entry(event_type.to_string()).or_insert(0) += 1;
                            }

                            if let Some(details) = entry.get("details") {
                                if let Some(symbol) = details.get("symbol").and_then(|v| v.as_str()) {
                                    *symbol_counts.entry(symbol.to_string()).or_insert(0) += 1;
                                }
                                if let Some(timeframe) = details.get("timeframe").and_then(|v| v.as_str()) {
                                    *timeframe_counts.entry(timeframe.to_string()).or_insert(0) += 1;
                                }
                            }
                        }
                        Err(_) => parse_errors += 1,
                    }
                }

                HttpResponse::Ok().json(serde_json::json!({
                    "status": "success",
                    "total_entries": total_entries,
                    "parse_errors": parse_errors,
                    "event_type_counts": event_type_counts,
                    "symbol_counts": symbol_counts,
                    "timeframe_counts": timeframe_counts,
                    "historical_analysis_complete": logger_guard.has_completed_historical_analysis(),
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
            Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "error": "Failed to read log file",
                "message": e.to_string()
            })),
        }
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade logger not available"
        }))
    }
}

async fn download_trade_log() -> impl Responder {
    if let Some(logger_arc) = get_global_trade_logger() {
        let logger_guard = logger_arc.lock().await;
        let log_path = logger_guard.get_log_file_path();

        match fs::read(log_path) {
            Ok(content) => {
                let filename = log_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("trade_log.log");

                HttpResponse::Ok()
                    .content_type("application/octet-stream")
                    .append_header((
                        "Content-Disposition",
                        format!("attachment; filename=\"{}\"", filename),
                    ))
                    .body(content)
            }
            Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "error": "Failed to read log file",
                "message": e.to_string()
            })),
        }
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade logger not available"
        }))
    }
}

async fn trigger_historical_analysis(
    shared_cache: web::Data<Arc<Mutex<MinimalZoneCache>>>,
) -> impl Responder {
    if let Some(logger_arc) = get_global_trade_logger() {
        let mut logger_guard = logger_arc.lock().await;

        if logger_guard.has_completed_historical_analysis() {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "status": "error",
                "error": "Historical analysis already completed",
                "message": "Analysis can only be run once per session"
            }));
        }

        match logger_guard.analyze_historical_trades(Arc::clone(&shared_cache)).await {
            Ok(analysis) => HttpResponse::Ok().json(serde_json::json!({
                "status": "success",
                "message": "Historical analysis completed",
                "analysis": analysis,
                "timestamp": chrono::Utc::now().to_rfc3339()
            })),
            Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "error": "Historical analysis failed",
                "message": e.to_string()
            })),
        }
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade logger not available"
        }))
    }
}

// --- Trade Tracking Handlers ---
async fn start_tracking_todays_trades(
    query: web::Query<HashMap<String, String>>,
) -> impl Responder {
    let tp_pips = query.get("tp_pips").and_then(|s| s.parse::<f64>().ok());
    let sl_pips = query.get("sl_pips").and_then(|s| s.parse::<f64>().ok());

    if let Some(tracker_arc) = get_global_trade_tracker() {
        let mut tracker_guard = tracker_arc.lock().await;
        
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let csv_path = std::path::PathBuf::from("trades").join(format!("{}_validated_trades.csv", today));

        match tracker_guard.load_trades_from_csv(&csv_path, tp_pips, sl_pips).await {
            Ok(count) => {
                info!("📈 Started tracking {} trades from today's CSV", count);
                HttpResponse::Ok().json(serde_json::json!({
                    "status": "success",
                    "message": format!("Started tracking {} trades", count),
                    "csv_file": csv_path.display().to_string(),
                    "default_tp_pips": tp_pips,
                    "default_sl_pips": sl_pips,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
            Err(e) => {
                error!("Failed to load trades from CSV: {}", e);
                HttpResponse::InternalServerError().json(serde_json::json!({
                    "status": "error",
                    "error": "Failed to load trades from CSV",
                    "message": e.to_string()
                }))
            }
        }
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade tracker not available"
        }))
    }
}

async fn track_custom_trades(
    request_body: web::Json<TradeTrackingRequest>,
) -> impl Responder {
    if let Some(tracker_arc) = get_global_trade_tracker() {
        let mut tracker_guard = tracker_arc.lock().await;
        
        tracker_guard.track_trades_from_request(request_body.into_inner());
        
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Custom trades added to tracking",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade tracker not available"
        }))
    }
}

async fn get_tracked_trades_status(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(tracker_arc) = get_global_trade_tracker() {
        let mut tracker_guard = tracker_arc.lock().await;
        
        let current_prices = if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
            monitor.get_current_prices_by_symbol().await
        } else {
            HashMap::new()
        };

        let response = tracker_guard.update_with_current_prices(&current_prices).await;
        HttpResponse::Ok().json(response)
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade tracker not available"
        }))
    }
}

async fn serve_trade_dashboard() -> impl Responder {
    let html = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Trade Dashboard</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; background: #f5f5f5; }
        .container { max-width: 1400px; margin: 0 auto; }
        .header { background: #2c3e50; color: white; padding: 20px; border-radius: 8px; margin-bottom: 20px; }
        .summary { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 15px; margin-bottom: 20px; }
        .card { background: white; padding: 15px; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        .profit { color: #27ae60; font-weight: bold; }
        .loss { color: #e74c3c; font-weight: bold; }
        .neutral { color: #7f8c8d; }
        table { width: 100%; border-collapse: collapse; background: white; border-radius: 8px; overflow: hidden; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        th, td { padding: 12px; text-align: left; border-bottom: 1px solid #ddd; }
        th { background: #34495e; color: white; }
        .status-open { background: #3498db; color: white; padding: 4px 8px; border-radius: 4px; font-size: 12px; }
        .status-tp { background: #27ae60; color: white; padding: 4px 8px; border-radius: 4px; font-size: 12px; }
        .status-sl { background: #e74c3c; color: white; padding: 4px 8px; border-radius: 4px; font-size: 12px; }
        .controls { margin-bottom: 20px; }
        .btn { background: #3498db; color: white; padding: 10px 20px; border: none; border-radius: 4px; cursor: pointer; margin-right: 10px; }
        .btn:hover { background: #2980b9; }
        .error { color: #e74c3c; background: #ffebee; padding: 10px; border-radius: 4px; margin: 10px 0; }
        .success { color: #27ae60; background: #e8f5e8; padding: 10px; border-radius: 4px; margin: 10px 0; }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>📈 Trade Dashboard</h1>
            <p>Real-time tracking of today's validated trades</p>
        </div>

        <div class="controls">
            <button class="btn" onclick="loadTodaysTrades()">Load Today's Trades</button>
            <button class="btn" onclick="refreshStatus()">Refresh Status</button>
            <input type="number" id="tpPips" placeholder="TP Pips (e.g. 50)" style="padding: 8px; margin: 0 10px;">
            <input type="number" id="slPips" placeholder="SL Pips (e.g. 25)" style="padding: 8px;">
        </div>

        <div id="messages"></div>

        <div class="summary" id="summary" style="display: none;">
            <div class="card">
                <h3>Total P&L</h3>
                <div id="totalPnl" class="neutral">$0.00</div>
            </div>
            <div class="card">
                <h3>Open Trades</h3>
                <div id="openTrades">0</div>
            </div>
            <div class="card">
                <h3>Win Rate</h3>
                <div id="winRate">0%</div>
            </div>
            <div class="card">
                <h3>Profit Factor</h3>
                <div id="profitFactor">0.00</div>
            </div>
        </div>

        <div id="tradesTable"></div>
    </div>

    <script>
        let refreshInterval;

        async function loadTodaysTrades() {
            const tpPips = document.getElementById('tpPips').value;
            const slPips = document.getElementById('slPips').value;
            
            let url = '/api/track/load-today';
            const params = new URLSearchParams();
            if (tpPips) params.append('tp_pips', tpPips);
            if (slPips) params.append('sl_pips', slPips);
            if (params.toString()) url += '?' + params.toString();

            try {
                const response = await fetch(url, { method: 'POST' });
                const data = await response.json();
                
                if (data.status === 'success') {
                    showMessage(data.message, 'success');
                    refreshStatus();
                    startAutoRefresh();
                } else {
                    showMessage(data.error || 'Failed to load trades', 'error');
                }
            } catch (error) {
                showMessage('Error loading trades: ' + error.message, 'error');
            }
        }

        async function refreshStatus() {
            try {
                const response = await fetch('/api/track/status');
                const data = await response.json();
                
                if (data.status === 'success') {
                    updateDashboard(data);
                    document.getElementById('summary').style.display = 'grid';
                } else {
                    showMessage(data.error || 'Failed to get status', 'error');
                }
            } catch (error) {
                showMessage('Error refreshing status: ' + error.message, 'error');
            }
        }

        function updateDashboard(data) {
            // Update summary
            document.getElementById('totalPnl').textContent = '$' + data.total_pnl.toFixed(2);
            document.getElementById('totalPnl').className = data.total_pnl >= 0 ? 'profit' : 'loss';
            document.getElementById('openTrades').textContent = data.open_trades;
            document.getElementById('winRate').textContent = data.summary.win_rate.toFixed(1) + '%';
            document.getElementById('profitFactor').textContent = data.summary.profit_factor.toFixed(2);

            // Update trades table
            const table = createTradesTable(data.trades);
            document.getElementById('tradesTable').innerHTML = table;
        }

        function createTradesTable(trades) {
            if (trades.length === 0) {
                return '<div class="card">No trades to display</div>';
            }

            let html = '<table><thead><tr>';
            html += '<th>Time</th><th>Symbol</th><th>Action</th><th>Entry</th><th>Current</th>';
            html += '<th>P&L ($)</th><th>P&L (Pips)</th><th>Status</th><th>TP</th><th>SL</th>';
            html += '</tr></thead><tbody>';

            trades.forEach(trade => {
                const pnl = trade.unrealized_pnl || 0;
                const pnlPips = trade.unrealized_pnl_pips || 0;
                const pnlClass = pnl >= 0 ? 'profit' : 'loss';
                
                let statusClass = 'status-open';
                let statusText = trade.status;
                if (trade.status === 'TakeProfitHit') {
                    statusClass = 'status-tp';
                    statusText = 'TP Hit';
                } else if (trade.status === 'StopLossHit') {
                    statusClass = 'status-sl';
                    statusText = 'SL Hit';
                }

                html += '<tr>';
                html += `<td>${new Date(trade.timestamp).toLocaleTimeString()}</td>`;
                html += `<td>${trade.symbol}</td>`;
                html += `<td>${trade.action}</td>`;
                html += `<td>${trade.entry_price.toFixed(5)}</td>`;
                html += `<td>${trade.current_price ? trade.current_price.toFixed(5) : 'N/A'}</td>`;
                html += `<td class="${pnlClass}">${pnl.toFixed(2)}</td>`;
                html += `<td class="${pnlClass}">${pnlPips.toFixed(1)}</td>`;
                html += `<td><span class="${statusClass}">${statusText}</span></td>`;
                html += `<td>${trade.take_profit ? trade.take_profit.toFixed(5) : 'N/A'}</td>`;
                html += `<td>${trade.stop_loss ? trade.stop_loss.toFixed(5) : 'N/A'}</td>`;
                html += '</tr>';
            });

            html += '</tbody></table>';
            return html;
        }

        function showMessage(message, type) {
            const div = document.getElementById('messages');
            div.innerHTML = `<div class="${type}">${message}</div>`;
            setTimeout(() => div.innerHTML = '', 5000);
        }

        function startAutoRefresh() {
            if (refreshInterval) clearInterval(refreshInterval);
            refreshInterval = setInterval(refreshStatus, 5000); // Refresh every 5 seconds
        }

        // Auto-load on page load if trades exist
        window.onload = () => refreshStatus();
    </script>
</body>
</html>
    "#;

    HttpResponse::Ok()
        .content_type("text/html")
        .body(html)
}

// --- Cache Setup Function ---
async fn setup_cache_system() -> Result<
    (
        Arc<Mutex<MinimalZoneCache>>,
        Option<Arc<NewRealTimeZoneMonitor>>,
        Option<Arc<tokio::sync::Mutex<TradeEventLogger>>>,
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

    // Initialize trade logger
    let trade_logger = match TradeEventLogger::new() {
        Ok(logger) => {
            info!("✅ [MAIN] Trade event logger initialized");
            let logger_arc = Arc::new(tokio::sync::Mutex::new(logger));
            
            *GLOBAL_TRADE_LOGGER.lock().unwrap() = Some(Arc::clone(&logger_arc));
            Some(logger_arc)
        }
        Err(e) => {
            error!("❌ [MAIN] Failed to initialize trade logger: {}", e);
            None
        }
    };

    // Initialize trade tracker
    let trade_tracker = Arc::new(tokio::sync::Mutex::new(TradeTracker::new()));
    *GLOBAL_TRADE_TRACKER.lock().unwrap() = Some(Arc::clone(&trade_tracker));
    info!("✅ [MAIN] Trade tracker initialized");

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

    // Perform historical trade analysis
    if let Some(ref logger_arc) = trade_logger {
        let enable_historical_analysis = env::var("ENABLE_HISTORICAL_ANALYSIS")
            .unwrap_or_else(|_| "true".to_string())
            .trim()
            .to_lowercase()
            == "true";

        if enable_historical_analysis {
            info!("📊 [MAIN] Starting historical trade analysis...");
            let mut logger_guard = logger_arc.lock().await;
            
            match logger_guard.analyze_historical_trades(Arc::clone(&cache)).await {
                Ok(analysis) => {
                    info!(
                        "✅ [MAIN] Historical analysis complete: {} zones analyzed, {} validated signals",
                        analysis.total_zones_analyzed, analysis.total_validated_signals
                    );
                }
                Err(e) => {
                    error!("❌ [MAIN] Historical analysis failed: {}", e);
                }
            }
            drop(logger_guard);
        } else {
            info!("⏸️ [MAIN] Historical analysis disabled via ENABLE_HISTORICAL_ANALYSIS");
        }
    }

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

    Ok((cache, zone_monitor, trade_logger))
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

    // Setup cache system with zone monitor and trade logger
    let (shared_cache, zone_monitor, _trade_logger) = match setup_cache_system().await {
        Ok(result) => {
            log::info!("✅ [MAIN] Cache system initialized successfully");
            result
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

        let zone_monitor_clone = zone_monitor.clone();

        tokio::spawn(async move {
            log::info!("🚀 [MAIN] Starting cTrader price feed integration with zone monitor");

            if let Some(broadcaster) = broadcaster_clone {
                if let Err(e) = connect_to_ctrader_websocket(&broadcaster, zone_monitor_clone).await {
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
            .route("/active-zones", web::get().to(detect::get_active_zones_handler))
            .route("/bulk-multi-tf-active-zones", web::post().to(detect::get_bulk_multi_tf_active_zones_handler))
            .route("/zone", web::get().to(get_zone_by_id::get_zone_by_id))
            .route("/latest-formed-zones", web::get().to(latest_zones_handler::get_latest_formed_zones_handler))
            .route("/find-and-verify-zone", web::get().to(detect::find_and_verify_zone_handler))
            // Backtest endpoints
            .route("/backtest", web::post().to(backtest::run_backtest))
            .route("/multi-symbol-backtest", web::post().to(multi_backtest_handler::run_multi_symbol_backtest))
            .route("/optimize-parameters", web::post().to(optimize_handler::run_parameter_optimization))
            .route("/portfolio-meta-backtest", web::post().to(optimize_handler::run_portfolio_meta_optimized_backtest))
            // Test/Debug endpoints
            .route("/testDataRequest", web::get().to(influx_fetcher::test_data_request_handler))
            .route("/test-cache", web::get().to(test_cache_endpoint))
            .route("/debug/minimal-cache-zones", web::get().to(get_minimal_cache_zones_debug_with_shared_cache))
            // Price endpoints
            .route("/current-prices", web::get().to(get_current_prices_handler))
            .route("/test-prices", web::get().to(test_prices_handler))
            // Trade notification endpoints
            .route("/trade-notifications", web::get().to(get_trade_notifications_handler))
            .route("/trade-notifications/clear", web::post().to(clear_trade_notifications_handler))
            .route("/validated-signals", web::get().to(get_validated_signals_handler))
            .route("/validated-signals/clear", web::post().to(clear_validated_signals_handler))
            // Trade logging endpoints
            .route("/trade-log/info", web::get().to(get_trade_log_info))
            .route("/trade-log/recent", web::get().to(get_recent_trade_logs))
            .route("/trade-log/stats", web::get().to(get_trade_log_stats))
            .route("/trade-log/download", web::get().to(download_trade_log))
            .route("/trade-log/analyze-historical", web::post().to(trigger_historical_analysis))
            // Trade tracking endpoints
            .route("/api/track/load-today", web::post().to(start_tracking_todays_trades))
            .route("/api/track/custom", web::post().to(track_custom_trades))
            .route("/api/track/status", web::get().to(get_tracked_trades_status))
            .route("/trade-dashboard", web::get().to(serve_trade_dashboard))
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
    
    println!("📊 Core Endpoints:");
    println!("  GET  /health");
    println!("  GET  /echo?...");
    
    println!("🎯 Zone Endpoints:");
    println!("  GET  /analyze?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...");
    println!("  GET  /active-zones?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...");
    println!("  POST /bulk-multi-tf-active-zones");
    println!("  GET  /zone?zone_id=...");
    println!("  GET  /latest-formed-zones");
    println!("  GET  /find-and-verify-zone");
    
    println!("📈 Backtest Endpoints:");
    println!("  POST /backtest");
    println!("  POST /multi-symbol-backtest");
    println!("  POST /optimize-parameters");
    println!("  POST /portfolio-meta-backtest");
    
    println!("💰 Price & Trading Endpoints:");
    println!("  GET  /current-prices");
    println!("  GET  /test-prices");
    println!("  GET  /trade-notifications");
    println!("  POST /trade-notifications/clear");
    println!("  GET  /validated-signals");
    println!("  POST /validated-signals/clear");
    
    println!("📝 Trade Logging Endpoints:");
    println!("  GET  /trade-log/info");
    println!("  GET  /trade-log/recent?lines=100");
    println!("  GET  /trade-log/stats");
    println!("  GET  /trade-log/download");
    println!("  POST /trade-log/analyze-historical");
    
    println!("📈 Trade Tracking Endpoints:");
    println!("  POST /api/track/load-today?tp_pips=50&sl_pips=25");
    println!("  POST /api/track/custom");
    println!("  GET  /api/track/status");
    println!("  GET  /trade-dashboard");
    
    println!("🔧 Debug Endpoints:");
    println!("  GET  /test-cache");
    println!("  GET  /debug/minimal-cache-zones");
    println!("  GET  /testDataRequest");
    
    println!("--------------------------------------------------");
    println!("📁 Trade logs will be saved to: ./trades/");
    println!("🕐 Historical analysis runs automatically on startup");
    println!("📊 Monitor real-time trading via /trade-log/recent");
    println!("📈 Live trade dashboard at /trade-dashboard");
    println!("--------------------------------------------------");
}