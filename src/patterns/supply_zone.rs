use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

pub struct SupplyZoneRecognizer;

impl PatternRecognizer for SupplyZoneRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Vec<serde_json::Value> {
        let mut zones = Vec::new();
        for i in 0..candles.len() - 2 {
            let first = &candles[i];
            let second = &candles[i + 1];
            let third = &candles[i + 2];
            let is_correct_pattern = first.close > first.open
                && second.close < second.open
                && third.close < third.open;
            let has_imbalance = third.high < first.low;
            if is_correct_pattern && has_imbalance {
                zones.push(json!({
                    "type": "sell_zone",
                    "start_time": first.time.clone(),
                    "upper_line": first.high,
                    "lower_line": first.low,
                }));
            }
        }
        zones
    }
}