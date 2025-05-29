// =============================================================================
// src/realtime_monitor.rs - Main real-time monitor logic
// =============================================================================

use std::time::SystemTime;
use tokio::sync::{broadcast, mpsc};
use serde::{Serialize, Deserialize};
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
    Touch,        // Price touched proximal line
    Breakout,     // Price broke through distal line (zone invalidated)
    Entry,        // Valid entry signal
    Retest,       // Price returned to test zone again
}

#[derive(Debug, Clone)]
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

// Simple zone cache for the real-time monitor
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
        info!("🔄 [ZONE_CACHE] Updated with {} zones", self.zones.len());
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
    price_cache: std::collections::HashMap<u32, f64>,
    zone_engine: ZoneDetectionEngine,
    data_fetcher: InfluxDataFetcher,
    event_sender: broadcast::Sender<ZoneEvent>,
    price_receiver: mpsc::Receiver<PriceUpdate>,
    config: MonitorConfig,
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
            config,
        };
        
        Ok((monitor, event_receiver))
    }
    
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🚀 [RT_MONITOR] Starting Real-time Zone Monitor");
        
        // Initial zone refresh
        self.refresh_zones().await?;
        
        // Start refresh timer
        let refresh_interval = self.config.refresh_interval_secs;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(refresh_interval));
            loop {
                interval.tick().await;
                // In a real implementation, you'd trigger refresh here
                debug!("⏰ [RT_MONITOR] Refresh timer tick");
            }
        });
        
        // Start price monitoring loop
        self.price_monitoring_loop().await?;
        
        Ok(())
    }
    
   async fn refresh_zones(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("🔄 [RT_MONITOR] Refreshing zones...");
    
    let mut all_zones = Vec::new();
    
    for symbol_config in &self.config.monitored_symbols {
        for timeframe in &symbol_config.timeframes {
            debug!("📊 [RT_MONITOR] Fetching zones for {}/{}", symbol_config.symbol, timeframe);
            
            // Fetch candles for this symbol/timeframe
            let candles = self.data_fetcher.fetch_candles(
                &symbol_config.symbol,
                timeframe,
                "-7d",
                "now()",
            ).await?;
            
            info!("🕯️ [RT_MONITOR] Got {} candles for {}/{}", candles.len(), symbol_config.symbol, timeframe); // ADD THIS
            
            if !candles.is_empty() {
                let request = ZoneDetectionRequest {
                    symbol: symbol_config.symbol.clone(),
                    timeframe: timeframe.clone(),
                    pattern: "fifty_percent_before_big_bar".to_string(),
                    candles,
                    max_touch_count: None,
                };
                
                match self.zone_engine.detect_zones(request) {
                    Ok(result) => {
                        info!("🔍 [RT_MONITOR] Found {} supply + {} demand zones for {}/{}", 
                              result.supply_zones.len(), result.demand_zones.len(), 
                              symbol_config.symbol, timeframe); // ADD THIS
                        
                        for zone in result.supply_zones {
                            if zone.is_active {
                                all_zones.push(zone);
                            }
                        }
                        for zone in result.demand_zones {
                            if zone.is_active {
                                all_zones.push(zone);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("⚠️  [RT_MONITOR] Zone detection failed for {}/{}: {}", 
                              symbol_config.symbol, timeframe, e);
                    }
                }
            } else {
                warn!("⚠️ [RT_MONITOR] No candles found for {}/{}", symbol_config.symbol, timeframe); // ADD THIS
            }
        }
    }
    
    info!("📦 [RT_MONITOR] Total zones before cache update: {}", all_zones.len()); // ADD THIS
    self.cache.update_zones(all_zones);
    info!("✅ [RT_MONITOR] Zone refresh complete");
    
    Ok(())
}
    
    async fn price_monitoring_loop(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("👀 [RT_MONITOR] Starting price monitoring loop");
        
        while let Some(price_update) = self.price_receiver.recv().await {
            self.process_price_update(price_update).await;
        }
        
        Ok(())
    }
    
    async fn process_price_update(&mut self, update: PriceUpdate) {
        let old_price = self.price_cache.get(&update.symbol_id).copied();
        self.price_cache.insert(update.symbol_id, update.price);
        
        // Only check zone interactions if we have a previous price
        if let Some(old_price) = old_price {
            self.check_zone_interactions(&update, old_price).await;
        }
        
        // On new bar formation, consider triggering zone refresh for this symbol
        if update.is_new_bar {
            debug!("🆕 [RT_MONITOR] New bar formed for {}/{}", update.symbol, update.timeframe);
        }
    }
    
    async fn check_zone_interactions(&self, update: &PriceUpdate, old_price: f64) {
        let zones = self.cache.get_zones_for_symbol(&update.symbol);
        
        for zone in zones {
            if let Some(zone_id) = &zone.zone_id {
                // Skip if timeframe doesn't match
                if let Some(zone_tf) = &zone.timeframe {
                    if zone_tf != &update.timeframe {
                        continue;
                    }
                }
                
                if let Some(event) = self.analyze_zone_interaction(&zone, old_price, update.price, update.timestamp) {
                    // Broadcast event to clients
                    if let Err(e) = self.event_sender.send(event) {
                        warn!("⚠️  [RT_MONITOR] Failed to broadcast zone event: {}", e);
                    }
                }
            }
        }
    }
    
    fn analyze_zone_interaction(
        &self,
        zone: &EnrichedZone,
        old_price: f64,
        new_price: f64,
        timestamp: u64,
    ) -> Option<ZoneEvent> {
        let zone_high = zone.zone_high?;
        let zone_low = zone.zone_low?;
        let zone_id = zone.zone_id.as_ref()?;
        let symbol = zone.symbol.as_ref()?;
        let timeframe = zone.timeframe.as_ref()?;
        
        // Determine zone type
        let is_supply = zone.zone_type.as_ref()
            .map(|zt| zt.contains("supply"))
            .unwrap_or(false);
        
        let (proximal_line, distal_line) = if is_supply {
            (zone_low, zone_high)  // Supply: entry at low, break at high
        } else {
            (zone_high, zone_low)  // Demand: entry at high, break at low
        };
        
        // Check for distal line break (invalidation)
        let distal_broken = if is_supply {
            new_price > distal_line && old_price <= distal_line
        } else {
            new_price < distal_line && old_price >= distal_line
        };
        
        if distal_broken {
            info!("💥 [RT_MONITOR] Zone {} invalidated by breakout", zone_id);
            
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
                    "direction": if is_supply { "bullish" } else { "bearish" },
                    "zone_type": zone.zone_type,
                    "invalidation": true
                })),
            });
        }
        
        // Check for proximal line touch (entry signal)
        let proximal_touched = if is_supply {
            new_price >= proximal_line && old_price < proximal_line
        } else {
            new_price <= proximal_line && old_price > proximal_line
        };
        
        if proximal_touched {
            info!("🎯 [RT_MONITOR] Zone {} touched for entry signal", zone_id);
            
            return Some(ZoneEvent {
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
                    "zone_type": zone.zone_type,
                    "strength_score": zone.strength_score,
                    "touch_count": zone.touch_count,
                    "bars_active": zone.bars_active
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

    pub async fn get_all_zones(&self) -> Vec<serde_json::Value> {
        let mut zones_json = Vec::new();
        
        for zone in self.cache.zones.values() {
            // Convert EnrichedZone to JSON
            match serde_json::to_value(zone) {
                Ok(zone_json) => zones_json.push(zone_json),
                Err(e) => {
                    warn!("Failed to serialize zone to JSON: {}", e);
                }
            }
        }
        
        info!("Returning {} zones to WebSocket client", zones_json.len());
        zones_json
    }
    
    pub async fn get_zones_for_symbol_ws(&self, symbol: &str) -> Vec<serde_json::Value> {
        let mut zones_json = Vec::new();
        
        for zone in self.cache.zones.values() {
            if zone.symbol.as_deref() == Some(symbol) {
                match serde_json::to_value(zone) {
                    Ok(zone_json) => zones_json.push(zone_json),
                    Err(e) => {
                        warn!("Failed to serialize zone to JSON: {}", e);
                    }
                }
            }
        }
        
        zones_json
    }
}