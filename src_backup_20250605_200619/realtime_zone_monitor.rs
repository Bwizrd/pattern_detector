// src/realtime_zone_monitor.rs - Fixed implementation
use chrono::{DateTime, Utc};
use log::{debug, info, warn, error};
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

    proximity_alert_cache: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
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
            proximity_alert_cache: Arc::new(RwLock::new(HashMap::new())),
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
    self.check_proximity_for_price_update(symbol, price, timeframe)
        .await;

    // Get cache notifications
    let cache_notifications = {
        let mut cache_guard = self.zone_cache.lock().await;
        cache_guard.update_price_and_check_triggers(symbol, price)
    };

    // Skip if no notifications
    if cache_notifications.is_empty() {
        return Ok(());
    }

    // Block repeated notifications for same zone until unblocked
    use std::sync::LazyLock;
    static BLOCKED_ZONES: LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

    let mut unique_notifications = Vec::new();
    {
        let mut blocked_set = BLOCKED_ZONES.lock().unwrap();
        
        for notification in cache_notifications {
            let zone_key = format!("{}_{}", notification.zone_id, notification.action);
            
            // If this zone+action is already blocked, skip it
            if blocked_set.contains(&zone_key) {
                continue;
            }
            
            unique_notifications.push(notification);
        }
    }

    // *** ENHANCED DEBUGGING: Log notification count ***
    debug!(
        "📊 [DEBUG_NOTIFICATIONS] Received {} cache notifications for {} @ {:.5} ({} unique)",
        unique_notifications.len() + {
            let blocked_set = BLOCKED_ZONES.lock().unwrap();
            blocked_set.len()
        },
        symbol,
        price,
        unique_notifications.len()
    );

    // Get trade logger from global state
    // use std::sync::LazyLock;
    static GLOBAL_TRADE_LOGGER: LazyLock<
        std::sync::Mutex<
            Option<Arc<tokio::sync::Mutex<crate::trade_event_logger::TradeEventLogger>>>,
        >,
    > = LazyLock::new(|| std::sync::Mutex::new(None));

    let trade_logger = GLOBAL_TRADE_LOGGER.lock().unwrap().clone();

    // *** ENHANCED DEBUGGING: Log each notification in detail ***
    for (index, notification) in unique_notifications.iter().enumerate() {
        info!(
            "🔔 [DEBUG_NOTIFICATION_{}] Cache notification: {} {} {}/{} @ {:.5} (timestamp: {})",
            index + 1,
            notification.action,
            notification.symbol,
            notification.timeframe,
            notification.zone_id,
            notification.price,
            notification.timestamp.format("%H:%M:%S%.3f")
        );

        // Log the notification received
        if let Some(logger_arc) = &trade_logger {
            let logger_guard = logger_arc.lock().await;
            logger_guard.log_notification_received(&notification).await;
            drop(logger_guard);
        }

        // *** ENHANCED DEBUGGING: Pre-validation logging ***
        info!(
            "🔍 [DEBUG_PRE_VALIDATION] About to process notification through trade engine..."
        );

        // Process through trade decision engine with ENHANCED DEBUGGING
        let validated_signal = {
            let mut engine_guard = self.trade_engine.lock().await;
            
            // *** ENHANCED DEBUGGING: Log engine state before processing ***
            let daily_count = engine_guard.get_daily_signal_count();
            let max_daily = engine_guard.get_rules().max_daily_signals;
            info!(
                "📈 [DEBUG_ENGINE_STATE] Daily signal count before processing: {}/{}",
                daily_count, max_daily
            );
            
            // *** LOG TRADING RULES STATUS ***
            let rules = engine_guard.get_rules();
            info!(
                "⚙️ [DEBUG_TRADING_RULES] Enabled: {}, Symbols: {:?}, Timeframes: {:?}",
                rules.trading_enabled,
                rules.allowed_symbols,
                rules.allowed_timeframes
            );
            
            // *** DETAILED DEBUG VALIDATION FIRST ***
            info!(
                "🔍 [DEBUG_DETAILED_VALIDATION] Running detailed validation check..."
            );
            let debug_result = engine_guard.debug_validate_notification(&notification).await;
            info!("📋 [DEBUG_VALIDATION_DETAILS]\n{}", debug_result);
            
            // *** NOW PROCESS NORMALLY ***
            info!(
                "⚙️ [DEBUG_ENGINE_PROCESSING] Calling engine.process_notification() for {}/{} action: {}",
                notification.symbol,
                notification.zone_id,
                notification.action
            );
            
            let result = engine_guard
                .process_notification(notification.clone())
                .await;
                
            // *** ENHANCED DEBUGGING: Log immediate result ***
            match &result {
                Some(signal) => {
                    info!(
                        "✅ [DEBUG_ENGINE_RESULT] Validation SUCCESS - Signal produced: {} {} @ {:.5}",
                        signal.action,
                        signal.symbol,
                        signal.price
                    );
                }
                None => {
                    error!(
                        "❌ [DEBUG_ENGINE_RESULT] Validation FAILED - No signal produced for {}/{} action: {}",
                        notification.symbol,
                        notification.zone_id,
                        notification.action
                    );
                    
                    // *** ADD DETAILED FAILURE ANALYSIS ***
                    let current_daily_count = engine_guard.get_daily_signal_count();
                    let max_allowed = engine_guard.get_rules().max_daily_signals;
                    error!(
                        "📊 [DEBUG_FAILURE_ANALYSIS] Daily count after: {}/{}, Trading enabled: {}",
                        current_daily_count,
                        max_allowed,
                        engine_guard.get_rules().trading_enabled
                    );
                }
            }
            
            result
        };

        if let Some(signal) = validated_signal {
            // This is a VALIDATED trade signal!
            info!(
                "🚨 [VALIDATED_SIGNAL] CONFIRMED TRADE SIGNAL: {} {} {}/{} @ {:.5} (validation_time: {})",
                signal.action, 
                signal.symbol, 
                signal.timeframe, 
                signal.zone_id, 
                signal.price,
                signal.validation_timestamp.format("%H:%M:%S%.3f")
            );

            // Log the validated signal
            if let Some(logger_arc) = &trade_logger {
                let logger_guard = logger_arc.lock().await;
                logger_guard.log_signal_validated(&signal).await;
                drop(logger_guard);
            }

            // *** ENHANCED DEBUGGING: Pre-notification logging ***
            info!(
                "📢 [DEBUG_PRE_NOTIFICATION] About to send notification for validated signal..."
            );

            // Send notifications for validated trade signal
            if let Some(notification_manager) = get_global_notification_manager() {
                info!(
                    "📲 [DEBUG_NOTIFICATION_MANAGER] Notification manager found - sending signal notification..."
                );
                notification_manager.notify_trade_signal(&signal).await;
                info!(
                    "✅ [DEBUG_NOTIFICATION_SENT] Trade signal notification sent successfully!"
                );
            } else {
                error!(
                    "❌ [DEBUG_NOTIFICATION_ERROR] Notification manager NOT AVAILABLE for signal notification!"
                );
            }

            // Broadcast as a special event
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
                    "⚠️ [DEBUG_EVENT_SEND_ERROR] Failed to send validated signal event: {}",
                    e
                );
            } else {
                info!(
                    "✅ [DEBUG_EVENT_SENT] Validated signal event broadcasted successfully"
                );
            }
        } else {
            // Signal was rejected - Enhanced debugging with detailed reason
            let rejection_reason = {
                let engine_guard = self.trade_engine.lock().await;
                let debug_result = engine_guard.debug_validate_notification(&notification).await;
                
                // Extract the specific rejection reason from debug output
                if let Some(rejection_line) = debug_result.lines().find(|line| line.contains("❌ REJECTION:")) {
                    rejection_line.replace("❌ REJECTION:", "").trim().to_string()
                } else {
                    "Unknown validation failure".to_string()
                }
            };

            error!(
                "❌ [DEBUG_SIGNAL_REJECTED] Signal rejected for {}/{} action: {} @ {:.5} - Reason: {}",
                notification.symbol, 
                notification.zone_id,
                notification.action,
                notification.price,
                rejection_reason
            );

            // *** SEND TELEGRAM NOTIFICATION FOR BLOCKED TRADE ***
            if let Some(notification_manager) = get_global_notification_manager() {
                info!(
                    "📢 [DEBUG_BLOCKED_TRADE_NOTIFICATION] Sending blocked trade alert to Telegram..."
                );
                notification_manager.notify_blocked_trade_signal(
                    &notification,
                    &rejection_reason
                ).await;
                
                // Mark this zone+action as blocked to prevent repeated notifications
                {
                    let mut blocked_set = BLOCKED_ZONES.lock().unwrap();
                    let zone_key = format!("{}_{}", notification.zone_id, notification.action);
                    blocked_set.insert(zone_key);
                }
                
                info!(
                    "✅ [DEBUG_BLOCKED_TRADE_SENT] Blocked trade notification sent to Telegram!"
                );
            } else {
                warn!(
                    "⚠️ [DEBUG_BLOCKED_TRADE_ERROR] Notification manager not available for blocked trade alert"
                );
            }

            // Log the rejection
            if let Some(logger_arc) = &trade_logger {
                let logger_guard = logger_arc.lock().await;
                logger_guard
                    .log_signal_rejected(
                        &notification,
                        rejection_reason.clone(),
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
                "⚠️ [DEBUG_ZONE_EVENT_ERROR] Failed to send cache notification event: {}",
                e
            );
        } else {
            debug!(
                "✅ [DEBUG_ZONE_EVENT_SENT] Cache notification event broadcasted"
            );
        }
    }

    debug!(
        "🏁 [DEBUG_PROCESSING_COMPLETE] Finished processing {} notifications for {} @ {:.5}",
        unique_notifications.len(),
        symbol,
        price
    );

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

    pub async fn get_proximity_distances_debug(&self) -> Vec<ProximityDebugInfo> {
        let zones_guard = self.active_zones.read().await;
        let prices_guard = self.latest_prices.read().await;
        let mut distances = Vec::new();

        // Get proximity threshold from environment
        let proximity_threshold_pips = std::env::var("PROXIMITY_THRESHOLD_PIPS")
            .unwrap_or_else(|_| "15.0".to_string())
            .parse::<f64>()
            .unwrap_or(15.0);

        for (zone_id, live_zone_status) in zones_guard.iter() {
            let zone = &live_zone_status.zone;

            if let (Some(symbol), Some(timeframe), Some(zone_high), Some(zone_low)) =
                (&zone.symbol, &zone.timeframe, zone.zone_high, zone.zone_low)
            {
                // Try different price key formats to find current price
                let price_candidates = vec![
                    format!("{}_{}", symbol, timeframe),
                    symbol.clone(),
                    format!("{}_SB_{}", symbol.replace("_SB", ""), timeframe),
                ];

                let mut current_price = None;
                for price_key in price_candidates {
                    if let Some((price, _time)) = prices_guard.get(&price_key) {
                        current_price = Some(*price);
                        break;
                    }
                }

                let zone_type = zone.zone_type.as_deref().unwrap_or("");
                let is_supply = zone_type.to_lowercase().contains("supply");

                // Calculate proximal line (entry point)
                let proximal_line = if is_supply {
                    zone_low // For supply zones, we enter when price reaches the low
                } else {
                    zone_high // For demand zones, we enter when price reaches the high
                };

                let (distance_pips, within_threshold) = if let Some(price) = current_price {
                    let distance = calculate_pips_distance_simple(symbol, price, proximal_line);
                    let within = distance <= proximity_threshold_pips;
                    (Some(distance), within)
                } else {
                    (None, false)
                };

                distances.push(ProximityDebugInfo {
                    symbol: symbol.clone(),
                    timeframe: timeframe.clone(),
                    zone_type: zone_type.to_string(),
                    current_price,
                    proximal_line,
                    distance_pips,
                    zone_high,
                    zone_low,
                    is_active: zone.is_active,
                    zone_id: zone_id.clone(),
                    within_threshold,
                    proximity_threshold_pips,
                });
            }
        }

        // Sort by distance (closest first)
        distances.sort_by(|a, b| match (a.distance_pips, b.distance_pips) {
            (Some(a_dist), Some(b_dist)) => a_dist
                .partial_cmp(&b_dist)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        distances
    }

    /// Get zones that are currently within proximity threshold
    pub async fn get_zones_within_proximity(&self) -> Vec<ProximityDebugInfo> {
        let all_distances = self.get_proximity_distances_debug().await;
        all_distances
            .into_iter()
            .filter(|info| info.within_threshold && info.is_active)
            .collect()
    }

    async fn check_proximity_for_price_update(&self, symbol: &str, price: f64, timeframe: &str) {
        let proximity_threshold_pips = std::env::var("PROXIMITY_THRESHOLD_PIPS")
            .unwrap_or_else(|_| "15.0".to_string())
            .parse::<f64>()
            .unwrap_or(15.0);

        let cooldown_minutes = std::env::var("PROXIMITY_ALERT_COOLDOWN_MINUTES")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<i64>()
            .unwrap_or(30);

        // Get allowed timeframes for proximity alerts
        let allowed_timeframes = std::env::var("PROXIMITY_ALLOWED_TIMEFRAMES")
            .unwrap_or_else(|_| "30m,1h,4h,1d".to_string())
            .split(',')
            .map(|tf| tf.trim().to_string())
            .collect::<Vec<String>>();

        // Skip if this timeframe is not allowed
        if !allowed_timeframes.contains(&timeframe.to_string()) {
            debug!("🔍 [PROXIMITY_FILTER] Skipping proximity check for {} (timeframe {} not in allowed list: {:?})", 
           symbol, timeframe, allowed_timeframes);
            return;
        }

        debug!(
            "🔍 [PROXIMITY_DEBUG] Checking proximity for: {} @ {:.5} (timeframe: {} - ALLOWED)",
            symbol, price, timeframe
        );

        let zones_guard = self.active_zones.read().await;

        // Use static cooldown cache for simplicity
        use std::sync::LazyLock;
        static PROXIMITY_COOLDOWN: LazyLock<std::sync::Mutex<HashMap<String, DateTime<Utc>>>> =
            LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

        for (zone_id, live_zone) in zones_guard.iter() {
            let zone = &live_zone.zone;

            if let Some(zone_symbol) = &zone.symbol {
                if zone_symbol == symbol && zone.is_active {
                    // Only check zones that match the current timeframe
                    if zone.timeframe.as_deref().unwrap_or("") != timeframe {
                        continue;
                    }

                    if let (Some(zone_high), Some(zone_low), Some(zone_type)) =
                        (zone.zone_high, zone.zone_low, &zone.zone_type)
                    {
                        let is_supply = zone_type.to_lowercase().contains("supply");
                        let proximal_line = if is_supply { zone_low } else { zone_high };
                        let distal_line = if is_supply { zone_high } else { zone_low };

                        // Check if price is INSIDE the zone
                        let is_inside_zone = if is_supply {
                            // For supply: inside if price is between zone_low (proximal) and zone_high (distal)
                            price >= zone_low && price <= zone_high
                        } else {
                            // For demand: inside if price is between zone_high (proximal) and zone_low (distal)
                            price <= zone_high && price >= zone_low
                        };

                        if is_inside_zone {
                            // Price is INSIDE the zone - different handling
                            let should_alert = {
                                let now = Utc::now();
                                let mut cooldown = PROXIMITY_COOLDOWN.lock().unwrap();
                                let cooldown_key = format!("{}_INSIDE", zone_id); // Different key for inside alerts

                                if let Some(last_alert) = cooldown.get(&cooldown_key) {
                                    let elapsed = (now - *last_alert).num_minutes();
                                    if elapsed >= cooldown_minutes {
                                        cooldown.insert(cooldown_key, now);
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    cooldown.insert(cooldown_key, now);
                                    true
                                }
                            };

                            if should_alert {
                                let distance_from_proximal =
                                    calculate_pips_distance_simple(symbol, price, proximal_line);
                                let distance_from_distal =
                                    calculate_pips_distance_simple(symbol, price, distal_line);

                                info!("🎯 [INSIDE_ZONE] PRICE INSIDE {} {} zone {} - {:.1} pips from proximal, {:.1} pips from distal ({})", 
                              symbol, zone_type, zone_id, distance_from_proximal, distance_from_distal, timeframe);

                                if let Some(notification_manager) =
                                    get_global_notification_manager()
                                {
                                    notification_manager
                                        .notify_inside_zone_alert(
                                            symbol,
                                            price,
                                            zone_type,
                                            zone_high,
                                            zone_low,
                                            zone_id,
                                            timeframe,
                                            distance_from_proximal,
                                            distance_from_distal,
                                        )
                                        .await;
                                }
                            } else {
                                debug!(
                                    "🔍 [COOLDOWN] Inside zone alert for {} in cooldown, skipping",
                                    zone_id
                                );
                            }
                        } else {
                            // Price is OUTSIDE the zone - check for proximity to proximal line
                            let distance_pips =
                                calculate_pips_distance_simple(symbol, price, proximal_line);

                            // Only alert if approaching (not inside) and within threshold
                            if distance_pips <= proximity_threshold_pips {
                                let should_alert = {
                                    let now = Utc::now();
                                    let mut cooldown = PROXIMITY_COOLDOWN.lock().unwrap();
                                    let cooldown_key = format!("{}_APPROACHING", zone_id); // Different key for approaching alerts

                                    if let Some(last_alert) = cooldown.get(&cooldown_key) {
                                        let elapsed = (now - *last_alert).num_minutes();
                                        if elapsed >= cooldown_minutes {
                                            cooldown.insert(cooldown_key, now);
                                            true
                                        } else {
                                            false
                                        }
                                    } else {
                                        cooldown.insert(cooldown_key, now);
                                        true
                                    }
                                };

                                if should_alert {
                                    info!("🎯 [PROXIMITY_ALERT] APPROACHING {} {} zone {} - {:.1} pips away ({})", 
                                  symbol, zone_type, zone_id, distance_pips, timeframe);

                                    if let Some(notification_manager) =
                                        get_global_notification_manager()
                                    {
                                        notification_manager
                                            .notify_proximity_alert(
                                                symbol,
                                                price,
                                                zone_type,
                                                zone_high,
                                                zone_low,
                                                zone_id,
                                                timeframe,
                                                distance_pips,
                                            )
                                            .await;
                                    }
                                } else {
                                    debug!("🔍 [COOLDOWN] Approaching zone alert for {} in cooldown, skipping", zone_id);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
#[derive(Debug, serde::Serialize)]
pub struct ProximityDebugInfo {
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub current_price: Option<f64>,
    pub proximal_line: f64,
    pub distance_pips: Option<f64>,
    pub zone_high: f64,
    pub zone_low: f64,
    pub is_active: bool,
    pub zone_id: String,
    pub within_threshold: bool,
    pub proximity_threshold_pips: f64,
}

// Helper function for pip calculation (add this at the end of the file)
fn calculate_pips_distance_simple(symbol: &str, price1: f64, price2: f64) -> f64 {
    let pip_value = if symbol.contains("JPY") { 0.01 } else { 0.0001 };
    (price1 - price2).abs() / pip_value
}
