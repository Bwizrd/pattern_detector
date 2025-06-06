// src/patterns/FiftyPercentBeforeBigBar.rs
use crate::api::detect::CandleData;
use crate::zones::patterns::PatternRecognizer;
use crate::trading::trades::{Trade, TradeConfig, TradeSummary};
use crate::trading::trading::TradeExecutor;
use log::{error, info, trace, warn, debug};
use serde_json::json; // Ensure log crate is available (add to Cargo.toml if needed)

pub struct FiftyPercentBeforeBigBarRecognizer {
    // Configuration parameters
    max_body_size_percent: f64, // Max body size for the setup candle (relative to its own range)
    min_big_bar_body_percent: f64, // Min body size for the big bar (relative to its own range)
    big_bar_range_factor: f64, // Min range of big bar relative to average range (e.g., 2.0 for 2x avg)
    max_fifty_candle_range_factor: f64, // **NEW**: Max range of the 50% candle relative to average range
    require_breakout_close: bool, // **NEW**: Require big bar to close beyond the 50% candle's high/low
    proximal_extension_percent: f64, // How much to extend the proximal side
}

impl Default for FiftyPercentBeforeBigBarRecognizer {
    fn default() -> Self {
        Self {
            max_body_size_percent: 50.0,         // <= 50% body for setup candle
            min_big_bar_body_percent: 0.65,      // >= 65% body for big bar
            big_bar_range_factor: 2.0,           // Big bar range must be >= 2 * avg_range
            max_fifty_candle_range_factor: 1.75, // **NEW**: 50% candle range <= 1.75 * avg_range
            require_breakout_close: true,        // **NEW**: Enforce the close breakout by default
            proximal_extension_percent: 0.02,    // Default 0.02% extension
        }
    }
}

