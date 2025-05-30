// src/realtime_monitor.rs

use std::time::SystemTime;
use tokio::sync::{broadcast, mpsc};
// use serde::{Serialize, Deserialize}; // Deserialize might not be needed at this level
use serde::Serialize; // Serialize is used for ZoneEvent and ZoneEventType
use log::{info, warn, error, debug};

use crate::zone_detection::{ZoneDetectionEngine, ZoneDetectionRequest, InfluxDataFetcher};
use crate::types::EnrichedZone;
use crate::price_feed::PriceUpdate;

#[derive(Debug, Clone, Serialize)]
pub struct ZoneEvent {
    pub event_type: ZoneEventType,
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub price: f64,
    pub timestamp: u64,
    pub zone_high: f64,
    pub zone_low: f64,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneEventType {
    Touch,
    Breakout,
    Entry,
    Retest,
}

#[derive(Debug, Clone)] // Added Clone for MonitorConfig
pub struct SymbolTimeframeConfig {
    pub symbol: String,
    pub timeframes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub refresh_interval_secs: u64,
    pub price_tolerance_pips: f64,
    pub max_events_per_second: usize,
    pub monitored_symbols: Vec<SymbolTimeframeConfig>,
}

pub struct ZoneCache {
    zones: std::collections::HashMap<String, EnrichedZone>,
    last_refresh: SystemTime,
}

impl ZoneCache {
    pub fn new() -> Self {
        Self {
            zones: std::collections::HashMap::new(),
            last_refresh: SystemTime::now(),
        }
    }
    
    pub fn update_zones(&mut self, zones: Vec<EnrichedZone>) {
        self.zones.clear();
        for zone in zones {
            if let Some(zone_id) = &zone.zone_id {
                self.zones.insert(zone_id.clone(), zone);
            }
        }
        self.last_refresh = SystemTime::now();
        info!("🔄 [RT_MONITOR_CACHE] Updated with {} zones", self.zones.len());
    }
    
    pub fn get_zones_for_symbol(&self, symbol: &str) -> Vec<EnrichedZone> {
        self.zones.values()
            .filter(|zone| zone.symbol.as_deref() == Some(symbol))
            .cloned()
            .collect()
    }
    
    pub fn get_stats(&self) -> (usize, SystemTime) {
        (self.zones.len(), self.last_refresh)
    }
}

pub struct RealTimeZoneMonitor {
    cache: ZoneCache,
    price_cache: std::collections::HashMap<u32, f64>, // Assuming symbol_id from PriceUpdate is u32
    zone_engine: ZoneDetectionEngine,
    data_fetcher: InfluxDataFetcher,
    event_sender: broadcast::Sender<ZoneEvent>,
    price_receiver: mpsc::Receiver<PriceUpdate>,
    config: MonitorConfig,
    // To manage the refresh task
    // We'll pass a clone of config for the refresh task to use
    // refresh_task_config: MonitorConfig, // No longer needed if refresh_zones takes config directly
}

impl RealTimeZoneMonitor {
    pub fn new(
        config: MonitorConfig,
        price_receiver: mpsc::Receiver<PriceUpdate>,
    ) -> Result<(Self, broadcast::Receiver<ZoneEvent>), Box<dyn std::error::Error + Send + Sync>> {
        let (event_sender, event_receiver) = broadcast::channel(1000);
        
        let monitor = Self {
            cache: ZoneCache::new(),
            price_cache: std::collections::HashMap::new(),
            zone_engine: ZoneDetectionEngine::new(),
            data_fetcher: InfluxDataFetcher::new()?,
            event_sender,
            price_receiver,
            // refresh_task_config: config.clone(), // Store a clone for the refresh task
            config, // The main config
        };
        
        Ok((monitor, event_receiver))
    }
    
     pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🚀 [RT_MONITOR] Starting Real-time Zone Monitor");
        
        // Clone necessary config data BEFORE the mutable borrow for refresh_zones_internal
        let symbols_to_refresh = self.config.monitored_symbols.clone(); // Clone here
        
        // Initial zone refresh
        self.refresh_zones_internal(&symbols_to_refresh, "-7d", "now()").await?; // Use the clone
        
        let refresh_interval = self.config.refresh_interval_secs;
        // Removed the unused monitored_symbols_clone variable

