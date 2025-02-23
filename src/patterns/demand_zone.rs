use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

pub struct DemandZoneRecognizer;

impl PatternRecognizer for DemandZoneRecognizer {
    fn detect(&self, candles: &[CandleData])  -> serde_json::Value {
        let mut zones = Vec::new();
        let total_bars = candles.len();
        for i in 0..candles.len() - 2 {
            let first = &candles[i];
            let second = &candles[i + 1];
            let third = &candles[i + 2];
            let is_correct_pattern = first.close < first.open
                && second.close > second.open
                && third.close > third.open;
            let has_imbalance = third.low > first.high;
            if is_correct_pattern && has_imbalance {
                zones.push(json!({
                    "type": "buy_zone",
                    "start_time": first.time.clone(),
                    "upper_line": first.high,
                    "lower_line": first.low,
                }));
            }
        }
        json!({
            "pattern": "demand_zone",
            "total_bars": total_bars,
            "total_detected": zones.len(),
            "data": zones,
        })
    }
}