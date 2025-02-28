//src/patterns/rally.rs
use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use chrono::{DateTime, Utc};
use serde_json::json;

pub struct RallyRecognizer;

impl PatternRecognizer for RallyRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        let mut rallies = Vec::new();
        let total_bars = candles.len();
        let min_candles = 3;              // At least 3 candles needed for a rally
        let min_price_change = 0.0005;    // Minimal price change required (reduced to catch more rallies)
        
        // Need at least the minimum number of candles to find a rally
        if candles.len() < min_candles {
            return json!({
                "pattern": "rally",
                "min_candles": min_candles,
                "min_price_change": min_price_change,
                "total_bars": total_bars,
                "total_detected": 0,
                "data": [],
            });
        }

        // Look through the data candle by candle
        for i in 0..candles.len() - (min_candles - 1) {
            // Check for a simple rally pattern: price higher at end than start
            let start_price = candles[i].open;
            let mut highest_price = candles[i].high;
            let mut rally_length = 0;
            let mut valid_rally = true;
            
            // Check next min_candles to see if we have a rally
            for j in 0..min_candles {
                if i + j >= candles.len() {
                    valid_rally = false;
                    break;
                }
                
                // Track highest price seen during the sequence
                if candles[i + j].high > highest_price {
                    highest_price = candles[i + j].high;
                }
                
                rally_length = j + 1;
            }
            
            // Get end price from the last candle in the sequence
            let end_idx = i + rally_length - 1;
            let end_price = candles[end_idx].close;
            
            // Calculate total price change
            let price_change = end_price - start_price;
            
            // Is this a valid rally?
            if valid_rally && price_change >= min_price_change {
                // Format start and end times
                let end_time = DateTime::parse_from_rfc3339(&candles[end_idx].time)
                    .unwrap_or(Utc::now().into())
                    .with_timezone(&Utc);
                
                // Create rally data object
                let rally_data = json!({
                    "type": "rally",
                    "start_time": candles[i].time.clone(),
                    "end_time": end_time.to_rfc3339(),
                    "start_price": start_price,
                    "end_price": end_price,
                    "highest_price": highest_price,
                    "price_change": price_change,
                    "num_candles": rally_length,
                });
                
                rallies.push(rally_data);
            }
        }

        // Construct the response object
        json!({
            "pattern": "rally",
            "min_candles": min_candles,
            "min_price_change": min_price_change,
            "total_bars": total_bars,
            "total_detected": rallies.len(),
            "data": rallies,
        })
    }
}

#[test]
fn test_rally_recognizer() {
    let recognizer = RallyRecognizer;
    
    // Create a series of candles with multiple small rallies
    let candles = vec![
        CandleData {
            time: "2023-10-01T00:00:00Z".to_string(),
            high: 1.3010,
            low: 1.2998,
            open: 1.3000,
            close: 1.3008,
        },
        CandleData {
            time: "2023-10-01T00:01:00Z".to_string(),
            high: 1.3015,
            low: 1.3005,
            open: 1.3008,
            close: 1.3012,
        },
        CandleData {
            time: "2023-10-01T00:02:00Z".to_string(),
            high: 1.3020,
            low: 1.3010,
            open: 1.3012,
            close: 1.3018,
        },
        // Small drop
        CandleData {
            time: "2023-10-01T00:03:00Z".to_string(),
            high: 1.3015,
            low: 1.3000,
            open: 1.3018,
            close: 1.3005,
        },
        // Another small rally
        CandleData {
            time: "2023-10-01T00:04:00Z".to_string(),
            high: 1.3010,
            low: 1.3000,
            open: 1.3005,
            close: 1.3008,
        },
        CandleData {
            time: "2023-10-01T00:05:00Z".to_string(),
            high: 1.3015,
            low: 1.3007,
            open: 1.3008,
            close: 1.3014,
        },
        CandleData {
            time: "2023-10-01T00:06:00Z".to_string(),
            high: 1.3025,
            low: 1.3012,
            open: 1.3014,
            close: 1.3022,
        },
    ];

    let result = recognizer.detect(&candles);
    println!("Rally detection result: {:?}", result);
    
    // Check if multiple rallies were detected
    let data = result.get("data").unwrap().as_array().unwrap();
    assert!(!data.is_empty());
    
    // Should detect at least 2 rallies in this test data
    assert!(data.len() >= 2);
}