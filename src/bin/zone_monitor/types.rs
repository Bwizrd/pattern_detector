// src/bin/zone_monitor/types.rs
// Data structures for the zone monitor

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    pub id: String,
    pub symbol: String,
    pub zone_type: String, // "supply" or "demand"
    pub high: f64,
    pub low: f64,
    pub strength: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneCache {
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub zones: HashMap<String, Vec<Zone>>, // symbol -> zones
    pub total_zones: usize,
}

impl Default for ZoneCache {
    fn default() -> Self {
        Self {
            last_updated: chrono::Utc::now(),
            zones: HashMap::new(),
            total_zones: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceUpdate {
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneAlert {
    pub zone_id: String,
    pub symbol: String,
    pub zone_type: String,
    pub current_price: f64,
    pub zone_high: f64,
    pub zone_low: f64,
    pub distance_pips: f64,
    pub strength: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}