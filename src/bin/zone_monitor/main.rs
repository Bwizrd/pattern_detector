// src/bin/zone_monitor/main.rs
// Clean, organized zone monitor entry point

mod html;
mod proximity;
mod state;
mod types;
mod websocket;

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
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

async fn health_check(State(state): State<MonitorState>) -> (StatusCode, Json<serde_json::Value>) {
    let connected = state.get_connection_status().await;
    let cache = state.get_zone_cache().await;
    let prices = state.get_latest_prices().await;
    let alerts = state.get_recent_alerts().await;

    let status = serde_json::json!({
        "status": if connected { "healthy" } else { "disconnected" },
        "websocket_connected": connected,
        "total_zones": cache.total_zones,
        "active_symbols": cache.zones.len(),
        "live_prices": prices.len(),
        "recent_alerts": alerts.len(),
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
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    info!("🚀 Starting Zone Monitor Dashboard...");

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

    // Initialize monitor state
    let state = MonitorState::new(cache_file_path);

    // Initialize all systems
    if let Err(e) = state.initialize().await {
        tracing::error!("❌ Failed to initialize monitor: {}", e);
        std::process::exit(1);
    }

    // Build web server
    let app = Router::new()
        .route("/", get(html::status_page))
        .route("/api/zones", get(zones_api))
        .route("/api/alerts", get(alerts_api))
        .route("/health", get(health_check))
        .with_state(state);

    info!("🌐 Zone Monitor Dashboard starting on http://localhost:{}", monitor_port);
    info!("🔗 Available endpoints:");
    info!("   📊 Dashboard: http://localhost:{}", monitor_port);
    info!("   📈 Zones API: http://localhost:{}/api/zones", monitor_port);
    info!("   🚨 Alerts API: http://localhost:{}/api/alerts", monitor_port);
    info!("   ❤️  Health Check: http://localhost:{}/health", monitor_port);
    info!("✅ Zone Monitor ready!");

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", monitor_port)).await?;
    axum::serve(listener, app).await?;

    Ok(())
}