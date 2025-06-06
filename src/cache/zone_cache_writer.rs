// src/cache/zone_cache_writer.rs
// Simple helper to write zones to JSON file for the monitor to read

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tracing::debug;

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
    pub created_at: chrono::DateTime<chrono::Utc>,
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
        SimpleZone {
            id: zone.zone_id.clone().unwrap_or_else(|| "unknown".to_string()),
            symbol: zone.symbol.clone().unwrap_or_else(|| "UNKNOWN".to_string()),
            zone_type: zone.zone_type.clone().unwrap_or_else(|| "unknown".to_string()),
            high: zone.zone_high.unwrap_or(0.0),
            low: zone.zone_low.unwrap_or(0.0),
            strength: zone.strength_score.unwrap_or(0.0),
            created_at: chrono::Utc::now(), // Use current time since we don't have detection_timestamp
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