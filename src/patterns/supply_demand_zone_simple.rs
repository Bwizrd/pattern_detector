// // src/patterns/supply_demand_zone_simple.rs
// use crate::detect::CandleData;
// use crate::patterns::PatternRecognizer;
// use serde_json::json;

// pub struct SimpleSupplyDemandZoneRecognizer;

// impl PatternRecognizer for SimpleSupplyDemandZoneRecognizer {
//     fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
//         // Ensure we have at least 2 candles for pattern detection
//         if candles.len() < 2 {
//             return json!({
//                 "pattern": "supply_demand_zone",
//                 "error": "Not enough candles for detection",
//                 "total_bars": candles.len(),
//                 "required_bars": 2,
//                 "total_detected": 0,
//                 "data": []
//             });
//         }

//         let mut demand_zones = Vec::new();
//         let mut supply_zones = Vec::new();

//         // Iterate through candles to find patterns
//         for i in 0..candles.len() - 1 {
//             let first_candle = &candles[i];
//             let second_candle = &candles[i + 1];

//             // Calculate body sizes
//             let first_body_size = (first_candle.close - first_candle.open).abs();
//             let second_body_size = (second_candle.close - second_candle.open).abs();

//             // Demand Zone: Bearish candle followed by a bullish candle
//             if first_candle.close < first_candle.open && second_candle.close > second_candle.open {
//                 // Check if the bullish candle is at least 50% bigger than the bearish candle
//                 if second_body_size >= 1.5 * first_body_size {
//                     demand_zones.push(json!({
//                         "type": "demand_zone",
//                         "start_time": first_candle.time,
//                         "zone_high": first_candle.high,
//                         "zone_low": first_candle.low,
//                         "fifty_percent_line": (first_candle.high + first_candle.low) / 2.0,
//                         "strength": second_body_size / first_body_size // Strength based on size ratio
//                     }));
//                 }
//             }

//             // Supply Zone: Bullish candle followed by a bearish candle
//             if first_candle.close > first_candle.open && second_candle.close < second_candle.open {
//                 // Check if the bearish candle is at least 50% bigger than the bullish candle
//                 if second_body_size >= 1.5 * first_body_size {
//                     supply_zones.push(json!({
//                         "type": "supply_zone",
//                         "start_time": first_candle.time,
//                         "zone_high": first_candle.high,
//                         "zone_low": first_candle.low,
//                         "fifty_percent_line": (first_candle.high + first_candle.low) / 2.0,
//                         "strength": second_body_size / first_body_size // Strength based on size ratio
//                     }));
//                 }
//             }
//         }

//         // Return the detected zones
//         json!({
//             "pattern": "supply_demand_zone",
//             "total_bars": candles.len(),
//             "total_detected": demand_zones.len() + supply_zones.len(),
//             "demand_zones": demand_zones,
//             "supply_zones": supply_zones,
//             "data": {
//                 "demand_zones": demand_zones,
//                 "supply_zones": supply_zones
//             }
//         })
//     }
// }
// The above splits the zones into separate objects which might be helpful later but the front-end can't process that yet.
// Below combines them into a single array of zones.

use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

pub struct SimpleSupplyDemandZoneRecognizer;

impl PatternRecognizer for SimpleSupplyDemandZoneRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        // Ensure we have at least 2 candles for pattern detection
        if candles.len() < 2 {
            return json!({
                "pattern": "supply_demand_zone",
                "error": "Not enough candles for detection",
                "total_bars": candles.len(),
                "required_bars": 2,
                "total_detected": 0,
                "data": []
            });
        }

        let mut zones = Vec::new();

        // Iterate through candles to find patterns
        for i in 0..candles.len() - 1 {
            let first_candle = &candles[i];
            let second_candle = &candles[i + 1];

            // Calculate body sizes
            let first_body_size = (first_candle.close - first_candle.open).abs();
            let second_body_size = (second_candle.close - second_candle.open).abs();

            // Demand Zone: Bearish candle followed by a bullish candle
            if first_candle.close < first_candle.open && second_candle.close > second_candle.open {
                // Check if the bullish candle is at least 50% bigger than the bearish candle
                if second_body_size >= 1.5 * first_body_size {
                    zones.push(json!({
                        "type": "demand_zone",
                        "start_time": first_candle.time,
                        "end_time": second_candle.time, // Add end_time
                        "high": first_candle.high,
                        "low": first_candle.low,
                        "fifty_percent_line": (first_candle.high + first_candle.low) / 2.0,
                        "strength": second_body_size / first_body_size // Strength based on size ratio
                    }));
                }
            }

            // Supply Zone: Bullish candle followed by a bearish candle
            if first_candle.close > first_candle.open && second_candle.close < second_candle.open {
                // Check if the bearish candle is at least 50% bigger than the bullish candle
                if second_body_size >= 1.5 * first_body_size {
                    zones.push(json!({
                        "type": "supply_zone",
                        "start_time": first_candle.time,
                        "end_time": second_candle.time, // Add end_time
                        "high": first_candle.high,
                        "low": first_candle.low,
                        "fifty_percent_line": (first_candle.high + first_candle.low) / 2.0,
                        "strength": second_body_size / first_body_size // Strength based on size ratio
                    }));
                }
            }
        }

        // Return the detected zones in a single data array
        json!({
            "pattern": "simple_supply_demand_zone",
            "total_bars": candles.len(),
            "total_detected": zones.len(),
            "data": zones
        })
    }
}