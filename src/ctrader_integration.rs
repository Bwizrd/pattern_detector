// src/ctrader_integration.rs - cTrader WebSocket integration with zone monitor
use std::env;
use log;
use serde_json::json;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use std::sync::Arc;

use crate::simple_price_websocket::broadcast_price_update;
use crate::realtime_zone_monitor::NewRealTimeZoneMonitor;

pub async fn connect_to_ctrader_websocket(
    price_broadcaster: &tokio::sync::broadcast::Sender<String>,
    zone_monitor: Option<Arc<NewRealTimeZoneMonitor>>, // Add zone monitor parameter
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ctrader_ws_url =
        env::var("CTRADER_WS_URL").unwrap_or_else(|_| "ws://localhost:8081".to_string());

    log::info!("🔌 [CTRADER] Connecting to cTrader WebSocket at {}", ctrader_ws_url);

    let (ws_stream, _) = connect_async(&ctrader_ws_url).await?;
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Subscribe to symbols we're monitoring
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

    let timeframes = vec!["5m", "15m", "30m", "1h", "4h", "1d"];

    // Send subscription requests to dummy server
    for (symbol_id, symbol_name) in &symbols_to_subscribe {
        for timeframe in &timeframes {
            let subscribe_msg = json!({
                "type": "SUBSCRIBE",
                "symbolId": symbol_id,
                "timeframe": timeframe
            });

            if let Err(e) = ws_sender.send(Message::Text(subscribe_msg.to_string())).await {
                log::error!("❌ [CTRADER] Failed to send subscription for {}/{}: {}", symbol_name, timeframe, e);
            } else {
                log::debug!("✅ [CTRADER] Subscribed to {}/{} ({})", symbol_name, timeframe, symbol_id);
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    log::info!("📡 [CTRADER] Sent all subscription requests to dummy server");

    // Process incoming messages
    let mut message_count = 0;
    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                message_count += 1;
                if let Err(e) = process_ctrader_message(&text, price_broadcaster, &zone_monitor).await {
                    log::warn!("⚠️ [CTRADER] Error processing message #{}: {}", message_count, e);
                }
            }
            Ok(Message::Close(_)) => {
                log::warn!("🔌 [CTRADER] WebSocket connection closed");
                break;
            }
            Ok(Message::Ping(payload)) => {
                if let Err(e) = ws_sender.send(Message::Pong(payload)).await {
                    log::error!("❌ [CTRADER] Failed to send pong: {}", e);
                }
            }
            Err(e) => {
                log::error!("❌ [CTRADER] WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    log::warn!("🔌 [CTRADER] WebSocket connection ended after {} messages", message_count);
    Ok(())
}

async fn process_ctrader_message(
    message: &str,
    price_broadcaster: &tokio::sync::broadcast::Sender<String>,
    zone_monitor: &Option<Arc<NewRealTimeZoneMonitor>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data: serde_json::Value = serde_json::from_str(message)?;
    
    // Broadcast to simple price WebSocket
    broadcast_price_update(price_broadcaster, message);

    match data.get("type").and_then(|t| t.as_str()) {
        Some("BAR_UPDATE") => {
            if let Some(bar_data) = data.get("data") {
                let symbol_id = bar_data["symbolId"].as_u64().unwrap_or(0);
                let timeframe = bar_data["timeframe"].as_str().unwrap_or("");
                let close = bar_data["close"].as_f64().unwrap_or(0.0);
                let is_new_bar = bar_data["isNewBar"].as_bool().unwrap_or(false);
                let symbol_name = bar_data["symbol"].as_str().unwrap_or("");

                // Convert symbol name (remove _SB suffix for zone monitor)
                let clean_symbol = symbol_name.trim_end_matches("_SB");

                // Update zone monitor with price
                if let Some(monitor) = zone_monitor {
                    if let Err(e) = monitor.update_price_with_cache_notifications(clean_symbol, close, timeframe).await {
                        log::debug!("⚠️ [CTRADER] Zone monitor update failed for {}/{}: {}", clean_symbol, timeframe, e);
                    }
                }

                if is_new_bar {
                    log::debug!("🆕 [CTRADER] New bar: {} {} {} @ {:.5}", symbol_id, clean_symbol, timeframe, close);
                }
            }
        }
        Some("SUBSCRIPTION_CONFIRMED") => {
            if let (Some(symbol_id), Some(timeframe)) = (
                data.get("symbolId").and_then(|s| s.as_u64()),
                data.get("timeframe").and_then(|t| t.as_str()),
            ) {
                log::debug!("✅ [CTRADER] Subscription confirmed: {}/{}", symbol_id, timeframe);
            }
        }
        Some("CONNECTED") => {
            log::info!("🔌 [CTRADER] Connected to dummy cTrader WebSocket");
        }
        Some("ERROR") => {
            log::error!("❌ [CTRADER] Server error: {:?}", data);
        }
        _ => {
            log::trace!("❓ [CTRADER] Unknown message type: {:?}", data.get("type"));
        }
    }

    Ok(())
}