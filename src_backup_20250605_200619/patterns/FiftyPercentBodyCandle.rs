use crate::api::detect::CandleData;
use crate::zones::patterns::PatternRecognizer;
use serde_json::json;

pub struct FiftyPercentBodyCandleRecognizer {
    // Configuration parameter for % threshold
    max_body_size_percent: f64,
}

impl Default for FiftyPercentBodyCandleRecognizer {
    fn default() -> Self {
        Self {
            max_body_size_percent: 50.0,
        }
    }
}

impl PatternRecognizer for FiftyPercentBodyCandleRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        // Ensure we have at least 1 candle
        if candles.is_empty() {
            return json!({
                "pattern": "fifty_percent_body_candle",
                "error": "Not enough candles for detection",
                "total_bars": 0,
                "required_bars": 1,
                "total_detected": 0,
                "datasets": {
                    "price": 0,
                    "oscillators": 0,
                    "lines": 0
                },
                "data": {
                    "price": {
                        "fifty_percent_candles_zones": {
                            "total": 0,
                            "zones": []
                        }
                    },
                    "oscillators": [],
                    "lines": []
                }
            });
        }
        
        // Identify all 50% body candles
        let mut fifty_percent_candles = Vec::new();
        
        for (_i, candle) in candles.iter().enumerate() {
            let body_size = (candle.close - candle.open).abs();
            let candle_range = candle.high - candle.low;
            
            // Check if body is less than or equal to max_body_size_percent of total range
            if candle_range > 0.0 && body_size <= (candle_range * self.max_body_size_percent / 100.0) {
                // Important: Adapt to the zone format expected by the chart rendering
                fifty_percent_candles.push(json!({
                    "category": "Price",
                    "type": "fifty_percent_candle",
                    "start_time": candle.time,
                    "end_time": candle.time, // Same as start for individual candles
                    "zone_high": candle.high,
                    "zone_low": candle.low,
                    "fifty_percent_line": (candle.high + candle.low) / 2.0,
                    "body_size_percent": (body_size / candle_range * 100.0).round(),
                    "detection_method": "50% body candle"
                }));
            }
        }
        
        // Count total candles found
        let total_detected = fifty_percent_candles.len();
        
        // Return the result with proper structure matching the chart expectations
        json!({
            "pattern": "fifty_percent_body_candle",
            "total_bars": candles.len(),
            "total_detected": total_detected,
            "datasets": {
                "price": 1,
                "oscillators": 0,
                "lines": 0
            },
            "data": {
                "price": {
                    "fifty_percent_candles_zones": {
                        "total": total_detected,
                        "zones": fifty_percent_candles
                    }
                },
                "oscillators": [],
                "lines": []
            }
        })
    }
}