// zone_proximity_analyzer.rs - Analyzes zones that came close to being touched
use crate::types::{ZoneData, PriceCandle};
use chrono::{DateTime, Utc};
use log::info;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct ZoneProximityEvent {
    pub timestamp: DateTime<Utc>,
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub zone_high: f64,
    pub zone_low: f64,
    pub zone_strength: f64,
    pub current_price: f64,
    pub proximal_line: f64,
    pub distance_from_proximal: f64,
    pub distance_pips: f64,
    pub proximity_threshold: f64,
    pub proximity_threshold_pips: f64,
    pub is_actual_touch: bool,
    pub closest_approach: f64,
    pub proximity_percentage: f64, // How close as percentage (100% = exact touch)
}

#[derive(Debug, Serialize)]
pub struct ZoneProximitySummary {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub zone_high: f64,
    pub zone_low: f64,
    pub zone_strength: f64,
    pub total_proximity_events: usize,
    pub actual_touches: usize,
    pub closest_distance_pips: f64,
    pub closest_timestamp: DateTime<Utc>,
    pub closest_price: f64,
    pub average_distance_pips: f64,
    pub first_proximity_time: DateTime<Utc>,
    pub last_proximity_time: DateTime<Utc>,
}

pub struct ZoneProximityAnalyzer {
    proximity_events: Vec<ZoneProximityEvent>,
    zone_summaries: HashMap<String, ZoneProximitySummary>,
}

impl ZoneProximityAnalyzer {
    pub fn new() -> Self {
        Self {
            proximity_events: Vec::new(),
            zone_summaries: HashMap::new(),
        }
    }

    pub fn analyze_zone_proximity(
        &mut self,
        zones: &[ZoneData],
        price_data: &HashMap<String, Vec<PriceCandle>>,
        proximity_multiplier: f64, // How many times larger than touch threshold to track (e.g., 5.0 = track within 5x touch distance)
    ) {
        info!("🔍 Starting comprehensive zone proximity analysis...");
        info!("📊 Analyzing {} zones across {} symbols", zones.len(), price_data.len());
        info!("🎯 Tracking approaches within {}x touch threshold", proximity_multiplier);

        // Get all unique timestamps
        let mut all_timestamps = std::collections::BTreeSet::new();
        for candles in price_data.values() {
            for candle in candles {
                all_timestamps.insert(candle.timestamp);
            }
        }

        let total_minutes = all_timestamps.len();
        info!("⏱️ Processing {} minutes of data...", total_minutes);

        let mut minute_count = 0;

        for current_time in all_timestamps {
            minute_count += 1;
            
            if minute_count % 100 == 0 {
                info!("⏳ Processing minute {}/{} ({})", minute_count, total_minutes, current_time.format("%H:%M"));
            }

            // Get current prices for all symbols
            let current_prices = self.get_prices_at_timestamp(current_time, price_data);

            // Check each zone for proximity
            for zone in zones {
                if let Some(current_price) = current_prices.get(&zone.symbol) {
                    self.check_zone_proximity(zone, *current_price, current_time, proximity_multiplier);
                }
            }
        }

        // Generate summaries
        self.generate_zone_summaries();

        info!("✅ Proximity analysis complete!");
        info!("📊 Found {} proximity events across {} unique zones", 
              self.proximity_events.len(), self.zone_summaries.len());
    }

    fn check_zone_proximity(
        &mut self,
        zone: &ZoneData,
        current_price: f64,
        timestamp: DateTime<Utc>,
        proximity_multiplier: f64,
    ) {
        let zone_height = zone.zone_high - zone.zone_low;
        let pip_value = self.get_pip_value(&zone.symbol);
        
        // Calculate touch threshold (same as main analyzer)
        let pip_threshold: f64 = 0.0002;
        let percent_threshold: f64 = zone_height * 0.02;
        let touch_threshold = pip_threshold.min(percent_threshold);
        
        // Calculate proximity tracking threshold
        let proximity_threshold = touch_threshold * proximity_multiplier;
        
        let is_supply = zone.zone_type.contains("supply");
        let proximal_line = if is_supply { zone.zone_low } else { zone.zone_high };
        
        let distance_from_proximal = (current_price - proximal_line).abs();
        
        // Check if within proximity tracking range
        if distance_from_proximal <= proximity_threshold {
            let distance_pips = distance_from_proximal / pip_value;
            let touch_threshold_pips = touch_threshold / pip_value;
        let _touch_threshold_pips = touch_threshold / pip_value; // Prefix with underscore since it's unused
            let proximity_threshold_pips = proximity_threshold / pip_value;
            let is_actual_touch = distance_from_proximal <= touch_threshold;
            let proximity_percentage = if touch_threshold > 0.0 {
                ((touch_threshold - distance_from_proximal) / touch_threshold * 100.0).max(0.0)
            } else {
                0.0
            };

            let proximity_event = ZoneProximityEvent {
                timestamp,
                zone_id: zone.zone_id.clone(),
                symbol: zone.symbol.clone(),
                timeframe: zone.timeframe.clone(),
                zone_type: zone.zone_type.clone(),
                zone_high: zone.zone_high,
                zone_low: zone.zone_low,
                zone_strength: zone.strength_score,
                current_price,
                proximal_line,
                distance_from_proximal,
                distance_pips,
                proximity_threshold,
                proximity_threshold_pips,
                is_actual_touch,
                closest_approach: distance_from_proximal, // Will be updated in summary
                proximity_percentage,
            };

            self.proximity_events.push(proximity_event);
        }
    }

