// src/bin/zone_monitor/state.rs
// Core state management for the zone monitor

use crate::proximity::ProximityDetector;
use crate::types::{PriceUpdate, ZoneAlert, ZoneCache};
use crate::websocket::WebSocketClient;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct MonitorState {
    pub zone_cache: Arc<RwLock<ZoneCache>>,
    pub latest_prices: Arc<RwLock<HashMap<String, PriceUpdate>>>,
    pub recent_alerts: Arc<RwLock<Vec<ZoneAlert>>>,
    pub websocket_client: Arc<WebSocketClient>,
    pub proximity_detector: Arc<ProximityDetector>,
    pub cache_file_path: String,
    pub websocket_url: String,
}

impl MonitorState {
    pub fn new(cache_file_path: String) -> Self {
        let websocket_url = std::env::var("CTRADER_WS_URL")
            .unwrap_or_else(|_| "ws://localhost:8081".to_string());

        Self {
            zone_cache: Arc::new(RwLock::new(ZoneCache::default())),
            latest_prices: Arc::new(RwLock::new(HashMap::new())),
            recent_alerts: Arc::new(RwLock::new(Vec::new())),
            websocket_client: Arc::new(WebSocketClient::new()),
            proximity_detector: Arc::new(ProximityDetector::new(10.0)), // 10 pips threshold
            cache_file_path,
            websocket_url,
        }
    }

    pub async fn initialize(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Load initial zones from file
        self.load_zones_from_file().await?;

        // Start background tasks
        self.start_cache_refresh_task().await;
        self.start_websocket_task().await;

        Ok(())
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

        info!("📄 Loaded {} zones from cache file", zone_cache.total_zones);
        Ok(())
    }

    async fn start_cache_refresh_task(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));

            loop {
                interval.tick().await;

                if let Err(e) = state.load_zones_from_file().await {
                    error!("❌ Failed to refresh zone cache: {}", e);
                }
            }
        });
    }

    async fn start_websocket_task(&self) {
        let websocket_client = Arc::clone(&self.websocket_client);
        let ws_url = self.websocket_url.clone();
        let state_for_handler = self.clone();

        tokio::spawn(async move {
            let message_handler = move |price_update: PriceUpdate| {
                let state_clone = state_for_handler.clone();
                tokio::spawn(async move {
                    state_clone.handle_price_update(price_update).await;
                });
            };

            websocket_client
                .start_connection_loop(ws_url, message_handler)
                .await;
        });
    }

    async fn handle_price_update(&self, price_update: PriceUpdate) {
        // Store the latest price
        {
            let mut prices = self.latest_prices.write().await;
            prices.insert(price_update.symbol.clone(), price_update.clone());
        }

        // Check zone proximity
        let zones = {
            let cache = self.zone_cache.read().await;
            cache
                .zones
                .get(&price_update.symbol)
                .cloned()
                .unwrap_or_default()
        };

        if !zones.is_empty() {
            let alerts = self
                .proximity_detector
                .check_zone_proximity(&price_update, &zones)
                .await;

            if !alerts.is_empty() {
                let mut recent_alerts = self.recent_alerts.write().await;
                for alert in alerts {
                    recent_alerts.push(alert);
                }
                
                // Keep only the last 50 alerts
                let len = recent_alerts.len();
                if len > 50 {
                    recent_alerts.drain(0..len - 50);
                }
            }
        }
    }

    // Getter methods for the web interface
    pub async fn get_connection_status(&self) -> bool {
        *self.websocket_client.connected.read().await
    }

    pub async fn get_connection_attempts(&self) -> u32 {
        *self.websocket_client.connection_attempts.read().await
    }

    pub async fn get_last_message_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.websocket_client.last_message_time.read().await
    }

    pub async fn get_zone_cache(&self) -> ZoneCache {
        self.zone_cache.read().await.clone()
    }

    pub async fn get_latest_prices(&self) -> HashMap<String, PriceUpdate> {
        self.latest_prices.read().await.clone()
    }

    pub async fn get_recent_alerts(&self) -> Vec<ZoneAlert> {
        self.recent_alerts.read().await.clone()
    }
}