impl PatternRecognizer for FiftyPercentBeforeBigBarRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        let total_bars = candles.len();

        if candles.len() < 2 {
            return json!({
               "pattern": "fifty_percent_before_big_bar",
               "error": "Not enough candles for detection",
               "total_bars": total_bars,
               "required_bars": 2,
               "total_detected": 0,
               "config": self.get_config_json(),
               "datasets": { "price": 2, "oscillators": 0, "lines": 0 },
               "data": {
                   "price": {
                       "supply_zones": { "total": 0, "zones": [] },
                       "demand_zones": { "total": 0, "zones": [] }
                   }, "oscillators": [], "lines": []
               }
            });
        }

        let mut total_range = 0.0;
        let mut valid_candles_for_avg = 0;
        for candle in candles {
            let range = candle.high - candle.low;
            if range > 1e-9 { // Use epsilon for float comparison
                total_range += range;
                valid_candles_for_avg += 1;
            }
        }

        let avg_range = if valid_candles_for_avg > 0 && total_range > 1e-9 {
            total_range / valid_candles_for_avg as f64
        } else {
            0.0
        };

        if avg_range <= 1e-9 {
            warn!( // Changed to warn as it might be a data issue not an error in logic
                "[FiftyPercentRecognizer] Average range ({}) is too small or zero, cannot perform detection. Total bars: {}",
                avg_range, total_bars
            );
            return json!({
               "pattern": "fifty_percent_before_big_bar",
               "error": "Could not calculate a meaningful average range from the provided candles.",
               "total_bars": total_bars,
               "avg_range": avg_range,
               "required_bars": 2,
               "total_detected": 0,
               "config": self.get_config_json(),
               "datasets": { "price": 2, "oscillators": 0, "lines": 0 },
               "data": {
                   "price": {
                       "supply_zones": { "total": 0, "zones": [] },
                       "demand_zones": { "total": 0, "zones": [] }
                   }, "oscillators": [], "lines": []
               }
            });
        }

        let big_bar_threshold = avg_range * self.big_bar_range_factor;
        let max_fifty_range_threshold = avg_range * self.max_fifty_candle_range_factor;

        let mut supply_zones = Vec::new();
        let mut demand_zones = Vec::new();

        // For debugging the specific zone:
        let target_fifty_candle_time = "2025-05-25T23:00:00Z";

        for i in 0..candles.len().saturating_sub(1) {
            let fifty_candle = &candles[i];
            let big_candle = &candles[i + 1];

            // <<< --- START DEBUG LOGGING FOR TARGET --- >>>
            let is_target_fifty_candle = fifty_candle.time == target_fifty_candle_time;
            if is_target_fifty_candle {
                debug!("[RECOGNIZER_DEBUG] Checking target fifty_candle at index {}: Time={}, O={:.5}, H={:.5}, L={:.5}, C={:.5}",
                    i, fifty_candle.time, fifty_candle.open, fifty_candle.high, fifty_candle.low, fifty_candle.close);
                debug!("[RECOGNIZER_DEBUG] Corresponding big_candle: Time={}, O={:.5}, H={:.5}, L={:.5}, C={:.5}",
                    big_candle.time, big_candle.open, big_candle.high, big_candle.low, big_candle.close);
                debug!("[RECOGNIZER_DEBUG] Current Run Config: avg_range={:.5}, big_bar_thresh={:.5}, max_fifty_range_thresh={:.5}",
                    avg_range, big_bar_threshold, max_fifty_range_threshold);
            }
            // <<< --- END DEBUG LOGGING FOR TARGET (initial info) --- >>>

            let fifty_range = fifty_candle.high - fifty_candle.low;
            if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] fifty_range={:.5}", fifty_range); }
            if fifty_range <= 1e-9 {
                if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] SKIP: fifty_range too small"); }
                trace!("[FiftyPercentRecognizer] Skipping i={}: fifty_candle range ({}) is zero or negative", i, fifty_range);
                continue;
            }
            if fifty_range > max_fifty_range_threshold {
                if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] SKIP: fifty_range ({:.5}) > max_fifty_range_threshold ({:.5})", fifty_range, max_fifty_range_threshold); }
                trace!("[FiftyPercentRecognizer] Skipping i={}: fifty_candle range ({:.5}) exceeds max threshold ({:.5})", i, fifty_range, max_fifty_range_threshold);
                continue;
            }

            let fifty_body_size = (fifty_candle.close - fifty_candle.open).abs();
            if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] fifty_body_size={:.5}", fifty_body_size); }
            if fifty_body_size > (fifty_range * self.max_body_size_percent / 100.0) {
                if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] SKIP: fifty_body_size too large relative to its range"); }
                trace!("[FiftyPercentRecognizer] Skipping i={}: fifty_candle body size ({:.5}) exceeds max percent ({}) of its range ({:.5})", i, fifty_body_size, self.max_body_size_percent, fifty_range);
                continue;
            }

            let big_range = big_candle.high - big_candle.low;
            if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] big_range={:.5}", big_range); }
            if big_range <= 1e-9 {
                if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] SKIP: big_range too small"); }
                trace!("[FiftyPercentRecognizer] Skipping i={}: big_candle range ({}) is zero or negative", i, big_range);
                continue;
            }
            if big_range <= big_bar_threshold {
                if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] SKIP: big_range ({:.5}) <= big_bar_threshold ({:.5})", big_range, big_bar_threshold); }
                trace!("[FiftyPercentRecognizer] Skipping i={}: big_candle range ({:.5}) does not exceed threshold ({:.5})", i, big_range, big_bar_threshold);
                continue;
            }

            let big_body_size = (big_candle.close - big_candle.open).abs();
            let big_body_percentage = if big_range > 1e-9 { big_body_size / big_range } else { 0.0 };
            if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] big_body_percentage={:.2}", big_body_percentage); }
            if big_body_percentage < self.min_big_bar_body_percent {
                if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] SKIP: big_body_percentage too small"); }
                trace!("[FiftyPercentRecognizer] Skipping i={}: big_candle body percentage ({:.2}) is below minimum ({:.2})", i, big_body_percentage, self.min_big_bar_body_percent);
                continue;
            }

            let is_bullish_big_bar = big_candle.close > big_candle.open;
            let is_confirmed = if !self.require_breakout_close { true }
                               else if is_bullish_big_bar { big_candle.close > fifty_candle.high }
                               else { big_candle.close < fifty_candle.low };
            if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] is_bullish_big_bar={}, require_breakout_close={}, is_confirmed={}", is_bullish_big_bar, self.require_breakout_close, is_confirmed); }
            if !is_confirmed {
                if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] SKIP: breakout not confirmed"); }
                trace!("[FiftyPercentRecognizer] Skipping i={}: big_candle close ({:.5}) did not confirm breakout of fifty_candle range [{:.5}, {:.5}]", i, big_candle.close, fifty_candle.low, fifty_candle.high);
                continue;
            }

            // If all checks pass, create the zone
            if is_target_fifty_candle {
                debug!("[RECOGNIZER_DEBUG] ALL CHECKS PASSED for target fifty_candle at index {}. Proceeding to create zone.", i);
            }
            debug!(
                "[FiftyPercentRecognizer] Pattern candidate CONFIRMED at i={}: fifty_time={}, big_time={}, fifty_H={:.5}, fifty_L={:.5}, big_C={:.5}",
                i, fifty_candle.time, big_candle.time, fifty_candle.high, fifty_candle.low, big_candle.close
            );


            let relative_size = if avg_range > 1e-9 {(big_range / avg_range * 100.0).round() as i32} else {0};
            let body_percent = (big_body_percentage * 100.0).round() as i32;
            let zone_low_base = fifty_candle.low;
            let zone_high_base = fifty_candle.high;
            let mut zone_end_index = i + 1;

            for j in (i + 2)..candles.len() {
                let candle = &candles[j];
                let (actual_zone_low, actual_zone_high) = if is_bullish_big_bar {
                    let extension_amount = if self.proximal_extension_percent > 0.0 { zone_high_base * (self.proximal_extension_percent / 100.0) } else { 0.0 };
                    (fifty_candle.low, zone_high_base + extension_amount)
                } else {
                    let extension_amount = if self.proximal_extension_percent > 0.0 { zone_low_base * (self.proximal_extension_percent / 100.0) } else { 0.0 };
                    (zone_low_base - extension_amount, fifty_candle.high)
                };
                let zone_broken = if is_bullish_big_bar { candle.close < actual_zone_low } else { candle.close > actual_zone_high };

                if zone_broken {
                    if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] Target zone broken internally by recognizer at candle j={}: C={:.5}, relevant_boundary={:.5}", j, candle.close, if is_bullish_big_bar {actual_zone_low} else {actual_zone_high}); }
                    zone_end_index = j;
                    break;
                }
                if j >= i + 100 { // Limit extension
                    if is_target_fifty_candle { debug!("[RECOGNIZER_DEBUG] Target zone extension limit reached at j={}", j); }
                    zone_end_index = j;
                    break;
                }
                zone_end_index = j;
            }
            zone_end_index = zone_end_index.max(i + 1).min(candles.len() - 1);

            let method_text_core = format!(
                "(i={i}) 50% Candle → {} Big Bar ({size}%, {body}% body{}){}",
                if is_bullish_big_bar { "Bullish" } else { "Bearish" },
                if self.require_breakout_close { ", Confirmed" } else { "" },
                if self.proximal_extension_percent > 0.0 { format!(", {:.2}% ext", self.proximal_extension_percent) } else { "".to_string() },
                size = relative_size,
                body = body_percent
            );

            let (zone_h, zone_l, zone_type_str) = if is_bullish_big_bar {
                let extension_amount = if self.proximal_extension_percent > 0.0 { zone_high_base * (self.proximal_extension_percent / 100.0) } else { 0.0 };
                (zone_high_base + extension_amount, fifty_candle.low, "demand_zone")
            } else {
                let extension_amount = if self.proximal_extension_percent > 0.0 { zone_low_base * (self.proximal_extension_percent / 100.0) } else { 0.0 };
                (fifty_candle.high, zone_low_base - extension_amount, "supply_zone")
            };

            let zone_json = json!({
                "category": "Price",
                "type": zone_type_str,
                "start_time": fifty_candle.time.clone(),
                "end_time": candles[zone_end_index].time.clone(),
                "zone_high": zone_h,
                "zone_low": zone_l,
                "fifty_percent_line": (zone_h + zone_l) / 2.0,
                "start_idx": i as u64, // Ensure u64 for EnrichedZone
                "end_idx": zone_end_index as u64, // Ensure u64
                "detection_method": method_text_core,
                "extended": self.proximal_extension_percent > 0.0,
                "extension_percent": self.proximal_extension_percent
            });

            if is_bullish_big_bar {
                demand_zones.push(zone_json);
            } else {
                supply_zones.push(zone_json);
            }
        }

        json!({
            "pattern": "fifty_percent_before_big_bar",
            "config": self.get_config_json(),
            "total_bars": total_bars,
            "total_detected": supply_zones.len() + demand_zones.len(),
            "avg_range": avg_range,
            "big_bar_threshold": big_bar_threshold,
            "max_fifty_range_threshold": max_fifty_range_threshold,
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

    fn trade(&self, candles: &[CandleData], config: TradeConfig) -> (Vec<Trade>, TradeSummary) {
        let pattern_data = self.detect(candles);
        let default_symbol_for_recognizer_trade = "UNKNOWN_SYMBOL_IN_RECOGNIZER"; // Consider passing actual symbol
        let executor = TradeExecutor::new(config, default_symbol_for_recognizer_trade, None, None); // Pass actual timeframe
        let trades = executor.execute_trades_for_pattern(
            "fifty_percent_before_big_bar",
            &pattern_data,
            candles,
        );
        let summary = TradeSummary::from_trades(&trades);
        (trades, summary)
    }
}

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