        info!("⏳ [RT_MONITOR] Placeholder for periodic refresh timer (interval: {}s). Actual periodic refresh needs shared state.", refresh_interval);

        // Start price monitoring loop
        self.price_monitoring_loop().await?;
        
        Ok(())
    }
    
    // Renamed to refresh_zones_internal to distinguish from a potential public API
    // This method contains the core logic we fixed in MinimalZoneCache
    async fn refresh_zones_internal(
        &mut self, // Takes &mut self to update self.cache
        symbols_to_monitor: &[SymbolTimeframeConfig],
        fetch_start_time: &str,
        fetch_end_time: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🔄 [RT_MONITOR_REFRESH] Refreshing zones internal. Fetch window: {} to {}", fetch_start_time, fetch_end_time);
        
        let mut all_refreshed_zones = Vec::new(); // Collect all zones here before updating cache
        
        for symbol_config in symbols_to_monitor {
            for timeframe in &symbol_config.timeframes {
                let symbol_from_config = &symbol_config.symbol; // e.g., "EURUSD"
                let current_timeframe = timeframe;
                
                debug!("📊 [RT_MONITOR_REFRESH] Processing {}/{}", symbol_from_config, current_timeframe);
                
                // Apply _SB suffix for fetching, consistent with bulk handler's internal logic
                let core_symbol_for_fetch = if symbol_from_config.ends_with("_SB") {
                    symbol_from_config.clone()
                } else {
                    format!("{}_SB", symbol_from_config)
                };
                
                info!("📊 [RT_MONITOR_REFRESH] Fetching data for core_symbol: {} (from config: {})", core_symbol_for_fetch, symbol_from_config);
                
                // Fetch candles using self.data_fetcher
                let candles = match self.data_fetcher.fetch_candles(
                    &core_symbol_for_fetch,
                    current_timeframe,
                    fetch_start_time, 
                    fetch_end_time,
                ).await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("⚠️ [RT_MONITOR_REFRESH] fetch_candles failed for {}/{}: {}", core_symbol_for_fetch, current_timeframe, e);
                        continue; // Skip to next symbol/timeframe
                    }
                };
                
                if candles.is_empty() {
                    warn!("⚠️ [RT_MONITOR_REFRESH] No candles for {}/{}", core_symbol_for_fetch, current_timeframe);
                    continue;
                }
                
                info!("🕯️ [RT_MONITOR_REFRESH] Got {} candles for {}/{}", candles.len(), core_symbol_for_fetch, current_timeframe);
                
                // Create ZoneDetectionRequest.
                // The `symbol` field in ZoneDetectionRequest should be the non-SB version
                // so that EnrichedZone.symbol is also the non-SB version.
                let detection_request = ZoneDetectionRequest {
                    symbol: symbol_from_config.clone(), // Use "EURUSD" (non-SB)
                    timeframe: current_timeframe.clone(),
                    pattern: "fifty_percent_before_big_bar".to_string(), // Standard pattern
                    candles,
                    max_touch_count: None, // No touch count filter during initial detection for cache
                };
                
                // Run zone detection using self.zone_engine
                match self.zone_engine.detect_zones(detection_request) {
                    Ok(result) => { // result is ZoneDetectionResult
                        info!("🔍 [RT_MONITOR_REFRESH] {}/{}: Detected {} supply + {} demand zones by engine", 
                              result.symbol, result.timeframe, // These come from ZoneDetectionRequest
                              result.supply_zones.len(), result.demand_zones.len());
                        
                        // Filter for active zones only before adding to all_refreshed_zones
                        for zone in result.supply_zones {
                            if zone.is_active {
                                all_refreshed_zones.push(zone);
                            }
                        }
                        for zone in result.demand_zones {
                            if zone.is_active {
                                all_refreshed_zones.push(zone);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("⚠️  [RT_MONITOR_REFRESH] Zone detection failed for {}/{}: {}", 
                              symbol_from_config, current_timeframe, e);
                    }
                }
            }
        }
        
        info!("📦 [RT_MONITOR_REFRESH] Total active zones collected from refresh: {}", all_refreshed_zones.len());
        self.cache.update_zones(all_refreshed_zones); // Update the internal cache
        
        Ok(())
    }
    
