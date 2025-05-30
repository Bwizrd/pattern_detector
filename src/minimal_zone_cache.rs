// src/minimal_zone_cache.rs
use log::{debug, error, info, warn};
use serde::Serialize;
use std::collections::HashMap;
// Ensure these are correctly imported
use crate::types::EnrichedZone;
use crate::zone_detection::{InfluxDataFetcher, ZoneDetectionEngine, ZoneDetectionRequest};
// CandleData might be needed if you pass it around, but ZoneDetectionRequest takes it
// use crate::detect::CandleData;

#[derive(Debug, Clone, Serialize)]
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
    pub fn new(
        monitored_symbols: Vec<CacheSymbolConfig>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            zones: HashMap::new(),
            monitored_symbols,
            zone_engine: ZoneDetectionEngine::new(),
            data_fetcher: InfluxDataFetcher::new()?,
        })
    }

    pub async fn refresh_zones(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🔄 [CACHE] Starting zone refresh...");

        let mut all_zones_for_cache = HashMap::new();

        // --- MATCH FRONTEND REQUEST DATES ---
        let start_time_to_match_frontend = "2025-03-01T00:00:00Z";
        let end_time_to_match_frontend = "2025-05-30T23:59:59Z";
        
        info!(
            "📅 [CACHE] USING FRONTEND-MATCHING DATE RANGE: {} to {}",
            start_time_to_match_frontend, end_time_to_match_frontend
        );

        for symbol_config in &self.monitored_symbols {
            for timeframe in &symbol_config.timeframes {
                let current_symbol_from_config = symbol_config.symbol.clone();
                let current_timeframe = timeframe.clone();

                info!(
                    "📊 [CACHE] Processing {}/{}",
                    current_symbol_from_config, current_timeframe
                );

                // Apply _SB suffix for fetching (consistent with bulk handler)
                let core_symbol_for_fetch = if current_symbol_from_config.ends_with("_SB") {
                    current_symbol_from_config.clone()
                } else {
                    format!("{}_SB", current_symbol_from_config)
                };

                let candles = match self
                    .data_fetcher
                    .fetch_candles(
                        &core_symbol_for_fetch,
                        &timeframe,
                        start_time_to_match_frontend, // NOW MATCHES FRONTEND
                        end_time_to_match_frontend,   // NOW MATCHES FRONTEND
                    )
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(
                            "⚠️ [CACHE] fetch_candles failed for {}/{}: {}",
                            core_symbol_for_fetch, current_timeframe, e
                        );
                        continue;
                    }
                };

                if candles.is_empty() {
                    warn!(
                        "⚠️ [CACHE] No candles for {}/{}",
                        core_symbol_for_fetch, current_timeframe
                    );
                    continue;
                }

                info!(
                    "🕯️ [CACHE] Got {} candles for {}/{}",
                    candles.len(),
                    core_symbol_for_fetch,
                    current_timeframe
                );

                let detection_request = ZoneDetectionRequest {
                    symbol: current_symbol_from_config.clone(), // Use non-SB version for zone.symbol field
                    timeframe: current_timeframe.clone(),
                    pattern: "fifty_percent_before_big_bar".to_string(),
                    candles,
                    max_touch_count: None,
                };

                match self.zone_engine.detect_zones(detection_request) {
                    Ok(result) => {
                        info!(
                            "🔍 [CACHE] {}/{}: Found {} supply + {} demand zones",
                            result.symbol,
                            result.timeframe,
                            result.supply_zones.len(),
                            result.demand_zones.len()
                        );

                        // Filter for active zones only (to match bulk handler behavior)
                        let active_supply_zones = result
                            .supply_zones
                            .into_iter()
                            .filter(|zone| zone.is_active)
                            .collect::<Vec<_>>();
                        let active_demand_zones = result
                            .demand_zones
                            .into_iter()
                            .filter(|zone| zone.is_active)
                            .collect::<Vec<_>>();

                        info!(
                            "✅ [CACHE] {}/{}: Active zones: {} supply, {} demand",
                            result.symbol,
                            result.timeframe,
                            active_supply_zones.len(),
                            active_demand_zones.len()
                        );

                        // Add active zones to cache
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
                        warn!(
                            "⚠️ [CACHE] Zone detection failed for {}/{}: {}",
                            current_symbol_from_config, current_timeframe, e
                        );
                    }
                }
            }
        }

        let total_zones = all_zones_for_cache.len();
        self.zones = all_zones_for_cache;

        info!("✅ [CACHE] Zone refresh complete with frontend-matching dates. Total active zones: {}", total_zones);
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
        writeln!(
            file,
            "# Cache Zones Debug (HARCODED TEST) - Generated at {}",
            timestamp
        )?;
        writeln!(file, "# Total zones: {}", self.zones.len())?;
        writeln!(file, "")?;

        let zones_json = serde_json::to_string_pretty(&self.zones.values().collect::<Vec<_>>())?;
        writeln!(file, "{}", zones_json)?;

        info!(
            "📝 [CACHE] Wrote {} zones to cache_zones_debug_hardcoded_test.json",
            self.zones.len()
        );
        Ok(())
    }

    pub fn get_all_zones(&self) -> Vec<EnrichedZone> {
        self.zones.values().cloned().collect()
    }

    pub fn get_stats(&self) -> (usize, usize, usize) {
        let supply_count = self
            .zones
            .values()
            .filter(|z| {
                z.zone_type
                    .as_ref()
                    .map_or(false, |zt| zt.contains("supply"))
            })
            .count();
        let demand_count = self
            .zones
            .values()
            .filter(|z| {
                z.zone_type
                    .as_ref()
                    .map_or(false, |zt| zt.contains("demand"))
            })
            .count();

        (self.zones.len(), supply_count, demand_count)
    }
}

// test_minimal_cache function remains the same
// Update the test_minimal_cache() function in minimal_zone_cache.rs:

pub async fn get_minimal_cache() -> Result<MinimalZoneCache, Box<dyn std::error::Error + Send + Sync>> {
    info!("🧪 [CACHE] Creating minimal zone cache with full symbol list...");

    // THE SINGLE SOURCE OF TRUTH - all symbols and timeframes
    let symbols = vec![
        CacheSymbolConfig {
            symbol: "EURUSD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "GBPUSD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "USDJPY".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "USDCHF".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "AUDUSD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "USDCAD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "NZDUSD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "EURGBP".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "EURJPY".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "EURCHF".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "EURAUD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "EURCAD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "EURNZD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "GBPJPY".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "GBPCHF".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "GBPAUD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "GBPCAD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "GBPNZD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "AUDJPY".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "AUDNZD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "AUDCAD".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "NZDJPY".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "CADJPY".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "CHFJPY".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "NAS100".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
        CacheSymbolConfig {
            symbol: "US500".to_string(),
            timeframes: vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()],
        },
    ];

    let mut cache = MinimalZoneCache::new(symbols)?;
    cache.refresh_zones().await?;

    let (total, supply, demand) = cache.get_stats();
    info!(
        "📊 [CACHE] Created cache: {} total ({} supply, {} demand)",
        total, supply, demand
    );

    Ok(cache)
}
