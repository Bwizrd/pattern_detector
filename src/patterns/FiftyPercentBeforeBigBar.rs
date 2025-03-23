use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

pub struct FiftyPercentBeforeBigBarRecognizer {
    // Configuration parameters
    max_body_size_percent: f64,    // For 50% body candles (default 50%)
    min_big_bar_body_percent: f64, // For big bars (default 65%)
}

impl Default for FiftyPercentBeforeBigBarRecognizer {
    fn default() -> Self {
        Self {
            max_body_size_percent: 50.0,
            min_big_bar_body_percent: 0.65,
        }
    }
}

impl PatternRecognizer for FiftyPercentBeforeBigBarRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        let total_bars = candles.len();
        
        // Need at least 2 candles for the pattern (50% body + big bar)
        if candles.len() < 2 {
            return json!({
                "pattern": "fifty_percent_before_big_bar",
                "error": "Not enough candles for detection",
                "total_bars": total_bars,
                "required_bars": 2,
                "total_detected": 0,
                "datasets": {
                    "price": 2,
                    "oscillators": 0,
                    "lines": 0
                },
                "data": {
                    "price": {
                        "supply_zones": {
                            "total": 0,
                            "zones": []
                        },
                        "demand_zones": {
                            "total": 0,
                            "zones": []
                        }
                    },
                    "oscillators": [],
                    "lines": []
                }
            });
        }
        
        // Calculate average candle range for big bar detection
        let mut total_range = 0.0;
        for candle in candles {
            total_range += candle.high - candle.low;
        }
        let avg_range = total_range / candles.len() as f64;
        let big_bar_threshold = avg_range * 2.0;
        
        let mut supply_zones = Vec::new();
        let mut demand_zones = Vec::new();
        
        // Scan for pattern: 50% body candle followed directly by a big bar
        for i in 0..candles.len().saturating_sub(1) {
            let fifty_candle = &candles[i];
            let big_candle = &candles[i + 1];
            
            // Check if first candle is a 50% body candle
            let fifty_body_size = (fifty_candle.close - fifty_candle.open).abs();
            let fifty_range = fifty_candle.high - fifty_candle.low;
            let is_fifty_body = fifty_range > 0.0 && 
                                fifty_body_size <= (fifty_range * self.max_body_size_percent / 100.0);
            
            if !is_fifty_body {
                continue; // Skip if not a 50% body candle
            }
            
            // Check if second candle is a big bar with substantial body
            let big_range = big_candle.high - big_candle.low;
            let big_body_size = (big_candle.close - big_candle.open).abs();
            let big_body_percentage = if big_range > 0.0 { big_body_size / big_range } else { 0.0 };
            
            let is_big_bar = big_range > big_bar_threshold && 
                             big_body_percentage >= self.min_big_bar_body_percent;
            
            if !is_big_bar {
                continue; // Skip if not a big bar
            }
            
            // We found a pattern! Now determine if it's a supply or demand zone based on the big bar direction
            let is_bullish_big_bar = big_candle.close > big_candle.open;
            
            // Create zone using ONLY the 50% body candle's range
            if is_bullish_big_bar {
                // Bullish big bar = demand zone
                let zone_low = fifty_candle.low;
                let zone_high = fifty_candle.high;
                
                // Find how far to extend the zone (until price touches or breaks the zone)
                let mut zone_end_index = i + 1;
                for j in (i + 2)..candles.len() {
                    if candles[j].low <= zone_high && candles[j].high >= zone_low {
                        // Price has touched the zone, include this candle plus one more
                        zone_end_index = j.saturating_add(1).min(candles.len() - 1);
                        break;
                    }
                    zone_end_index = j;
                }
                
                // Calculate relative size for display
                let relative_size = (big_range / avg_range * 100.0).round() as i32;
                let body_percent = (big_body_percentage * 100.0).round() as i32;
                
                demand_zones.push(json!({
                    "category": "Price",
                    "type": "demand_zone",
                    "start_time": fifty_candle.time.clone(),
                    "end_time": candles[zone_end_index].time.clone(),
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                    "detection_method": format!("50% Candle → Bullish Big Bar ({relative_size}% size, {body_percent}% body)"),
                    "quality_score": 75.0, // High quality by default since it meets our strict criteria
                    "strength": "Strong"
                }));
            } else {
                // Bearish big bar = supply zone
                let zone_high = fifty_candle.high;
                let zone_low = fifty_candle.low;
                
                // Find how far to extend the zone (until price touches or breaks the zone)
                let mut zone_end_index = i + 1;
                for j in (i + 2)..candles.len() {
                    if candles[j].low <= zone_high && candles[j].high >= zone_low {
                        // Price has touched the zone, include this candle plus one more
                        zone_end_index = j.saturating_add(1).min(candles.len() - 1);
                        break;
                    }
                    zone_end_index = j;
                }
                
                // Calculate relative size for display
                let relative_size = (big_range / avg_range * 100.0).round() as i32;
                let body_percent = (big_body_percentage * 100.0).round() as i32;
                
                supply_zones.push(json!({
                    "category": "Price",
                    "type": "supply_zone",
                    "start_time": fifty_candle.time.clone(),
                    "end_time": candles[zone_end_index].time.clone(),
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                    "detection_method": format!("50% Candle → Bearish Big Bar ({relative_size}% size, {body_percent}% body)"),
                    "quality_score": 75.0, // High quality by default since it meets our strict criteria
                    "strength": "Strong"
                }));
            }
        }
        
        // Construct the response
        json!({
            "pattern": "fifty_percent_before_big_bar",
            "total_bars": total_bars,
            "total_detected": supply_zones.len() + demand_zones.len(),
            "avg_range": avg_range,
            "big_bar_threshold": big_bar_threshold,
            "datasets": {
                "price": 2, // Two datasets: supply_zones and demand_zones
                "oscillators": 0,
                "lines": 0
            },
            "data": {
                "price": {
                    "supply_zones": {
                        "total": supply_zones.len(),
                        "zones": supply_zones
                    },
                    "demand_zones": {
                        "total": demand_zones.len(),
                        "zones": demand_zones
                    }
                },
                "oscillators": [],
                "lines": []
            }
        })
    }
}