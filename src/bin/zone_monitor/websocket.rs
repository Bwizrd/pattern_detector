// src/bin/zone_monitor/websocket.rs
// Improved WebSocket connection based on ctrader_integration.rs

use crate::types::PriceUpdate;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, trace, warn};

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
            info!("🔌 [CTRADER] Attempting to connect to cTrader WebSocket...");

            // Increment connection attempts
            {
                let mut attempts = self.connection_attempts.write().await;
                *attempts += 1;
                info!("🔌 [CTRADER] Connection attempt #{}", *attempts);
            }

            // Set connected status to false
            {
                let mut connected = self.connected.write().await;
                *connected = false;
            }

            match self.connect_and_process(&ws_url, message_handler.clone()).await {
                Ok(_) => {
                    info!("✅ [CTRADER] WebSocket connection ended normally");
                }
                Err(e) => {
                    error!("❌ [CTRADER] WebSocket connection failed: {}", e);
                }
            }

            // Wait before reconnecting
            info!("⏳ [CTRADER] Reconnecting in 10 seconds...");
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
        info!("🔌 [CTRADER] Connecting to cTrader WebSocket at {}", ws_url);

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

                    if let Some(price_update) = self.process_ctrader_message(&text).await? {
                        message_handler(price_update);
                    }
                }
                Ok(Message::Close(_)) => {
                    warn!("🔌 [CTRADER] WebSocket connection closed");
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    if let Err(e) = ws_sender.send(Message::Pong(payload)).await {
                        error!("❌ [CTRADER] Failed to send pong: {}", e);
                    }
                }
                Err(e) => {
                    error!("❌ [CTRADER] WebSocket error: {}", e);
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

        info!("🔌 [CTRADER] WebSocket connection ended after {} messages", message_count);
        Ok(())
    }

    async fn send_subscriptions(
        &self,
        ws_sender: &mut futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
            Message,
        >,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Comprehensive symbol list from ctrader_integration.rs
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

        // Extended timeframes
        let timeframes = vec!["5m", "15m", "30m", "1h", "4h", "1d"];

        // Send subscription requests to cTrader server
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
                        "❌ [CTRADER] Failed to send subscription for {}/{}: {}",
                        symbol_name, timeframe, e
                    );
                } else {
                    debug!(
                        "✅ [CTRADER] Subscribed to {}/{} ({})",
                        symbol_name, timeframe, symbol_id
                    );
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }

        info!("📡 [CTRADER] Sent all subscription requests to server");
        Ok(())
    }

    async fn process_ctrader_message(
        &self,
        message: &str,
    ) -> Result<Option<PriceUpdate>, Box<dyn std::error::Error + Send + Sync>> {
        let data: serde_json::Value = serde_json::from_str(message)?;

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

                    if close > 0.0 && !symbol_name.is_empty() {
                        let price_update = PriceUpdate {
                            symbol: clean_symbol.to_string(),
                            bid: close - 0.00001, // Approximate bid
                            ask: close + 0.00001, // Approximate ask
                            timestamp: chrono::Utc::now(),
                        };

                        if is_new_bar {
                            info!("🆕 [CTRADER] New bar: {} {} {} @ {:.5}", symbol_id, clean_symbol, timeframe, close);
                        } else {
                            debug!("💹 [CTRADER] Price update: {} {} @ {:.5}", clean_symbol, timeframe, close);
                        }

                        return Ok(Some(price_update));
                    }
                }
            }
            Some("SUBSCRIPTION_CONFIRMED") => {
                if let (Some(symbol_id), Some(timeframe)) = (
                    data.get("symbolId").and_then(|s| s.as_u64()),
                    data.get("timeframe").and_then(|t| t.as_str()),
                ) {
                    debug!("✅ [CTRADER] Subscription confirmed: {}/{}", symbol_id, timeframe);
                }
            }
            Some("CONNECTED") => {
                info!("🔌 [CTRADER] Connected to cTrader WebSocket");
            }
            Some("ERROR") => {
                error!("❌ [CTRADER] Server error: {:?}", data);
            }
            _ => {
                trace!("❓ [CTRADER] Unknown message type: {:?}", data.get("type"));
            }
        }

        Ok(None)
    }
}