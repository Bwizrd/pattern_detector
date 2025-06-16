// src/bin/zone_monitor/zone_state_manager.rs
// Zone state management for proximity and trading flow

use crate::types::{PriceUpdate, Zone, ZoneAlert};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

/// Helper function to get pip value for a symbol
fn get_pip_value(symbol: &str) -> f64 {
    match symbol {
        // JPY pairs use 0.01 as pip value
        s if s.ends_with("JPY") => 0.01,
        // Indices
        "NAS100" => 1.0,
        "US500" => 0.1,
        // All other pairs use 0.0001
        _ => 0.0001,
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum ZoneState {
    NeverAlerted,      // Initial state - never been in proximity
    ProximityAlerted,  // Price has entered proximity range (sent notification)
    TradingSignalSent, // Trading signal has been sent (but trade not executed yet)
    Traded,            // Trade has been executed on this zone
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ZoneStateInfo {
    pub state: ZoneState,
    pub last_proximity_alert: Option<chrono::DateTime<chrono::Utc>>,
    pub trade_executed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_out_of_proximity: Option<chrono::DateTime<chrono::Utc>>,
    pub trading_signal_sent_at: Option<chrono::DateTime<chrono::Utc>>,
    pub symbol: String,
}

#[derive(Debug)]
pub struct ZoneStateManager {
    zone_states: Arc<RwLock<HashMap<String, ZoneStateInfo>>>,
    zone_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>, // Per-zone locks
    proximity_threshold_pips: f64,
    trading_threshold_pips: f64,
    proximity_reset_minutes: i64,
}

impl Clone for ZoneStateManager {
    fn clone(&self) -> Self {
        Self {
            zone_states: Arc::clone(&self.zone_states),
            zone_locks: Arc::clone(&self.zone_locks),
            proximity_threshold_pips: self.proximity_threshold_pips,
            trading_threshold_pips: self.trading_threshold_pips,
            proximity_reset_minutes: self.proximity_reset_minutes,
        }
    }
}

impl ZoneStateManager {
    pub fn new() -> Self {
        let proximity_threshold_pips = std::env::var("PROXIMITY_THRESHOLD_PIPS")
            .unwrap_or_else(|_| "10.0".to_string())
            .parse()
            .unwrap_or(10.0);

        let trading_threshold_pips = std::env::var("TRADING_THRESHOLD_PIPS")
            .unwrap_or_else(|_| "5.0".to_string())
            .parse()
            .unwrap_or(5.0);

        let proximity_reset_minutes = std::env::var("PROXIMITY_RESET_MINUTES")
            .unwrap_or_else(|_| "60".to_string())
            .parse()
            .unwrap_or(60);

        info!("🎯 Zone State Manager initialized:");
        info!(
            "   Proximity threshold: {:.1} pips",
            proximity_threshold_pips
        );
        info!("   Trading threshold: {:.1} pips", trading_threshold_pips);
        info!("   Proximity reset: {} minutes", proximity_reset_minutes);

        Self {
            zone_states: Arc::new(RwLock::new(HashMap::new())),
            zone_locks: Arc::new(Mutex::new(HashMap::new())),
            proximity_threshold_pips,
            trading_threshold_pips,
            proximity_reset_minutes,
        }
    }

    // Extract timeframe from zone data directly
    fn get_timeframe(&self, zone: &Zone) -> String {
        zone.timeframe.clone()
    }

    // Check if zone should be excluded based on timeframe or other criteria
    fn should_exclude_zone(&self, zone: &Zone) -> bool {
        // Get excluded timeframes from environment
        let excluded_timeframes =
            std::env::var("EXCLUDED_PROXIMITY_TIMEFRAMES").unwrap_or_else(|_| "5m,15m".to_string());

        let excluded_list: Vec<&str> = excluded_timeframes.split(',').map(|s| s.trim()).collect();

        // Check if zone timeframe is in the excluded list
        if excluded_list.contains(&zone.timeframe.as_str()) {
            debug!(
                "🚫 Excluding zone {} (timeframe: {}, excluded)",
                zone.id, zone.timeframe
            );
            return true;
        }

        // Check max touch count limit
        let max_touch_count = std::env::var("MAX_TOUCH_COUNT_FOR_TRADING")
            .unwrap_or_else(|_| "3".to_string())
            .parse::<i32>()
            .unwrap_or(3);

        if zone.touch_count > max_touch_count {
            debug!(
                "🚫 Excluding zone {} (touch_count: {} > max: {})",
                zone.id, zone.touch_count, max_touch_count
            );
            return true;
        }

        // Check minimum zone strength
        let min_zone_strength = std::env::var("TRADING_MIN_ZONE_STRENGTH")
            .unwrap_or_else(|_| "70.0".to_string())
            .parse::<f64>()
            .unwrap_or(70.0);

        if zone.strength < min_zone_strength {
            debug!(
                "🚫 Excluding zone {} (strength: {:.1} < min: {:.1})", 
                zone.id, zone.strength, min_zone_strength
            );
            return true;
        }

        false
    }

    // Get or create a lock for a specific zone
    async fn get_zone_lock(&self, zone_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.zone_locks.lock().await;
        locks
            .entry(zone_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    // Main function to process price updates and determine what actions to take
    pub async fn process_price_update(
        &self,
        price_update: &PriceUpdate,
        zones: &[Zone],
        zone_interactions: Option<&pattern_detector::zone_interactions::ZoneInteractionContainer>,
    ) -> ProcessResult {
        let mut result = ProcessResult::default();
        let current_price = (price_update.bid + price_update.ask) / 2.0;

        for zone in zones {
            // Skip excluded zones (e.g., certain timeframes)
            if self.should_exclude_zone(zone) {
                continue;
            }

            let distance_pips = self.calculate_distance_to_zone(current_price, zone);

            // Check if we're within proximity threshold
            if distance_pips <= self.proximity_threshold_pips {
                // Get zone-specific lock to prevent race conditions
                let zone_lock = self.get_zone_lock(&zone.id).await;
                let _guard = zone_lock.lock().await;

                let mut zone_state = self.get_zone_state(&zone.id).await;

                // Check if we should reset zones due to time away
                if matches!(
                    zone_state.state,
                    ZoneState::ProximityAlerted | ZoneState::TradingSignalSent
                ) {
                    if let Some(last_out) = zone_state.last_out_of_proximity {
                        let now = chrono::Utc::now();
                        let reset_duration =
                            chrono::Duration::minutes(self.proximity_reset_minutes);

                        if now - last_out >= reset_duration {
                            info!(
                                "🔄 Resetting zone {} after {} minutes away from proximity",
                                zone.id, self.proximity_reset_minutes
                            );
                            zone_state.state = ZoneState::NeverAlerted;
                            zone_state.last_out_of_proximity = None;
                            zone_state.trading_signal_sent_at = None;
                            self.update_zone_state_direct(&zone.id, zone_state.clone())
                                .await;
                        }
                    }
                }

                debug!(
                    "🔍 Zone {} current state: {:?}, distance: {:.1} pips",
                    zone.id, zone_state.state, distance_pips
                );

                // Get zone interaction data for first-time entry detection
                let (has_ever_entered, zone_entries) = if let Some(interactions) = zone_interactions {
                    if let Some(metrics) = interactions.metrics.get(&zone.id) {
                        (metrics.has_ever_entered, metrics.zone_entries)
                    } else {
                        (false, 0)
                    }
                } else {
                    (false, 0)
                };

                let is_first_time_entry = !has_ever_entered && zone_entries == 0;

                match zone_state.state {
                    ZoneState::NeverAlerted => {
                        // First time in proximity - send proximity alert
                        self.update_zone_state(&zone.id, ZoneState::ProximityAlerted, &zone.symbol)
                            .await;

                        let alert = self.create_zone_alert(price_update, zone, distance_pips);
                        result.proximity_alerts.push(alert);

                        let timeframe = self.get_timeframe(zone);
                        info!(
                            "🎯 NEW proximity: {} {} zone @ {:.1} pips [{}] (Zone: {}, {} touches)",
                            zone.symbol,
                            zone.zone_type,
                            distance_pips,
                            timeframe,
                            zone.id,
                            zone.touch_count
                        );
                    }
                    ZoneState::ProximityAlerted => {
                        // Check for TRIGGER condition: within trading threshold AND first time entry
                        if distance_pips <= self.trading_threshold_pips && is_first_time_entry {
                            // TRIGGER: First time within trading threshold - send trading signal
                            self.update_zone_state(
                                &zone.id,
                                ZoneState::TradingSignalSent,
                                &zone.symbol,
                            )
                            .await;

                            let alert = self.create_zone_alert(price_update, zone, distance_pips);
                            result.trading_signals.push(alert);

                            let timeframe = self.get_timeframe(zone);
                            info!("💰 TRADING signal (FIRST ENTRY @ {:.1} pips threshold): {} {} zone @ {:.1} pips [{}] (Zone: {}, {} touches)", 
                                  self.trading_threshold_pips, zone.symbol, zone.zone_type, distance_pips, timeframe, zone.id, zone.touch_count);
                        } else if distance_pips <= self.trading_threshold_pips && !is_first_time_entry {
                            // Within threshold but not first time - no trading signal
                            debug!("🔄 Zone {} within threshold but not first entry (entries: {})", 
                                   zone.id, zone_entries);
                        } else {
                            // Still in proximity but not within trading threshold - do nothing
                            debug!("🔄 Zone {} still in proximity ({:.1} pips) but not within trading threshold ({:.1})", 
                                   zone.id, distance_pips, self.trading_threshold_pips);
                        }
                    }
                    ZoneState::TradingSignalSent => {
                        // Trading signal already sent, do nothing until trade executed or zone resets
                        debug!(
                            "⏳ Zone {} waiting for trade execution or reset (signal already sent)",
                            zone.id
                        );
                    }
                    ZoneState::Traded => {
                        // Already traded this zone, ignore completely (never reset traded zones)
                        debug!("🚫 Zone {} already traded, ignoring", zone.id);
                    }
                }
            } else {
                // Not in proximity - mark zones as out of proximity for potential reset
                let zone_state = self.get_zone_state(&zone.id).await;
                if matches!(
                    zone_state.state,
                    ZoneState::ProximityAlerted | ZoneState::TradingSignalSent
                ) && zone_state.last_out_of_proximity.is_none()
                {
                    // Zone was in proximity but now isn't - start the reset timer
                    let zone_lock = self.get_zone_lock(&zone.id).await;
                    let _guard = zone_lock.lock().await;

                    let mut updated_state = zone_state.clone();
                    updated_state.last_out_of_proximity = Some(chrono::Utc::now());
                    self.update_zone_state_direct(&zone.id, updated_state).await;

                    debug!("📤 Zone {} left proximity ({:.1} pips > {:.1} threshold) - reset timer started", 
                           zone.id, distance_pips, self.proximity_threshold_pips);
                } else if matches!(
                    zone_state.state,
                    ZoneState::ProximityAlerted | ZoneState::TradingSignalSent | ZoneState::Traded
                ) {
                    debug!(
                        "📤 Zone {} no longer in proximity ({:.1} pips > {:.1} threshold)",
                        zone.id, distance_pips, self.proximity_threshold_pips
                    );
                }
            }
        }

        result
    }

    // Calculate distance from current price to zone proximal line (matches dashboard logic)
    fn calculate_distance_to_zone(&self, current_price: f64, zone: &Zone) -> f64 {
        // Determine zone type and proximal line (matching dashboard calculation)
        let is_supply = zone.zone_type.contains("supply") || zone.zone_type.contains("resistance");
        let proximal_line = if is_supply {
            zone.low  // Supply zone: proximal = low (entry from below)
        } else {
            zone.high // Demand zone: proximal = high (entry from above)  
        };
        
        // Calculate signed distance to proximal line
        let pip_value = get_pip_value(&zone.symbol);
        let signed_distance_pips = if is_supply {
            (current_price - proximal_line) / pip_value
        } else {
            (proximal_line - current_price) / pip_value
        };
        
        signed_distance_pips.abs()
    }

    // Create a zone alert object
    fn create_zone_alert(
        &self,
        price_update: &PriceUpdate,
        zone: &Zone,
        distance_pips: f64,
    ) -> ZoneAlert {
        ZoneAlert {
            zone_id: zone.id.clone(),
            symbol: price_update.symbol.clone(),
            zone_type: zone.zone_type.clone(),
            current_price: (price_update.bid + price_update.ask) / 2.0,
            zone_high: zone.high,
            zone_low: zone.low,
            distance_pips,
            strength: zone.strength,
            timeframe: zone.timeframe.clone(), // Add this line
            touch_count: zone.touch_count,     // Add this line
            timestamp: chrono::Utc::now(),
        }
    }

    // Get current state of a zone
    async fn get_zone_state(&self, zone_id: &str) -> ZoneStateInfo {
        let states = self.zone_states.read().await;
        states
            .get(zone_id)
            .cloned()
            .unwrap_or_else(|| ZoneStateInfo {
                state: ZoneState::NeverAlerted,
                last_proximity_alert: None,
                trade_executed_at: None,
                last_out_of_proximity: None,
                trading_signal_sent_at: None,
                symbol: "UNKNOWN".to_string(),
            })
    }

    // Update zone state directly (for internal use)
    async fn update_zone_state_direct(&self, zone_id: &str, state_info: ZoneStateInfo) {
        let mut states = self.zone_states.write().await;
        states.insert(zone_id.to_string(), state_info);
    }

    // Update zone state with better concurrency handling
    async fn update_zone_state(&self, zone_id: &str, new_state: ZoneState, symbol: &str) {
        let mut states = self.zone_states.write().await;
        let now = chrono::Utc::now();

        let state_info = states
            .entry(zone_id.to_string())
            .or_insert_with(|| ZoneStateInfo {
                state: ZoneState::NeverAlerted,
                last_proximity_alert: None,
                trade_executed_at: None,
                last_out_of_proximity: None,
                trading_signal_sent_at: None,
                symbol: symbol.to_string(),
            });

        let old_state = state_info.state.clone();

        // Only update if it's a valid state transition
        match (&old_state, &new_state) {
            (ZoneState::NeverAlerted, ZoneState::ProximityAlerted) => {
                state_info.state = new_state;
                state_info.last_proximity_alert = Some(now);
                state_info.last_out_of_proximity = None; // Clear reset timer
                info!(
                    "📍 Zone {} state: {:?} → ProximityAlerted (states in memory: {})",
                    zone_id,
                    old_state,
                    states.len()
                );
            }
            (ZoneState::ProximityAlerted, ZoneState::TradingSignalSent) => {
                state_info.state = new_state;
                state_info.trading_signal_sent_at = Some(now);
                state_info.last_out_of_proximity = None; // Clear reset timer since we're still active
                info!(
                    "📍 Zone {} state: {:?} → TradingSignalSent (states in memory: {})",
                    zone_id,
                    old_state,
                    states.len()
                );
            }
            (ZoneState::TradingSignalSent, ZoneState::Traded) => {
                state_info.state = new_state;
                state_info.trade_executed_at = Some(now);
                info!(
                    "📍 Zone {} state: {:?} → Traded (states in memory: {})",
                    zone_id,
                    old_state,
                    states.len()
                );
            }
            // Ignore invalid transitions
            _ => {
                debug!(
                    "🚫 Invalid state transition for zone {}: {:?} → {:?}",
                    zone_id, old_state, new_state
                );
            }
        }
    }

    // Public API methods for monitoring
    pub async fn get_zone_state_info(&self, zone_id: &str) -> ZoneStateInfo {
        self.get_zone_state(zone_id).await
    }

    pub async fn get_all_zone_states(&self) -> HashMap<String, ZoneStateInfo> {
        self.zone_states.read().await.clone()
    }

    pub async fn get_zone_stats(&self) -> ZoneStats {
        let states = self.zone_states.read().await;
        let mut stats = ZoneStats::default();

        for (_, state_info) in states.iter() {
            stats.total_zones += 1;
            match state_info.state {
                ZoneState::NeverAlerted => stats.never_alerted += 1,
                ZoneState::ProximityAlerted => stats.proximity_alerted += 1,
                ZoneState::TradingSignalSent => stats.trading_signal_sent += 1,
                ZoneState::Traded => stats.traded += 1,
            }
        }

        stats
    }

    // Reset a zone state (for testing or manual intervention)
    pub async fn reset_zone_state(&self, zone_id: &str) {
        let mut states = self.zone_states.write().await;
        if states.remove(zone_id).is_some() {
            info!("🔄 Reset zone {} state to NeverAlerted", zone_id);
        }
    }

    // Clean up old zone states (optional maintenance)
    pub async fn cleanup_old_states(&self, days_old: i64) {
        let mut states = self.zone_states.write().await;
        let cutoff_date = chrono::Utc::now() - chrono::Duration::days(days_old);
        let initial_count = states.len();

        states.retain(|zone_id, state_info| {
            let should_keep = match &state_info.last_proximity_alert {
                Some(last_alert) => *last_alert > cutoff_date,
                None => true, // Keep never-alerted zones
            };

            if !should_keep {
                debug!("🧹 Removing old state for zone {}", zone_id);
            }

            should_keep
        });

        let removed_count = initial_count - states.len();
        if removed_count > 0 {
            info!("🧹 Cleaned up {} old zone states", removed_count);
        }
    }

    pub fn get_thresholds(&self) -> (f64, f64) {
        (self.proximity_threshold_pips, self.trading_threshold_pips)
    }

    // Mark a zone as traded (called AFTER successful trade execution)
    pub async fn mark_zone_as_traded(&self, zone_id: &str, symbol: &str) {
        let zone_lock = self.get_zone_lock(zone_id).await;
        let _guard = zone_lock.lock().await;

        let mut states = self.zone_states.write().await;
        let now = chrono::Utc::now();

        let state_info = states
            .entry(zone_id.to_string())
            .or_insert_with(|| ZoneStateInfo {
                state: ZoneState::NeverAlerted,
                last_proximity_alert: None,
                trade_executed_at: None,
                last_out_of_proximity: None,
                trading_signal_sent_at: None,
                symbol: symbol.to_string(),
            });

        if state_info.state == ZoneState::TradingSignalSent {
            state_info.state = ZoneState::Traded;
            state_info.trade_executed_at = Some(now);
            state_info.last_out_of_proximity = None; // Clear any reset timer
            info!(
                "📍 Zone {} state: TradingSignalSent → Traded (FINAL - trade executed)",
                zone_id
            );
        } else {
            warn!(
                "⚠️ Attempted to mark zone {} as traded but state was {:?}",
                zone_id, state_info.state
            );
        }
    }

    // Revert a zone from TradingSignalSent back to ProximityAlerted (if trade was rejected)
    pub async fn revert_zone_to_proximity_alerted(&self, zone_id: &str) {
        let zone_lock = self.get_zone_lock(zone_id).await;
        let _guard = zone_lock.lock().await;

        let mut states = self.zone_states.write().await;
        if let Some(state_info) = states.get_mut(zone_id) {
            if state_info.state == ZoneState::TradingSignalSent {
                state_info.state = ZoneState::ProximityAlerted;
                state_info.trading_signal_sent_at = None;
                info!(
                    "🔄 Reverted zone {} from TradingSignalSent → ProximityAlerted",
                    zone_id
                );
            }
        }
    }

    // Check and reset zones that have been out of proximity for too long
    pub async fn check_and_reset_expired_zones(&self) {
        let mut states = self.zone_states.write().await;
        let now = chrono::Utc::now();
        let reset_duration = chrono::Duration::minutes(self.proximity_reset_minutes);
        let mut reset_count = 0;

        // Find zones that should be reset
        let zones_to_reset: Vec<String> = states
            .iter()
            .filter_map(|(zone_id, state_info)| {
                if matches!(
                    state_info.state,
                    ZoneState::ProximityAlerted | ZoneState::TradingSignalSent
                ) {
                    if let Some(last_out) = state_info.last_out_of_proximity {
                        if now - last_out >= reset_duration {
                            return Some(zone_id.clone());
                        }
                    }
                }
                None
            })
            .collect();

        // Reset the zones
        for zone_id in zones_to_reset {
            if let Some(state_info) = states.get_mut(&zone_id) {
                state_info.state = ZoneState::NeverAlerted;
                state_info.last_out_of_proximity = None;
                state_info.trading_signal_sent_at = None;
                reset_count += 1;
                info!(
                    "🔄 Auto-reset zone {} after {} minutes away from proximity",
                    zone_id, self.proximity_reset_minutes
                );
            }
        }

        if reset_count > 0 {
            info!(
                "🔄 Auto-reset {} zones after proximity timeout",
                reset_count
            );
        }
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ProcessResult {
    pub proximity_alerts: Vec<ZoneAlert>, // New proximity alerts to send notifications
    pub trading_signals: Vec<ZoneAlert>,  // Trading signals to execute trades
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ZoneStats {
    pub total_zones: usize,
    pub never_alerted: usize,
    pub proximity_alerted: usize,
    pub trading_signal_sent: usize,
    pub traded: usize,
}
