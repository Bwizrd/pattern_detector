// src/trading.rs
use crate::api::detect::CandleData;
use crate::trading::trades::{Trade, TradeConfig, TradeDirection, TradeStatus};
use serde_json::Value;
// use std::collections::HashMap; // Commented out as zone_trade_counts is not used currently
use chrono::{DateTime, Utc, Weekday, Timelike, Datelike}; // Added Datelike
use log::{debug, info, warn, trace}; // Added log macros

// Commenting out PendingZoneTrade as it's not directly used in the current flow
// #[derive(Debug, Clone)]
// struct PendingZoneTrade {
//     zone_id: String,
//     pattern_type: String,
//     direction: TradeDirection,
//     proximal_line: f64,
//     zone_start_idx: usize,
//     zone_end_idx: usize,
//     lot_size: f64,
//     sl_pips: f64,
//     tp_pips: f64,
// }

pub struct TradeExecutor {
    pub config: TradeConfig,
    pub minute_candles: Option<Vec<CandleData>>,
    pip_size: f64,
    symbol: String,
    allowed_trade_days: Option<Vec<Weekday>>,
    trade_end_hour_utc: Option<u32>,
}

impl TradeExecutor {
    pub fn new(config: TradeConfig, symbol: &str, allowed_days: Option<Vec<Weekday>>, end_hour: Option<u32>) -> Self {
        let pip_size = Self::get_pip_size_for_symbol(symbol);
        Self {
            config,
            minute_candles: None,
            pip_size,
            symbol: symbol.to_string(),
            allowed_trade_days: allowed_days,
            trade_end_hour_utc: end_hour,
        }
    }

    fn get_pip_size_for_symbol(symbol: &str) -> f64 {
        if symbol.contains("JPY") {
            0.01
        } else if symbol.contains("NAS100") || symbol.contains("US500") || symbol.contains("XAU") {
            1.0
        } else {
            0.0001
        }
    }

    pub fn set_minute_candles(&mut self, candles: Vec<CandleData>) {
        self.minute_candles = Some(candles);
    }

    pub fn execute_trades_for_pattern(
        &self,
        pattern_type: &str,
        pattern_data: &Value,
        candles: &[CandleData],
    ) -> Vec<Trade> {
        let mut initial_trades = Vec::new();

        if !self.config.enabled || candles.len() < 2 {
            warn!("TradeExecutor ({}): Trading disabled or insufficient candles.", self.symbol);
            return initial_trades;
        }

        info!(
            "TradeExecutor ({}): Generating initial trades for pattern type '{}'",
            self.symbol, pattern_type
        );

        match pattern_type {
            "fifty_percent_before_big_bar" => {
                self.generate_fifty_percent_zone_trades(pattern_data, candles, &mut initial_trades);
            }
            "specific_time_entry" => {
                self.trade_specific_time_events(pattern_data, candles, &mut initial_trades); // Corrected method name
            }
            _ => {
                warn!(
                    "TradeExecutor ({}): Unknown pattern type '{}', no initial trades generated.",
                    self.symbol, pattern_type
                );
            }
        }

        info!(
            "TradeExecutor ({}): Generated {} initial trade signals.",
            self.symbol, initial_trades.len()
        );

        if !initial_trades.is_empty() {
            info!(
                "TradeExecutor ({}): Calling process_trades for {} generated trades...",
                self.symbol, initial_trades.len()
            );
            self.process_trades(&mut initial_trades, candles);
        } else {
            info!("TradeExecutor ({}): No initial trades to process.", self.symbol);
        }

        let closed_trades: Vec<Trade> = initial_trades
            .into_iter()
            .filter(|t| matches!(t.status, TradeStatus::Closed))
            .collect();

        info!(
            "TradeExecutor ({}): Returning {} closed trades.",
            self.symbol, closed_trades.len()
        );
        closed_trades
    }

    fn process_trades(&self, trades: &mut Vec<Trade>, hourly_candles: &[CandleData]) {
        if let Some(minute_candles) = &self.minute_candles {
            self.process_trades_with_minute_data(trades, minute_candles, hourly_candles);
        } else {
            self.process_trades_with_hourly_data(trades, hourly_candles);
        }
    }

