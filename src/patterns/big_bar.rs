//src/patterns/big_bar.rs
use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

pub struct BigBarRecognizer;

impl PatternRecognizer for BigBarRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        let mut big_bars = Vec::new();
        let pip_value = 0.0002; // Define the pip value for this pattern
        let total_bars = candles.len();

        for candle in candles {
            let range = candle.high - candle.low;
            if range > pip_value {
                // Parse start_time and add 1 minute for end_time
                let start_time = DateTime::parse_from_rfc3339(&candle.time)
                    .unwrap_or(Utc::now().into())
                    .with_timezone(&Utc);
                let end_time = start_time + Duration::minutes(1); // Assuming 1m timeframe
                big_bars.push(json!({
                    "type": "big_bar",
                    "start_time": candle.time.clone(),
                    "end_time": end_time.to_rfc3339(), // ISO 8601 with 'Z'
                    "high": candle.high,
                    "low": candle.low,
                }));
            }
        }

        // Construct the response object
        json!({
            "pattern": "big_bar",
            "pip_value": pip_value,
            "total_bars": total_bars,
            "total_detected": big_bars.len(),
            "data": big_bars,
        })
    }
}

#[test]
fn test_big_bar_recognizer() {
    let recognizer = BigBarRecognizer;
    let candles = vec![
        CandleData {
            time: "2023-10-01T00:00:00Z".to_string(),
            high: 1.3100,
            low: 1.2998,
            open: 1.2999,
            close: 1.29995,
            volume: 110
        },
        CandleData {
            time: "2023-10-01T00:01:00Z".to_string(), // Adjusted to 1-minute increments
            high: 1.3005,
            low: 1.3000,
            open: 1.3001,
            close: 1.3003,
            volume: 90
        },
        CandleData {
            time: "2023-10-01T00:02:00Z".to_string(), // Adjusted to 1-minute increments
            high: 1.3010,
            low: 1.3005,
            open: 1.3006,
            close: 1.3008,
            volume: 100
        },
    ];

    let result = recognizer.detect(&candles);
    println!("Detection result: {:?}", result);
    let data = result.get("data").unwrap().as_array().unwrap();
    assert!(!data.is_empty());
}
