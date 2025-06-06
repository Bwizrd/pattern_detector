// src/proximity_detector.rs
use std::collections::HashMap;
use std::env;
use chrono::{DateTime, Utc, Duration};
use log::{debug, info, warn};
use crate::types::StoredZone;
use crate::notifications::notification_manager::get_global_notification_manager;

#[derive(Debug, Clone)]
pub struct ProximityState {
    pub last_alert_time: Option<DateTime<Utc>>,
    pub alert_sent_for_current_approach: bool,
    pub last_price: Option<f64>,
}

pub struct ProximityDetector {
    enabled: bool,
    proximity_threshold_pips: f64,
    cooldown_minutes: i64,
    // Track proximity state per zone
    zone_proximity_states: HashMap<String, ProximityState>,
}

impl ProximityDetector {
    pub fn new() -> Self {
        let enabled = env::var("ENABLE_PROXIMITY_ALERTS")
            .unwrap_or_else(|_| "true".to_string())
            .trim()
            .to_lowercase() == "true";

        let proximity_threshold_pips = env::var("PROXIMITY_THRESHOLD_PIPS")
            .unwrap_or_else(|_| "15.0".to_string())
            .parse::<f64>()
            .unwrap_or(15.0);

        let cooldown_minutes = env::var("PROXIMITY_ALERT_COOLDOWN_MINUTES")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<i64>()
            .unwrap_or(30);

        if enabled {
            info!("🎯 Proximity detector initialized - threshold: {:.1} pips, cooldown: {} min", 
                  proximity_threshold_pips, cooldown_minutes);
        } else {
            info!("🎯 Proximity detector disabled");
        }

        Self {
            enabled,
            proximity_threshold_pips,
            cooldown_minutes,
            zone_proximity_states: HashMap::new(),
        }
    }

    /// Check if price is approaching a zone and send alerts if needed
    pub async fn check_proximity(&mut self, symbol: &str, current_price: f64, zone: &StoredZone, timeframe: &str) {
        if !self.enabled || !zone.is_active {
            return;
        }

        let zone_id = match &zone.zone_id {
            Some(id) => id,
            None => return,
        };

        let (zone_high, zone_low) = match (zone.zone_high, zone.zone_low) {
            (Some(high), Some(low)) => (high, low),
            _ => return,
        };

        let zone_type = zone.zone_type.as_deref().unwrap_or("");
        
        // Determine if this is a supply or demand zone
        let is_supply = zone_type.to_lowercase().contains("supply");
        
        // Calculate the proximal line (entry point)
        let proximal_line = if is_supply {
            zone_low  // For supply zones, we enter when price reaches the low (proximal)
        } else {
            zone_high // For demand zones, we enter when price reaches the high (proximal)
        };

        // Calculate distance in pips
        let distance_pips = calculate_pips_distance(symbol, current_price, proximal_line);
        
        // Get or create proximity state for this zone
        let proximity_state = self.zone_proximity_states
            .entry(zone_id.clone())
            .or_insert_with(|| ProximityState {
                last_alert_time: None,
                alert_sent_for_current_approach: false,
                last_price: None,
            });

        // Check if we're within proximity threshold
        let within_proximity = distance_pips <= self.proximity_threshold_pips;
        
        debug!("🎯 Proximity check for {} {}: current={:.5}, proximal={:.5}, distance={:.1}pips, within_threshold={}", 
               symbol, zone_type, current_price, proximal_line, distance_pips, within_proximity);

        if within_proximity {
            // Check if we should send an alert - do this check before any mutations
            let should_alert = {
                // Don't send if we already alerted for this approach
                if proximity_state.alert_sent_for_current_approach {
                    false
                } else {
                    // Check cooldown period
                    let cooldown_ok = if let Some(last_alert) = proximity_state.last_alert_time {
                        let cooldown_duration = Duration::minutes(self.cooldown_minutes);
                        Utc::now() - last_alert >= cooldown_duration
                    } else {
                        true
                    };

                    if !cooldown_ok {
                        false
                    } else {
                        // Check if price is moving in the right direction (approaching the zone)
                        if let Some(last_price) = proximity_state.last_price {
                            let price_moving_toward_zone = if is_supply {
                                current_price > last_price // Price rising toward supply zone
                            } else {
                                current_price < last_price // Price falling toward demand zone
                            };

                            if !price_moving_toward_zone {
                                debug!("🎯 Price not moving toward zone, suppressing alert");
                                false
                            } else {
                                true
                            }
                        } else {
                            true
                        }
                    }
                }
            };
            
            if should_alert {
                info!("🎯 PROXIMITY ALERT: {} approaching {} zone {} - {:.1} pips away", 
                      symbol, zone_type, zone_id, distance_pips);

                // Send notifications
                if let Some(notification_manager) = get_global_notification_manager() {
                    notification_manager.notify_proximity_alert(
                        symbol,
                        current_price,
                        zone_type,
                        zone_high,
                        zone_low,
                        zone_id,
                        timeframe,
                        distance_pips
                    ).await;
                }

                // Update state to prevent spam
                proximity_state.alert_sent_for_current_approach = true;
                proximity_state.last_alert_time = Some(Utc::now());
                
                debug!("🎯 Proximity alert sent for zone {}, marking as alerted", zone_id);
            } else {
                debug!("🎯 Within proximity but alert suppressed for zone {} (cooldown or already alerted)", zone_id);
            }
        } else {
            // Price moved away from zone - reset alert state for next approach
            if proximity_state.alert_sent_for_current_approach {
                debug!("🎯 Price moved away from zone {} - resetting alert state for next approach", zone_id);
                proximity_state.alert_sent_for_current_approach = false;
            }
        }

        // Always update last price for trend detection
        proximity_state.last_price = Some(current_price);
    }

