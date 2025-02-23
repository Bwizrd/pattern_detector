use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

pub struct PinBarRecognizer;

impl PatternRecognizer for PinBarRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        let mut zones = Vec::new();
        let total_bars = candles.len();
        for i in 0..candles.len() {
            let candle = &candles[i];
            let body = (candle.close - candle.open).abs();
            let upper_wick = candle.high - candle.close.max(candle.open);
            let lower_wick = candle.open.min(candle.close) - candle.low;
            let is_bullish_pin = lower_wick > 2.0 * body && upper_wick < body && candle.close > candle.open;
            if is_bullish_pin {
                zones.push(json!({
                    "type": "pin_bar",
                    "start_time": candle.time.clone(),
                    "high": candle.high,
                    "low": candle.low,
                    "is_bullish": true
                }));
            }
        }
        json!({
            "pattern": "pin_bar",
            "total_bars": total_bars,
            "total_detected": zones.len(),
            "data": zones,
        })
    }
}