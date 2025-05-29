// src/minimal_zone_cache.rs
use std::collections::HashMap;
use log::{info, warn, error, debug};
// Ensure these are correctly imported
use crate::types::EnrichedZone;
use crate::zone_detection::{ZoneDetectionEngine, ZoneDetectionRequest, InfluxDataFetcher};
// CandleData might be needed if you pass it around, but ZoneDetectionRequest takes it
// use crate::detect::CandleData;


#[derive(Debug, Clone)]
pub struct CacheSymbolConfig {
    pub symbol: String,
    pub timeframes: Vec<String>,
}

pub struct MinimalZoneCache {
    zones: HashMap<String, EnrichedZone>,
    monitored_symbols: Vec<CacheSymbolConfig>,
    zone_engine: ZoneDetectionEngine,
    data_fetcher: InfluxDataFetcher,
}

impl MinimalZoneCache {
    pub fn new(monitored_symbols: Vec<CacheSymbolConfig>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            zones: HashMap::new(),
            monitored_symbols,
            zone_engine: ZoneDetectionEngine::new(),
            data_fetcher: InfluxDataFetcher::new()?,
        })
    }

    pub async fn refresh_zones(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🔄 [CACHE] Starting zone refresh...");
        
        let mut all_zones_for_cache = HashMap::new(); // Renamed to avoid confusion

        // --- HARDCODED DATES FOR 100% IDENTICAL RESPONSE TEST ---
        let start_time_hardcoded = "2025-05-22T00:00:00Z";
        let end_time_hardcoded = "2025-05-29T23:59:59Z";
        info!("⚠️ [CACHE] USING HARDCODED DATE RANGE FOR TEST: {} to {}", start_time_hardcoded, end_time_hardcoded);

        // --- Original date logic (commented out for this test) ---
        // let start_time_default = "-7d";
        // let end_time_default = "now()";
        
        for symbol_config in &self.monitored_symbols {
            for timeframe in &symbol_config.timeframes {
                let current_symbol_from_config = symbol_config.symbol.clone(); // e.g., "EURUSD"
                let current_timeframe = timeframe.clone();
                
                info!("📊 [CACHE] Processing {}/{}", current_symbol_from_config, current_timeframe);
                
                // Apply _SB suffix for fetching, consistent with bulk handler's internal logic
                let core_symbol_for_fetch = if current_symbol_from_config.ends_with("_SB") {
                    current_symbol_from_config.clone()
                } else {
                    format!("{}_SB", current_symbol_from_config)
                };
                
                info!("📊 [CACHE] Fetching data for core_symbol: {} (from config: {})", core_symbol_for_fetch, current_symbol_from_config);
                
                let candles = match self.data_fetcher.fetch_candles(
                    &core_symbol_for_fetch, // Use suffixed symbol for fetching
                    &timeframe,
                    start_time_hardcoded,   // Using hardcoded start time
                    end_time_hardcoded,     // Using hardcoded end time
                    // start_time_default,  // Original
                    // end_time_default,    // Original
                ).await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("⚠️ [CACHE] fetch_candles failed for {}/{}: {}", core_symbol_for_fetch, current_timeframe, e);
                        continue;
                    }
                };
                
                if candles.is_empty() {
                    warn!("⚠️ [CACHE] No candles for {}/{}", core_symbol_for_fetch, current_timeframe);
                    continue;
                }
                
                info!("🕯️ [CACHE] Got {} candles for {}/{}", candles.len(), core_symbol_for_fetch, current_timeframe);
                
                // For ZoneDetectionRequest, the `symbol` field should be what you want in the EnrichedZone's `symbol` field.
                // The bulk handler ultimately sets `EnrichedZone.symbol` to the `core_symbol` (e.g. "EURUSD_SB")
                // if `ZoneDetectionRequest.symbol` was `core_symbol`.
                // Let's ensure `EnrichedZone.symbol` becomes "EURUSD" (without _SB) if the bulk handler's response has it that way.
                // The example response has "EURUSD". ZoneDetectionEngine uses the `request.symbol` for `result.symbol`.
                // So, `detection_request.symbol` should be "EURUSD" (non-SB version)
                // if we want `EnrichedZone.symbol` to be "EURUSD".
                // However, your `ZoneDetectionEngine::enrich_zones_with_activity` has:
                // `if z.symbol.is_none() { z.symbol = Some(symbol.to_string()); }`
                // where `symbol` is passed from `self.extract_and_enrich_zones`'s `symbol` param,
                // which comes from `ZoneDetectionEngine::detect_zones`'s `request.symbol`.
                // If `MinimalZoneCache` passes `core_symbol_for_fetch` ("EURUSD_SB") to `detect_zones`, then `EnrichedZone.symbol` will be "EURUSD_SB".
                // Let's align `EnrichedZone.symbol` to be the non-SB version for consistency with the desired output.
                // The easiest is to pass the non-SB symbol to `ZoneDetectionRequest` and let the `EnrichedZone` pick it up.
                // The example response shows "EURUSD" in EnrichedZone.symbol, so the request should reflect that.

                let detection_request = ZoneDetectionRequest {
                    symbol: current_symbol_from_config.clone(), // Use "EURUSD" (non-SB)
                    timeframe: current_timeframe.clone(),
                    pattern: "fifty_percent_before_big_bar".to_string(), // Matches Postman
                    candles,
                    max_touch_count: None, // Matches Postman implied behavior (no filter)
                };
                
                match self.zone_engine.detect_zones(detection_request) {
                    Ok(mut result) => { // `result` is ZoneDetectionResult
                        // The `result.symbol` and `result.timeframe` will be `current_symbol_from_config` and `current_timeframe`.
                        // The `EnrichedZone` objects within `result.supply_zones` and `result.demand_zones`
                        // should also have their `symbol` and `timeframe` fields correctly set by the `ZoneDetectionEngine`.
                        // We need to verify `EnrichedZone.symbol` is "EURUSD" not "EURUSD_SB".
                        // If `ZoneDetectionEngine`'s `enrich_zones_with_activity` takes `symbol` as param (which is request.symbol)
                        // and sets `enriched_zone.symbol = Some(symbol.to_string())`, then it should be correct.
                        // Let's double check `EnrichedZone`'s symbol field. Your `EnrichedZone` has `symbol: Option<String>`.
                        // In `ZoneDetectionEngine::enrich_zones_with_activity`:
                        // `if z.symbol.is_none() { z.symbol = Some(symbol.to_string()); }`
                        // The `symbol` param here is `request.symbol` from `detect_zones`.
                        // So if `ZoneDetectionRequest.symbol` is "EURUSD", `EnrichedZone.symbol` will be "EURUSD". This is correct.


                        info!("🔍 [CACHE] {}/{}: Found {} supply + {} demand zones by engine",
                              result.symbol, result.timeframe,
                              result.supply_zones.len(), result.demand_zones.len());
                        
                        let active_supply_zones = result.supply_zones.into_iter()
                            .filter(|zone| zone.is_active)
                            .collect::<Vec<_>>();
                        let active_demand_zones = result.demand_zones.into_iter()
                            .filter(|zone| zone.is_active)
                            .collect::<Vec<_>>();
                        
                        info!("✅ [CACHE] {}/{}: Active zones: {} supply, {} demand", 
                              result.symbol, result.timeframe,
                              active_supply_zones.len(), active_demand_zones.len());
                        
                        for zone in active_supply_zones {
                            if let Some(zone_id) = &zone.zone_id {
                                all_zones_for_cache.insert(zone_id.clone(), zone);
                            }
                        }
                        for zone in active_demand_zones {
                            if let Some(zone_id) = &zone.zone_id {
                                all_zones_for_cache.insert(zone_id.clone(), zone);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("⚠️ [CACHE] Zone detection failed for {}/{}: {}", current_symbol_from_config, current_timeframe, e);
                    }
                }
            }
        }
        
        let total_zones = all_zones_for_cache.len();
        self.zones = all_zones_for_cache;
        
        // You might want to disable this file writing during the 100% test or make filename unique
        // self.write_zones_to_file().await?; 
        
        info!("✅ [CACHE] Zone refresh complete with hardcoded dates. Total active zones in cache: {}", total_zones);
        Ok(())
    }
    
    // Add this helper if you want to keep writing to file, otherwise comment out call above
    async fn write_zones_to_file(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::fs::OpenOptions;
        use std::io::Write;
        
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("cache_zones_debug_hardcoded_test.json")?; // Different filename for this test
        
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
        writeln!(file, "# Cache Zones Debug (HARCODED TEST) - Generated at {}", timestamp)?;
        writeln!(file, "# Total zones: {}", self.zones.len())?;
        writeln!(file, "")?;
        
        let zones_json = serde_json::to_string_pretty(&self.zones.values().collect::<Vec<_>>())?;
        writeln!(file, "{}", zones_json)?;
        
        info!("📝 [CACHE] Wrote {} zones to cache_zones_debug_hardcoded_test.json", self.zones.len());
        Ok(())
    }
    
    pub fn get_all_zones(&self) -> Vec<EnrichedZone> {
        self.zones.values().cloned().collect()
    }
    
    pub fn get_stats(&self) -> (usize, usize, usize) {
        let supply_count = self.zones.values()
            .filter(|z| z.zone_type.as_ref().map_or(false, |zt| zt.contains("supply")))
            .count();
        let demand_count = self.zones.values()
            .filter(|z| z.zone_type.as_ref().map_or(false, |zt| zt.contains("demand")))
            .count();
        
        (self.zones.len(), supply_count, demand_count)
    }
}

// test_minimal_cache function remains the same
pub async fn test_minimal_cache() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("🧪 [TEST] Testing minimal zone cache (hardcoded date test in refresh_zones)...");
    
    let symbols = vec![
        CacheSymbolConfig {
            symbol: "EURUSD".to_string(), // This should match the symbol used in Postman request
            timeframes: vec!["1h".to_string(), "4h".to_string()],
        },
    ];
    
    let mut cache = MinimalZoneCache::new(symbols)?;
    cache.refresh_zones().await?;
    
    let (total, supply, demand) = cache.get_stats();
    info!("📊 [TEST] Cache stats: {} total ({} supply, {} demand)", total, supply, demand);
    
    let all_zones_from_cache = cache.get_all_zones();
    info!("📋 [TEST] Retrieved {} zones from cache", all_zones_from_cache.len());
    
    if let Some(zone) = all_zones_from_cache.first() {
        info!("🎯 [TEST] First cached zone example: ID: {:?}, Symbol: {:?}, TF: {:?}, StartTime: {:?}, High: {:?}, Low: {:?}",
            zone.zone_id, zone.symbol, zone.timeframe, zone.start_time, zone.zone_high, zone.zone_low);
    }
    Ok(())
}