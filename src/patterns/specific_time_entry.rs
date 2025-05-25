// src/patterns/specific_time_entry.rs

use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use crate::trades::{Trade, TradeConfig, TradeSummary};
use crate::trading::TradeExecutor;
use serde_json::{json, Value};
use chrono::{DateTime, Timelike, Datelike, Utc, Weekday}; // Import needed chrono items

pub struct SpecificTimeEntryRecognizer; // No config needed

impl Default for SpecificTimeEntryRecognizer {
    fn default() -> Self { Self }
}

impl PatternRecognizer for SpecificTimeEntryRecognizer {
    // Detect method is simple: just identifies the specific candles
    fn detect(&self, candles: &[CandleData]) -> Value {
        log::info!("Specific Time DETECT: Running detection...");
        let mut events = Vec::new();
        let pattern_name = "specific_time_entry";

        for (idx, candle) in candles.iter().enumerate() {
            // Attempt to parse the timestamp string
             if let Ok(dt_utc) = DateTime::parse_from_rfc3339(&candle.time).map(|dt| dt.with_timezone(&Utc)) {
                 let weekday = dt_utc.weekday();
                 let hour = dt_utc.hour();
                 let minute = dt_utc.minute();

                // Check for Buy Signal: Tuesday 16:40 (4:40 PM)
                if weekday == Weekday::Tue && hour == 16 && minute == 40 {
                    log::info!("Specific Time DETECT: Found Buy Signal candle at index {}, time {}", idx, candle.time);
                    events.push(json!({
                        "time": candle.time.clone(),
                        "type": "Buy Time",
                        "candle_index": idx,
                    }));
                }
                // Check for Sell Signal: Wednesday 12:30 PM
                else if weekday == Weekday::Wed && hour == 12 && minute == 30 {
                     log::info!("Specific Time DETECT: Found Sell Signal candle at index {}, time {}", idx, candle.time);
                     events.push(json!({
                        "time": candle.time.clone(),
                        "type": "Sell Time",
                        "candle_index": idx,
                    }));
                }
            } else {
                 log::warn!("Specific Time DETECT: Could not parse timestamp: {}", candle.time);
             }
        }
        log::info!("Specific Time DETECT: Found {} specific time events.", events.len());

        // Return simple structure
        json!({
            "pattern": pattern_name,
            "total_bars": candles.len(),
            "total_detected": events.len(),
            "datasets": { "price": 0, "oscillators": 0, "lines": 0, "events": 1 },
            "data": {
                "events": { "signals": events } // Use "signals" key
            }
        })
    }


    // Trade method delegates directly to executor
    fn trade(&self, candles: &[CandleData], config: TradeConfig) -> (Vec<Trade>, TradeSummary) {
        log::info!("Specific Time TRADE: Delegating to executor.");
        let pattern_data = self.detect(candles);
        let default_symbol_for_recognizer_trade = "UNKNOWN_SYMBOL_IN_RECOGNIZER"; 
        let executor = TradeExecutor::new(config, 
            default_symbol_for_recognizer_trade,
            None,
            None);
        // Note: executor needs minute data set externally if needed for exits
        let trades = executor.execute_trades_for_pattern(
            "specific_time_entry", // Use correct pattern name
            &pattern_data,
            candles
        );
        let summary = TradeSummary::from_trades(&trades);
        (trades, summary)
    }
} // End impl PatternRecognizer

// Ensure TradeSummary::default() exists or add it