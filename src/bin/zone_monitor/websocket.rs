// src/bin/zone_monitor/websocket.rs
// WebSocket connection and message processing

use crate::types::PriceUpdate;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

#[derive(Debug)]
pub struct WebSocketClient {
    pub connected: Arc<RwLock<bool>>,
    pub connection_attempts: Arc<RwLock<u32>>,
    pub last_message_time: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
}

impl WebSocketClient {
    pub fn new() -> Self {
        Self {
            connected: Arc::new(RwLock::new(false)),
            connection_attempts: Arc::new(RwLock::new(0)),
            last_message_time: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start_connection_loop<F>(&self, ws_url: String, message_handler: F)
    where
        F: Fn(PriceUpdate) + Send + Sync + 'static,
        F: Clone,
    {
        let message_handler = Arc::new(message_handler);
        
        loop {
            info!("🔌 Attempting to connect to WebSocket...");

            // Increment connection attempts
            {
                let mut attempts = self.connection_attempts.write().await;
                *attempts += 1;
                info!("Connection attempt #{}", *attempts);
            }

            // Set connected status to false
            {
                let mut connected = self.connected.write().await;
                *connected = false;
            }

            match self.connect_and_process(&ws_url, message_handler.clone()).await {
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
    }

    async fn connect_and_process<F>(
        &self,
        ws_url: &str,
        message_handler: Arc<F>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: Fn(PriceUpdate) + Send + Sync + 'static,
    {
        info!("🔌 Connecting to WebSocket at {}", ws_url);

        let (ws_stream, _) = connect_async(ws_url).await?;
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Set connected status
        {
            let mut connected = self.connected.write().await;
            *connected = true;
        }

        // Send subscription requests
        self.send_subscriptions(&mut ws_sender).await?;

        // Process incoming messages
        let mut message_count = 0;
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    message_count += 1;

                    // Update last message time
                    {
                        let mut last_time = self.last_message_time.write().await;
                        *last_time = Some(chrono::Utc::now());
                    }

                    if let Some(price_update) = self.process_message(&text).await? {
                        message_handler(price_update);
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
            let mut connected = self.connected.write().await;
            *connected = false;
        }

        info!("🔌 WebSocket connection ended after {} messages", message_count);
        Ok(())
    }

    async fn send_subscriptions(
        &self,
        ws_sender: &mut futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
            Message,
        >,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Subscribe to major currency pairs
        let symbols_to_subscribe = vec![
            (185, "EURUSD_SB"),
            (199, "GBPUSD_SB"),
            (226, "USDJPY_SB"),
            (222, "USDCHF_SB"),
            (158, "AUDUSD_SB"),
            (221, "USDCAD_SB"),
        ];

        let timeframes = vec!["5m", "15m", "1h"];

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
        Ok(())
    }

    async fn process_message(
        &self,
        message: &str,
    ) -> Result<Option<PriceUpdate>, Box<dyn std::error::Error + Send + Sync>> {
        let data: serde_json::Value = serde_json::from_str(message)?;

        // Handle the wrapped message format from simple_price_websocket
        let actual_message =
            if data.get("type").and_then(|t| t.as_str()) == Some("raw_price_update") {
                data.get("data").unwrap_or(&data)
            } else {
                &data
            };

        match actual_message.get("type").and_then(|t| t.as_str()) {
            Some("BAR_UPDATE") => {
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

                        if is_new_bar {
                            info!("🆕 New bar: {} @ {:.5}", clean_symbol, close);
                        } else {
                            debug!("💹 Price update: {} @ {:.5}", clean_symbol, close);
                        }

                        return Ok(Some(price_update));
                    }
                }
            }
            Some("SUBSCRIPTION_CONFIRMED") => {
                if let (Some(symbol_id), Some(timeframe)) = (
                    actual_message.get("symbolId").and_then(|s| s.as_u64()),
                    actual_message.get("timeframe").and_then(|t| t.as_str()),
                ) {
                    debug!("✅ Subscription confirmed: {}/{}", symbol_id, timeframe);
                }
            }
            Some("CONNECTED") => {
                info!("🔌 Connected to WebSocket server");
            }
            Some("ERROR") => {
                error!("❌ Server error: {:?}", actual_message);
            }
            _ => {
                debug!("❓ Unknown message type: {:?}", actual_message.get("type"));
            }
        }

        Ok(None)
    }
}