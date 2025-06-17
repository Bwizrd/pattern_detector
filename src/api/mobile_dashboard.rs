// src/api/mobile_dashboard.rs
use actix_web::{web, HttpResponse, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use crate::cache::minimal_zone_cache::MinimalZoneCache;
use log::{debug, warn};
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};

// Price update structure matching the WebSocket client
#[derive(Debug, Clone)]
pub struct PriceUpdate {
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub timestamp: DateTime<Utc>,
}

// Global price storage for the mobile dashboard
pub struct LivePriceManager {
    pub prices: Arc<RwLock<HashMap<String, PriceUpdate>>>,
    pub websocket_connected: Arc<RwLock<bool>>,
}

impl LivePriceManager {
    pub fn new() -> Self {
        Self {
            prices: Arc::new(RwLock::new(HashMap::new())),
            websocket_connected: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn update_price(&self, price_update: PriceUpdate) {
        let mut prices = self.prices.write().await;
        prices.insert(price_update.symbol.clone(), price_update);
    }

    pub async fn get_prices(&self) -> HashMap<String, f64> {
        let prices = self.prices.read().await;
        prices.iter().map(|(symbol, update)| {
            (symbol.clone(), (update.bid + update.ask) / 2.0) // Use mid price
        }).collect()
    }

    pub async fn set_connection_status(&self, connected: bool) {
        let mut status = self.websocket_connected.write().await;
        *status = connected;
    }

    pub async fn is_connected(&self) -> bool {
        *self.websocket_connected.read().await
    }
}

pub async fn serve_dashboard() -> Result<HttpResponse> {
    let html = include_str!("../../static/dashboard.html");
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

pub async fn get_dashboard_data(
    cache: web::Data<Arc<tokio::sync::Mutex<MinimalZoneCache>>>,
    price_manager: web::Data<Arc<LivePriceManager>>,
) -> Result<HttpResponse> {
    debug!("Fetching dashboard data...");
    
    // Get zones from cache (same as terminal dashboard)
    let zones_data = match get_zones_from_cache(&cache).await {
        Ok(zones) => zones,
        Err(e) => {
            warn!("Failed to get zones from cache: {}", e);
            Vec::new()
        }
    };

    // Get current prices from live WebSocket feed
    let prices = price_manager.get_prices().await;
    let prices_json = serde_json::to_value(&prices).unwrap_or_else(|_| json!({}));
    
    // Process zones similar to terminal dashboard logic
    let processed_zones = process_zones_for_mobile(&zones_data, &prices);
    
    // Calculate stats
    let stats = calculate_zone_stats(&processed_zones);
    
    // Check WebSocket connection status
    let websocket_connected = price_manager.is_connected().await;
    
    // Get live trades data
    let live_trades = get_live_trades().await;
    let total_pips = live_trades.iter()
        .filter(|trade| trade["status"].as_str().unwrap_or("") == "Open")
        .map(|trade| trade["unrealized_pips"].as_f64().unwrap_or(0.0))
        .sum::<f64>();

    let dashboard_data = json!({
        "zones": processed_zones,
        "stats": stats,
        "trades": live_trades,
        "total_pips": total_pips,
        "prices": prices_json,
        "last_update": chrono::Utc::now().to_rfc3339(),
        "websocket_connected": websocket_connected,
        "trading_enabled": std::env::var("TRADING_ENABLED").unwrap_or_else(|_| "false".to_string()) == "true"
    });

    Ok(HttpResponse::Ok().json(dashboard_data))
}

async fn get_zones_from_cache(
    cache: &web::Data<Arc<tokio::sync::Mutex<MinimalZoneCache>>>,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let cache_guard = cache.lock().await;
    
    // Get all zones from cache using the get_all_zones method
    let enriched_zones = cache_guard.get_all_zones();
    
    let mut all_zones = Vec::new();
    
    for zone in &enriched_zones {
        // Convert to the format expected by mobile dashboard
        let zone_json = json!({
            "zone_id": zone.zone_id.as_deref().unwrap_or("unknown"),
            "symbol": zone.symbol.as_deref().unwrap_or("unknown"),
            "timeframe": zone.timeframe.as_deref().unwrap_or("unknown"),
            "type": zone.zone_type.as_deref().unwrap_or("unknown"),
            "zone_high": zone.zone_high.unwrap_or(0.0),
            "zone_low": zone.zone_low.unwrap_or(0.0),
            "touch_count": zone.touch_count.unwrap_or(0),
            "strength_score": zone.strength_score.unwrap_or(0.0),
            "created_at": zone.start_time.as_deref(), // Use start_time as created_at
            "start_time": zone.start_time.as_deref()
        });
        all_zones.push(zone_json);
    }
    
    debug!("Retrieved {} zones from cache", all_zones.len());
    Ok(all_zones)
}

fn process_zones_for_mobile(zones_data: &[Value], live_prices: &HashMap<String, f64>) -> Vec<Value> {
    
    let mut processed_zones = Vec::new();
    
    for zone_json in zones_data {
        if let Ok(zone_info) = extract_zone_info_for_mobile(zone_json, live_prices) {
            processed_zones.push(zone_info);
        }
    }
    
    // Sort by status priority (triggers first, then by distance)
    processed_zones.sort_by(|a, b| {
        let a_priority = get_status_priority(a["status"].as_str().unwrap_or(""));
        let b_priority = get_status_priority(b["status"].as_str().unwrap_or(""));
        
        b_priority.partial_cmp(&a_priority).unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let a_dist = a["distance"].as_f64().unwrap_or(0.0).abs();
                let b_dist = b["distance"].as_f64().unwrap_or(0.0).abs();
                a_dist.partial_cmp(&b_dist).unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    
    processed_zones
}

fn extract_zone_info_for_mobile(
    zone_json: &Value,
    prices: &HashMap<String, f64>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let symbol = zone_json["symbol"].as_str().ok_or("Missing symbol")?;
    let timeframe = zone_json["timeframe"].as_str().ok_or("Missing timeframe")?;
    let zone_type = zone_json["type"].as_str().ok_or("Missing type")?;
    let zone_high = zone_json["zone_high"].as_f64().ok_or("Missing zone_high")?;
    let zone_low = zone_json["zone_low"].as_f64().ok_or("Missing zone_low")?;
    let strength = zone_json["strength_score"].as_f64().unwrap_or(0.0);
    let touch_count = zone_json["touch_count"].as_i64().unwrap_or(0);
    let zone_id = zone_json["zone_id"].as_str().unwrap_or("unknown");
    
    // Get current price
    let current_price = prices.get(symbol).copied().unwrap_or(0.0);
    
    // Calculate distance and status (simplified logic)
    let (proximal_line, distal_line) = if zone_type.contains("supply") {
        (zone_low, zone_high) // Supply: proximal = low, distal = high
    } else {
        (zone_high, zone_low) // Demand: proximal = high, distal = low
    };
    
    let distance_to_proximal = current_price - proximal_line;
    let pip_value = get_pip_value(symbol);
    let distance_pips = distance_to_proximal / pip_value;
    
    // Determine status based on distance
    let status = if current_price >= zone_low && current_price <= zone_high {
        "InsideZone"
    } else if distance_pips.abs() <= 0.5 { // Within trading threshold
        "AtProximal"
    } else if distance_pips.abs() <= 10.0 {
        "Approaching"
    } else if distance_pips.abs() <= 25.0 {
        "Watching"
    } else {
        "Distant"
    };
    
    Ok(json!({
        "zone_id": zone_id,
        "symbol": symbol,
        "timeframe": timeframe,
        "type": zone_type,
        "current_price": current_price,
        "proximal_line": proximal_line,
        "distal_line": distal_line,
        "distance": distance_pips,
        "status": status,
        "strength": strength,
        "touch_count": touch_count,
        "created_at": zone_json["created_at"]
    }))
}

fn calculate_zone_stats(zones: &[Value]) -> Value {
    let total = zones.len();
    let triggers = zones.iter().filter(|z| z["status"] == "AtProximal").count();
    let inside = zones.iter().filter(|z| z["status"] == "InsideZone").count();
    let close = zones.iter().filter(|z| {
        z["distance"].as_f64().unwrap_or(999.0).abs() < 10.0
    }).count();
    let watch = zones.iter().filter(|z| {
        z["distance"].as_f64().unwrap_or(999.0).abs() < 25.0
    }).count();
    
    json!({
        "total": total,
        "triggers": triggers,
        "inside": inside,
        "close": close,
        "watch": watch
    })
}

fn get_status_priority(status: &str) -> i32 {
    match status {
        "AtProximal" => 100,
        "InsideZone" => 90,
        "Approaching" => 80,
        "Watching" => 70,
        _ => 0,
    }
}

fn get_pip_value(symbol: &str) -> f64 {
    match symbol {
        s if s.contains("JPY") => 0.01,
        "NAS100" => 1.0,
        "US500" => 0.1,
        _ => 0.0001,
    }
}

// Get live trades from shared trade data (similar to desktop dashboard)
async fn get_live_trades() -> Vec<Value> {
    // Try to read from shared trade files that zone monitor writes
    if let Ok(trades_content) = tokio::fs::read_to_string("shared_notifications.json").await {
        if let Ok(notifications) = serde_json::from_str::<Vec<Value>>(&trades_content) {
            return notifications.into_iter()
                .filter_map(|notification| {
                    // Convert trade notifications to live trade format
                    let symbol = notification["symbol"].as_str().unwrap_or("");
                    let action = notification["action"].as_str().unwrap_or("");
                    let price = notification["price"].as_f64().unwrap_or(0.0);
                    let timeframe = notification["timeframe"].as_str().unwrap_or("");
                    let zone_id = notification["zone_id"].as_str().unwrap_or("");
                    
                    if !symbol.is_empty() && price > 0.0 {
                        // Calculate current P&L (simplified - would use real prices in production)
                        let current_price = price; // TODO: Use live prices when available
                        let pip_value = get_pip_value(symbol);
                        let direction = if action.contains("BUY") { "BUY" } else { "SELL" };
                        
                        let unrealized_pips = if direction == "BUY" {
                            (current_price - price) / pip_value
                        } else {
                            (price - current_price) / pip_value
                        };
                        
                        Some(json!({
                            "symbol": symbol,
                            "timeframe": timeframe,
                            "direction": direction,
                            "entry_price": price,
                            "unrealized_pips": unrealized_pips,
                            "zone_id": zone_id,
                            "status": "Open",
                            "timestamp": notification["timestamp"]
                        }))
                    } else {
                        None
                    }
                })
                .collect();
        }
    }
    
    // If no live trades, return empty array
    Vec::new()
}

// WebSocket client setup for live price feeds
pub async fn start_price_websocket(price_manager: Arc<LivePriceManager>) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use serde_json::json;
    use std::time::Duration;

    let ws_url = std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:8081".to_string());
    let price_manager_clone = Arc::clone(&price_manager);

    tokio::spawn(async move {
        loop {
            debug!("🔌 [MOBILE] Attempting to connect to WebSocket at {}", ws_url);
            
            match connect_async(&ws_url).await {
                Ok((ws_stream, _)) => {
                    debug!("✅ [MOBILE] Connected to price WebSocket");
                    price_manager_clone.set_connection_status(true).await;
                    
                    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
                    
                    // Send subscription requests for all symbols
                    let symbols_to_subscribe = vec![
                        (185, "EURUSD_SB"), (199, "GBPUSD_SB"), (226, "USDJPY_SB"),
                        (222, "USDCHF_SB"), (158, "AUDUSD_SB"), (221, "USDCAD_SB"),
                        (211, "NZDUSD_SB"), (175, "EURGBP_SB"), (177, "EURJPY_SB"),
                        (173, "EURCHF_SB"), (171, "EURAUD_SB"), (172, "EURCAD_SB"),
                        (180, "EURNZD_SB"), (192, "GBPJPY_SB"), (191, "GBPCHF_SB"),
                        (189, "GBPAUD_SB"), (190, "GBPCAD_SB"), (195, "GBPNZD_SB"),
                        (155, "AUDJPY_SB"), (156, "AUDNZD_SB"), (153, "AUDCAD_SB"),
                        (210, "NZDJPY_SB"), (162, "CADJPY_SB"), (163, "CHFJPY_SB"),
                        (205, "NAS100_SB"), (220, "US500_SB"),
                    ];
                    
                    for (symbol_id, _symbol_name) in &symbols_to_subscribe {
                        let subscribe_msg = json!({
                            "type": "SUBSCRIBE",
                            "symbolId": symbol_id,
                            "timeframe": "1h"
                        });
                        
                        if let Err(e) = ws_sender.send(Message::Text(subscribe_msg.to_string())).await {
                            warn!("❌ [MOBILE] Failed to subscribe to {}: {}", symbol_id, e);
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    
                    // Process messages
                    while let Some(msg) = ws_receiver.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if data.get("type").and_then(|t| t.as_str()) == Some("BAR_UPDATE") {
                                        if let Some(bar_data) = data.get("data") {
                                            let close = bar_data["close"].as_f64().unwrap_or(0.0);
                                            let symbol_name = bar_data["symbol"].as_str().unwrap_or("");
                                            let clean_symbol = symbol_name.trim_end_matches("_SB");
                                            
                                            if close > 0.0 && !clean_symbol.is_empty() {
                                                let price_update = PriceUpdate {
                                                    symbol: clean_symbol.to_string(),
                                                    bid: close - 0.00001,
                                                    ask: close + 0.00001,
                                                    timestamp: Utc::now(),
                                                };
                                                
                                                price_manager_clone.update_price(price_update).await;
                                                debug!("💹 [MOBILE] Price update: {} @ {:.5}", clean_symbol, close);
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(Message::Close(_)) => {
                                warn!("🔌 [MOBILE] WebSocket connection closed");
                                break;
                            }
                            Err(e) => {
                                warn!("❌ [MOBILE] WebSocket error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    warn!("❌ [MOBILE] Failed to connect to WebSocket: {}", e);
                }
            }
            
            price_manager_clone.set_connection_status(false).await;
            debug!("⏳ [MOBILE] Reconnecting in 10 seconds...");
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
}