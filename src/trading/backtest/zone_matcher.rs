// Zone matching logic for backtest reconciliation
use serde::{Deserialize, Serialize};
use std::fs;
use chrono::{DateTime, Utc};
use log::{debug, warn, info};

#[derive(Debug, Clone, Deserialize)]
pub struct CachedZone {
    pub id: String,
    pub symbol: String,
    pub zone_type: String,
    pub high: f64,
    pub low: f64,
    pub strength: f64,
    pub timeframe: String,
    pub touch_count: i32,
    pub created_at: String,
    pub start_time: String,
    pub bars_active: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PendingOrder {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub zone_high: f64,
    pub zone_low: f64,
    pub placed_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedZones {
    pub last_updated: String,
    pub zones: std::collections::HashMap<String, Vec<CachedZone>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedPendingOrders {
    pub last_updated: String,
    pub orders: std::collections::HashMap<String, PendingOrder>,
}

#[derive(Debug, Clone)]
pub struct ZoneMatchResult {
    pub matched: bool,
    pub cached_zone_id: Option<String>,
    pub match_score: f64,
    pub match_details: String,
}

pub struct ZoneMatcher {
    cached_zones: Vec<CachedZone>,
    pending_orders: Vec<PendingOrder>,
}

impl ZoneMatcher {
    pub fn new() -> Self {
        Self {
            cached_zones: Vec::new(),
            pending_orders: Vec::new(),
        }
    }

    /// Load cached zones and pending orders from shared JSON files
    pub fn load_cache_data(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Load shared zones
        match fs::read_to_string("shared_zones.json") {
            Ok(zones_content) => {
                let shared_zones: SharedZones = serde_json::from_str(&zones_content)?;
                self.cached_zones.clear();
                
                for (_symbol, zones) in shared_zones.zones {
                    self.cached_zones.extend(zones);
                }
                
                info!("Loaded {} cached zones for matching", self.cached_zones.len());
            }
            Err(e) => {
                warn!("Failed to load shared_zones.json: {}", e);
            }
        }

        // Load pending orders
        match fs::read_to_string("shared_pending_orders.json") {
            Ok(orders_content) => {
                let shared_orders: SharedPendingOrders = serde_json::from_str(&orders_content)?;
                self.pending_orders.clear();
                
                for (_order_id, order) in shared_orders.orders {
                    self.pending_orders.push(order);
                }
                
                info!("Loaded {} pending orders for matching", self.pending_orders.len());
            }
            Err(e) => {
                warn!("Failed to load shared_pending_orders.json: {}", e);
            }
        }

        Ok(())
    }

    /// Match a trade entry price against cached zone boundaries
    pub fn match_trade_to_zones(
        &self,
        symbol: &str,
        timeframe: &str,
        zone_type: &str,
        entry_price: f64,
        entry_time: Option<&str>,
    ) -> ZoneMatchResult {
        info!("🔍 Zone matching started for {} {} {} entry_price={}", symbol, timeframe, zone_type, entry_price);
        info!("📋 Available cached zones: {}, pending orders: {}", self.cached_zones.len(), self.pending_orders.len());
        
        let mut best_match: Option<(String, f64, String)> = None;
        let threshold = 0.95; // 95% similarity threshold

        // Clean inputs for comparison
        let clean_symbol = symbol.replace("_SB", "");
        let clean_timeframe = timeframe.to_lowercase();
        let clean_zone_type = if zone_type.contains("supply") { "supply_zone" } else { "demand_zone" };
        
        info!("🧹 Cleaned inputs: symbol={}, timeframe={}, zone_type={}", clean_symbol, clean_timeframe, clean_zone_type);

        // Check cached zones first
        for cached_zone in &self.cached_zones {
            if cached_zone.symbol == clean_symbol 
                && cached_zone.timeframe == clean_timeframe 
                && cached_zone.zone_type == clean_zone_type {
                
                // Check if entry price is within zone boundaries
                if entry_price >= cached_zone.low && entry_price <= cached_zone.high {
                    let match_score = self.calculate_trade_to_zone_similarity(
                        entry_price, entry_time,
                        cached_zone.high, cached_zone.low, Some(&cached_zone.start_time)
                    );

                    if match_score > threshold {
                        let details = format!(
                            "Cached zone match: {:.4}% (entry: {:.5} within zone {:.5}-{:.5})",
                            match_score * 100.0, entry_price, cached_zone.low, cached_zone.high
                        );
                        
                        if best_match.is_none() || match_score > best_match.as_ref().unwrap().1 {
                            best_match = Some((cached_zone.id.clone(), match_score, details));
                        }
                    }
                }
            }
        }

        // Check pending orders
        info!("🔍 Checking {} pending orders", self.pending_orders.len());
        for (idx, pending_order) in self.pending_orders.iter().enumerate() {
            info!("📦 Pending order {}: {} {} {} zone_id={}", idx, pending_order.symbol, pending_order.timeframe, pending_order.zone_type, pending_order.zone_id);
            
            if pending_order.symbol == clean_symbol 
                && pending_order.timeframe == clean_timeframe 
                && pending_order.zone_type == clean_zone_type {
                
                info!("✅ Symbol/timeframe/type match! Checking price boundaries...");
                info!("💰 Entry: {}, Zone: {:.8} to {:.8}", entry_price, pending_order.zone_low, pending_order.zone_high);
                
                // Check if entry price is within zone boundaries
                if entry_price >= pending_order.zone_low && entry_price <= pending_order.zone_high {
                    info!("🎯 PRICE MATCH! Entry price within zone boundaries");
                    let match_score = self.calculate_trade_to_zone_similarity(
                        entry_price, entry_time,
                        pending_order.zone_high, pending_order.zone_low, None
                    );

                    if match_score > threshold {
                        let details = format!(
                            "Pending order match: {:.4}% (entry: {:.5} within zone {:.5}-{:.5})",
                            match_score * 100.0, entry_price, pending_order.zone_low, pending_order.zone_high
                        );
                        
                        info!("🏆 MATCH FOUND! Score: {:.4}%, Details: {}", match_score * 100.0, details);
                        
                        if best_match.is_none() || match_score > best_match.as_ref().unwrap().1 {
                            best_match = Some((pending_order.zone_id.clone(), match_score, details));
                        }
                    } else {
                        info!("❌ Match score {:.4}% below threshold {:.4}%", match_score * 100.0, threshold * 100.0);
                    }
                } else {
                    info!("❌ Price not in range: {} not in [{:.8}, {:.8}]", entry_price, pending_order.zone_low, pending_order.zone_high);
                }
            } else {
                info!("❌ No match: symbol={} vs {}, timeframe={} vs {}, zone_type={} vs {}", 
                    pending_order.symbol, clean_symbol, 
                    pending_order.timeframe, clean_timeframe,
                    pending_order.zone_type, clean_zone_type);
            }
        }

        if let Some((zone_id, score, details)) = best_match {
            debug!("Trade-to-zone match found: {}", details);
            ZoneMatchResult {
                matched: true,
                cached_zone_id: Some(zone_id),
                match_score: score,
                match_details: details,
            }
        } else {
            ZoneMatchResult {
                matched: false,
                cached_zone_id: None,
                match_score: 0.0,
                match_details: format!("No zone contains entry price {:.5} for {}:{} {} zone", entry_price, clean_symbol, clean_timeframe, clean_zone_type),
            }
        }
    }

    /// Match a backtest zone with cached zones/pending orders
    pub fn match_zone(
        &self,
        symbol: &str,
        timeframe: &str,
        zone_type: &str,
        zone_high: f64,
        zone_low: f64,
        start_time: Option<&str>,
    ) -> ZoneMatchResult {
        let mut best_match: Option<(String, f64, String)> = None;
        let threshold = 0.95; // 95% similarity threshold

        // Clean inputs for comparison
        let clean_symbol = symbol.replace("_SB", "");
        let clean_timeframe = timeframe.to_lowercase();
        let clean_zone_type = if zone_type.contains("supply") { "supply_zone" } else { "demand_zone" };

        // Check cached zones first
        for cached_zone in &self.cached_zones {
            if cached_zone.symbol == clean_symbol 
                && cached_zone.timeframe == clean_timeframe 
                && cached_zone.zone_type == clean_zone_type {
                
                let match_score = self.calculate_zone_similarity(
                    zone_high, zone_low, start_time,
                    cached_zone.high, cached_zone.low, Some(&cached_zone.start_time)
                );

                if match_score > threshold {
                    let details = format!(
                        "Cached zone match: {:.4}% (high: {:.5} vs {:.5}, low: {:.5} vs {:.5})",
                        match_score * 100.0, zone_high, cached_zone.high, zone_low, cached_zone.low
                    );
                    
                    if best_match.is_none() || match_score > best_match.as_ref().unwrap().1 {
                        best_match = Some((cached_zone.id.clone(), match_score, details));
                    }
                }
            }
        }

        // Check pending orders
        for pending_order in &self.pending_orders {
            if pending_order.symbol == clean_symbol 
                && pending_order.timeframe == clean_timeframe 
                && pending_order.zone_type == clean_zone_type {
                
                let match_score = self.calculate_zone_similarity(
                    zone_high, zone_low, start_time,
                    pending_order.zone_high, pending_order.zone_low, None
                );

                if match_score > threshold {
                    let details = format!(
                        "Pending order match: {:.4}% (high: {:.5} vs {:.5}, low: {:.5} vs {:.5})",
                        match_score * 100.0, zone_high, pending_order.zone_high, zone_low, pending_order.zone_low
                    );
                    
                    if best_match.is_none() || match_score > best_match.as_ref().unwrap().1 {
                        best_match = Some((pending_order.zone_id.clone(), match_score, details));
                    }
                }
            }
        }

        if let Some((zone_id, score, details)) = best_match {
            debug!("Zone match found: {}", details);
            ZoneMatchResult {
                matched: true,
                cached_zone_id: Some(zone_id),
                match_score: score,
                match_details: details,
            }
        } else {
            ZoneMatchResult {
                matched: false,
                cached_zone_id: None,
                match_score: 0.0,
                match_details: format!("No match found for {}:{} {} zone", clean_symbol, clean_timeframe, clean_zone_type),
            }
        }
    }

    /// Calculate similarity between trade entry and zone (0.0 to 1.0)
    fn calculate_trade_to_zone_similarity(
        &self,
        entry_price: f64, entry_time: Option<&str>,
        zone_high: f64, zone_low: f64, zone_time: Option<&str>
    ) -> f64 {
        // Since entry price is already within zone boundaries, give high base score
        let price_score = 1.0; // Entry price is within zone, perfect match

        // Time similarity (20% weight) - allow more flexibility
        let time_similarity = match (entry_time, zone_time) {
            (Some(t1), Some(t2)) => {
                match (DateTime::parse_from_rfc3339(t1), DateTime::parse_from_rfc3339(t2)) {
                    (Ok(dt1), Ok(dt2)) => {
                        let time_diff = (dt1.timestamp() - dt2.timestamp()).abs();
                        let max_time_diff = 7 * 24 * 3600; // 1 week tolerance for trade reconciliation
                        if time_diff <= max_time_diff {
                            1.0 - (time_diff as f64 / max_time_diff as f64)
                        } else {
                            0.5 // Partial credit for different times
                        }
                    }
                    _ => 0.8, // Partial credit if time parsing fails
                }
            }
            _ => 0.9, // High partial credit if one time is missing (focus on price match)
        };

        // Weighted average: price match is most important for trade reconciliation
        (price_score * 0.8) + (time_similarity * 0.2)
    }

    /// Calculate similarity between two zones (0.0 to 1.0)
    fn calculate_zone_similarity(
        &self,
        high1: f64, low1: f64, time1: Option<&str>,
        high2: f64, low2: f64, time2: Option<&str>
    ) -> f64 {
        // Price similarity (most important - 80% weight)
        let price_tolerance = 0.0001; // 0.01% price tolerance
        let high_diff = (high1 - high2).abs() / high1.max(high2);
        let low_diff = (low1 - low2).abs() / low1.max(low2);
        let price_similarity = if high_diff <= price_tolerance && low_diff <= price_tolerance {
            1.0 - (high_diff + low_diff) / 2.0
        } else {
            0.0 // If price difference is too large, no match
        };

        // Time similarity (20% weight) - allow more flexibility
        let time_similarity = match (time1, time2) {
            (Some(t1), Some(t2)) => {
                match (DateTime::parse_from_rfc3339(t1), DateTime::parse_from_rfc3339(t2)) {
                    (Ok(dt1), Ok(dt2)) => {
                        let time_diff = (dt1.timestamp() - dt2.timestamp()).abs();
                        let max_time_diff = 3600; // 1 hour tolerance
                        if time_diff <= max_time_diff {
                            1.0 - (time_diff as f64 / max_time_diff as f64)
                        } else {
                            0.5 // Partial credit for different times
                        }
                    }
                    _ => 0.7, // Partial credit if time parsing fails
                }
            }
            _ => 0.8, // Partial credit if one time is missing
        };

        // Weighted average
        (price_similarity * 0.8) + (time_similarity * 0.2)
    }
}