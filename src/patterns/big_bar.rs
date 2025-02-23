use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

pub struct BigBarRecognizer;

impl PatternRecognizer for BigBarRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Vec<serde_json::Value> {
        let mut big_bars = Vec::new();
        let pip_value = 0.0002; // Define the pip value for this pattern
        let total_bars = candles.len();

        for candle in candles {
            let range = candle.high - candle.low;
            if range > pip_value {
                big_bars.push(json!({
                    "type": "big_bar",
                    "start_time": candle.time.clone(),
                    "high": candle.high,
                    "low": candle.low,
                }));
            }
        }

        // Construct the enhanced response
        let response = json!({
            "pattern": "big_bar",
            "pip_value": pip_value,
            "total_bars": total_bars,
            "total_detected": big_bars.len(),
            "data": big_bars,
        });

        vec![response] // Return the response as a Vec<Value>
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
        },
        CandleData {
            time: "2023-10-01T01:00:00Z".to_string(),
            high: 1.3005,
            low: 1.3000,
            open: 1.3001,
            close: 1.3003,
        },
        CandleData {
            time: "2023-10-01T02:00:00Z".to_string(),
            high: 1.3010,
            low: 1.3005,
            open: 1.3006,
            close: 1.3008,
        },
    ];

    let result = recognizer.detect(&candles);
    println!("Detection result: {:?}", result); // Log the result
    assert!(!result.is_empty()); // Ensure at least one big bar is detected
}
