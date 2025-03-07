use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

pub struct SupplyDemandZoneRecognizer;

impl PatternRecognizer for SupplyDemandZoneRecognizer {
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
        // Iterate through candles to find patterns
        // Iterate through candles to find patterns
        for i in 0..candles.len().saturating_sub(2) {
            let c1 = &candles[i];
            let c2 = &candles[i + 1];
            let c3 = &candles[i + 2];

            let c1_body = (c1.close - c1.open).abs();
            let c2_body = (c2.close - c2.open).abs();

            // Demand zone: first candle bearish, second candle bullish & 50% bigger, third candle's low > first candle's high
            if c1.close < c1.open
                && c2.close > c2.open
                && c2_body >= 1.5 * c1_body
                && c3.low > c1.high
            {
                let zone_high = c1.high;
                let zone_low = c1.low;
                let mut zone_end_index = i + 2;
                // Extend the zone until a future candle's low touches or goes below zone_high
                for k in (i + 3)..candles.len() {
                    if candles[k].low <= zone_high {
                        break;
                    }
                    zone_end_index = k;
                }
                // Extend one more candle to show price entering the box, if possible
                zone_end_index = std::cmp::min(zone_end_index + 2, candles.len() - 1);
                demand_zones.push(json!({
                    "category": "Price",
                    "type": "demand_zone",
                    "start_time": c1.time,
                    "end_time": candles[zone_end_index].time,
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0
                }));
            }

            // Supply zone: first candle bullish, second candle bearish & 50% bigger, third candle's high < first candle's low
            if c1.close > c1.open
                && c2.close < c2.open
                && c2_body >= 1.5 * c1_body
                && c3.high < c1.low
            {
                let zone_high = c1.high;
                let zone_low = c1.low;
                let mut zone_end_index = i + 2;
                // Extend the zone until a future candle's high touches or exceeds zone_low
                for k in (i + 3)..candles.len() {
                    if candles[k].high >= zone_low {
                        break;
                    }
                    zone_end_index = k;
                }
                // Extend one more candle to show price entering the box, if possible
                zone_end_index = std::cmp::min(zone_end_index + 2, candles.len() - 1);
                supply_zones.push(json!({
                    "category": "Price",
                    "type": "supply_zone",
                    "start_time": c1.time,
                    "end_time": candles[zone_end_index].time,
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0
                }));
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
