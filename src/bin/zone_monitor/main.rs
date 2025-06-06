// src/bin/zone_monito/main.rs
// Simple zone monitor that reads JSON cache and shows status via HTML

use axum::{extract::State, http::StatusCode, response::Html, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// Import your existing modules (adjust paths as needed)
// use pattern_detector::data::ctrader_integration::CTraderWebSocket;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    pub id: String,
    pub symbol: String,
    pub zone_type: String, // "supply" or "demand"
    pub high: f64,
    pub low: f64,
    pub strength: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneCache {
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub zones: HashMap<String, Vec<Zone>>, // symbol -> zones
    pub total_zones: usize,
}

impl Default for ZoneCache {
    fn default() -> Self {
        Self {
            last_updated: chrono::Utc::now(),
            zones: HashMap::new(),
            total_zones: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceUpdate {
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct MonitorState {
    pub zone_cache: Arc<RwLock<ZoneCache>>,
    pub latest_prices: Arc<RwLock<HashMap<String, PriceUpdate>>>,
    pub websocket_connected: Arc<RwLock<bool>>,
    pub cache_file_path: String,
}

impl MonitorState {
    pub fn new(cache_file_path: String) -> Self {
        Self {
            zone_cache: Arc::new(RwLock::new(ZoneCache::default())),
            latest_prices: Arc::new(RwLock::new(HashMap::new())),
            websocket_connected: Arc::new(RwLock::new(false)),
            cache_file_path,
        }
    }

    pub async fn load_zones_from_file(&self) -> Result<(), Box<dyn std::error::Error>> {
        let content = match tokio::fs::read_to_string(&self.cache_file_path).await {
            Ok(content) => content,
            Err(e) => {
                warn!("Could not read cache file {}: {}", self.cache_file_path, e);
                return Ok(()); // Don't error if file doesn't exist yet
            }
        };

        let cache: ZoneCache = serde_json::from_str(&content)?;
        let mut zone_cache = self.zone_cache.write().await;
        *zone_cache = cache;

        info!("Loaded {} zones from cache file", zone_cache.total_zones);
        Ok(())
    }

    pub async fn start_cache_refresh_task(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60)); // Check every minute

            loop {
                interval.tick().await;

                if let Err(e) = state.load_zones_from_file().await {
                    error!("Failed to refresh zone cache: {}", e);
                }
            }
        });
    }

    pub async fn start_websocket_task(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                info!("🔌 Attempting to connect to cTrader WebSocket...");

                // Set connected status to false while attempting connection
                {
                    let mut connected = state.websocket_connected.write().await;
                    *connected = false;
                }

                // Get WebSocket URL from environment or use default
                let price_ws_url = std::env::var("PRICE_WS_URL")
                    .unwrap_or_else(|_| "ws://localhost:8083".to_string());

                match state.connect_to_price_feed(&price_ws_url).await {
                    Ok(_) => {
                        info!("✅ WebSocket connection ended normally");
                    }
                    Err(e) => {
                        error!("❌ WebSocket connection failed: {}", e);
                    }
                }

                // Wait before reconnecting
                info!("⏳ Reconnecting in 10 seconds...");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
    }

    async fn check_zone_proximity(&self, price_update: &PriceUpdate) {
        let zones = {
            let cache = self.zone_cache.read().await;
            cache
                .zones
                .get(&price_update.symbol)
                .cloned()
                .unwrap_or_default()
        };

        let current_price = (price_update.bid + price_update.ask) / 2.0;

        for zone in zones {
            let zone_mid = (zone.high + zone.low) / 2.0;
            let distance_pips = ((current_price - zone_mid).abs() * 10000.0).round();

            // If within 10 pips, log it (later we'll add notifications)
            if distance_pips <= 10.0 {
                info!(
                    "ZONE PROXIMITY: {} price {:.5} is {:.1} pips from {} zone at {:.5}-{:.5}",
                    price_update.symbol,
                    current_price,
                    distance_pips,
                    zone.zone_type,
                    zone.low,
                    zone.high
                );
            }
        }
    }

    async fn connect_to_price_feed(
        &self,
        ws_url: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🔌 Connecting to cTrader WebSocket at {}", ws_url);

        let (ws_stream, _) = connect_async(ws_url).await?;
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Set connected status
        {
            let mut connected = self.websocket_connected.write().await;
            *connected = true;
        }

        // Subscribe to major currency pairs (same as your main app)
        let symbols_to_subscribe = vec![
            (185, "EURUSD_SB"),
            (199, "GBPUSD_SB"),
            (226, "USDJPY_SB"),
            (222, "USDCHF_SB"),
            (158, "AUDUSD_SB"),
            (221, "USDCAD_SB"),
        ];

        let timeframes = vec!["5m", "15m", "1h"];

        // Send subscription requests
        for (symbol_id, symbol_name) in &symbols_to_subscribe {
            for timeframe in &timeframes {
                let subscribe_msg = json!({
                    "type": "SUBSCRIBE",
                    "symbolId": symbol_id,
                    "timeframe": timeframe
                });

                // Add this logging:
                info!("📤 Sending subscription: {}", subscribe_msg.to_string());

                if let Err(e) = ws_sender
                    .send(Message::Text(subscribe_msg.to_string()))
                    .await
                {
                    error!(
                        "❌ Failed to send subscription for {}/{}: {}",
                        symbol_name, timeframe, e
                    );
                } else {
                    debug!(
                        "📡 Subscribed to {}/{} ({})",
                        symbol_name, timeframe, symbol_id
                    );
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }

        info!("📡 Sent all subscription requests");

        // Process incoming messages
        let mut message_count = 0;
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    message_count += 1;

                    if let Err(e) = self.process_ctrader_message(&text).await {
                        warn!("⚠️ Error processing message #{}: {}", message_count, e);
                    }
                }
                Ok(Message::Close(_)) => {
                    warn!("🔌 WebSocket connection closed by server");
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    if let Err(e) = ws_sender.send(Message::Pong(payload)).await {
                        error!("❌ Failed to send pong: {}", e);
                    }
                }
                Err(e) => {
                    error!("❌ WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        // Set disconnected status
        {
            let mut connected = self.websocket_connected.write().await;
            *connected = false;
        }

        info!(
            "🔌 WebSocket connection ended after {} messages",
            message_count
        );
        Ok(())
    }

    // Add this method to process messages (same format as your main app):
    async fn process_ctrader_message(
        &self,
        message: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let data: serde_json::Value = serde_json::from_str(message)?;

        // Handle the wrapped message format
        let actual_message =
            if data.get("type").and_then(|t| t.as_str()) == Some("raw_price_update") {
                // Extract the inner data
                data.get("data").unwrap_or(&data)
            } else {
                &data
            };

        // CHANGE THIS LINE: Use actual_message instead of data
        match actual_message.get("type").and_then(|t| t.as_str()) {
            Some("BAR_UPDATE") => {
                // CHANGE THIS LINE: Use actual_message instead of data
                if let Some(bar_data) = actual_message.get("data") {
                    let symbol_name = bar_data
                        .get("symbol")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let close = bar_data
                        .get("close")
                        .and_then(|c| c.as_f64())
                        .unwrap_or(0.0);
                    let is_new_bar = bar_data
                        .get("isNewBar")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);

                    if close > 0.0 && !symbol_name.is_empty() {
                        let clean_symbol = symbol_name.trim_end_matches("_SB");

                        let price_update = PriceUpdate {
                            symbol: clean_symbol.to_string(),
                            bid: close - 0.00001,
                            ask: close + 0.00001,
                            timestamp: chrono::Utc::now(),
                        };

                        {
                            let mut prices = self.latest_prices.write().await;
                            prices.insert(clean_symbol.to_string(), price_update.clone());
                        }

                        self.check_zone_proximity(&price_update).await;

                        if is_new_bar {
                            info!("🆕 New bar: {} @ {:.5}", clean_symbol, close);
                        } else {
                            debug!("💹 Price update: {} @ {:.5}", clean_symbol, close);
                        }
                    }
                }
            }
            // Also change these to use actual_message:
            Some("SUBSCRIPTION_CONFIRMED") => {
                if let (Some(symbol_id), Some(timeframe)) = (
                    actual_message.get("symbolId").and_then(|s| s.as_u64()),
                    actual_message.get("timeframe").and_then(|t| t.as_str()),
                ) {
                    debug!("✅ Subscription confirmed: {}/{}", symbol_id, timeframe);
                }
            }
            Some("CONNECTED") => {
                info!("🔌 Connected to cTrader WebSocket");
            }
            Some("ERROR") => {
                error!("❌ Server error: {:?}", actual_message);
            }
            _ => {
                debug!("❓ Unknown message type: {:?}", actual_message.get("type"));
            }
        }

        Ok(())
    }
}

// HTML status page handler
async fn status_page(State(state): State<MonitorState>) -> Html<String> {
    let cache = state.zone_cache.read().await;
    let prices = state.latest_prices.read().await;
    let connected = *state.websocket_connected.read().await;

    let mut zones_by_symbol = Vec::new();
    for (symbol, zones) in &cache.zones {
        zones_by_symbol.push((symbol.clone(), zones.len()));
    }
    zones_by_symbol.sort_by(|a, b| a.0.cmp(&b.0));

    let html = format!(r#"
<!DOCTYPE html>
<html>
<head>
    <title>Zone Monitor Status</title>
    <meta http-equiv="refresh" content="5">
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        .status {{ padding: 10px; margin: 10px 0; border-radius: 5px; }}
        .connected {{ background-color: #d4edda; border: 1px solid #c3e6cb; }}
        .disconnected {{ background-color: #f8d7da; border: 1px solid #f5c6cb; }}
        .info {{ background-color: #d1ecf1; border: 1px solid #bee5eb; }}
        table {{ border-collapse: collapse; width: 100%; margin: 20px 0; }}
        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
        th {{ background-color: #f2f2f2; }}
        .supply {{ color: red; font-weight: bold; }}
        .demand {{ color: green; font-weight: bold; }}
    </style>
</head>
<body>
    <h1>Zone Monitor Status</h1>
    
    <div class="status {}">
        <h3>Connection Status</h3>
        <p>WebSocket: {}</p>
        <p>Last Updated: {}</p>
    </div>
    
    <div class="info">
        <h3>Zone Cache Summary</h3>
        <p>Total Zones: {}</p>
        <p>Cache Last Updated: {}</p>
        <p>Cache File: {}</p>
    </div>
    
    <h3>Zones by Symbol</h3>
    <table>
        <tr>
            <th>Symbol</th>
            <th>Zone Count</th>
            <th>Latest Price</th>
        </tr>
        {}
    </table>
    
    <h3>Recent Zone Details</h3>
    <table>
        <tr>
            <th>Symbol</th>
            <th>Type</th>
            <th>High</th>
            <th>Low</th>
            <th>Strength</th>
            <th>Created</th>
        </tr>
        {}
    </table>
    
    <p><em>Page auto-refreshes every 5 seconds</em></p>
</body>
</html>
    "#,
        if connected { "connected" } else { "disconnected" },
        if connected { "Connected ✅" } else { "Disconnected ❌" },
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        cache.total_zones,
        cache.last_updated.format("%Y-%m-%d %H:%M:%S UTC"),
        state.cache_file_path,
        zones_by_symbol.iter().map(|(symbol, count)| {
            let price_info = prices.get(symbol)
                .map(|p| format!("{:.5}", (p.bid + p.ask) / 2.0))
                .unwrap_or_else(|| "No data".to_string());
            format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", symbol, count, price_info)
        }).collect::<Vec<String>>().join("\n"),
        cache.zones.values().flatten()
            .take(20) // Show first 20 zones
            .map(|zone| format!(
                "<tr><td>{}</td><td class=\"{}\">{}</td><td>{:.5}</td><td>{:.5}</td><td>{:.2}</td><td>{}</td></tr>",
                zone.symbol,
                zone.zone_type,
                zone.zone_type.to_uppercase(),
                zone.high,
                zone.low,
                zone.strength,
                zone.created_at.format("%m/%d %H:%M")
            )).collect::<Vec<String>>().join("\n")
    );

    Html(html)
}

// JSON API endpoint for zone data
async fn zones_api(State(state): State<MonitorState>) -> Json<ZoneCache> {
    let cache = state.zone_cache.read().await;
    Json(cache.clone())
}

// Health check endpoint
async fn health_check() -> (StatusCode, &'static str) {
    (StatusCode::OK, "Zone Monitor OK")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    info!("Starting Zone Monitor...");

    // Get cache file path from environment or use default
    let cache_file_path =
        std::env::var("ZONE_CACHE_FILE").unwrap_or_else(|_| "shared_zones.json".to_string());

    // Initialize state
    let state = MonitorState::new(cache_file_path);

    // Load initial zones
    state.load_zones_from_file().await?;

    // Start background tasks
    state.start_cache_refresh_task().await;
    state.start_websocket_task().await;

    // Build web server for status
    let app = Router::new()
        .route("/", get(status_page))
        .route("/api/zones", get(zones_api))
        .route("/health", get(health_check))
        .with_state(state);

    // Start server
    let port = std::env::var("MONITOR_PORT")
        .unwrap_or_else(|_| "3003".to_string())
        .parse::<u16>()
        .unwrap_or(3003);

    info!("Zone Monitor starting on http://localhost:{}", port);
    info!("View status at: http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