    fn generate_fifty_percent_zone_trades(
        &self,
        pattern_data: &Value,
        candles: &[CandleData],
        initial_trades: &mut Vec<Trade>,
    ) {
        info!("EXECUTOR ({}): Generating FiftyPercent zone trades...", self.symbol);
        // let mut zone_trade_counts: HashMap<String, usize> = HashMap::new(); // Keep if needed for max_trades_per_pattern > 1

        if let Some(price_data) = pattern_data.get("data").and_then(|d| d.get("price")) {
            for (zone_category_key, is_supply_zone_type) in [("demand_zones", false), ("supply_zones", true)].iter() {
                if let Some(zones_array) = price_data
                    .get(zone_category_key)
                    .and_then(|dz_or_sz| dz_or_sz.get("zones"))
                    .and_then(|z| z.as_array())
                {
                    for zone_json_value in zones_array {
                        if let (
                            Some(proximal_line_raw),
                            Some(_distal_line_raw), // Keep if needed for context, using underscore if not directly used
                            Some(start_idx_u64),
                            Some(detection_method)
                        ) = (
                            zone_json_value[if *is_supply_zone_type { "zone_low"} else { "zone_high"}].as_f64(),
                            zone_json_value[if *is_supply_zone_type { "zone_high"} else { "zone_low"}].as_f64(),
                            zone_json_value["start_idx"].as_u64(),
                            zone_json_value["detection_method"].as_str()
                        ) {
                            let proximal_line = proximal_line_raw;

                            let start_idx_usize = start_idx_u64 as usize;
                            let loop_start_index = start_idx_usize + 2;

                            if loop_start_index >= candles.len() {
                                debug!("Zone {}-{}: Loop start index {} out of bounds ({} candles) for {}", detection_method, start_idx_u64, loop_start_index, candles.len(), self.symbol);
                                continue;
                            }

                            let zone_id = format!("{}-{}-{}", if *is_supply_zone_type {"supply"} else {"demand"}, detection_method, start_idx_u64);
                            let pattern_type_str = format!("fifty_percent_{}_{}", if *is_supply_zone_type {"supply"} else {"demand"}, detection_method);
                            
                            let mut entered_this_specific_zone_instance = false;

                            for i in loop_start_index..candles.len() {
                                if entered_this_specific_zone_instance && self.config.max_trades_per_pattern == 1 { break; }
                                // If max_trades_per_pattern > 1, you'd need zone_trade_counts and more logic here

                                let current_candle = &candles[i];
                                let trade_direction = if *is_supply_zone_type { TradeDirection::Short } else { TradeDirection::Long };

                                if let Ok(entry_candle_dt) = DateTime::parse_from_rfc3339(&current_candle.time).map(|dt| dt.with_timezone(&Utc)) { // Corrected: Pass &str
                                    if let Some(allowed_days) = &self.allowed_trade_days {
                                        if !allowed_days.contains(&entry_candle_dt.weekday()) { // Corrected: Uses Datelike
                                            trace!("Zone {} on {}: Skipped due to day filter ({} not allowed). Entry time: {}", zone_id, self.symbol, entry_candle_dt.weekday(), current_candle.time);
                                            continue;
                                        }
                                    }
                                    if let Some(end_hour) = self.trade_end_hour_utc {
                                        if entry_candle_dt.hour() >= end_hour { // Corrected: Uses Timelike
                                            trace!("Zone {} on {}: Skipped due to hour filter ({} >= {}). Entry time: {}", zone_id, self.symbol, entry_candle_dt.hour(), end_hour, current_candle.time);
                                            continue;
                                        }
                                    }
                                } else {
                                    warn!("Could not parse candle time '{}' for time filtering in symbol {}", current_candle.time, self.symbol);
                                    continue;
                                }

                                let mut entry_triggered = false;
                                if trade_direction == TradeDirection::Long && current_candle.low <= proximal_line {
                                    entry_triggered = true;
                                } else if trade_direction == TradeDirection::Short && current_candle.high >= proximal_line {
                                    entry_triggered = true;
                                }

                                if entry_triggered {
                                    info!("!!! {} ZONE TOUCHED !!! Symbol: {}, Zone ID: {}, Candle Idx: {} ({})",
                                        if *is_supply_zone_type {"SUPPLY"} else {"DEMAND"}, self.symbol, zone_id, i, current_candle.time);

                                    let entry_price = proximal_line;
                                    let stop_loss = match trade_direction {
                                        TradeDirection::Long => entry_price - (self.config.default_stop_loss_pips * self.pip_size),
                                        TradeDirection::Short => entry_price + (self.config.default_stop_loss_pips * self.pip_size),
                                    };
                                    let take_profit = match trade_direction {
                                        TradeDirection::Long => entry_price + (self.config.default_take_profit_pips * self.pip_size),
                                        TradeDirection::Short => entry_price - (self.config.default_take_profit_pips * self.pip_size),
                                    };

                                    let trade = Trade::new(
                                          self.symbol.clone(), 
                                        zone_id.clone(), pattern_type_str.clone(), trade_direction,
                                        current_candle, entry_price, self.config.lot_size,
                                        stop_loss, take_profit, i,
                                    );
                                    info!(
                                        "EXECUTOR ({} {}): Generated Trade ID {} at index {}, EntryTime: {}, Entry: {:.5}, SL: {:.5}, TP: {:.5}",
                                        self.symbol, pattern_type_str, trade.id, i, trade.entry_time, trade.entry_price, trade.stop_loss, trade.take_profit
                                    );
                                    initial_trades.push(trade);
                                    entered_this_specific_zone_instance = true;
                                    if self.config.max_trades_per_pattern == 1 { break; } // Only one trade per zone instance
                                }
                            }
                        } else {
                            warn!("Skipping {} zone due to missing critical data: {:?} for symbol {}", if *is_supply_zone_type {"supply"} else {"demand"}, zone_json_value, self.symbol);
                        }
                    }
                }
            }
        } else {
            warn!("No 'price' data found in pattern data for fifty_percent_before_big_bar for symbol {}", self.symbol);
        }
    }