    async fn price_monitoring_loop(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("👀 [RT_MONITOR] Starting price monitoring loop");
        
        while let Some(price_update) = self.price_receiver.recv().await {
            // Ensure symbol in PriceUpdate matches the non-SB version stored in EnrichedZone.symbol
            // The PriceUpdate.symbol should be "EURUSD", not "EURUSD_SB" or its ID.
            // The cTrader integration already maps ID to "EURUSD_SB".
            // We need to ensure `price_update.symbol` is consistently "EURUSD" for `get_zones_for_symbol`.

            // Let's assume PriceUpdate.symbol is already the clean, non-SB version.
            // If not, it needs to be mapped here or in the price_feed module.
            // For now, assuming price_update.symbol = "EURUSD".

            self.process_price_update(price_update).await;
        }
        
        info!("[RT_MONITOR] Price monitoring loop ended.");
        Ok(())
    }
    
    async fn process_price_update(&mut self, update: PriceUpdate) {
        // Using symbol_id for price_cache key is fine.
        let old_price = self.price_cache.get(&update.symbol_id).copied();
        self.price_cache.insert(update.symbol_id, update.price);
        
        if let Some(prev_price) = old_price {
            // The `update.symbol` here (e.g., "EURUSD_SB" from cTrader feed) needs to match
            // what `self.cache.get_zones_for_symbol()` expects.
            // `EnrichedZone.symbol` is "EURUSD".
            // So, we need to map `update.symbol` if it's different.
            
            let base_symbol = update.symbol.replace("_SB", ""); // Ensure we use "EURUSD" for cache lookup

            self.check_zone_interactions(&base_symbol, &update.timeframe, prev_price, update.price, update.timestamp).await;
        }
        
        if update.is_new_bar {
            debug!("🆕 [RT_MONITOR] New bar formed for {}/{} (ID: {})", update.symbol, update.timeframe, update.symbol_id);
            // Potentially trigger a targeted refresh for this symbol/timeframe if logic dictates.
            // For example:
            // self.refresh_zones_internal(&[SymbolTimeframeConfig { symbol: update.symbol.replace("_SB", ""), timeframes: vec![update.timeframe.clone()]}], "-1d", "now()").await.ok();
        }
    }
    
    async fn check_zone_interactions(
        &self, 
        symbol_for_cache: &str, // e.g., "EURUSD"
        timeframe_of_price: &str,
        old_price: f64, 
        new_price: f64,
        timestamp: u64
    ) {
        // `symbol_for_cache` should be the non-SB version like "EURUSD"
        let zones = self.cache.get_zones_for_symbol(symbol_for_cache);
        
        for zone in zones {
            // Ensure the zone's timeframe matches the timeframe of the price update.
            // This is important if a symbol is monitored on multiple timeframes (e.g. EURUSD 1h, EURUSD 4h)
            // and a price update comes for a specific timeframe's bar.
            if zone.timeframe.as_deref() == Some(timeframe_of_price) {
                 if let Some(zone_id) = &zone.zone_id { // Ensure zone_id exists
                    if let Some(event) = self.analyze_zone_interaction(&zone, old_price, new_price, timestamp) {
                        if let Err(e) = self.event_sender.send(event) {
                            warn!("⚠️  [RT_MONITOR] Failed to broadcast zone event for {}: {}", zone_id, e);
                        }
                    }
                }
            }
        }
    }
    
