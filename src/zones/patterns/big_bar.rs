//src/patterns/big_bar.rs
use crate::api::detect::CandleData;
use crate::zones::patterns::PatternRecognizer;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

pub struct BigBarRecognizer;

impl PatternRecognizer for BigBarRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        let total_bars = candles.len();
        
        // Need at least a few candles to establish context
        if candles.len() < 5 {
            return json!({
                "pattern": "big_bar",
                "error": "Not enough candles for detection",
                "total_bars": total_bars,
                "total_detected": 0,
                "datasets": {
                    "price": 0,
                    "oscillators": 0,
                    "lines": 0
                },
                "data": {
                    "price": {
                        "big_bar_zones": {
                            "total": 0,
                            "zones": []
                        }
                    },
                    "oscillators": [],
                    "lines": []
                }
            });
        }
        
        // Calculate average candle range for the timeframe
        let mut total_range = 0.0;
        for candle in candles {
            total_range += candle.high - candle.low;
        }
        let avg_range = total_range / candles.len() as f64;
        
        // Define a big bar as one that's at least 2x the average range
        let big_bar_threshold = avg_range * 2.0;
        
        // Minimum body percentage requirement (65%)
        let min_body_percentage = 0.65;
        
        // Find big bars
        let mut big_bars = Vec::new();
        for candle in candles {
            let range = candle.high - candle.low;
            let body_size = (candle.close - candle.open).abs();
            let body_percentage = if range > 0.0 { body_size / range } else { 0.0 };
            
            // Check if this candle is significantly larger than average AND has substantial body
            if range > big_bar_threshold && body_percentage >= min_body_percentage {
                // Parse start_time for end_time calculation
                let start_time = DateTime::parse_from_rfc3339(&candle.time)
                    .unwrap_or(Utc::now().into())
                    .with_timezone(&Utc);
                
                // Estimate timeframe from data (very basic approach)
                let minutes_diff = if candles.len() > 1 {
                    let next_time = DateTime::parse_from_rfc3339(&candles[1].time)
                        .unwrap_or(Utc::now().into())
                        .with_timezone(&Utc);
                    let first_time = DateTime::parse_from_rfc3339(&candles[0].time)
                        .unwrap_or(Utc::now().into())
                        .with_timezone(&Utc);
                    
                    (next_time - first_time).num_minutes().max(1)
                } else {
                    1 // Default to 1 minute if we can't determine
                };
                
                let end_time = start_time + Duration::minutes(minutes_diff);
                
                // Calculate metrics for display
                let relative_size = (range / avg_range * 100.0).round() as i32;
                let body_percent = (body_percentage * 100.0).round() as i32;
                
                // Direction of the bar (bullish or bearish)
                let direction = if candle.close > candle.open { "Bullish" } else { "Bearish" };
                
                big_bars.push(json!({
                    "category": "Price",
                    "type": "big_bar",
                    "start_time": candle.time.clone(),
                    "end_time": end_time.to_rfc3339(),
                    "zone_high": candle.high,
                    "zone_low": candle.low,
                    "fifty_percent_line": (candle.high + candle.low) / 2.0,
                    "detection_method": format!("{} Big Bar ({relative_size}% size, {body_percent}% body)", direction)
                }));
            }
        }

        // Construct the response
        json!({
            "pattern": "big_bar",
            "total_bars": total_bars,
            "total_detected": big_bars.len(),
            "avg_range": avg_range,
            "big_bar_threshold": big_bar_threshold,
            "min_body_percentage": min_body_percentage,
            "datasets": {
                "price": 1,
                "oscillators": 0,
                "lines": 0
            },
            "data": {
                "price": {
                    "big_bar_zones": {
                        "total": big_bars.len(),
                        "zones": big_bars
                    }
                },
                "oscillators": [],
                "lines": []
            }
        })
    }
}