    // Corrected this method name in the match statement above
    fn trade_specific_time_events(
        &self,
        pattern_data: &Value,
        candles: &[CandleData],
        initial_trades: &mut Vec<Trade>, // Changed 'trades' to 'initial_trades' to match usage
    ) {
        info!("EXECUTOR ({} Specific Time): Processing events...", self.symbol);
        let signal_events = match pattern_data.get("data").and_then(|d| d.get("events")).and_then(|e| e.get("signals")).and_then(|s| s.as_array()) {
            Some(events) if !events.is_empty() => events,
            _ => { warn!("EXECUTOR ({} Specific Time): No signal events found.", self.symbol); return; }
        };
        info!("EXECUTOR ({} Specific Time): Found {} events.", self.symbol, signal_events.len());
        let pattern_name = "specific_time_entry";

        for event in signal_events {
            if let (Some(event_type), Some(idx_val)) = (event["type"].as_str(), event["candle_index"].as_u64()) {
                let entry_candle_idx = idx_val as usize;
                if entry_candle_idx >= candles.len() { continue; }
                let entry_candle = &candles[entry_candle_idx];

                // --- Time Filter Check for specific_time_entry ---
                if let Ok(entry_candle_dt) = DateTime::parse_from_rfc3339(&entry_candle.time).map(|dt| dt.with_timezone(&Utc)) { // Corrected: Pass &str
                    if let Some(allowed_days) = &self.allowed_trade_days {
                        if !allowed_days.contains(&entry_candle_dt.weekday()) { // Corrected: Uses Datelike
                            trace!("Event {} on {}: Skipped due to day filter. Entry time: {}", event_type, self.symbol, entry_candle.time);
                            continue;
                        }
                    }
                    if let Some(end_hour) = self.trade_end_hour_utc {
                        if entry_candle_dt.hour() >= end_hour { // Corrected: Uses Timelike
                             trace!("Event {} on {}: Skipped due to hour filter. Entry time: {}", event_type, self.symbol, entry_candle.time);
                            continue;
                        }
                    }
                } else {
                    warn!("Could not parse candle time '{}' for time filtering in specific_time_entry for symbol {}", entry_candle.time, self.symbol);
                    continue;
                }
                // --- End Time Filter Check ---


                // Note: Using self.pip_size here, not the hardcoded one
                let direction = match event_type {
                    "Buy Time" => Some(TradeDirection::Long),
                    "Sell Time" => Some(TradeDirection::Short),
                    _ => None,
                };

                if let Some(dir) = direction {
                    let entry_price = entry_candle.close;
                    let (sl_price, tp_price) = match dir {
                        TradeDirection::Long => (
                            entry_price - self.config.default_stop_loss_pips * self.pip_size, // Use self.pip_size
                            entry_price + self.config.default_take_profit_pips * self.pip_size, // Use self.pip_size
                        ),
                        TradeDirection::Short => (
                            entry_price + self.config.default_stop_loss_pips * self.pip_size, // Use self.pip_size
                            entry_price - self.config.default_take_profit_pips * self.pip_size, // Use self.pip_size
                        ),
                    };
                    let pattern_id = format!("{}-{}-{}", self.symbol, pattern_name, entry_candle.time); // Added symbol for uniqueness
                    let trade = Trade::new(
                           self.symbol.clone(),
                        pattern_id, pattern_name.to_string(), dir,
                        entry_candle, entry_price, self.config.lot_size,
                        sl_price, tp_price, entry_candle_idx,
                    );
                    info!("EXECUTOR ({} Specific Time): Generated {:?} Trade ID {}", self.symbol, trade.direction, trade.id);
                    initial_trades.push(trade);
                }
            } else {
                warn!("EXECUTOR ({} Specific Time): Failed to parse event details: {:?}", self.symbol, event);
            }
        }
        info!("EXECUTOR ({} Specific Time): Finished generating initial trades from events.", self.symbol);
    }

