// src/terminal_dashboard.rs - Completely Fixed Real-time terminal zone dashboard
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use chrono::{DateTime, Utc};

use crate::realtime_zone_monitor::NewRealTimeZoneMonitor;

#[derive(Debug, Clone)]
pub struct ZoneDistanceInfo {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String, // "supply" or "demand"
    pub current_price: f64,
    pub proximal_line: f64,
    pub distal_line: f64,
    pub distance_pips: f64,
    pub last_update: DateTime<Utc>,
    pub touch_count: i32,
    pub strength_score: f64,
}

pub struct TerminalDashboard {
    zone_monitor: Arc<NewRealTimeZoneMonitor>,
    pip_values: HashMap<String, f64>,
    update_interval: Duration,
}

impl TerminalDashboard {
    pub fn new(zone_monitor: Arc<NewRealTimeZoneMonitor>) -> Self {
        let mut pip_values = HashMap::new();
        
        // Define pip values for major currency pairs
        pip_values.insert("EURUSD".to_string(), 0.0001);
        pip_values.insert("GBPUSD".to_string(), 0.0001);
        pip_values.insert("AUDUSD".to_string(), 0.0001);
        pip_values.insert("NZDUSD".to_string(), 0.0001);
        pip_values.insert("USDCAD".to_string(), 0.0001);
        pip_values.insert("USDCHF".to_string(), 0.0001);
        pip_values.insert("EURGBP".to_string(), 0.0001);
        pip_values.insert("EURAUD".to_string(), 0.0001);
        pip_values.insert("EURNZD".to_string(), 0.0001);
        pip_values.insert("EURJPY".to_string(), 0.01);
        pip_values.insert("GBPJPY".to_string(), 0.01);
        pip_values.insert("AUDJPY".to_string(), 0.01);
        pip_values.insert("NZDJPY".to_string(), 0.01);
        pip_values.insert("USDJPY".to_string(), 0.01);
        pip_values.insert("CADJPY".to_string(), 0.01);
        pip_values.insert("CHFJPY".to_string(), 0.01);
        pip_values.insert("AUDCAD".to_string(), 0.0001);
        pip_values.insert("AUDCHF".to_string(), 0.0001);
        pip_values.insert("AUDNZD".to_string(), 0.0001);
        pip_values.insert("CADCHF".to_string(), 0.0001);
        pip_values.insert("EURCHF".to_string(), 0.0001);
        pip_values.insert("EURCAD".to_string(), 0.0001);
        pip_values.insert("GBPAUD".to_string(), 0.0001);
        pip_values.insert("GBPCAD".to_string(), 0.0001);
        pip_values.insert("GBPCHF".to_string(), 0.0001);
        pip_values.insert("GBPNZD".to_string(), 0.0001);
        pip_values.insert("NZDCAD".to_string(), 0.0001);
        pip_values.insert("NZDCHF".to_string(), 0.0001);
        
        // Indices (larger pip values)
        pip_values.insert("NAS100".to_string(), 1.0);
        pip_values.insert("US500".to_string(), 0.1);

        Self {
            zone_monitor,
            pip_values,
            update_interval: Duration::from_secs(1), // Update every second
        }
    }

    pub async fn start(&self) {
        println!("🎯 Starting Real-time Zone Dashboard...");
        println!("Press Ctrl+C to exit\n");

        let mut ticker = interval(self.update_interval);
        
        loop {
            ticker.tick().await;
            self.update_dashboard().await;
        }
    }

