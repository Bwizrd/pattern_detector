// src/zone_interactions.rs
// Zone interaction tracking system for monitoring price behavior with zones

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use log::{debug, error, info, warn};

/// Configuration for interaction tracking
#[derive(Debug, Clone)]
pub struct InteractionConfig {
    /// Minimum time between proximal crossings to count as separate events (in seconds)
    pub debounce_time_seconds: u64,
    /// File path for storing zone interactions
    pub file_path: String,
}

impl Default for InteractionConfig {
    fn default() -> Self {
        Self {
            debounce_time_seconds: 60, // Only count one crossing per minute
            file_path: "zone_interactions.json".to_string(),
        }
    }
}

/// Price interaction event with a zone
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PriceInteractionEvent {
    pub timestamp: String,
    pub price: f64,
    pub event_type: InteractionEventType,
    pub distance_from_proximal: f64, // Positive when inside zone, negative when outside
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum InteractionEventType {
    EnteredZone,
    ExitedZone,
    ProximalCrossFromBelow,
    ProximalCrossFromAbove,
    DistalBreak, // Zone invalidation
}

/// Complete interaction metrics for a single zone
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ZoneInteractionMetrics {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String, // "supply_zone" or "demand_zone"
    pub zone_high: f64,
    pub zone_low: f64,
    
    // Current state
    pub price_inside: bool,
    pub time_entered: Option<String>,
    pub has_ever_entered: bool,
    
    // Interaction statistics
    pub total_time_inside_seconds: u64,
    pub proximal_crossings_from_below: u32,
    pub proximal_crossings_from_above: u32,
    pub zone_entries: u32, // Only count actual zone entries, not exits
    pub last_zone_entry_time: Option<String>, // For debouncing zone entries
    pub last_crossing_time: Option<String>,
    
    // Price interaction history (limited to last N events to prevent unbounded growth)
    pub price_interaction_history: Vec<PriceInteractionEvent>,
    
    // Zone status
    pub is_zone_active: bool,
    pub zone_invalidation_time: Option<String>,
    
    // Last update timestamp
    pub last_updated: String,
}

impl ZoneInteractionMetrics {
    pub fn new(zone_id: String, symbol: String, timeframe: String, zone_type: String, zone_high: f64, zone_low: f64) -> Self {
        Self {
            zone_id,
            symbol,
            timeframe,
            zone_type,
            zone_high,
            zone_low,
            price_inside: false,
            time_entered: None,
            has_ever_entered: false,
            total_time_inside_seconds: 0,
            proximal_crossings_from_below: 0,
            proximal_crossings_from_above: 0,
            zone_entries: 0,
            last_zone_entry_time: None,
            last_crossing_time: None,
            price_interaction_history: Vec::new(),
            is_zone_active: true,
            zone_invalidation_time: None,
            last_updated: Utc::now().to_rfc3339(),
        }
    }

    /// Get the proximal line price (entry side of the zone)
    pub fn get_proximal_line(&self) -> f64 {
        if self.zone_type == "supply_zone" {
            self.zone_low // For supply zones, price enters from below (proximal = low)
        } else {
            self.zone_high // For demand zones, price enters from above (proximal = high)
        }
    }

    /// Get the distal line price (exit side of the zone)
    pub fn get_distal_line(&self) -> f64 {
        if self.zone_type == "supply_zone" {
            self.zone_high // For supply zones, distal break is above (distal = high)
        } else {
            self.zone_low // For demand zones, distal break is below (distal = low)
        }
    }

    /// Check if price is inside the zone
    pub fn is_price_inside_zone(&self, price: f64) -> bool {
        price >= self.zone_low && price <= self.zone_high
    }

    /// Update interaction metrics based on new price data
    pub fn update_with_price(&mut self, timestamp: String, price: f64, config: &InteractionConfig) {
        let now = Utc::now().to_rfc3339();
        let was_inside = self.price_inside;
        let is_inside = self.is_price_inside_zone(price);
        let proximal_line = self.get_proximal_line();
        let distal_line = self.get_distal_line();

        // Check for zone invalidation (distal break)
        if self.is_zone_active {
            let zone_invalidated = if self.zone_type == "supply_zone" {
                price > distal_line // Price breaks above supply zone
            } else {
                price < distal_line // Price breaks below demand zone
            };

            if zone_invalidated {
                self.is_zone_active = false;
                self.zone_invalidation_time = Some(timestamp.clone());
                self.add_interaction_event(timestamp.clone(), price, InteractionEventType::DistalBreak);
                debug!("[ZoneInteraction] Zone {} invalidated at price {}", self.zone_id, price);
            }
        }

        // Handle zone entry/exit
        if was_inside != is_inside {
            if is_inside {
                // Entering zone
                self.price_inside = true;
                self.time_entered = Some(timestamp.clone());
                self.has_ever_entered = true;
                
                // Only count zone entry if enough time has passed (1-minute debounce)
                if self.should_count_zone_entry(&timestamp) {
                    self.zone_entries += 1;
                    self.last_zone_entry_time = Some(timestamp.clone());
                    debug!("[ZoneInteraction] Price {} entered zone {} (entry #{}) - counted after debounce", price, self.zone_id, self.zone_entries);
                } else {
                    debug!("[ZoneInteraction] Price {} entered zone {} but entry not counted due to debounce", price, self.zone_id);
                }
                
                self.add_interaction_event(timestamp.clone(), price, InteractionEventType::EnteredZone);
            } else {
                // Exiting zone
                if let Some(entry_time) = &self.time_entered {
                    if let (Ok(entry_dt), Ok(exit_dt)) = (
                        DateTime::parse_from_rfc3339(entry_time),
                        DateTime::parse_from_rfc3339(&timestamp)
                    ) {
                        let duration = exit_dt.signed_duration_since(entry_dt);
                        if duration.num_seconds() > 0 {
                            self.total_time_inside_seconds += duration.num_seconds() as u64;
                        }
                    }
                }
                
                self.price_inside = false;
                self.time_entered = None;
                self.add_interaction_event(timestamp.clone(), price, InteractionEventType::ExitedZone);
                debug!("[ZoneInteraction] Price {} exited zone {}", price, self.zone_id);
            }
        }

        // Handle proximal line crossings (with debouncing)
        if self.should_count_proximal_crossing(&timestamp, config) {
            let crossed_proximal = if self.zone_type == "supply_zone" {
                // For supply zones, crossing from below means price >= proximal_line
                !was_inside && price >= proximal_line
            } else {
                // For demand zones, crossing from above means price <= proximal_line  
                !was_inside && price <= proximal_line
            };

            if crossed_proximal {
                if self.zone_type == "supply_zone" {
                    self.proximal_crossings_from_below += 1;
                    self.add_interaction_event(timestamp.clone(), price, InteractionEventType::ProximalCrossFromBelow);
                    debug!("[ZoneInteraction] Proximal crossing from below at price {} for zone {}", price, self.zone_id);
                } else {
                    self.proximal_crossings_from_above += 1;
                    self.add_interaction_event(timestamp.clone(), price, InteractionEventType::ProximalCrossFromAbove);
                    debug!("[ZoneInteraction] Proximal crossing from above at price {} for zone {}", price, self.zone_id);
                }
                self.last_crossing_time = Some(timestamp.clone());
            }
        }

        self.last_updated = now;
    }

    /// Check if we should count this proximal crossing (debouncing logic)
    fn should_count_proximal_crossing(&self, timestamp: &str, config: &InteractionConfig) -> bool {
        if let Some(last_crossing) = &self.last_crossing_time {
            if let (Ok(last_dt), Ok(current_dt)) = (
                DateTime::parse_from_rfc3339(last_crossing),
                DateTime::parse_from_rfc3339(timestamp)
            ) {
                let duration = current_dt.signed_duration_since(last_dt);
                return duration.num_seconds() >= config.debounce_time_seconds as i64;
            }
        }
        true // First crossing or parse error - allow it
    }

    /// Check if we should count this zone entry (1-minute debouncing)
    fn should_count_zone_entry(&self, timestamp: &str) -> bool {
        if let Some(last_entry) = &self.last_zone_entry_time {
            if let (Ok(last_dt), Ok(current_dt)) = (
                DateTime::parse_from_rfc3339(last_entry),
                DateTime::parse_from_rfc3339(timestamp)
            ) {
                let duration = current_dt.signed_duration_since(last_dt);
                return duration.num_seconds() >= 60; // 1-minute debounce for zone entries
            }
        }
        true // First entry or parse error - allow it
    }

    /// Add an interaction event to the history (with size limit)
    fn add_interaction_event(&mut self, timestamp: String, price: f64, event_type: InteractionEventType) {
        let distance_from_proximal = price - self.get_proximal_line();
        
        let event = PriceInteractionEvent {
            timestamp,
            price,
            event_type,
            distance_from_proximal,
        };

        self.price_interaction_history.push(event);

        // Keep only the last 100 events to prevent unbounded growth
        if self.price_interaction_history.len() > 100 {
            self.price_interaction_history.remove(0);
        }
    }

    /// Get the current distance from proximal line (positive = inside zone, negative = outside)
    pub fn get_distance_from_proximal(&self, current_price: f64) -> f64 {
        let proximal = self.get_proximal_line();
        if self.zone_type == "supply_zone" {
            current_price - proximal // Positive when above proximal (inside zone)
        } else {
            proximal - current_price // Positive when below proximal (inside zone)
        }
    }
}

/// Main container for all zone interaction data
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ZoneInteractionContainer {
    pub last_updated: String,
    pub metrics: HashMap<String, ZoneInteractionMetrics>, // key = zone_id
}

impl ZoneInteractionContainer {
    pub fn new() -> Self {
        Self {
            last_updated: Utc::now().to_rfc3339(),
            metrics: HashMap::new(),
        }
    }

    /// Load zone interactions from file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref();
        if !path.exists() {
            info!("[ZoneInteraction] Creating new zone interactions file at {:?}", path);
            return Ok(Self::new());
        }

        let content = fs::read_to_string(path)?;
        let container: ZoneInteractionContainer = serde_json::from_str(&content)?;
        info!("[ZoneInteraction] Loaded {} zone interactions from {:?}", container.metrics.len(), path);
        Ok(container)
    }

    /// Save zone interactions to file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path.as_ref(), content)?;
        debug!("[ZoneInteraction] Saved {} zone interactions to {:?}", self.metrics.len(), path.as_ref());
        Ok(())
    }

    /// Get or create zone interaction metrics
    pub fn get_or_create_zone_metrics(&mut self, zone_id: String, symbol: String, timeframe: String, zone_type: String, zone_high: f64, zone_low: f64) -> &mut ZoneInteractionMetrics {
        self.metrics.entry(zone_id.clone()).or_insert_with(|| {
            debug!("[ZoneInteraction] Creating new interaction metrics for zone {}", zone_id);
            ZoneInteractionMetrics::new(zone_id, symbol, timeframe, zone_type, zone_high, zone_low)
        })
    }

    /// Clean up stale interaction data (older than 24 hours for last_crossing_time)
    pub fn cleanup_stale_data(&mut self) {
        let now = Utc::now();
        let mut zones_to_clean = Vec::new();
        
        for (zone_id, metrics) in &self.metrics {
            // Reset last_crossing_time if older than 24 hours and price is far from zone
            if let Some(last_crossing) = &metrics.last_crossing_time {
                if let Ok(last_crossing_dt) = DateTime::parse_from_rfc3339(last_crossing) {
                    let duration = now.signed_duration_since(last_crossing_dt.with_timezone(&Utc));
                    if duration.num_hours() > 24 {
                        zones_to_clean.push(zone_id.clone());
                    }
                }
            }
        }
        
        for zone_id in zones_to_clean {
            if let Some(metrics) = self.metrics.get_mut(&zone_id) {
                metrics.last_crossing_time = None;
                debug!("Cleaned stale crossing data for zone {}", zone_id);
            }
        }
    }

    /// Update all zone interactions with new price data
    /// Only tracks zones within max_distance_pips from their proximal line
    pub fn update_all_zones_with_price(&mut self, symbol: &str, price: f64, timestamp: String, config: &InteractionConfig) {
        // Validate price data - reject obviously invalid prices
        if price <= 0.0 || price.is_nan() || price.is_infinite() {
            warn!("Invalid price data for {}: {}", symbol, price);
            return;
        }
        
        // Additional sanity checks for major pairs
        match symbol {
            "EURJPY" | "GBPJPY" | "USDJPY" => {
                if price < 50.0 || price > 300.0 {
                    warn!("Suspicious JPY price for {}: {} - rejecting", symbol, price);
                    return;
                }
            },
            "EURUSD" | "GBPUSD" | "AUDUSD" | "NZDUSD" => {
                if price < 0.5 || price > 2.0 {
                    warn!("Suspicious USD price for {}: {} - rejecting", symbol, price);
                    return;
                }
            },
            _ => {}
        }
        
        let max_distance_pips = 50.0; // Only track zones within 50 pips of proximal
        let pip_value = self.get_pip_value(symbol);
        
        let mut zones_to_update = Vec::new();
        
        // Collect zones for this symbol that are close enough to be relevant
        for (zone_id, metrics) in &self.metrics {
            if metrics.symbol == symbol && metrics.is_zone_active {
                // Calculate distance from current price to proximal line
                let proximal_line = metrics.get_proximal_line();
                let distance_to_proximal_pips = (price - proximal_line).abs() / pip_value;
                
                // Only track zones within 50 pips of their proximal line
                if distance_to_proximal_pips <= max_distance_pips {
                    zones_to_update.push(zone_id.clone());
                }
            }
        }

        // Update each relevant zone
        for zone_id in zones_to_update {
            if let Some(metrics) = self.metrics.get_mut(&zone_id) {
                metrics.update_with_price(timestamp.clone(), price, config);
            }
        }

        self.last_updated = Utc::now().to_rfc3339();
    }

    /// Get pip value for a symbol (helper method)
    fn get_pip_value(&self, symbol: &str) -> f64 {
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

    /// Clean up old inactive zones (optional maintenance)
    pub fn cleanup_old_zones(&mut self, max_age_days: u64) {
        let cutoff_time = Utc::now() - chrono::Duration::days(max_age_days as i64);
        let cutoff_rfc3339 = cutoff_time.to_rfc3339();

        let mut zones_to_remove = Vec::new();
        for (zone_id, metrics) in &self.metrics {
            if !metrics.is_zone_active {
                if let Some(invalidation_time) = &metrics.zone_invalidation_time {
                    if invalidation_time < &cutoff_rfc3339 {
                        zones_to_remove.push(zone_id.clone());
                    }
                }
            }
        }

        for zone_id in zones_to_remove {
            self.metrics.remove(&zone_id);
            debug!("[ZoneInteraction] Removed old inactive zone {}", zone_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zone_interaction_metrics_creation() {
        let metrics = ZoneInteractionMetrics::new(
            "test_zone".to_string(),
            "EURUSD".to_string(),
            "1h".to_string(),
            "supply_zone".to_string(),
            1.1000,
            1.0950,
        );

        assert_eq!(metrics.zone_id, "test_zone");
        assert_eq!(metrics.symbol, "EURUSD");
        assert!(!metrics.price_inside);
        assert!(!metrics.has_ever_entered);
        assert_eq!(metrics.get_proximal_line(), 1.0950); // Supply zone proximal is low
        assert_eq!(metrics.get_distal_line(), 1.1000); // Supply zone distal is high
    }

    #[test]
    fn test_price_inside_zone_detection() {
        let metrics = ZoneInteractionMetrics::new(
            "test_zone".to_string(),
            "EURUSD".to_string(),
            "1h".to_string(),
            "supply_zone".to_string(),
            1.1000,
            1.0950,
        );

        assert!(!metrics.is_price_inside_zone(1.0940)); // Below zone
        assert!(metrics.is_price_inside_zone(1.0975)); // Inside zone
        assert!(!metrics.is_price_inside_zone(1.1010)); // Above zone
    }
}