    fn process_trades_with_hourly_data(&self, trades: &mut Vec<Trade>, candles: &[CandleData]) {
        for trade in trades.iter_mut() {
            if trade.status != TradeStatus::Open { continue; }
            let start_idx = trade.candlestick_idx_entry + 1;
            if start_idx >= candles.len() { continue; }
            for i in start_idx..candles.len() {
                let candle = &candles[i];
                match trade.direction {
                    TradeDirection::Long => {
                        if candle.high >= trade.take_profit { trade.close(candle, "Take Profit", i); break; }
                        if candle.low <= trade.stop_loss { trade.close(candle, "Stop Loss", i); break; }
                    }
                    TradeDirection::Short => {
                        if candle.low <= trade.take_profit { trade.close(candle, "Take Profit", i); break; }
                        if candle.high >= trade.stop_loss { trade.close(candle, "Stop Loss", i); break; }
                    }
                }
            }
            if trade.status == TradeStatus::Open && !candles.is_empty() {
                trade.close(&candles[candles.len() - 1], "End of Data", candles.len() - 1);
            }
        }
    }

    fn process_trades_with_minute_data(
        &self,
        trades: &mut Vec<Trade>,
        minute_candles: &[CandleData],
        hourly_candles: &[CandleData],
    ) {
        for trade in trades.iter_mut() {
            if trade.status != TradeStatus::Open { continue; }
            let hourly_entry_candle = &hourly_candles[trade.candlestick_idx_entry]; // This could panic if idx_entry is out of bounds for hourly_candles
            let entry_time = &hourly_entry_candle.time;
            let entry_minute_idx = minute_candles.iter().position(|c| c.time >= *entry_time).unwrap_or(0); // Deref entry_time for comparison

            if entry_minute_idx >= minute_candles.len() { continue; }
            for i in (entry_minute_idx + 1)..minute_candles.len() {
                let minute_candle = &minute_candles[i];
                match trade.direction {
                    TradeDirection::Long => {
                        if minute_candle.high >= trade.take_profit { trade.close(minute_candle, "Take Profit", i); break; }
                        if minute_candle.low <= trade.stop_loss { trade.close(minute_candle, "Stop Loss", i); break; }
                    }
                    TradeDirection::Short => {
                        if minute_candle.low <= trade.take_profit { trade.close(minute_candle, "Take Profit", i); break; }
                        if minute_candle.high >= trade.stop_loss { trade.close(minute_candle, "Stop Loss", i); break; }
                    }
                }
            }
            if trade.status == TradeStatus::Open && !minute_candles.is_empty() {
                trade.close(&minute_candles[minute_candles.len() - 1], "End of Data", minute_candles.len() - 1);
            }
        }
    }
}