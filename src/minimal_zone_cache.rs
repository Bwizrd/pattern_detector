// src/minimal_zone_cache.rs - Updated with dynamic date calculation
use log::{debug, error, info, warn};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};
use std::time::Instant; 
// Ensure these are correctly imported
use crate::types::EnrichedZone;
use crate::zone_detection::{InfluxDataFetcher, ZoneDetectionEngine, ZoneDetectionRequest};

#[derive(Debug, Clone, Serialize)]
pub struct CacheSymbolConfig {
    pub symbol: String,
    pub timeframes: Vec<String>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeNotification {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub timeframe: String,
    pub action: String, // BUY or SELL
    pub price: f64,
    pub zone_type: String,
    pub strength: f64,
    pub zone_id: String,
}
pub struct MinimalZoneCache {
    zones: HashMap<String, EnrichedZone>,
    monitored_symbols: Vec<CacheSymbolConfig>,
    zone_engine: ZoneDetectionEngine,
    data_fetcher: InfluxDataFetcher,
    trade_notifications: Vec<TradeNotification>,
    previous_zone_triggers: HashMap<String, bool>, // zone_id -> was_triggered
    current_prices: HashMap<String, f64>, // symbol -> price
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
             // NEW: Initialize the new fields
            trade_notifications: Vec::new(),
            previous_zone_triggers: HashMap::new(),
            current_prices: HashMap::new(),
        })
    }

    /// Calculate the same 90-day lookback period that the frontend component uses
    fn calculate_frontend_matching_dates() -> (String, String) {
        let now = Utc::now();
        let ninety_days_ago = now - Duration::days(90);
        
        // Format to match frontend: YYYY-MM-DDTHH:MM:SSZ
        let start_time = format!("{}T00:00:00Z", ninety_days_ago.format("%Y-%m-%d"));
        let end_time = format!("{}T23:59:59Z", now.format("%Y-%m-%d"));
        
        (start_time, end_time)
    }

     pub async fn refresh_zones(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let total_start_time = Instant::now();
        info!("🔄 [CACHE] Starting zone refresh with dynamic date calculation...");

        let mut all_zones_for_cache = HashMap::new();

        // --- CALCULATE DYNAMIC DATES TO MATCH FRONTEND ---
        let date_calc_start = Instant::now();
        let (start_time_dynamic, end_time_dynamic) = Self::calculate_frontend_matching_dates();
        let date_calc_duration = date_calc_start.elapsed();
        
        info!(
            "📅 [CACHE] USING DYNAMIC DATE RANGE (90 days back from today): {} to {} (calculated in {:.3}ms)",
            start_time_dynamic, end_time_dynamic, date_calc_duration.as_secs_f64() * 1000.0
        );

        let mut total_candles_fetched = 0;
        let mut total_zones_detected = 0;
        let mut successful_symbol_tf_combinations = 0;
        let mut failed_symbol_tf_combinations = 0;
        let mut total_fetch_time = std::time::Duration::ZERO;
        let mut total_detection_time = std::time::Duration::ZERO;

        for symbol_config in &self.monitored_symbols {
            for timeframe in &symbol_config.timeframes {
                let symbol_tf_start = Instant::now();
                let current_symbol_from_config = symbol_config.symbol.clone();
                let current_timeframe = timeframe.clone();

                debug!(
                    "📊 [CACHE] Processing {}/{}",
                    current_symbol_from_config, current_timeframe
                );

                // Apply _SB suffix for fetching (consistent with bulk handler)
                let core_symbol_for_fetch = if current_symbol_from_config.ends_with("_SB") {
                    current_symbol_from_config.clone()
                } else {
                    format!("{}_SB", current_symbol_from_config)
                };

                let fetch_start = Instant::now();
                let candles = match self
                    .data_fetcher
                    .fetch_candles(
                        &core_symbol_for_fetch,
                        &timeframe,
                        &start_time_dynamic,
                        &end_time_dynamic,
                    )
                    .await
                {
                    Ok(c) => {
                        let fetch_duration = fetch_start.elapsed();
                        total_fetch_time += fetch_duration;
                        debug!(
                            "🕯️ [CACHE] Fetched {} candles for {}/{} in {:.3}ms",
                            c.len(),
                            core_symbol_for_fetch,
                            current_timeframe,
                            fetch_duration.as_secs_f64() * 1000.0
                        );
                        c
                    }
                    Err(e) => {
                        failed_symbol_tf_combinations += 1;
                        warn!(
                            "⚠️ [CACHE] fetch_candles failed for {}/{}: {} (took {:.3}ms)",
                            core_symbol_for_fetch, current_timeframe, e,
                            fetch_start.elapsed().as_secs_f64() * 1000.0
                        );
                        continue;
                    }
                };

                if candles.is_empty() {
                    failed_symbol_tf_combinations += 1;
                    warn!(
                        "⚠️ [CACHE] No candles for {}/{}",
                        core_symbol_for_fetch, current_timeframe
                    );
                    continue;
                }

                total_candles_fetched += candles.len();

                let detection_start = Instant::now();
                let detection_request = ZoneDetectionRequest {
                    symbol: current_symbol_from_config.clone(),
                    timeframe: current_timeframe.clone(),
                    pattern: "fifty_percent_before_big_bar".to_string(),
                    candles,
                    max_touch_count: None,
                };

                match self.zone_engine.detect_zones(detection_request) {
                    Ok(result) => {
                        let detection_duration = detection_start.elapsed();
                        total_detection_time += detection_duration;
                        
                        let total_zones_found = result.supply_zones.len() + result.demand_zones.len();
                        total_zones_detected += total_zones_found;

                        debug!(
                            "🔍 [CACHE] {}/{}: Found {} zones ({} supply + {} demand) in {:.3}ms",
                            result.symbol,
                            result.timeframe,
                            total_zones_found,
                            result.supply_zones.len(),
                            result.demand_zones.len(),
                            detection_duration.as_secs_f64() * 1000.0
                        );

                        // Filter for active zones only
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

                        debug!(
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

                        successful_symbol_tf_combinations += 1;
                    }
                    Err(e) => {
                        failed_symbol_tf_combinations += 1;
                        warn!(
                            "⚠️ [CACHE] Zone detection failed for {}/{}: {} (took {:.3}ms)",
                            current_symbol_from_config, current_timeframe, e,
                            detection_start.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                }

                let symbol_tf_duration = symbol_tf_start.elapsed();
                debug!(
                    "⏱️ [CACHE] {}/{} total processing time: {:.3}ms",
                    current_symbol_from_config, current_timeframe,
                    symbol_tf_duration.as_secs_f64() * 1000.0
                );
            }
        }

        let total_zones = all_zones_for_cache.len();
        self.zones = all_zones_for_cache;

        let total_duration = total_start_time.elapsed();

        // COMPREHENSIVE PERFORMANCE SUMMARY
        info!("🏁 [CACHE] === PERFORMANCE SUMMARY ===");
        info!("📊 [CACHE] Total Duration: {:.2}s ({:.0}ms)", total_duration.as_secs_f64(), total_duration.as_secs_f64() * 1000.0);
        info!("📊 [CACHE] Date Range: {} to {}", start_time_dynamic, end_time_dynamic);
        info!("📊 [CACHE] Symbol/Timeframe Combinations:");
        info!("📊 [CACHE]   ✅ Successful: {}", successful_symbol_tf_combinations);
        info!("📊 [CACHE]   ❌ Failed: {}", failed_symbol_tf_combinations);
        info!("📊 [CACHE]   📈 Total: {}", successful_symbol_tf_combinations + failed_symbol_tf_combinations);
        info!("📊 [CACHE] Data Processing:");
        info!("📊 [CACHE]   🕯️  Total Candles Fetched: {}", total_candles_fetched);
        info!("📊 [CACHE]   🔍 Total Zones Detected: {}", total_zones_detected);
        info!("📊 [CACHE]   ✅ Active Zones in Cache: {}", total_zones);
        info!("📊 [CACHE] Timing Breakdown:");
        info!("📊 [CACHE]   📥 Total Fetch Time: {:.2}s ({:.1}%)", total_fetch_time.as_secs_f64(), (total_fetch_time.as_secs_f64() / total_duration.as_secs_f64()) * 100.0);
        info!("📊 [CACHE]   🔍 Total Detection Time: {:.2}s ({:.1}%)", total_detection_time.as_secs_f64(), (total_detection_time.as_secs_f64() / total_duration.as_secs_f64()) * 100.0);
        info!("📊 [CACHE]   ⚙️  Other Processing: {:.2}s ({:.1}%)", (total_duration - total_fetch_time - total_detection_time).as_secs_f64(), ((total_duration - total_fetch_time - total_detection_time).as_secs_f64() / total_duration.as_secs_f64()) * 100.0);
        info!("📊 [CACHE] Performance Metrics:");
        if successful_symbol_tf_combinations > 0 {
            info!("📊 [CACHE]   ⚡ Avg per Symbol/TF: {:.0}ms", (total_duration.as_secs_f64() * 1000.0) / successful_symbol_tf_combinations as f64);
            info!("📊 [CACHE]   📥 Avg Fetch per S/TF: {:.0}ms", (total_fetch_time.as_secs_f64() * 1000.0) / successful_symbol_tf_combinations as f64);
            info!("📊 [CACHE]   🔍 Avg Detection per S/TF: {:.0}ms", (total_detection_time.as_secs_f64() * 1000.0) / successful_symbol_tf_combinations as f64);
        }
        if total_candles_fetched > 0 {
            info!("📊 [CACHE]   🕯️  Candles per Second: {:.0}", total_candles_fetched as f64 / total_duration.as_secs_f64());
        }
        if total_zones_detected > 0 {
            info!("📊 [CACHE]   🔍 Zones per Second: {:.1}", total_zones_detected as f64 / total_duration.as_secs_f64());
            info!("📊 [CACHE]   📈 Active Zone Ratio: {:.1}%", (total_zones as f64 / total_zones_detected as f64) * 100.0);
        }
        info!("📊 [CACHE] === END SUMMARY ===");
        
        // Optionally write debug file with timing info
        if let Err(e) = self.write_zones_to_file(&start_time_dynamic, &end_time_dynamic, &total_duration).await {
            warn!("⚠️ [CACHE] Failed to write debug file: {}", e);
        }
        
        Ok(())
    }


    // Updated to include date range in debug output
    async fn write_zones_to_file(
        &self, 
        start_time: &str, 
        end_time: &str,
        processing_duration: &std::time::Duration
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::fs::OpenOptions;
        use std::io::Write;

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("cache_zones_debug_dynamic.json")?;

        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
        writeln!(
            file,
            "# Cache Zones Debug (DYNAMIC DATES) - Generated at {}",
            timestamp
        )?;
        writeln!(file, "# Date range used: {} to {}", start_time, end_time)?;
        writeln!(file, "# Processing time: {:.2}s", processing_duration.as_secs_f64())?;
        writeln!(file, "# Total zones: {}", self.zones.len())?;
        writeln!(file, "")?;

        let zones_json = serde_json::to_string_pretty(&self.zones.values().collect::<Vec<_>>())?;
        writeln!(file, "{}", zones_json)?;

        info!(
            "📝 [CACHE] Wrote {} zones to cache_zones_debug_dynamic.json (range: {} to {}, processed in {:.2}s)",
            self.zones.len(), start_time, end_time, processing_duration.as_secs_f64()
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

    /// Get the current date range being used by the cache
    pub fn get_current_date_range() -> (String, String) {
        Self::calculate_frontend_matching_dates()
    }
    pub fn update_price_and_check_triggers(
        &mut self, 
        symbol: &str, 
        price: f64
    ) -> Vec<TradeNotification> {
        // Update current price
        self.current_prices.insert(symbol.to_string(), price);
        
        let mut new_notifications = Vec::new();
        
        // Check all zones for this symbol
        for (zone_id, zone) in &self.zones {
            // Only check zones for this symbol
            if zone.symbol.as_deref() != Some(symbol) {
                continue;
            }
            
            // Only check active zones
            if !zone.is_active {
                continue;
            }
            
            // Check if this zone is now triggered (proximal line touched)
            let is_triggered = self.check_zone_trigger(zone, price);
            let was_previously_triggered = self.previous_zone_triggers
                .get(zone_id)
                .copied()
                .unwrap_or(false);
            
            // New trigger detected!
            if is_triggered && !was_previously_triggered {
                info!("🎯 [CACHE] Zone trigger detected: {} at price {:.5}", zone_id, price);
                
                let notification = TradeNotification {
                    timestamp: Utc::now(),
                    symbol: symbol.to_string(),
                    timeframe: zone.timeframe.clone().unwrap_or_default(),
                    action: if zone.zone_type.as_deref().unwrap_or("").contains("supply") { 
                        "SELL".to_string() 
                    } else { 
                        "BUY".to_string() 
                    },
                    price,
                    zone_type: zone.zone_type.clone().unwrap_or_default(),
                    strength: zone.strength_score.unwrap_or(0.0),
                    zone_id: zone_id.clone(),
                };
                
                new_notifications.push(notification.clone());
                self.trade_notifications.insert(0, notification);
                
                // Keep only last 50 notifications
                if self.trade_notifications.len() > 50 {
                    self.trade_notifications.truncate(50);
                }
            }
            
            // Update trigger state
            self.previous_zone_triggers.insert(zone_id.clone(), is_triggered);
        }
        
        new_notifications
    }
    
    // 6. Add this helper method to detect proximal line touches
    fn check_zone_trigger(&self, zone: &EnrichedZone, current_price: f64) -> bool {
        let zone_high = zone.zone_high.unwrap_or(0.0);
        let zone_low = zone.zone_low.unwrap_or(0.0);
        let is_supply = zone.zone_type.as_deref().unwrap_or("").contains("supply");
        
        // Define proximity threshold - when we consider price "at" the proximal line
        let zone_height = zone_high - zone_low;
        let pip_threshold: f64 = 0.0002; // 2 pips for most pairs
        let percent_threshold: f64 = zone_height * 0.02; // 2% of zone height
        let proximity_threshold = pip_threshold.min(percent_threshold);
        
        if is_supply {
            // Supply zone: trigger when price touches proximal line (zone_low)
            (current_price - zone_low).abs() <= proximity_threshold
        } else {
            // Demand zone: trigger when price touches proximal line (zone_high)  
            (current_price - zone_high).abs() <= proximity_threshold
        }
    }
    
    // 7. Add getter methods for the dashboard
    pub fn get_trade_notifications(&self) -> &Vec<TradeNotification> {
        &self.trade_notifications
    }
    
    pub fn clear_trade_notifications(&mut self) {
        self.trade_notifications.clear();
    }
    
    pub fn get_current_prices(&self) -> &HashMap<String, f64> {
        &self.current_prices
    }
}

// Update the test_minimal_cache() function to also use dynamic dates
pub async fn get_minimal_cache() -> Result<MinimalZoneCache, Box<dyn std::error::Error + Send + Sync>> {
    info!("🧪 [CACHE] Creating minimal zone cache with dynamic dates (matching frontend 90-day calculation)...");

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
    let (current_start, current_end) = MinimalZoneCache::get_current_date_range();
    
    info!(
        "📊 [CACHE] Created cache with dynamic dates ({} to {}): {} total ({} supply, {} demand)",
        current_start, current_end, total, supply, demand
    );

    Ok(cache)
}