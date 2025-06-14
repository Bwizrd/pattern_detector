// src/bin/zone_monitor/main.rs
// Clean, organized zone monitor entry point with notifications and CSV logging

mod html;
mod notifications;
mod proximity;
mod sound_notifier;
mod state;
mod trading_engine;
mod types;
mod websocket;
mod zone_state_manager;
mod csv_logger;
mod telegram_notifier;

use axum::{extract::State, http::StatusCode, routing::get, routing::post, Json, Router};
use state::MonitorState;
use tracing::info;
use types::{ZoneAlert, ZoneCache};

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

async fn test_notification_api(State(state): State<MonitorState>) -> (StatusCode, Json<serde_json::Value>) {
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

async fn trade_history_api(State(state): State<MonitorState>) -> Json<Vec<crate::trading_engine::Trade>> {
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

async fn cooldown_stats_api(State(state): State<MonitorState>) -> Json<serde_json::Value> {
    let stats = state.get_cooldown_stats().await;
    Json(stats)
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file from current directory (root of project)
    if let Err(e) = dotenv::dotenv() {
        println!("Warning: Could not load .env file: {}", e);
    }

    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

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
    info!("   📢 Notifications enabled: {}", 
          std::env::var("NOTIFICATIONS_ENABLED").unwrap_or_else(|_| "false".to_string()));
    info!("   🔗 Webhook URL: {}", 
          std::env::var("WEBHOOK_URL").unwrap_or_else(|_| "Not configured".to_string()));
    info!("   📊 CSV logging: logs/proximity_YYYY-MM-DD.csv & logs/trading_signal_YYYY-MM-DD.csv");

    // Initialize monitor state
    let state = MonitorState::new(cache_file_path);

    // Initialize all systems
    if let Err(e) = state.initialize().await {
        tracing::error!("❌ Failed to initialize monitor: {}", e);
        std::process::exit(1);
    }

    // Build web server with new notification endpoints
    let app = Router::new()
        .route("/", get(html::status_page))
        .route("/closest", get(html::closest_zones_page))
        .route("/api/zones", get(zones_api))
        .route("/api/alerts", get(alerts_api))
        .route("/api/notifications/status", get(notifications_status_api))
        .route("/api/notifications/test", post(test_notification_api))
        .route("/api/notifications/cooldowns", get(cooldown_stats_api))
        .route("/api/zones/stats", get(zone_stats_api))
        .route("/api/trading/stats", get(trading_stats_api))
        .route("/api/trading/history", get(trade_history_api))
        .route("/health", get(health_check))
        .with_state(state);

    info!("🌐 Zone Monitor Dashboard starting on http://localhost:{}", monitor_port);
    info!("🔗 Available endpoints:");
    info!("   📊 Dashboard: http://localhost:{}", monitor_port);
    info!("   📈 Zones API: http://localhost:{}/api/zones", monitor_port);
    info!("   🚨 Alerts API: http://localhost:{}/api/alerts", monitor_port);
    info!("   📢 Notifications Status: http://localhost:{}/api/notifications/status", monitor_port);
    info!("   🧪 Test Notification: POST http://localhost:{}/api/notifications/test", monitor_port);
    info!("   ⏱️ Cooldown Stats: http://localhost:{}/api/notifications/cooldowns", monitor_port);
    info!("   📊 Zone Stats: http://localhost:{}/api/zones/stats", monitor_port);
    info!("   💰 Trading Stats: http://localhost:{}/api/trading/stats", monitor_port);
    info!("   📈 Trade History: http://localhost:{}/api/trading/history", monitor_port);
    info!("   ❤️  Health Check: http://localhost:{}/health", monitor_port);
    info!("✅ Zone Monitor with Notifications and CSV Logging ready!");

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", monitor_port)).await?;
    axum::serve(listener, app).await?;

    Ok(())
}