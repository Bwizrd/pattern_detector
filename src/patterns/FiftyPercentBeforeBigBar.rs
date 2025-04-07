// src/patterns/FiftyPercentBeforeBigBar.rs
use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use crate::trading::TradeExecutor;
use crate::trades::{Trade, TradeConfig, TradeSummary};
use serde_json::json;
use log; // Ensure log crate is available (add to Cargo.toml if needed)

pub struct FiftyPercentBeforeBigBarRecognizer {
    // Configuration parameters
    max_body_size_percent: f64,    // Max body size for the setup candle (relative to its own range)
    min_big_bar_body_percent: f64, // Min body size for the big bar (relative to its own range)
    big_bar_range_factor: f64,     // Min range of big bar relative to average range (e.g., 2.0 for 2x avg)
    max_fifty_candle_range_factor: f64, // **NEW**: Max range of the 50% candle relative to average range
    require_breakout_close: bool,   // **NEW**: Require big bar to close beyond the 50% candle's high/low
    proximal_extension_percent: f64, // How much to extend the proximal side
}

impl Default for FiftyPercentBeforeBigBarRecognizer {
    fn default() -> Self {
        Self {
            max_body_size_percent: 50.0,          // <= 50% body for setup candle
            min_big_bar_body_percent: 0.65,       // >= 65% body for big bar
            big_bar_range_factor: 2.0,            // Big bar range must be >= 2 * avg_range
            max_fifty_candle_range_factor: 1.75,  // **NEW**: 50% candle range <= 1.75 * avg_range
            require_breakout_close: true,         // **NEW**: Enforce the close breakout by default
            proximal_extension_percent: 0.02,     // Default 0.02% extension
        }
    }
}

