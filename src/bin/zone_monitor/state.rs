// src/bin/zone_monitor/state.rs
// Core state management with zone state tracking, trading, and CSV logging

use crate::proximity::ProximityDetector;
use crate::types::{PriceUpdate, ZoneAlert, ZoneCache};
use crate::websocket::WebSocketClient;
use crate::notifications::NotificationManager;
use crate::zone_state_manager::ZoneStateManager;
use crate::trading_engine::TradingEngine;
use crate::csv_logger::CsvLogger;
use crate::telegram_notifier::TelegramNotifier;
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
    pub notification_manager: Arc<NotificationManager>,
    pub zone_state_manager: Arc<ZoneStateManager>,
    pub trading_engine: Arc<TradingEngine>,
    pub csv_logger: Arc<CsvLogger>,
    pub telegram_notifier: Arc<TelegramNotifier>,
    pub cache_file_path: String,
    pub websocket_url: String,
}

impl MonitorState {
    pub fn new(cache_file_path: String) -> Self {
        let websocket_url = std::env::var("PRICE_WS_URL")
            .unwrap_or_else(|_| "ws://localhost:8083".to_string());

        // Get proximity threshold for the detector (this will be deprecated)
        let proximity_threshold = std::env::var("PROXIMITY_THRESHOLD_PIPS")
            .unwrap_or_else(|_| "10.0".to_string())
            .parse()
            .unwrap_or(10.0);

        Self {
            zone_cache: Arc::new(RwLock::new(ZoneCache::default())),
            latest_prices: Arc::new(RwLock::new(HashMap::new())),
            recent_alerts: Arc::new(RwLock::new(Vec::new())),
            websocket_client: Arc::new(WebSocketClient::new()),
            proximity_detector: Arc::new(ProximityDetector::new(proximity_threshold)),
            notification_manager: Arc::new(NotificationManager::new()),
            zone_state_manager: Arc::new(ZoneStateManager::new()),
            trading_engine: Arc::new(TradingEngine::new()),
            csv_logger: Arc::new(CsvLogger::new()),
            telegram_notifier: Arc::new(TelegramNotifier::new()),
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
        self.start_notification_cleanup_task().await;
        self.start_zone_reset_task().await;

        // Test notifications if configured
        if self.notification_manager.is_configured() {
            info!("🧪 Testing notification system...");
            
            // Play startup sound
            self.notification_manager.play_startup_sound().await;
            
            let test_result = self.notification_manager.send_test_notification().await;
            if test_result {
                info!("✅ Notification system test successful");
            } else {
                warn!("⚠️ Notification system test failed");
            }
        }

        // Test Telegram if configured
        if self.telegram_notifier.is_enabled() {
            info!("🧪 Testing Telegram notifications...");
            if let Err(e) = self.telegram_notifier.send_test_message().await {
                warn!("⚠️ Telegram test failed: {}", e);
            } else {
                info!("✅ Telegram test successful");
            }
        }

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

    async fn start_zone_reset_task(&self) {
        let zone_state_manager = Arc::clone(&self.zone_state_manager);
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300)); // Check every 5 minutes
            
            loop {
                interval.tick().await;
                zone_state_manager.check_and_reset_expired_zones().await;
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

    async fn start_notification_cleanup_task(&self) {
        let zone_state_manager = Arc::clone(&self.zone_state_manager);
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600)); // Clean up every hour
            
            loop {
                interval.tick().await;
                zone_state_manager.cleanup_old_states(7).await; // Clean up states older than 7 days
            }
        });
    }

    async fn handle_price_update(&self, price_update: PriceUpdate) {
        // Store the latest price
        {
            let mut prices = self.latest_prices.write().await;
            prices.insert(price_update.symbol.clone(), price_update.clone());
        }

        // Get zones for this symbol
        let zones = {
            let cache = self.zone_cache.read().await;
            cache
                .zones
                .get(&price_update.symbol)
                .cloned()
                .unwrap_or_default()
        };

        if !zones.is_empty() {
            // Use the new ZoneStateManager to process the price update
            let result = self
                .zone_state_manager
                .process_price_update(&price_update, &zones)
                .await;

            // Handle proximity alerts (first time in proximity)
            for alert in result.proximity_alerts {
                // Store alert
                {
                    let mut recent_alerts = self.recent_alerts.write().await;
                    recent_alerts.push(alert.clone());
                    
                    // Keep only the last 50 alerts
                    let len = recent_alerts.len();
                    if len > 50 {
                        recent_alerts.drain(0..len - 50);
                    }
                }

                // Log to CSV
                self.csv_logger.log_proximity_alert(&alert).await;

                // Send to Telegram
                if let Err(e) = self.telegram_notifier.send_proximity_alert(&alert).await {
                    error!("Failed to send Telegram proximity alert: {}", e);
                }

                // Send proximity notification (sound)
                self.notification_manager.notify_zone_proximity(&alert).await;
            }

            // Handle trading signals (when price gets close enough to trade)
            for signal in result.trading_signals {
                info!("🎯 TRADING SIGNAL: {} {} zone @ {:.1} pips - READY TO TRADE!", 
                      signal.symbol, signal.zone_type, signal.distance_pips);
                
                // Log to CSV before attempting trade
                self.csv_logger.log_trading_signal(&signal, "SIGNAL_GENERATED").await;
                
                // Send to Telegram
                if let Err(e) = self.telegram_notifier.send_trading_signal(&signal, "SIGNAL_GENERATED").await {
                    error!("Failed to send Telegram trading signal: {}", e);
                }
                
                // Execute trade through trading engine
                if let Some(trade) = self.trading_engine.evaluate_and_execute_trade(&signal, &price_update).await {
                    info!("💰 Trade executed: {} {} {:.2} lots @ {:.5}", 
                          trade.symbol, trade.trade_type.to_uppercase(), trade.lot_size, trade.entry_price);
                    
                    // Log successful execution
                    self.csv_logger.log_trading_signal(&signal, "TRADE_EXECUTED").await;
                    
                    // Send to Telegram
                    if let Err(e) = self.telegram_notifier.send_trading_signal(&signal, "TRADE_EXECUTED").await {
                        error!("Failed to send Telegram trade execution: {}", e);
                    }
                    
                    // IMPORTANT: Only mark zone as traded AFTER successful execution
                    self.zone_state_manager.mark_zone_as_traded(&signal.zone_id, &signal.symbol).await;
                } else {
                    info!("🚫 Trade rejected for zone {} - zone remains in ProximityAlerted state", signal.zone_id);
                    
                    // Log rejection
                    self.csv_logger.log_trading_signal(&signal, "TRADE_REJECTED").await;
                    
                    // Send to Telegram
                    if let Err(e) = self.telegram_notifier.send_trading_signal(&signal, "TRADE_REJECTED").await {
                        error!("Failed to send Telegram trade rejection: {}", e);
                    }
                }
                
                // Store the trading signal as an alert too
                {
                    let mut recent_alerts = self.recent_alerts.write().await;
                    recent_alerts.push(signal.clone());
                    
                    let len = recent_alerts.len();
                    if len > 50 {
                        recent_alerts.drain(0..len - 50);
                    }
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

    // New method to get notification status
    pub async fn get_notification_status(&self) -> serde_json::Value {
        self.notification_manager.get_status()
    }

    // Manual test notification trigger
    pub async fn send_test_notification(&self) -> bool {
        self.notification_manager.send_test_notification().await
    }

    // Get cooldown stats - now from zone state manager
    pub async fn get_cooldown_stats(&self) -> serde_json::Value {
        let zone_stats = self.zone_state_manager.get_zone_stats().await;
        let thresholds = self.zone_state_manager.get_thresholds();
        
        serde_json::json!({
            "zone_stats": zone_stats,
            "proximity_threshold_pips": thresholds.0,
            "trading_threshold_pips": thresholds.1,
        })
    }

    // Check zone cooldown status - now returns zone state info
    pub async fn get_zone_cooldown_status(&self, zone_id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        let state_info = self.zone_state_manager.get_zone_state_info(zone_id).await;
        state_info.last_proximity_alert
    }

    // Zone state management methods
    pub async fn get_zone_state_info(&self, zone_id: &str) -> crate::zone_state_manager::ZoneStateInfo {
        self.zone_state_manager.get_zone_state_info(zone_id).await
    }

    pub async fn get_zone_stats(&self) -> crate::zone_state_manager::ZoneStats {
        self.zone_state_manager.get_zone_stats().await
    }

    pub async fn reset_zone_state(&self, zone_id: &str) {
        self.zone_state_manager.reset_zone_state(zone_id).await
    }

    pub async fn get_thresholds(&self) -> (f64, f64) {
        self.zone_state_manager.get_thresholds()
    }

    // Trading engine methods
    pub async fn get_trading_stats(&self) -> crate::trading_engine::TradingStats {
        self.trading_engine.get_trading_stats().await
    }

    pub async fn get_active_trades(&self) -> HashMap<String, crate::trading_engine::Trade> {
        self.trading_engine.get_active_trades().await
    }

    pub async fn get_trade_history(&self) -> Vec<crate::trading_engine::Trade> {
        self.trading_engine.get_trade_history().await
    }

    pub async fn get_trading_config(&self) -> crate::trading_engine::TradingConfig {
        self.trading_engine.get_config().await
    }

    pub async fn close_trade(&self, trade_id: &str, close_price: f64, reason: &str) -> Result<crate::trading_engine::Trade, String> {
        self.trading_engine.close_trade(trade_id, close_price, reason).await
    }
}