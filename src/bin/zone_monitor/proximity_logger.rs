// src/bin/zone_monitor/proximity_logger.rs
use std::collections::HashMap;
use std::env;
use std::fs;
use chrono::{DateTime, Utc};
use tracing::{debug, error, info};
use serde_json::json;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use crate::types::{Zone, PriceUpdate};
use crate::trade_rules::TradeRulesEngine;

#[derive(Debug, Clone)]
pub struct ProximityLogEntry {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub current_price: f64,
    pub proximal_price: f64,
    pub distance_pips: f64,
    pub zone_strength: f64,
    pub touch_count: i32,
    pub last_updated: DateTime<Utc>,
    pub zone_high: f64,
    pub zone_low: f64,
}

#[derive(Debug, Clone)]
pub struct TradeTriggeredEntry {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub trigger_price: f64,
    pub proximal_price: f64,
    pub zone_strength: f64,
    pub touch_count: i32,
    pub trigger_time: DateTime<Utc>,
    pub zone_high: f64,
    pub zone_low: f64,
    pub expected_trade_direction: String, // BUY or SELL
}

#[derive(Debug)]
pub struct ProximityLogger {
    enabled: bool,
    log_threshold_pips: f64,
    log_dir: String,
    // Track zones currently within proximity
    proximity_zones: HashMap<String, ProximityLogEntry>,
    // Track zones that have already triggered to avoid duplicates
    triggered_zones: HashMap<String, DateTime<Utc>>,
    // Centralized trade rules engine
    trade_rules: TradeRulesEngine,
}

impl ProximityLogger {
    pub fn new() -> Self {
        let enabled = env::var("ENABLE_PROXIMITY_LOGGING")
            .unwrap_or_else(|_| "false".to_string())
            .trim()
            .to_lowercase() == "true";

        let log_threshold_pips = env::var("PROXIMITY_LOG_THRESHOLD_PIPS")
            .unwrap_or_else(|_| "10.0".to_string())
            .parse::<f64>()
            .unwrap_or(10.0);

        let log_dir = env::var("PROXIMITY_LOG_DIR")
            .unwrap_or_else(|_| "logs/proximity".to_string());

        if enabled {
            // Create log directory if it doesn't exist
            if let Err(e) = fs::create_dir_all(&log_dir) {
                error!("📝 Failed to create proximity log directory {}: {}", log_dir, e);
            } else {
                info!("📝 Proximity logger initialized - threshold: {:.1} pips, dir: {}", 
                      log_threshold_pips, log_dir);
            }
        } else {
            info!("📝 Proximity logging disabled");
        }

        Self {
            enabled,
            log_threshold_pips,
            log_dir,
            proximity_zones: HashMap::new(),
            triggered_zones: HashMap::new(),
            trade_rules: TradeRulesEngine::new(),
        }
    }

    /// Log proximity information for zones
    pub async fn log_proximity(&mut self, price_update: &PriceUpdate, zones: &[Zone], zone_interactions: Option<&pattern_detector::zone_interactions::ZoneInteractionContainer>) {
        if !self.enabled {
            return;
        }

        let current_price = (price_update.bid + price_update.ask) / 2.0;

        for zone in zones {
            if !zone.is_active {
                continue;
            }

            let zone_type = &zone.zone_type;
            let zone_strength = zone.strength;
            let touch_count = zone.touch_count;

            // Determine if this is a supply or demand zone
            let is_supply = zone_type.to_lowercase().contains("supply");
            
            // Calculate the proximal line (entry point)
            let proximal_price = if is_supply {
                zone.low  // For supply zones, we enter when price reaches the low (proximal)
            } else {
                zone.high // For demand zones, we enter when price reaches the high (proximal)
            };

            // Calculate distance in pips
            let distance_pips = self.calculate_pips_distance(&price_update.symbol, current_price, proximal_price);

            // Use centralized trade rules for trade trigger determination
            let evaluation = self.trade_rules.evaluate_trade_opportunity(price_update, zone, zone_interactions);
            
            // Log trade trigger ONLY if centralized rules say it should trigger
            if evaluation.should_trigger_trade && !self.triggered_zones.contains_key(&zone.id) {
                self.log_trade_triggered(&price_update.symbol, current_price, zone, proximal_price, is_supply).await;
                self.triggered_zones.insert(zone.id.clone(), Utc::now());
            }

            // Only log proximity if within threshold (ORIGINAL LOGIC)
            if distance_pips <= self.log_threshold_pips {
                let entry = ProximityLogEntry {
                    zone_id: zone.id.clone(),
                    symbol: price_update.symbol.clone(),
                    timeframe: zone.timeframe.clone(),
                    zone_type: zone_type.clone(),
                    current_price,
                    proximal_price,
                    distance_pips,
                    zone_strength,
                    touch_count,
                    last_updated: Utc::now(),
                    zone_high: zone.high,
                    zone_low: zone.low,
                };

                // Update or insert the proximity entry
                let was_new = !self.proximity_zones.contains_key(&zone.id);
                self.proximity_zones.insert(zone.id.clone(), entry.clone());

                if was_new {
                    debug!("📝 New zone entered proximity tracking: {} {} @ {:.1} pips", 
                           price_update.symbol, zone_type, distance_pips);
                }
            } else {
                // Remove from proximity tracking if moved beyond threshold
                if self.proximity_zones.remove(&zone.id).is_some() {
                    debug!("📝 Zone removed from proximity tracking: {} {} (moved beyond {:.1} pips)", 
                           price_update.symbol, zone_type, self.log_threshold_pips);
                }
            }
        }

        // Write the current proximity state to file
        if !self.proximity_zones.is_empty() {
            self.write_proximity_log().await;
        }
    }

