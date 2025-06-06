use crate::api::detect::CandleData;
use crate::zones::patterns::PatternRecognizer;
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
                "datasets": {
                    "Price": 0,
                    "Oscillators": 0,
                    "Lines": 0
                },
                "data": {
                    "price": {
                        "supply_zones": {
                            "Total": 0,
                            "zones": []
                        },
                        "demand_zones": {
                            "Total": 0,
                            "zones": []
                        }
                    },
                    "oscillators": [],
                    "lines": []
                }
            });
        }

        let mut supply_zones = Vec::new();
        let mut demand_zones = Vec::new();

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
                    demand_zones.push(json!({
                        "category": "Price",
                        "type": "demand_zone",
                        "start_time": first_candle.time,
                        "end_time": second_candle.time,
                        "zone_high": first_candle.high,
                        "zone_low": first_candle.low,
                        "fifty_percent_line": (first_candle.high + first_candle.low) / 2.0
                    }));
                }
            }

            // Supply Zone: Bullish candle followed by a bearish candle
            if first_candle.close > first_candle.open && second_candle.close < second_candle.open {
                // Check if the bearish candle is at least 50% bigger than the bullish candle
                if second_body_size >= 1.5 * first_body_size {
                    supply_zones.push(json!({
                        "category": "Price",
                        "type": "supply_zone",
                        "start_time": first_candle.time,
                        "end_time": second_candle.time,
                        "zone_high": first_candle.high,
                        "zone_low": first_candle.low,
                        "fifty_percent_line": (first_candle.high + first_candle.low) / 2.0
                    }));
                }
            }
        }

        // Count the number of zones
        let supply_zone_count = supply_zones.len();
        let demand_zone_count = demand_zones.len();

        // Return the detected zones and dataset information
        json!({
            "pattern": "supply_demand_zone",
            "total_bars": candles.len(),
            "total_detected": supply_zone_count + demand_zone_count,
            "datasets": {
                "price": 2, // Two datasets: supply_zones and demand_zones
                "oscillators": 0, // No oscillators in this pattern
                "lines": 0        // No trendlines in this pattern
            },
            "data": {
                "price": {
                    "supply_zones": {
                        "total": supply_zone_count,
                        "zones": supply_zones
                    },
                    "demand_zones": {
                        "total": demand_zone_count,
                        "zones": demand_zones
                    }
                },
                "oscillators": [], // Empty array for oscillators
                "lines": []        // Empty array for trendlines
            }
        })
    }
}