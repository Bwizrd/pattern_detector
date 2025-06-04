// zone_fetcher.rs
use crate::types::ZoneData;
use crate::config::AnalysisConfig;
use log::{info, warn};
use reqwest::Client;

pub struct ZoneFetcher {
    client: Client,
}

impl ZoneFetcher {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn fetch_zones(&self, config: &AnalysisConfig) -> Result<Vec<ZoneData>, Box<dyn std::error::Error>> {
        info!("📊 Fetching zones from main application...");
        
        let url = format!("{}/debug/minimal-cache-zones", config.app_url);
        let response = self.client.get(&url).send().await?;
            
        if !response.status().is_success() {
            return Err(format!("Failed to fetch zones: HTTP {}", response.status()).into());
        }
        
        let zones_json: serde_json::Value = response.json().await?;
        
        let zones_array = zones_json.get("retrieved_zones")
            .and_then(|v| v.as_array())
            .ok_or("Expected 'retrieved_zones' field containing an array")?;
            
        let mut filtered_zones = Vec::new();
        
        for zone_value in zones_array {
            if let Ok(zone) = self.parse_zone_from_json(zone_value) {
                if self.is_timeframe_eligible(&zone.timeframe) && zone.is_active {
                    filtered_zones.push(zone);
                }
            }
        }
        
        info!("✅ Loaded {} eligible zones", filtered_zones.len());
        Ok(filtered_zones)
    }

    fn parse_zone_from_json(&self, zone_json: &serde_json::Value) -> Result<ZoneData, Box<dyn std::error::Error>> {
        let zone_id = zone_json.get("zone_id").and_then(|v| v.as_str()).ok_or("Missing zone_id")?.to_string();
        let symbol = zone_json.get("symbol").and_then(|v| v.as_str()).ok_or("Missing symbol")?.to_string();
        let timeframe = zone_json.get("timeframe").and_then(|v| v.as_str()).ok_or("Missing timeframe")?.to_string();
        let zone_high = zone_json.get("zone_high").and_then(|v| v.as_f64()).ok_or("Missing zone_high")?;
        let zone_low = zone_json.get("zone_low").and_then(|v| v.as_f64()).ok_or("Missing zone_low")?;
        let zone_type = zone_json.get("type").or_else(|| zone_json.get("zone_type")).and_then(|v| v.as_str()).ok_or("Missing type")?.to_string();
        let strength_score = zone_json.get("strength_score").and_then(|v| v.as_f64()).unwrap_or(50.0);
        let is_active = zone_json.get("is_active").and_then(|v| v.as_bool()).unwrap_or(false);
        let touch_count = zone_json.get("touch_count").and_then(|v| v.as_i64()).map(|v| v as i32); // Added touch_count parsing
            
        Ok(ZoneData { zone_id, symbol, timeframe, zone_high, zone_low, zone_type, strength_score, is_active, touch_count })
    }
    
    fn is_timeframe_eligible(&self, timeframe: &str) -> bool {
        matches!(timeframe, "30m" | "1h" | "4h" | "1d")
    }
}