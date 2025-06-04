// src/realtime_zone_monitor.rs - Fixed implementation
use chrono::{DateTime, Utc};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use crate::minimal_zone_cache::MinimalZoneCache;
use crate::minimal_zone_cache::TradeNotification;
use crate::trade_decision_engine::{TradeDecisionEngine, ValidatedTradeSignal};
use crate::types::EnrichedZone;

use crate::notification_manager::get_global_notification_manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewZoneEvent {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub event_type: NewZoneEventType,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
    pub zone_type: String, // "supply" or "demand"
    pub confirmed: bool,   // true = cache confirmed, false = websocket only
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NewZoneEventType {
    Touch,        // Price reached proximal line
    Invalidation, // Price broke distal line
    NewZone,      // New zone detected in cache
    ZoneRemoved,  // Zone no longer active
}

#[derive(Debug, Clone)]
pub struct LiveZoneStatus {
    pub zone: EnrichedZone,
    pub last_price: Option<f64>,
    pub last_price_time: Option<DateTime<Utc>>,
    pub pending_touch: bool,
    pub pending_invalidation: bool,
    pub websocket_touch_count: i32, // Real-time touches (unconfirmed)
    pub cache_touch_count: i32,     // Cache-confirmed touches
}

pub struct NewRealTimeZoneMonitor {
    // Zone data synchronized with cache
    active_zones: Arc<RwLock<HashMap<String, LiveZoneStatus>>>,

    // Event broadcasting
    event_sender: tokio::sync::broadcast::Sender<NewZoneEvent>,

    // Price tracking
    latest_prices: Arc<RwLock<HashMap<String, (f64, DateTime<Utc>)>>>, // symbol -> (price, time)

    // Reference to shared cache
    zone_cache: Arc<Mutex<MinimalZoneCache>>,

    trade_engine: Arc<Mutex<TradeDecisionEngine>>,
}

impl NewRealTimeZoneMonitor {
    pub fn new(
        zone_cache: Arc<Mutex<MinimalZoneCache>>,
        event_capacity: usize,
    ) -> (Self, tokio::sync::broadcast::Receiver<NewZoneEvent>) {
        let (event_sender, event_receiver) = tokio::sync::broadcast::channel(event_capacity);

        let monitor = Self {
            active_zones: Arc::new(RwLock::new(HashMap::new())),
            event_sender,
            latest_prices: Arc::new(RwLock::new(HashMap::new())),
            zone_cache: zone_cache.clone(),
            trade_engine: Arc::new(Mutex::new(TradeDecisionEngine::new_with_cache(zone_cache))),
        };

        (monitor, event_receiver)
    }

    /// Log mismatches between WebSocket and Cache detections
    fn log_mismatch(&self, mismatch_type: &str, zone_id: &str, details: &str) {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_entry = format!(
            "[{}] MISMATCH_{}: Zone {} - {}\n",
            timestamp, mismatch_type, zone_id, details
        );

        // Log to file
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open("zone_mismatch_log.txt")
        {
            let _ = file.write_all(log_entry.as_bytes());
        }

        // Also log to console
        warn!(
            "🔍 [MISMATCH] {}: Zone {} - {}",
            mismatch_type, zone_id, details
        );
    }

    /// Sync zones from cache (called after cache updates)
    pub async fn sync_with_cache(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("🔄 [NEW_ZONE_MONITOR] Syncing with cache...");

        // Get fresh zones from cache
        let cache_zones = {
            let cache_guard = self.zone_cache.lock().await;
            cache_guard.get_all_zones()
        };

        let mut zones_guard = self.active_zones.write().await;
        let mut events_to_send = Vec::new();

        // Track current zone IDs
        let mut new_zone_ids = std::collections::HashSet::new();

        // Process each zone from cache
        for cache_zone in cache_zones {
            if let Some(zone_id) = &cache_zone.zone_id {
                new_zone_ids.insert(zone_id.clone());

                match zones_guard.get_mut(zone_id) {
                    Some(existing_live_zone) => {
                        // Update existing zone
                        let old_touch_count = existing_live_zone.cache_touch_count;
                        existing_live_zone.zone = cache_zone.clone();
                        existing_live_zone.cache_touch_count =
                            cache_zone.touch_count.unwrap_or(0) as i32;

                        // Check if cache confirmed a touch
                        if existing_live_zone.cache_touch_count > old_touch_count {
                            if existing_live_zone.pending_touch {
                                info!("✅ [NEW_ZONE_MONITOR] Cache CONFIRMED WebSocket touch for zone {}", zone_id);
                                existing_live_zone.pending_touch = false;
                            } else {
                                // Cache detected touch that WebSocket missed
                                self.log_mismatch(
                                    "CACHE_TOUCH_ONLY",
                                    zone_id,
                                    &format!(
                                        "Cache detected touch (count: {} -> {}) but WebSocket had no pending touch. Last price: {:?}",
                                        old_touch_count,
                                        existing_live_zone.cache_touch_count,
                                        existing_live_zone.last_price
                                    )
                                );
                            }
                        } else if existing_live_zone.pending_touch {
                            // WebSocket detected touch but cache didn't confirm
                            self.log_mismatch(
                                "WEBSOCKET_FALSE_POSITIVE",
                                zone_id,
                                &format!(
                                    "WebSocket detected touch but cache didn't confirm. WS touches: {}, Cache touches: {}. Last price: {:?}",
                                    existing_live_zone.websocket_touch_count,
                                    existing_live_zone.cache_touch_count,
                                    existing_live_zone.last_price
                                )
                            );
                            existing_live_zone.pending_touch = false;
                        }

                        // Check if zone was invalidated by cache
                        if !cache_zone.is_active && existing_live_zone.zone.is_active {
                            if existing_live_zone.pending_invalidation {
                                info!("✅ [NEW_ZONE_MONITOR] Cache CONFIRMED WebSocket invalidation for zone {}", zone_id);
                                existing_live_zone.pending_invalidation = false;
                            } else {
                                // Cache detected invalidation that WebSocket missed
                                self.log_mismatch(
                                    "CACHE_INVALIDATION_ONLY",
                                    zone_id,
                                    &format!(
                                        "Cache invalidated zone but WebSocket had no pending invalidation. Last price: {:?}",
                                        existing_live_zone.last_price
                                    )
                                );
                            }

                            events_to_send.push(NewZoneEvent {
                                zone_id: zone_id.clone(),
                                symbol: cache_zone.symbol.clone().unwrap_or_default(),
                                timeframe: cache_zone.timeframe.clone().unwrap_or_default(),
                                event_type: NewZoneEventType::Invalidation,
                                price: existing_live_zone.last_price.unwrap_or(0.0),
                                timestamp: Utc::now(),
                                zone_type: if cache_zone
                                    .zone_type
                                    .as_deref()
                                    .unwrap_or("")
                                    .contains("supply")
                                {
                                    "supply".to_string()
                                } else {
                                    "demand".to_string()
                                },
                                confirmed: true,
                            });
                        } else if existing_live_zone.pending_invalidation && cache_zone.is_active {
                            // WebSocket detected invalidation but cache didn't confirm
                            self.log_mismatch(
                                "WEBSOCKET_INVALIDATION_FALSE_POSITIVE",
                                zone_id,
                                &format!(
                                    "WebSocket detected invalidation but cache still shows zone active. Last price: {:?}",
                                    existing_live_zone.last_price
                                )
                            );
                            existing_live_zone.pending_invalidation = false;
                        }
                    }
                    None => {
                        // New zone detected
                        info!("🆕 [NEW_ZONE_MONITOR] New zone detected: {}", zone_id);

                        let live_zone = LiveZoneStatus {
                            zone: cache_zone.clone(),
                            last_price: None,
                            last_price_time: None,
                            pending_touch: false,
                            pending_invalidation: false,
                            websocket_touch_count: 0,
                            cache_touch_count: cache_zone.touch_count.unwrap_or(0) as i32,
                        };

                        zones_guard.insert(zone_id.clone(), live_zone);

                        events_to_send.push(NewZoneEvent {
                            zone_id: zone_id.clone(),
                            symbol: cache_zone.symbol.clone().unwrap_or_default(),
                            timeframe: cache_zone.timeframe.clone().unwrap_or_default(),
                            event_type: NewZoneEventType::NewZone,
                            price: 0.0,
                            timestamp: Utc::now(),
                            zone_type: if cache_zone
                                .zone_type
                                .as_deref()
                                .unwrap_or("")
                                .contains("supply")
                            {
                                "supply".to_string()
                            } else {
                                "demand".to_string()
                            },
                            confirmed: true,
                        });
                    }
                }
            }
        }

        // Remove zones that are no longer in cache
        zones_guard.retain(|zone_id, _| {
            if new_zone_ids.contains(zone_id) {
                true
            } else {
                info!(
                    "🗑️ [NEW_ZONE_MONITOR] Removing zone no longer in cache: {}",
                    zone_id
                );
                events_to_send.push(NewZoneEvent {
                    zone_id: zone_id.clone(),
                    symbol: "".to_string(),
                    timeframe: "".to_string(),
                    event_type: NewZoneEventType::ZoneRemoved,
                    price: 0.0,
                    timestamp: Utc::now(),
                    zone_type: "".to_string(),
                    confirmed: true,
                });
                false
            }
        });

        drop(zones_guard);

        // // Send all events
        for event in events_to_send {
            // if let Err(e) = self.event_sender.send(event) {
            //     warn!("⚠️ [NEW_ZONE_MONITOR] Failed to send event: {}", e);
            // }
            let _ = self.event_sender.send(event);
        }

        info!(
            "✅ [NEW_ZONE_MONITOR] Sync complete with {} active zones",
            new_zone_ids.len()
        );
        Ok(())
    }

    /// Update price for a symbol and check all zones
    pub async fn update_price(
        &self,
        symbol: &str,
        price: f64,
        timeframe: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = Utc::now();

        // Update latest price
        {
            let mut prices_guard = self.latest_prices.write().await;
            let price_key = format!("{}_{}", symbol, timeframe);
            prices_guard.insert(price_key, (price, now));
        }

        // Check all zones for this symbol/timeframe
        let mut zones_guard = self.active_zones.write().await;
        let mut events_to_send = Vec::new();

        for (zone_id, live_zone) in zones_guard.iter_mut() {
            // Only check zones matching this symbol/timeframe
            let zone_symbol = live_zone.zone.symbol.as_deref().unwrap_or("");
            let zone_timeframe = live_zone.zone.timeframe.as_deref().unwrap_or("");

            if zone_symbol != symbol || zone_timeframe != timeframe {
                continue;
            }

            // Skip inactive zones
            if !live_zone.zone.is_active {
                continue;
            }

            // Update price tracking
            live_zone.last_price = Some(price);
            live_zone.last_price_time = Some(now);

            let zone_high = live_zone.zone.zone_high.unwrap_or(0.0);
            let zone_low = live_zone.zone.zone_low.unwrap_or(0.0);
            let is_supply = live_zone
                .zone
                .zone_type
                .as_deref()
                .unwrap_or("")
                .contains("supply");

            let (proximal_line, distal_line) = if is_supply {
                (zone_low, zone_high)
            } else {
                (zone_high, zone_low)
            };

            // Check for invalidation (distal break)
            let invalidated = if is_supply {
                price > distal_line
            } else {
                price < distal_line
            };

            if invalidated && !live_zone.pending_invalidation {
                warn!("❌ [NEW_ZONE_MONITOR] WebSocket detected invalidation for zone {} at price {:.5}", zone_id, price);
                live_zone.pending_invalidation = true;

                events_to_send.push(NewZoneEvent {
                    zone_id: zone_id.clone(),
                    symbol: symbol.to_string(),
                    timeframe: timeframe.to_string(),
                    event_type: NewZoneEventType::Invalidation,
                    price,
                    timestamp: now,
                    zone_type: if is_supply {
                        "supply".to_string()
                    } else {
                        "demand".to_string()
                    },
                    confirmed: false, // WebSocket detection, needs cache confirmation
                });
                continue; // Skip touch check if invalidated
            }

            // Check for touch (proximal reach)
            let touched = if is_supply {
                price >= proximal_line
            } else {
                price <= proximal_line
            };

            if touched && !live_zone.pending_touch {
                info!(
                    "🎯 [NEW_ZONE_MONITOR] WebSocket detected touch for zone {} at price {:.5}",
                    zone_id, price
                );
                live_zone.pending_touch = true;
                live_zone.websocket_touch_count += 1;

                events_to_send.push(NewZoneEvent {
                    zone_id: zone_id.clone(),
                    symbol: symbol.to_string(),
                    timeframe: timeframe.to_string(),
                    event_type: NewZoneEventType::Touch,
                    price,
                    timestamp: now,
                    zone_type: if is_supply {
                        "supply".to_string()
                    } else {
                        "demand".to_string()
                    },
                    confirmed: false, // WebSocket detection, needs cache confirmation
                });
            }
        }

        drop(zones_guard);

        // Send all events
        for event in events_to_send {
            // if let Err(e) = self.event_sender.send(event) {
            //     warn!("⚠️ [NEW_ZONE_MONITOR] Failed to send event: {}", e);
            // }
            let _ = self.event_sender.send(event);
        }

        Ok(())
    }

    /// Get all zones for WebSocket clients (compatible with existing websocket_server.rs)
    pub async fn get_all_zones_for_ws(&self) -> Vec<serde_json::Value> {
        let zones_guard = self.active_zones.read().await;
        let live_zones: Vec<LiveZoneStatus> = zones_guard.values().cloned().collect();

        // Convert to JSON format expected by websocket_server.rs
        live_zones
            .into_iter()
            .filter_map(|live_zone| serde_json::to_value(&live_zone.zone).ok())
            .collect()
    }

    /// Get current event receiver for new subscribers
    pub fn subscribe_to_events(&self) -> tokio::sync::broadcast::Receiver<NewZoneEvent> {
        self.event_sender.subscribe()
    }

    pub async fn get_current_price(&self, symbol: &str) -> Option<f64> {
        let prices_guard = self.latest_prices.read().await;

        // Try to find the most recent price for this symbol across all timeframes
        let mut latest_price = None;
        let mut latest_time = None;

        for (price_key, (price, time)) in prices_guard.iter() {
            if price_key.starts_with(symbol) {
                if latest_time.is_none() || time > &latest_time.unwrap() {
                    latest_price = Some(*price);
                    latest_time = Some(*time);
                }
            }
        }

        latest_price
    }

    /// Get current price for a specific symbol and timeframe
    pub async fn get_current_price_for_timeframe(
        &self,
        symbol: &str,
        timeframe: &str,
    ) -> Option<f64> {
        let prices_guard = self.latest_prices.read().await;
        let price_key = format!("{}_{}", symbol, timeframe);

        prices_guard.get(&price_key).map(|(price, _time)| *price)
    }

    /// Get all current prices (for debugging)
    pub async fn get_all_current_prices(&self) -> HashMap<String, (f64, DateTime<Utc>)> {
        let prices_guard = self.latest_prices.read().await;
        prices_guard.clone()
    }

    /// Get current prices by symbol (aggregated across timeframes)
    pub async fn get_current_prices_by_symbol(&self) -> HashMap<String, f64> {
        let prices_guard = self.latest_prices.read().await;
        let mut symbol_prices = HashMap::new();

        for (price_key, (price, time)) in prices_guard.iter() {
            if let Some(symbol) = price_key.split('_').next() {
                // Keep the most recent price for each symbol
                if let Some((existing_time, _)) = symbol_prices.get(symbol) {
                    if time > existing_time {
                        symbol_prices.insert(symbol.to_string(), (*time, *price));
                    }
                } else {
                    symbol_prices.insert(symbol.to_string(), (*time, *price));
                }
            }
        }

        // Return just the prices, not the timestamps
        symbol_prices
            .into_iter()
            .map(|(symbol, (_time, price))| (symbol, price))
            .collect()
    }

    pub async fn update_price_with_cache_notifications(
        &self,
        symbol: &str,
        price: f64,
        timeframe: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Do your existing price update logic first
        self.update_price(symbol, price, timeframe).await?;

        // Get cache notifications
        let cache_notifications = {
            let mut cache_guard = self.zone_cache.lock().await;
            cache_guard.update_price_and_check_triggers(symbol, price)
        };

        // Get trade logger from global state
        use std::sync::LazyLock;
        static GLOBAL_TRADE_LOGGER: LazyLock<
            std::sync::Mutex<
                Option<Arc<tokio::sync::Mutex<crate::trade_event_logger::TradeEventLogger>>>,
            >,
        > = LazyLock::new(|| std::sync::Mutex::new(None));

        let trade_logger = GLOBAL_TRADE_LOGGER.lock().unwrap().clone();

        // Process each notification through the trade decision engine
        for notification in cache_notifications {
            info!(
                "🔔 [ZONE_MONITOR] Cache notification: {} {} {}/{} @ {:.5}",
                notification.action,
                notification.symbol,
                notification.timeframe,
                notification.zone_id,
                notification.price
            );

            // Log the notification received
            if let Some(logger_arc) = &trade_logger {
                let logger_guard = logger_arc.lock().await;
                logger_guard.log_notification_received(&notification).await;
                drop(logger_guard);
            }

            // Process through trade decision engine
            let validated_signal = {
                let mut engine_guard = self.trade_engine.lock().await;
                engine_guard
                    .process_notification(notification.clone())
                    .await
            };

            if let Some(signal) = validated_signal {
                // This is a VALIDATED trade signal!
                info!(
                    "🚨 [ZONE_MONITOR] VALIDATED TRADE SIGNAL: {} {} {}/{} @ {:.5}",
                    signal.action, signal.symbol, signal.timeframe, signal.zone_id, signal.price
                );

                // Log the validated signal
                if let Some(logger_arc) = &trade_logger {
                    let logger_guard = logger_arc.lock().await;
                    logger_guard.log_signal_validated(&signal).await;
                    drop(logger_guard);
                }

                // *** ADD THIS NEW SECTION FOR NOTIFICATIONS ***
                // Send notifications for validated trade signal
                if let Some(notification_manager) = get_global_notification_manager() {
                    notification_manager.notify_trade_signal(&signal).await;
                } else {
                    warn!("📢 Notification manager not available for signal notification");
                }

                // TODO: Here you would:
                // 1. Send to your cTrader API bridge for actual trading
                // 2. Send notification to dashboard
                // 3. Maybe send email/SMS alert

                // For now, just broadcast as a special event
                let trade_event = NewZoneEvent {
                    zone_id: signal.zone_id.clone(),
                    symbol: signal.symbol.clone(),
                    timeframe: signal.timeframe.clone(),
                    event_type: NewZoneEventType::Touch,
                    price: signal.price,
                    timestamp: signal.validation_timestamp,
                    zone_type: if signal.action == "SELL" {
                        "supply".to_string()
                    } else {
                        "demand".to_string()
                    },
                    confirmed: true,
                };

                if let Err(e) = self.event_sender.send(trade_event) {
                    warn!(
                        "⚠️ [ZONE_MONITOR] Failed to send validated signal event: {}",
                        e
                    );
                }
            } else {
                // Signal was rejected
                debug!(
                    "❌ [ZONE_MONITOR] Signal rejected for {}/{}",
                    notification.symbol, notification.zone_id
                );

                // Log the rejection
                if let Some(logger_arc) = &trade_logger {
                    let logger_guard = logger_arc.lock().await;
                    logger_guard
                        .log_signal_rejected(
                            &notification,
                            "Failed validation criteria".to_string(),
                        )
                        .await;
                    drop(logger_guard);
                }
            }

            // Always broadcast the original cache notification too
            let zone_event = NewZoneEvent {
                zone_id: notification.zone_id.clone(),
                symbol: notification.symbol.clone(),
                timeframe: notification.timeframe.clone(),
                event_type: NewZoneEventType::Touch,
                price: notification.price,
                timestamp: notification.timestamp,
                zone_type: if notification.action == "SELL" {
                    "supply".to_string()
                } else {
                    "demand".to_string()
                },
                confirmed: true,
            };

            if let Err(e) = self.event_sender.send(zone_event) {
                debug!(
                    "⚠️ [ZONE_MONITOR] Failed to send cache notification event: {}",
                    e
                );
            }
        }

        Ok(())
    }

    // Helper method to get notifications from cache
    pub async fn get_trade_notifications_from_cache(&self) -> Vec<TradeNotification> {
        let cache_guard = self.zone_cache.lock().await;
        cache_guard.get_trade_notifications().clone()
    }

    // Method to clear notifications
    pub async fn clear_cache_notifications(&self) {
        let mut cache_guard = self.zone_cache.lock().await;
        cache_guard.clear_trade_notifications();
    }

    pub async fn get_validated_signals(&self) -> Vec<ValidatedTradeSignal> {
        let engine_guard = self.trade_engine.lock().await;
        engine_guard.get_validated_signals().clone()
    }

    pub async fn get_daily_signal_count(&self) -> u32 {
        let engine_guard = self.trade_engine.lock().await;
        engine_guard.get_daily_signal_count()
    }

    pub async fn reload_trading_rules(&self) {
        let mut engine_guard = self.trade_engine.lock().await;
        engine_guard.reload_rules();
    }

    pub async fn clear_validated_signals(&self) {
        let mut engine_guard = self.trade_engine.lock().await;
        engine_guard.clear_signals();
    }
}