    /// Log when a trade should be triggered (price enters zone)
    async fn log_trade_triggered(&self, symbol: &str, trigger_price: f64, zone: &Zone, proximal_price: f64, is_supply: bool) {
        let expected_direction = if is_supply { "SELL" } else { "BUY" };

        let trigger_entry = TradeTriggeredEntry {
            zone_id: zone.id.clone(),
            symbol: symbol.to_string(),
            timeframe: zone.timeframe.clone(),
            zone_type: zone.zone_type.clone(),
            trigger_price,
            proximal_price,
            zone_strength: zone.strength,
            touch_count: zone.touch_count,
            trigger_time: Utc::now(),
            zone_high: zone.high,
            zone_low: zone.low,
            expected_trade_direction: expected_direction.to_string(),
        };

        info!("🚨 TRADE_TRIGGERED: {} {} zone {} @ {:.5} - {} trade should be booked NOW!", 
              symbol, zone.zone_type, zone.id, trigger_price, expected_direction);

        // Write to trade triggers log
        self.write_trade_trigger_log(trigger_entry).await;
    }

    /// Write current proximity state to log file
    async fn write_proximity_log(&self) {
        let date_str = Utc::now().format("%Y-%m-%d").to_string();
        let file_path = format!("{}/proximity_zones_{}.jsonl", self.log_dir, date_str);

        // Create file content with all current proximity zones
        let mut lines = Vec::new();
        for entry in self.proximity_zones.values() {
            let json_entry = json!({
                "timestamp": entry.last_updated.to_rfc3339(),
                "zone_id": entry.zone_id,
                "symbol": entry.symbol,
                "timeframe": entry.timeframe,
                "zone_type": entry.zone_type,
                "current_price": entry.current_price,
                "proximal_price": entry.proximal_price,
                "distance_pips": entry.distance_pips,
                "zone_strength": entry.zone_strength,
                "touch_count": entry.touch_count,
                "zone_high": entry.zone_high,
                "zone_low": entry.zone_low,
                "status": "PROXIMITY_TRACKED"
            });
            lines.push(json_entry.to_string());
        }

        // Write to file (overwrite entire file with current state)
        if let Err(e) = self.write_file_content(&file_path, &lines.join("\n")).await {
            error!("📝 Failed to write proximity log: {}", e);
        } else {
            debug!("📝 Updated proximity log: {} zones tracked", self.proximity_zones.len());
        }
    }

    /// Write trade trigger event to separate log file
    async fn write_trade_trigger_log(&self, entry: TradeTriggeredEntry) {
        let date_str = Utc::now().format("%Y-%m-%d").to_string();
        let file_path = format!("{}/trade_triggers_{}.jsonl", self.log_dir, date_str);

        let json_entry = json!({
            "timestamp": entry.trigger_time.to_rfc3339(),
            "zone_id": entry.zone_id,
            "symbol": entry.symbol,
            "timeframe": entry.timeframe,
            "zone_type": entry.zone_type,
            "trigger_price": entry.trigger_price,
            "proximal_price": entry.proximal_price,
            "zone_strength": entry.zone_strength,
            "touch_count": entry.touch_count,
            "zone_high": entry.zone_high,
            "zone_low": entry.zone_low,
            "expected_trade_direction": entry.expected_trade_direction,
            "status": "TRADE_SHOULD_BE_TRIGGERED",
            "alert": "🚨 PRICE ENTERED ZONE - TRADE SHOULD BE BOOKED NOW! 🚨"
        });

        // Append to triggers log file
        if let Err(e) = self.append_to_file(&file_path, &format!("{}\n", json_entry.to_string())).await {
            error!("📝 Failed to write trade trigger log: {}", e);
        } else {
            info!("📝 Logged trade trigger: {} {} zone {} @ {:.5}", 
                  entry.symbol, entry.zone_type, entry.zone_id, entry.trigger_price);
        }
    }

    /// Write content to file (overwrite)
    async fn write_file_content(&self, file_path: &str, content: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut file = File::create(file_path).await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    /// Append content to file
    async fn append_to_file(&self, file_path: &str, content: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)
            .await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    /// Get current proximity zones count for debugging
    pub fn get_proximity_zones_count(&self) -> usize {
        self.proximity_zones.len()
    }

    /// Clean up old triggered zones to prevent memory leaks
    pub fn cleanup_old_triggered_zones(&mut self) {
        let cutoff_time = Utc::now() - chrono::Duration::hours(24);
        let initial_count = self.triggered_zones.len();
        
        self.triggered_zones.retain(|_zone_id, trigger_time| {
            *trigger_time > cutoff_time
        });

        let removed_count = initial_count - self.triggered_zones.len();
        if removed_count > 0 {
            debug!("📝 Cleaned up {} old triggered zone records", removed_count);
        }
    }

    /// Check if logging is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Calculate distance between two prices in pips for a given symbol
    fn calculate_pips_distance(&self, symbol: &str, price1: f64, price2: f64) -> f64 {
        let pip_value = self.get_pip_value_for_symbol(symbol);
        (price1 - price2).abs() / pip_value
    }

    /// Get pip value for different currency pairs
    fn get_pip_value_for_symbol(&self, symbol: &str) -> f64 {
        // JPY pairs use 0.01 as pip value
        if symbol.contains("JPY") {
            0.01
        } else if symbol.ends_with("_SB") {
            // All SB suffixed pairs use 0.0001 (except JPY which is handled above)
            0.0001
        } else {
            // Legacy support for non-SB symbols
            match symbol {
                // Indices
                "NAS100" => 1.0,
                "US500" => 0.1,
                // All other pairs use 0.0001
                _ => 0.0001,
            }
        }
    }
}