    fn generate_zone_summaries(&mut self) {
        info!("📋 Generating zone proximity summaries...");

        // Group events by zone_id
        let mut zone_events: HashMap<String, Vec<&ZoneProximityEvent>> = HashMap::new();
        for event in &self.proximity_events {
            zone_events.entry(event.zone_id.clone()).or_insert_with(Vec::new).push(event);
        }

        for (zone_id, events) in zone_events {
            if let Some(first_event) = events.first() {
                let actual_touches = events.iter().filter(|e| e.is_actual_touch).count();
                let closest_event = events.iter().min_by(|a, b| 
                    a.distance_from_proximal.partial_cmp(&b.distance_from_proximal).unwrap()
                ).unwrap();
                
                let average_distance_pips = events.iter()
                    .map(|e| e.distance_pips)
                    .sum::<f64>() / events.len() as f64;

                let first_time = events.iter().map(|e| e.timestamp).min().unwrap();
                let last_time = events.iter().map(|e| e.timestamp).max().unwrap();

                let summary = ZoneProximitySummary {
                    zone_id: zone_id.clone(),
                    symbol: first_event.symbol.clone(),
                    timeframe: first_event.timeframe.clone(),
                    zone_type: first_event.zone_type.clone(),
                    zone_high: first_event.zone_high,
                    zone_low: first_event.zone_low,
                    zone_strength: first_event.zone_strength,
                    total_proximity_events: events.len(),
                    actual_touches,
                    closest_distance_pips: closest_event.distance_pips,
                    closest_timestamp: closest_event.timestamp,
                    closest_price: closest_event.current_price,
                    average_distance_pips,
                    first_proximity_time: first_time,
                    last_proximity_time: last_time,
                };

                self.zone_summaries.insert(zone_id, summary);
            }
        }
    }

    fn get_prices_at_timestamp(&self, timestamp: DateTime<Utc>, price_data: &HashMap<String, Vec<PriceCandle>>) -> HashMap<String, f64> {
        let mut prices = HashMap::new();
        for (symbol, candles) in price_data {
            if let Some(candle) = candles.iter().find(|c| c.timestamp == timestamp) {
                prices.insert(symbol.clone(), candle.close);
            }
        }
        prices
    }

    fn get_pip_value(&self, symbol: &str) -> f64 {
        if symbol.contains("JPY") { 
            0.01 
        } else { 
            0.0001 
        }
    }

    pub async fn write_proximity_summary_only(&self) -> Result<(), Box<dyn std::error::Error>> {
        let output_dir = std::env::var("DEBUG_FILES_DIR").unwrap_or_else(|_| "trades".to_string());
        std::fs::create_dir_all(&output_dir)?;
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        
        // Write ONLY zone summaries (closest approach per zone)
        let summary_filename = format!("{}/zone_proximity_summary_{}.csv", output_dir, timestamp);
        info!("📝 Writing {} zone summaries to: {}", self.zone_summaries.len(), summary_filename);
        
        let summary_file = std::fs::File::create(&summary_filename)?;
        let mut summary_writer = csv::Writer::from_writer(summary_file);
        
        summary_writer.write_record(&[
            "zone_id", "symbol", "timeframe", "zone_type", "zone_high", "zone_low", 
            "zone_strength", "closest_distance_pips", "closest_timestamp", "closest_price",
            "was_actual_touch"
        ])?;

        let mut summaries: Vec<_> = self.zone_summaries.values().collect();
        summaries.sort_by(|a, b| a.closest_distance_pips.partial_cmp(&b.closest_distance_pips).unwrap());

        for summary in &summaries {
            let was_actual_touch = summary.actual_touches > 0;
            let record = vec![
                summary.zone_id.clone(),
                summary.symbol.clone(),
                summary.timeframe.clone(),
                summary.zone_type.clone(),
                format!("{:.5}", summary.zone_high),
                format!("{:.5}", summary.zone_low),
                format!("{:.1}", summary.zone_strength),
                format!("{:.1}", summary.closest_distance_pips),
                summary.closest_timestamp.to_rfc3339(),
                format!("{:.5}", summary.closest_price),
                was_actual_touch.to_string(),
            ];
            summary_writer.write_record(&record)?;
        }
        summary_writer.flush()?;

        println!("📄 Zone proximity summary saved: {}", summary_filename);

        // Print top 10 closest approaches
        info!("🎯 TOP 10 CLOSEST ZONE APPROACHES:");
        for (i, summary) in summaries.iter().take(10).enumerate() {
            let status = if summary.actual_touches > 0 { "✅ TOUCHED" } else { "📍 CLOSE" };
            info!("   {}: {} {} {} - {:.1} pips away (at {}) {}", 
                  i+1, summary.symbol, summary.timeframe, &summary.zone_id[..8],
                  summary.closest_distance_pips, summary.closest_timestamp.format("%H:%M:%S"), status);
        }

        Ok(())
    }

    pub fn get_proximity_events(&self) -> &Vec<ZoneProximityEvent> {
        &self.proximity_events
    }

    pub fn get_zone_summaries(&self) -> &HashMap<String, ZoneProximitySummary> {
        &self.zone_summaries
    }
}