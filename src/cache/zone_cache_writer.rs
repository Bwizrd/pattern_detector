// src/cache/zone_cache_writer.rs
// Simple fix to include existing timeframe and touch_count fields

use std::collections::HashMap;
use std::io::Write;
use serde::{Deserialize, Serialize};
use log::{debug, warn};

// Import your actual zone type
use crate::types::EnrichedZone;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleZone {
    pub id: String,
    pub symbol: String,
    pub zone_type: String,
    pub high: f64,
    pub low: f64,
    pub strength: f64,
    pub timeframe: String,        // Now using actual field
    pub touch_count: i32,         // Now using actual field
    pub created_at: chrono::DateTime<chrono::Utc>, // Keep for backward compatibility
    pub start_time: String,       // Zone creation time - always present with fallback
    pub bars_active: u64,         // Number of bars since zone creation - default to 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneCache {
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub zones: HashMap<String, Vec<SimpleZone>>,
    pub total_zones: usize,
}

pub struct ZoneCacheWriter {
    file_path: String,
}

impl ZoneCacheWriter {
    pub fn new(file_path: String) -> Self {
        Self { file_path }
    }

    /// Convert EnrichedZone to SimpleZone for JSON export
    fn convert_enriched_zone(zone: &EnrichedZone) -> SimpleZone {
        let now = chrono::Utc::now();
        
        // Debug logging to see what we're getting from the enriched zone
        let zone_id = zone.zone_id.as_deref().unwrap_or("unknown");
        debug!("Converting zone {} - start_time: {:?}, bars_active: {:?}", 
               zone_id, zone.start_time, zone.bars_active);
        
        // Also print to file for debugging
        let debug_msg = format!("Converting zone {} - start_time: {:?}, bars_active: {:?}\n", 
                               zone_id, zone.start_time, zone.bars_active);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("zone_conversion_debug.log")
            .and_then(|mut f| std::io::Write::write_all(&mut f, debug_msg.as_bytes()));
        
        SimpleZone {
            id: zone.zone_id.clone().unwrap_or_else(|| "unknown".to_string()),
            symbol: zone.symbol.clone().unwrap_or_else(|| "UNKNOWN".to_string()),
            zone_type: zone.zone_type.clone().unwrap_or_else(|| "unknown".to_string()),
            high: zone.zone_high.unwrap_or(0.0),
            low: zone.zone_low.unwrap_or(0.0),
            strength: zone.strength_score.unwrap_or(0.0),
            timeframe: zone.timeframe.clone().unwrap_or_else(|| "unknown".to_string()),  // Use actual field
            touch_count: zone.touch_count.unwrap_or(0) as i32,  // Use actual field
            created_at: now, // Keep for backward compatibility
            // Use the actual start_time from zone detection, fallback to current time if missing
            start_time: zone.start_time.clone().unwrap_or_else(|| {
                warn!("Zone {} missing start_time, using current time as fallback", 
                      zone.zone_id.as_deref().unwrap_or("unknown"));
                now.to_rfc3339()
            }),
            // Use the actual bars_active from zone detection, fallback to 0 if missing
            bars_active: zone.bars_active.unwrap_or_else(|| {
                warn!("Zone {} missing bars_active, using 0 as fallback", 
                      zone.zone_id.as_deref().unwrap_or("unknown"));
                0
            }),
        }
    }

    /// Write zones from MinimalZoneCache to JSON file
    pub async fn write_zones_from_cache(&self, zone_cache: &crate::cache::minimal_zone_cache::MinimalZoneCache) -> Result<(), Box<dyn std::error::Error>> {
        // Get all zones using your existing method
        let all_zones = zone_cache.get_all_zones();
        
        // Convert and group by symbol
        let mut zones_by_symbol: HashMap<String, Vec<SimpleZone>> = HashMap::new();
        
        for enriched_zone in all_zones {
            let simple_zone = Self::convert_enriched_zone(&enriched_zone);
            
            zones_by_symbol
                .entry(simple_zone.symbol.clone())
                .or_insert_with(Vec::new)
                .push(simple_zone);
        }
        
        let total_zones = zones_by_symbol.values().map(|zones| zones.len()).sum();
        
        let cache = ZoneCache {
            last_updated: chrono::Utc::now(),
            zones: zones_by_symbol,
            total_zones,
        };

        // Write to JSON file atomically
        let json_content = serde_json::to_string_pretty(&cache)?;
        let temp_path = format!("{}.tmp", self.file_path);
        tokio::fs::write(&temp_path, json_content).await?;
        tokio::fs::rename(&temp_path, &self.file_path).await?;
        
        debug!("Wrote {} zones to cache file: {}", total_zones, self.file_path);
        Ok(())
    }
}