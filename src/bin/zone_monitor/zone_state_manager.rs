// src/bin/zone_monitor/zone_state_manager.rs
// Zone state management for proximity and trading flow

use crate::types::{PriceUpdate, Zone, ZoneAlert};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, Mutex};
use tracing::{info, debug};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum ZoneState {
    NeverAlerted,      // Initial state - never been in proximity
    ProximityAlerted,  // Price has entered proximity range (sent notification)
    Traded,            // Trade has been executed on this zone
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ZoneStateInfo {
    pub state: ZoneState,
    pub last_proximity_alert: Option<chrono::DateTime<chrono::Utc>>,
    pub trade_executed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub symbol: String,
}

#[derive(Debug)]
pub struct ZoneStateManager {
    zone_states: Arc<RwLock<HashMap<String, ZoneStateInfo>>>,
    zone_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>, // Per-zone locks
    proximity_threshold_pips: f64,
    trading_threshold_pips: f64,
}

impl Clone for ZoneStateManager {
    fn clone(&self) -> Self {
        Self {
            zone_states: Arc::clone(&self.zone_states),
            zone_locks: Arc::clone(&self.zone_locks),
            proximity_threshold_pips: self.proximity_threshold_pips,
            trading_threshold_pips: self.trading_threshold_pips,
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

        info!("🎯 Zone State Manager initialized:");
        info!("   Proximity threshold: {:.1} pips", proximity_threshold_pips);
        info!("   Trading threshold: {:.1} pips", trading_threshold_pips);

        Self {
            zone_states: Arc::new(RwLock::new(HashMap::new())),
            zone_locks: Arc::new(Mutex::new(HashMap::new())),
            proximity_threshold_pips,
            trading_threshold_pips,
        }
    }

    // Extract timeframe from zone ID or other fields
    fn extract_timeframe(&self, zone: &Zone) -> String {
        // Common timeframe patterns to look for in zone ID
        let timeframes = ["1m", "5m", "15m", "30m", "1h", "4h", "1d", "1w"];
        
        for tf in timeframes {
            if zone.id.contains(tf) {
                return tf.to_string();
            }
        }
        
        // If no timeframe found in ID, check if there's a pattern we can extract
        "unknown".to_string()
    }

    // Check if zone should be excluded based on timeframe or other criteria
    fn should_exclude_zone(&self, zone: &Zone) -> bool {
        // Get excluded timeframes from environment
        let excluded_timeframes = std::env::var("EXCLUDED_PROXIMITY_TIMEFRAMES")
            .unwrap_or_else(|_| "5m,15m".to_string());
        
        let excluded_list: Vec<&str> = excluded_timeframes
            .split(',')
            .map(|s| s.trim())
            .collect();

        // Check if zone ID contains any excluded timeframe patterns
        for excluded_tf in excluded_list {
            if zone.id.contains(excluded_tf) {
                let timeframe = self.extract_timeframe(zone);
                info!("🚫 Excluding zone {} (timeframe: {}, excluded: {})", zone.id, timeframe, excluded_tf);
                return true;
            }
        }

        false
    }

    // Get or create a lock for a specific zone
    async fn get_zone_lock(&self, zone_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.zone_locks.lock().await;
        locks.entry(zone_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    // Main function to process price updates and determine what actions to take
    pub async fn process_price_update(
        &self,
        price_update: &PriceUpdate,
        zones: &[Zone],
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

                let zone_state = self.get_zone_state(&zone.id).await;
                
                debug!("🔍 Zone {} current state: {:?}, distance: {:.1} pips", 
                       zone.id, zone_state.state, distance_pips);
                
                match zone_state.state {
                    ZoneState::NeverAlerted => {
                        // First time in proximity - send proximity alert
                        self.update_zone_state(&zone.id, ZoneState::ProximityAlerted, &zone.symbol).await;
                        
                        let alert = self.create_zone_alert(price_update, zone, distance_pips);
                        result.proximity_alerts.push(alert);
                        
                        let timeframe = self.extract_timeframe(zone);
                        info!("🎯 NEW proximity: {} {} zone @ {:.1} pips [{}] (Zone: {})", 
                              zone.symbol, zone.zone_type, distance_pips, timeframe, zone.id);
                    }
                    ZoneState::ProximityAlerted => {
                        // Already alerted for proximity, check if we should trade
                        if distance_pips <= self.trading_threshold_pips {
                            // Within trading threshold - trigger trade
                            self.update_zone_state(&zone.id, ZoneState::Traded, &zone.symbol).await;
                            
                            let alert = self.create_zone_alert(price_update, zone, distance_pips);
                            result.trading_signals.push(alert);
                            
                            let timeframe = self.extract_timeframe(zone);
                            info!("💰 TRADING signal: {} {} zone @ {:.1} pips [{}] (Zone: {})", 
                                  zone.symbol, zone.zone_type, distance_pips, timeframe, zone.id);
                        } else {
                            // Still in proximity but not trading threshold - do nothing
                            debug!("🔄 Zone {} still in proximity ({:.1} pips) but not trading threshold ({:.1} pips)", 
                                   zone.id, distance_pips, self.trading_threshold_pips);
                        }
                    }
                    ZoneState::Traded => {
                        // Already traded this zone, ignore completely
                        debug!("🚫 Zone {} already traded, ignoring", zone.id);
                    }
                }
            } else {
                // Not in proximity - but let's check if we should reset state when price moves away
                let zone_state = self.get_zone_state(&zone.id).await;
                if matches!(zone_state.state, ZoneState::ProximityAlerted | ZoneState::Traded) {
                    debug!("📤 Zone {} no longer in proximity ({:.1} pips > {:.1} threshold)", 
                           zone.id, distance_pips, self.proximity_threshold_pips);
                }
            }
        }

        result
    }

    // Calculate distance from current price to zone
    fn calculate_distance_to_zone(&self, current_price: f64, zone: &Zone) -> f64 {
        let zone_mid = (zone.high + zone.low) / 2.0;
        ((current_price - zone_mid).abs() * 10000.0).round()
    }

    // Create a zone alert object
    fn create_zone_alert(&self, price_update: &PriceUpdate, zone: &Zone, distance_pips: f64) -> ZoneAlert {
        ZoneAlert {
            zone_id: zone.id.clone(),
            symbol: price_update.symbol.clone(),
            zone_type: zone.zone_type.clone(),
            current_price: (price_update.bid + price_update.ask) / 2.0,
            zone_high: zone.high,
            zone_low: zone.low,
            distance_pips,
            strength: zone.strength,
            timestamp: chrono::Utc::now(),
        }
    }

    // Get current state of a zone
    async fn get_zone_state(&self, zone_id: &str) -> ZoneStateInfo {
        let states = self.zone_states.read().await;
        states.get(zone_id).cloned().unwrap_or_else(|| ZoneStateInfo {
            state: ZoneState::NeverAlerted,
            last_proximity_alert: None,
            trade_executed_at: None,
            symbol: "UNKNOWN".to_string(),
        })
    }

    // Update zone state with better concurrency handling
    async fn update_zone_state(&self, zone_id: &str, new_state: ZoneState, symbol: &str) {
        let mut states = self.zone_states.write().await;
        let now = chrono::Utc::now();
        
        let state_info = states.entry(zone_id.to_string()).or_insert_with(|| ZoneStateInfo {
            state: ZoneState::NeverAlerted,
            last_proximity_alert: None,
            trade_executed_at: None,
            symbol: symbol.to_string(),
        });

        let old_state = state_info.state.clone();

        // Only update if it's a valid state transition
        match (&old_state, &new_state) {
            (ZoneState::NeverAlerted, ZoneState::ProximityAlerted) => {
                state_info.state = new_state;
                state_info.last_proximity_alert = Some(now);
                info!("📍 Zone {} state: {:?} → ProximityAlerted (states in memory: {})", 
                      zone_id, old_state, states.len());
            }
            (ZoneState::ProximityAlerted, ZoneState::Traded) => {
                state_info.state = new_state;
                state_info.trade_executed_at = Some(now);
                info!("📍 Zone {} state: {:?} → Traded (states in memory: {})", 
                      zone_id, old_state, states.len());
            }
            // Ignore invalid transitions
            _ => {
                debug!("🚫 Invalid state transition for zone {}: {:?} → {:?}", 
                       zone_id, old_state, new_state);
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
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ProcessResult {
    pub proximity_alerts: Vec<ZoneAlert>,  // New proximity alerts to send notifications
    pub trading_signals: Vec<ZoneAlert>,   // Trading signals to execute trades
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ZoneStats {
    pub total_zones: usize,
    pub never_alerted: usize,
    pub proximity_alerted: usize,
    pub traded: usize,
}