    /// Clean up old proximity states to prevent memory leaks
    pub fn cleanup_old_states(&mut self) {
        let cutoff_time = Utc::now() - Duration::hours(24);
        let initial_count = self.zone_proximity_states.len();
        
        self.zone_proximity_states.retain(|_zone_id, state| {
            if let Some(last_alert) = state.last_alert_time {
                last_alert > cutoff_time
            } else {
                true // Keep states that haven't been alerted yet
            }
        });

        let removed_count = initial_count - self.zone_proximity_states.len();
        if removed_count > 0 {
            debug!("🎯 Cleaned up {} old proximity states", removed_count);
        }
    }

    /// Get current proximity threshold for debugging
    pub fn get_proximity_threshold(&self) -> f64 {
        self.proximity_threshold_pips
    }

    /// Check if proximity detection is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Calculate distance between two prices in pips for a given symbol
fn calculate_pips_distance(symbol: &str, price1: f64, price2: f64) -> f64 {
    let pip_value = get_pip_value_for_symbol(symbol);
    (price1 - price2).abs() / pip_value
}

/// Get pip value for different currency pairs
fn get_pip_value_for_symbol(symbol: &str) -> f64 {
    match symbol {
        // JPY pairs - pip is 0.01
        s if s.contains("JPY") => 0.01,
        // Most major pairs - pip is 0.0001
        "EURUSD_SB" | "GBPUSD_SB" | "AUDUSD_SB" | "NZDUSD_SB" |
        "USDCHF_SB" | "USDCAD_SB" | "EURGBP_SB" | "EURAUD_SB" |
        "EURCAD_SB" | "EURNZD_SB" | "GBPAUD_SB" | "GBPCAD_SB" |
        "GBPNZD_SB" | "AUDNZD_SB" | "AUDCAD_SB" | "GBPCHF_SB" |
        "EURCHF_SB" => 0.0001,
        // Default to 0.0001 for unknown pairs
        _ => {
            warn!("🎯 Unknown symbol for pip calculation: {}, using default 0.0001", symbol);
            0.0001
        }
    }
}