    fn analyze_zone_interaction(
        &self,
        zone: &EnrichedZone, // Zone from cache, symbol should be "EURUSD"
        old_price: f64,
        new_price: f64,
        timestamp: u64,
    ) -> Option<ZoneEvent> {
        // These fields should be reliably present in a valid EnrichedZone from cache
        let zone_high = zone.zone_high?;
        let zone_low = zone.zone_low?;
        let zone_id = zone.zone_id.as_ref()?;
        // `zone.symbol` is "EURUSD", `zone.timeframe` is the zone's timeframe.
        let symbol = zone.symbol.as_ref()?; 
        let timeframe = zone.timeframe.as_ref()?;
        
        let is_supply = zone.zone_type.as_ref()?.contains("supply"); // Simpler check
        
        let (proximal_line, distal_line) = if is_supply {
            (zone_low, zone_high)
        } else {
            (zone_high, zone_low)
        };
        
        // Logic for breakout: price crosses and CLOSES beyond distal (or a tick beyond)
        // Using a simple cross for now.
        let distal_broken = if is_supply {
            new_price > distal_line && old_price <= distal_line // Crossed up through supply distal
        } else {
            new_price < distal_line && old_price >= distal_line // Crossed down through demand distal
        };
        
        if distal_broken {
            info!("💥 [RT_MONITOR] Zone {} ({}/{}) invalidated by breakout. Price {} -> {}. Distal: {:.5}", 
                zone_id, symbol, timeframe, old_price, new_price, distal_line);
            
            return Some(ZoneEvent {
                event_type: ZoneEventType::Breakout,
                zone_id: zone_id.clone(),
                symbol: symbol.clone(),
                timeframe: timeframe.clone(),
                price: new_price,
                timestamp,
                zone_high,
                zone_low,
                metadata: Some(serde_json::json!({
                    "direction": if is_supply { "bullish_breakout_of_supply" } else { "bearish_breakout_of_demand" },
                    "zone_type": zone.zone_type.clone(),
                    "invalidation": true
                })),
            });
        }
        
        // Logic for touch/entry: price crosses and CLOSES into zone (or a tick into)
        // Using a simple cross for now.
        let proximal_touched = if is_supply {
            new_price >= proximal_line && old_price < proximal_line // Crossed down into supply proximal
        } else {
            new_price <= proximal_line && old_price > proximal_line // Crossed up into demand proximal
        };
        
        if proximal_touched {
             info!("🎯 [RT_MONITOR] Zone {} ({}/{}) touched for entry. Price {} -> {}. Proximal: {:.5}", 
                zone_id, symbol, timeframe, old_price, new_price, proximal_line);
            
            return Some(ZoneEvent {
                // Could be ZoneEventType::Touch or ::Entry depending on more refined logic
                event_type: ZoneEventType::Entry, 
                zone_id: zone_id.clone(),
                symbol: symbol.clone(),
                timeframe: timeframe.clone(),
                price: new_price,
                timestamp,
                zone_high,
                zone_low,
                metadata: Some(serde_json::json!({
                    "entry_type": if is_supply { "short" } else { "long" },
                    "zone_type": zone.zone_type.clone(),
                    "strength_score": zone.strength_score,
                    "touch_count": zone.touch_count, // Historical touch count at time of cache
                    "bars_active": zone.bars_active, // Historical bars active at time of cache
                })),
            });
        }
        
        None
    }
    
    pub fn subscribe_to_events(&self) -> broadcast::Receiver<ZoneEvent> {
        self.event_sender.subscribe()
    }
    
    pub fn get_zone_stats(&self) -> (usize, SystemTime) {
        self.cache.get_stats()
    }

    // get_all_zones and get_zones_for_symbol_ws remain useful for WebSocket server state dumps
    pub async fn get_all_zones_for_ws(&self) -> Vec<serde_json::Value> {
        let zones_from_cache = self.cache.zones.values().cloned().collect::<Vec<_>>();
        match serde_json::to_value(zones_from_cache) {
            Ok(v_arr) => {
                if let serde_json::Value::Array(arr) = v_arr {
                    info!("[RT_MONITOR] Returning {} zones to WebSocket client (get_all_zones_for_ws)", arr.len());
                    arr
                } else {
                    warn!("[RT_MONITOR] Failed to convert zones to JSON array for WS");
                    Vec::new()
                }
            }
            Err(e) => {
                error!("[RT_MONITOR] Error serializing all zones for WS: {}", e);
                Vec::new()
            }
        }
    }
    
    pub async fn get_zones_for_symbol_ws(&self, symbol: &str) -> Vec<serde_json::Value> {
        // `symbol` here should be the non-SB version, e.g., "EURUSD"
        let zones_for_symbol_from_cache = self.cache.get_zones_for_symbol(symbol);
        match serde_json::to_value(zones_for_symbol_from_cache) {
             Ok(v_arr) => {
                if let serde_json::Value::Array(arr) = v_arr {
                    info!("[RT_MONITOR] Returning {} zones for symbol {} to WebSocket client", arr.len(), symbol);
                    arr
                } else {
                    warn!("[RT_MONITOR] Failed to convert zones for symbol {} to JSON array for WS", symbol);
                    Vec::new()
                }
            }
            Err(e) => {
                error!("[RT_MONITOR] Error serializing zones for symbol {} for WS: {}", symbol, e);
                Vec::new()
            }
        }
    }
}