impl PatternRecognizer for FiftyPercentBeforeBigBarRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        let total_bars = candles.len();

        // Need at least 2 candles for the pattern
        if candles.len() < 2 {
             return json!({
                "pattern": "fifty_percent_before_big_bar",
                "error": "Not enough candles for detection",
                "total_bars": total_bars,
                "required_bars": 2,
                "total_detected": 0,
                "config": self.get_config_json(), // Include config even on error
                "datasets": { "price": 2, "oscillators": 0, "lines": 0 },
                "data": {
                    "price": {
                        "supply_zones": { "total": 0, "zones": [] },
                        "demand_zones": { "total": 0, "zones": [] }
                    },
                    "oscillators": [], "lines": []
                }
             });
        }

        // Calculate average candle range
        let mut total_range = 0.0;
        let mut valid_candles_for_avg = 0;
        for candle in candles {
             let range = candle.high - candle.low;
             if range > 0.0 { // Only include candles with a non-zero range
                 total_range += range;
                 valid_candles_for_avg += 1;
             }
        }

        // Ensure we can calculate a meaningful average range
        let avg_range = if valid_candles_for_avg > 0 && total_range > 0.0 {
            total_range / valid_candles_for_avg as f64
        } else {
            0.0 // Cannot calculate average or thresholds
        };

        // Return early if avg_range could not be calculated meaningfully
        if avg_range <= 1e-9 { // Use a small epsilon instead of strict zero for float comparison
             log::warn!("Average range ({}) is too small or zero, cannot perform detection.", avg_range);
             return json!({
                "pattern": "fifty_percent_before_big_bar",
                "error": "Could not calculate a meaningful average range from the provided candles.",
                "total_bars": total_bars,
                "avg_range": avg_range,
                "required_bars": 2,
                "total_detected": 0,
                "config": self.get_config_json(), // Include config
                "datasets": { "price": 2, "oscillators": 0, "lines": 0 },
                "data": {
                    "price": {
                        "supply_zones": { "total": 0, "zones": [] },
                        "demand_zones": { "total": 0, "zones": [] }
                    },
                    "oscillators": [], "lines": []
                }
             });
        }

        let big_bar_threshold = avg_range * self.big_bar_range_factor;
        let max_fifty_range_threshold = avg_range * self.max_fifty_candle_range_factor;

        let mut supply_zones = Vec::new();
        let mut demand_zones = Vec::new();

        for i in 0..candles.len().saturating_sub(1) {
            let fifty_candle = &candles[i];
            let big_candle = &candles[i + 1];

            // --- Refined Checks ---

            // 1. Check 50% Candle Range Size (Absolute and Relative to Avg)
            let fifty_range = fifty_candle.high - fifty_candle.low;
            if fifty_range <= 1e-9 { // Skip effectively zero-range candles
                 // log::trace!("Skipping i={}: fifty_candle range ({}) is zero or negative", i, fifty_range);
                 continue;
            }
            if fifty_range > max_fifty_range_threshold {
                // log::trace!("Skipping i={}: fifty_candle range ({:.5}) exceeds max threshold ({:.5})", i, fifty_range, max_fifty_range_threshold);
                continue; // 50% candle is too large overall compared to average
            }


            // 2. Check 50% Candle Body Percentage (Relative to its own range)
            let fifty_body_size = (fifty_candle.close - fifty_candle.open).abs();
            if fifty_body_size > (fifty_range * self.max_body_size_percent / 100.0) {
                // log::trace!("Skipping i={}: fifty_candle body size ({:.5}) exceeds max percent ({}) of its range ({:.5})", i, fifty_body_size, self.max_body_size_percent, fifty_range);
                continue; // Body is too large relative to its own range
            }

            // 3. Check Big Bar Range Size (Absolute and Relative to Avg)
            let big_range = big_candle.high - big_candle.low;
             if big_range <= 1e-9 { // Skip effectively zero-range candles
                 // log::trace!("Skipping i={}: big_candle range ({}) is zero or negative", i, big_range);
                 continue;
             }
            if big_range <= big_bar_threshold {
                // log::trace!("Skipping i={}: big_candle range ({:.5}) does not exceed threshold ({:.5})", i, big_range, big_bar_threshold);
                continue; // Following bar is not a "big bar" relative to average
            }

            // 4. Check Big Bar Body Percentage (Relative to its own range)
            let big_body_size = (big_candle.close - big_candle.open).abs();
            let big_body_percentage = big_body_size / big_range; // Safe because big_range > 0 here
            if big_body_percentage < self.min_big_bar_body_percent {
                 // log::trace!("Skipping i={}: big_candle body percentage ({:.2}) is below minimum ({:.2})", i, big_body_percentage, self.min_big_bar_body_percent);
                continue; // Big bar body is too small relative to its range
            }

            // --- Determine Direction and Apply Confirmation Check ---
            let is_bullish_big_bar = big_candle.close > big_candle.open;

            // 5. **NEW** Confirmation Check: Does the big bar close beyond the 50% candle's range?
            let is_confirmed = if !self.require_breakout_close {
                true // Skip check if not required
            } else if is_bullish_big_bar {
                big_candle.close > fifty_candle.high // Must close CLEARLY above the high for demand
            } else { // Is Bearish Big Bar
                big_candle.close < fifty_candle.low // Must close CLEARLY below the low for supply
            };

            if !is_confirmed {
                 // log::trace!("Skipping i={}: big_candle close ({:.5}) did not confirm breakout of fifty_candle range [{:.5}, {:.5}]", i, big_candle.close, fifty_candle.low, fifty_candle.high);
                continue; // Failed confirmation breakout
            }

            // --- If all checks pass, create the zone ---
            log::info!( // Log only confirmed patterns
                "Pattern candidate CONFIRMED at i={}: fifty_time={}, big_time={}, big_open={:.5}, big_close={:.5}, fifty_low={:.5}, fifty_high={:.5}",
                i, fifty_candle.time, big_candle.time, big_candle.open, big_candle.close, fifty_candle.low, fifty_candle.high
            );

            let relative_size = (big_range / avg_range * 100.0).round() as i32;
            let body_percent = (big_body_percentage * 100.0).round() as i32;

            // Calculate zone end index common logic
            let zone_low_base = fifty_candle.low;
            let zone_high_base = fifty_candle.high;
             let mut zone_end_index = i + 1;
             for j in (i + 2)..candles.len() {
                 // A simple check: if the candle's range overlaps the base zone range
                 if candles[j].low <= zone_high_base && candles[j].high >= zone_low_base {
                     zone_end_index = j.saturating_add(1).min(candles.len() - 1);
                     break; // Stop extending once price revisits the base zone area
                 }
                 // If no interaction, the end index keeps advancing (up to the last candle)
                zone_end_index = j;
             }
             // Ensure end_index is at least i+1 (the big bar itself)
             zone_end_index = zone_end_index.max(i + 1);
             // Ensure end_index doesn't exceed bounds
             zone_end_index = zone_end_index.min(candles.len() - 1);


            if is_bullish_big_bar { // Create Demand Zone
                 log::info!(" -> Decision: Creating DEMAND zone.");
                 let zone_low = fifty_candle.low;
                 let extension_amount = if self.proximal_extension_percent > 0.0 { zone_high_base * (self.proximal_extension_percent / 100.0) } else { 0.0 };
                 let zone_high = zone_high_base + extension_amount;

                 let method_text = format!(
                    "(i={i}) 50% Candle → Bullish Big Bar ({size}%, {body}% body{}){}",
                    if self.require_breakout_close { ", Confirmed" } else { "" }, // Add 'Confirmed' if check was applied
                    if self.proximal_extension_percent > 0.0 { format!(", {:.2}% ext", self.proximal_extension_percent) } else { "".to_string() },
                    size = relative_size, body = body_percent
                 );

                 demand_zones.push(json!({
                     "category": "Price",
                     "type": "demand_zone",
                     "start_time": fifty_candle.time.clone(),
                     "end_time": candles[zone_end_index].time.clone(),
                     "zone_high": zone_high,
                     "zone_low": zone_low,
                     "fifty_percent_line": (zone_high + zone_low) / 2.0,
                     "start_idx": i,
                     "end_idx": zone_end_index,
                     "detection_method": method_text,
                     "quality_score": if is_confirmed { 85.0 } else { 70.0 }, // Example score boost for confirmation
                     "strength": if is_confirmed { "Very Strong" } else { "Strong" }, // Example strength boost
                     "extended": self.proximal_extension_percent > 0.0,
                     "extension_percent": self.proximal_extension_percent
                 }));

            } else { // Create Supply Zone
                 log::info!(" -> Decision: Creating SUPPLY zone.");
                 let zone_high = fifty_candle.high;
                 let extension_amount = if self.proximal_extension_percent > 0.0 { zone_low_base * (self.proximal_extension_percent / 100.0) } else { 0.0 };
                 let zone_low_extended = zone_low_base - extension_amount;

                 let method_text = format!(
                    "(i={i}) 50% Candle → Bearish Big Bar ({size}%, {body}% body{}){}",
                    if self.require_breakout_close { ", Confirmed" } else { "" }, // Add 'Confirmed' if check was applied
                    if self.proximal_extension_percent > 0.0 { format!(", {:.1}% ext", self.proximal_extension_percent) } else { "".to_string() },
                     size = relative_size, body = body_percent
                 );

                 supply_zones.push(json!({
                     "category": "Price",
                     "type": "supply_zone",
                     "start_time": fifty_candle.time.clone(),
                     "end_time": candles[zone_end_index].time.clone(),
                     "zone_high": zone_high,
                     "zone_low": zone_low_extended,
                     "fifty_percent_line": (zone_high + zone_low_extended) / 2.0,
                     "start_idx": i,
                     "end_idx": zone_end_index,
                     "detection_method": method_text,
                     "quality_score": if is_confirmed { 85.0 } else { 70.0 }, // Example score boost for confirmation
                     "strength": if is_confirmed { "Very Strong" } else { "Strong" }, // Example strength boost
                     "extended": self.proximal_extension_percent > 0.0,
                     "extension_percent": self.proximal_extension_percent
                 }));
            }
        } // End of for loop

        // Construct the final response JSON
        json!({
            "pattern": "fifty_percent_before_big_bar",
            "config": self.get_config_json(), // Include config details used for this detection run
            "total_bars": total_bars,
            "total_detected": supply_zones.len() + demand_zones.len(),
            "avg_range": avg_range, // Include calculated avg_range
            "big_bar_threshold": big_bar_threshold, // Include calculated threshold
            "max_fifty_range_threshold": max_fifty_range_threshold, // Include calculated threshold
            "datasets": { "price": 2, "oscillators": 0, "lines": 0 },
            "data": {
                "price": {
                    "supply_zones": { "total": supply_zones.len(), "zones": supply_zones },
                    "demand_zones": { "total": demand_zones.len(), "zones": demand_zones }
                },
                "oscillators": [], "lines": []
            }
        })
    }

    // trade function remains the same
    fn trade(&self, candles: &[CandleData], config: TradeConfig) -> (Vec<Trade>, TradeSummary) {
        let pattern_data = self.detect(candles);
        let executor = TradeExecutor::new(config);
        let trades = executor.execute_trades_for_pattern("fifty_percent_before_big_bar", &pattern_data, candles);
        let summary = TradeSummary::from_trades(&trades);
        (trades, summary)
    }
}

// Helper implementation to get config as JSON easily
impl FiftyPercentBeforeBigBarRecognizer {
    fn get_config_json(&self) -> serde_json::Value {
        json!({
             "max_body_size_percent": self.max_body_size_percent,
             "min_big_bar_body_percent": self.min_big_bar_body_percent,
             "big_bar_range_factor": self.big_bar_range_factor,
             "max_fifty_candle_range_factor": self.max_fifty_candle_range_factor,
             "require_breakout_close": self.require_breakout_close,
             "proximal_extension_percent": self.proximal_extension_percent,
        })
    }
}