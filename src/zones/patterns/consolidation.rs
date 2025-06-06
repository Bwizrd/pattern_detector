use crate::api::detect::CandleData;
use crate::zones::patterns::PatternRecognizer;
use serde_json::json;

#[allow(dead_code)]
#[derive(Clone)]
pub struct ConsolidationZone {
    pub start_index: usize, // Analyzer does not recognize this field
    pub end_index: usize,
    pub high: f64,
    pub low: f64,
    pub candle_count: usize,
    pub price_range_percent: f64,
    pub start_time: String,
    pub end_time: String,
    pub avg_volume: f64,
}

pub struct ConsolidationRecognizer {
    pub min_candles: usize,
    pub max_candles: usize,
    pub max_range_percent: f64,
    pub volume_threshold_ratio: f64,
}

impl Default for ConsolidationRecognizer {
    fn default() -> Self {
        Self {
            min_candles: 2,
            max_candles: 8,
            max_range_percent: 2.0,
            volume_threshold_ratio: 0.8, // Volume should be 80% or less of prior volume
        }
    }
}

impl PatternRecognizer for ConsolidationRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        // Ensure we have enough candles for pattern detection
        if candles.len() < self.min_candles {
            return json!({
                "pattern": "consolidation_zone",
                "error": "Not enough candles for detection",
                "total_bars": candles.len(),
                "required_bars": self.min_candles,
                "total_detected": 0,
                "datasets": {
                    "Price": 0,
                    "Oscillators": 0,
                    "Lines": 0
                },
                "data": {
                    "price": {
                        "consolidation_zones": {
                            "Total": 0,
                            "zones": []
                        }
                    },
                    "oscillators": [],
                    "lines": []
                }
            });
        }

        let zones = self.find_consolidation_zones(candles);
        let zone_count = zones.len();

        // Convert zones to JSON
        let zones_json: Vec<serde_json::Value> = zones.iter().map(|zone| {
            json!({
                "category": "Price",
                "type": "consolidation_zone",
                "start_time": zone.start_time,
                "end_time": zone.end_time,
                "zone_high": zone.high,
                "zone_low": zone.low,
                "candle_count": zone.candle_count,
                "price_range_percent": zone.price_range_percent,
                "avg_volume": zone.avg_volume
            })
        }).collect();

        // Return the detected zones
        json!({
            "pattern": "consolidation_zone",
            "total_bars": candles.len(),
            "total_detected": zone_count,
            "datasets": {
                "price": 1, // One dataset: consolidation_zones
                "oscillators": 0,
                "lines": 0
            },
            "data": {
                "price": {
                    "consolidation_zones": {
                        "total": zone_count,
                        "zones": zones_json
                    }
                },
                "oscillators": [],
                "lines": []
            }
        })
    }
}

impl ConsolidationRecognizer {
    pub fn find_consolidation_zones(&self, candles: &[CandleData]) -> Vec<ConsolidationZone> {
        if candles.len() < self.min_candles {
            return Vec::new();
        }

        let mut zones = Vec::new();
        let mut i = 0;

        while i <= candles.len() - self.min_candles {
            // Try to find a consolidation starting at this index
            if let Some(zone) = self.find_consolidation_starting_at(candles, i) {
                zones.push(zone.clone());
                // Skip past this zone for efficiency
                i = zone.end_index + 1;
            } else {
                i += 1;
            }
        }

        zones
    }

    fn find_consolidation_starting_at(&self, candles: &[CandleData], start_idx: usize) -> Option<ConsolidationZone> {
        if start_idx + self.min_candles > candles.len() {
            return None;
        }

        let mut zone_high = candles[start_idx].high;
        let mut zone_low = candles[start_idx].low;
        let mut volume_sum: u64 = candles[start_idx].volume as u64; // Use u64 to prevent overflow
        let mut end_idx = start_idx;
        let mut zone_candles = 1;
        
        // Calculate the average volume before the potential zone
        let pre_zone_volume_avg = if start_idx >= 3 {
            let mut sum: u64 = 0;
            for i in start_idx-3..start_idx {
                sum += candles[i].volume as u64;
            }
            sum as f64 / 3.0
        } else if start_idx > 0 {
            let mut sum: u64 = 0;
            for i in 0..start_idx {
                sum += candles[i].volume as u64;
            }
            sum as f64 / start_idx as f64
        } else {
            candles[start_idx].volume as f64 // Fallback if first candle
        };
        
        // Try to expand the zone
        while end_idx < candles.len() - 1 && zone_candles < self.max_candles {
            let next_idx = end_idx + 1;
            let next_candle = &candles[next_idx];
            
            // Update potential high and low
            let potential_high = zone_high.max(next_candle.high);
            let potential_low = zone_low.min(next_candle.low);
            
            // Calculate the percentage range
            let potential_range_percent = (potential_high - potential_low) / potential_low * 100.0;
            
            // If adding this candle would make the range too large, stop expansion
            if potential_range_percent > self.max_range_percent {
                break;
            }
            
            // Update zone boundary
            zone_high = potential_high;
            zone_low = potential_low;
            volume_sum += next_candle.volume as u64;
            end_idx = next_idx;
            zone_candles += 1;
        }
        
        // Check if we've met the minimum requirements
        if zone_candles >= self.min_candles {
            let zone_volume_avg = volume_sum as f64 / zone_candles as f64;
            let price_range_percent = (zone_high - zone_low) / zone_low * 100.0;
            
            // Check for volume reduction during consolidation
            let volume_ratio = zone_volume_avg / pre_zone_volume_avg;
            
            // Only recognize as consolidation if volume is reduced or price range is very tight
            if volume_ratio <= self.volume_threshold_ratio || price_range_percent < self.max_range_percent * 0.5 {
                return Some(ConsolidationZone {
                    start_index: start_idx,
                    end_index: end_idx,
                    high: zone_high,
                    low: zone_low,
                    candle_count: zone_candles,
                    price_range_percent,
                    start_time: candles[start_idx].time.clone(),
                    end_time: candles[end_idx].time.clone(),
                    avg_volume: zone_volume_avg,
                });
            }
        }
        
        None
    }
}