    async fn update_dashboard(&self) {
        // Clear screen and move cursor to top
        print!("\x1B[2J\x1B[H");
        
        let now = Utc::now();
        
        // Get all zones from monitor
        let zones_json = self.zone_monitor.get_all_zones_for_ws().await;
        
        if zones_json.is_empty() {
            println!("📊 Real-time Zone Dashboard - {}", now.format("%H:%M:%S"));
            println!("══════════════════════════════════════════════════════════════");
            println!("⚠️  No zones available. Waiting for data...");
            
            // Show available prices for debugging
            let current_prices = self.zone_monitor.get_current_prices_by_symbol().await;
            if !current_prices.is_empty() {
                println!("\n🔍 Available Prices:");
                for (symbol, price) in current_prices.iter().take(5) {
                    println!("   {} = {:.5}", symbol, price);
                }
                if current_prices.len() > 5 {
                    println!("   ... and {} more", current_prices.len() - 5);
                }
            }
            return;
        }

        // Get current prices for all symbols
        let current_prices = self.zone_monitor.get_current_prices_by_symbol().await;

        // Convert to LiveZoneStatus-like data and calculate distances
        let mut zone_distances: Vec<ZoneDistanceInfo> = Vec::new();
        
        for zone_json in &zones_json {
            if let Ok(zone_info) = self.extract_zone_distance_info(&zone_json, &current_prices).await {
                zone_distances.push(zone_info);
            }
        }

        // Filter out zones with no valid price data and invalid distances
        zone_distances.retain(|z| z.current_price > 0.0 && z.distance_pips >= 0.0 && z.distance_pips < 10000.0);

        // Sort by distance (closest first)
        zone_distances.sort_by(|a, b| a.distance_pips.partial_cmp(&b.distance_pips).unwrap_or(std::cmp::Ordering::Equal));

        // Display header
        println!("💡 Legend: DIST=Distance in pips, TCHES=Touch count, STRTH=Strength score");
        println!("🔴 <5 pips (URGENT) | 🟡 <10 pips (CLOSE) | 🟢 <25 pips (WATCH)");
        println!("🎯 Real-time Zone Dashboard - {}", now.format("%H:%M:%S"));
        println!("══════════════════════════════════════════════════════════════");
        println!("{:<15} {:<8} {:<6} {:<11} {:<11} {:<11} {:<6} {:<6}", 
                 "SYMBOL/TF", "TYPE", "DIST", "PRICE", "PROXIMAL", "DISTAL", "TCHES", "STRTH");
        println!("──────────────────────────────────────────────────────────────");

        // Show top 20 closest zones
        let display_count = zone_distances.len().min(20);
        
        if display_count == 0 {
            println!("⚠️  No zones with valid price data available");
            
            // Debug info
            println!("\n🔍 Debug Info:");
            println!("   Zones from monitor: {}", zones_json.len());
            println!("   Current prices available: {}", current_prices.len());
            
            if !current_prices.is_empty() {
                println!("   Sample prices:");
                for (symbol, price) in current_prices.iter().take(3) {
                    println!("     {} = {:.5}", symbol, price);
                }
            }
            return;
        }
        
        for zone in zone_distances.iter().take(display_count) {
            let color_code = if zone.distance_pips < 5.0 {
                "\x1B[91m" // Bright red for very close (< 5 pips)
            } else if zone.distance_pips < 10.0 {
                "\x1B[93m" // Bright yellow for close (< 10 pips)
            } else if zone.distance_pips < 25.0 {
                "\x1B[92m" // Bright green for medium (< 25 pips)
            } else {
                "\x1B[37m" // White for distant
            };
            
            let reset_code = "\x1B[0m";
            
            let symbol_tf = format!("{}/{}", zone.symbol, zone.timeframe);
            let zone_type_short = if zone.zone_type.contains("supply") { "SELL" } else { "BUY " };
            
            println!("{}{:<15} {:<8} {:<6.1} {:<11.5} {:<11.5} {:<11.5} {:<6} {:<6.0}{}",
                     color_code,
                     symbol_tf,
                     zone_type_short,
                     zone.distance_pips,
                     zone.current_price,
                     zone.proximal_line,
                     zone.distal_line,
                     zone.touch_count,
                     zone.strength_score,
                     reset_code);
        }

        // Show summary stats
        println!("──────────────────────────────────────────────────────────────");
        let very_close = zone_distances.iter().filter(|z| z.distance_pips < 5.0).count();
        let close = zone_distances.iter().filter(|z| z.distance_pips < 10.0).count();
        let medium = zone_distances.iter().filter(|z| z.distance_pips < 25.0).count();
        
        println!("📊 Total Zones: {} | 🔴 <5 pips: {} | 🟡 <10 pips: {} | 🟢 <25 pips: {}", 
                 zone_distances.len(), very_close, close, medium);
    }

    async fn extract_zone_distance_info(
        &self, 
        zone_json: &serde_json::Value,
        current_prices: &HashMap<String, f64>
    ) -> Result<ZoneDistanceInfo, Box<dyn std::error::Error + Send + Sync>> {
        // Extract zone data from JSON
        let zone_id = zone_json.get("zone_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
            
        let symbol = zone_json.get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();
            
        let timeframe = zone_json.get("timeframe")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();
            
        let zone_type = zone_json.get("zone_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
            
        let zone_high = zone_json.get("zone_high")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
            
        let zone_low = zone_json.get("zone_low")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
            
        let touch_count = zone_json.get("touch_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
            
        let strength_score = zone_json.get("strength_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        // Get REAL current price from the prices HashMap
        let current_price = current_prices.get(&symbol)
            .copied()
            .ok_or_else(|| format!("No current price available for symbol: {}", symbol))?;
        
        // Validate zone boundaries
        if zone_high <= 0.0 || zone_low <= 0.0 || zone_high <= zone_low {
            return Err(format!("Invalid zone boundaries: high={}, low={}", zone_high, zone_low).into());
        }
        
        // Determine which line is the proximal (entry) line based on zone type and current price
        let is_supply = zone_type.contains("supply");
        let (proximal_line, distal_line) = if is_supply {
            // For supply zones, price approaches from below
            // Proximal = zone_low (entry), Distal = zone_high (target)
            (zone_low, zone_high)
        } else {
            // For demand zones, price approaches from above  
            // Proximal = zone_high (entry), Distal = zone_low (target)
            (zone_high, zone_low)
        };

        // Calculate distance to proximal line (entry point) in pips
        let price_distance = (current_price - proximal_line).abs();
        let pip_value = self.pip_values.get(&symbol).cloned().unwrap_or(0.0001);
        
        // Validate pip value
        if pip_value <= 0.0 {
            return Err(format!("Invalid pip value for symbol: {}", symbol).into());
        }
        
        let distance_pips = price_distance / pip_value;

        // Additional validation
        if distance_pips.is_nan() || distance_pips.is_infinite() {
            return Err(format!("Invalid distance calculation for symbol: {}", symbol).into());
        }

        Ok(ZoneDistanceInfo {
            zone_id,
            symbol,
            timeframe,
            zone_type,
            current_price,
            proximal_line,
            distal_line,
            distance_pips,
            last_update: Utc::now(),
            touch_count,
            strength_score,
        })
    }
}