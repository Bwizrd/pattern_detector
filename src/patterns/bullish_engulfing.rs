use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

pub struct BullishEngulfingRecognizer;

impl PatternRecognizer for BullishEngulfingRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Vec<serde_json::Value> {
        let mut zones = Vec::new();
        for i in 0..candles.len() - 1 {
            let first = &candles[i];
            let second = &candles[i + 1];
            let is_bullish_engulfing = first.close < first.open
                && second.close > second.open
                && second.close > first.open
                && second.open < first.close;
            if is_bullish_engulfing {
                zones.push(json!({
                    "type": "buy_zone",
                    "start_time": second.time.clone(),
                    "upper_line": second.high,
                    "lower_line": second.low,
                }));
            }
        }
        zones
    }
}