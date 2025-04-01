// src/trading.rs
use crate::detect::CandleData;
use crate::trades::{Trade, TradeConfig, TradeDirection, TradeStatus};
use serde_json::Value;
use std::collections::HashMap;
// --- NEW: Struct to hold pending zone information ---
#[derive(Debug, Clone)]
struct PendingZoneTrade {
    zone_id: String,
    pattern_type: String, // e.g., "fifty_percent_demand_..."
    direction: TradeDirection,
    proximal_line: f64,
    // Store distal line too for context or other entry types? Optional.
    // distal_line: f64,
    zone_start_idx: usize, // Index on PRIMARY candles where zone starts
    zone_end_idx: usize,   // Index on PRIMARY candles where zone graphical representation ends
    // Add config items needed for Trade::new later
    lot_size: f64,
    sl_pips: f64,
    tp_pips: f64,
}
pub struct TradeExecutor {
    pub config: TradeConfig,
    pub minute_candles: Option<Vec<CandleData>>,
}

impl TradeExecutor {
    pub fn new(config: TradeConfig) -> Self {
        Self {
            config,
            minute_candles: None,
        }
    }

    // Add a method to set minute candles
    pub fn set_minute_candles(&mut self, candles: Vec<CandleData>) {
        self.minute_candles = Some(candles);
    }

    pub fn execute_trades_for_pattern(
        &self,
        pattern_type: &str,
        pattern_data: &Value,
        candles: &[CandleData], // Primary timeframe candles
    ) -> Vec<Trade> {
        // Returns FINAL closed trades
        let mut initial_trades = Vec::new(); // Vector for generated entry signals

        if !self.config.enabled || candles.len() < 2 {
            log::warn!("TradeExecutor: Trading disabled or insufficient candles.");
            return initial_trades;
        }

        log::info!(
            "TradeExecutor: Generating initial trades for pattern type '{}'",
            pattern_type
        );

        // --- Call specific helper to GENERATE initial trades ---
        match pattern_type {
            "fifty_percent_before_big_bar" => {
                // Calls the new helper function below
                self.generate_fifty_percent_zone_trades(pattern_data, candles, &mut initial_trades);
            }
            //  "price_sma_cross" => { // Keep this case if needed
            //      self.generate_price_sma_cross_trades(pattern_data, candles, &mut initial_trades);
            //  }
            //  "specific_time_entry" => { // Keep this case if needed
            //       self.generate_specific_time_trades(pattern_data, candles, &mut initial_trades);
            //  }
            // Add other pattern types here -> call their specific GENERATION function
            // e.g., "supply_demand_zone" => self.generate_supply_demand_trades(...)
            _ => {
                log::warn!(
                    "TradeExecutor: Unknown pattern type '{}', no initial trades generated.",
                    pattern_type
                );
            }
        } // End match

        log::info!(
            "TradeExecutor: Generated {} initial trade signals.",
            initial_trades.len()
        );

        // --- Simulate exits using the EXISTING process_trades ---
        if !initial_trades.is_empty() {
            log::info!(
                "TradeExecutor: Calling process_trades for {} generated trades...",
                initial_trades.len()
            );
            // Pass the generated trades to the existing simulation logic
            self.process_trades(&mut initial_trades, candles);
        } else {
            log::info!("TradeExecutor: No initial trades to process.");
        }
        // --- End Simulation ---

        // Filter out any trades that might still be open after simulation
        let closed_trades: Vec<Trade> = initial_trades
            .into_iter()
            .filter(|t| matches!(t.status, TradeStatus::Closed))
            .collect();

        log::info!(
            "TradeExecutor: Returning {} closed trades.",
            closed_trades.len()
        );
        closed_trades
    }

    // Modify process_trades to use minute data when available
    fn process_trades(&self, trades: &mut Vec<Trade>, hourly_candles: &[CandleData]) {
        // If we have minute data, use it for more precise execution
        if let Some(minute_candles) = &self.minute_candles {
            self.process_trades_with_minute_data(trades, minute_candles, hourly_candles);
        } else {
            // Use regular hourly processing if no minute data available
            self.process_trades_with_hourly_data(trades, hourly_candles);
        }
    }
    fn generate_fifty_percent_zone_trades(
        &self,
        pattern_data: &Value,
        candles: &[CandleData], // Still using primary candles here
        initial_trades: &mut Vec<Trade>,
    ) {
        log::info!("EXECUTOR: Generating FiftyPercent zone trades (Entry @ Proximal Line)..."); // Updated log
        let mut zone_trade_counts: HashMap<String, usize> = HashMap::new();

        // --- Helper to get pip size (assuming it exists or add it) ---
        // TODO: Ensure get_pip_size() is available or implement it based on symbol/price
        let get_pip_size = |price: f64| -> f64 {
            if price > 20.0 {
                0.01
            } else {
                0.0001
            } // Basic assumption
        };
        // ---

        if let Some(price_data) = pattern_data.get("data").and_then(|d| d.get("price")) {
            // --- Process DEMAND zones ---
            if let Some(zones_array) = price_data
                .get("demand_zones")
                .and_then(|dz| dz.get("zones"))
                .and_then(|z| z.as_array())
            {
                for zone in zones_array {
                    if let (
                        Some(proximal_line), // zone_high for demand
                        Some(distal_line),   // zone_low for demand
                        Some(start_idx_u64),
                        Some(end_idx_u64),
                        Some(detection_method),
                    ) = (
                        zone["zone_high"].as_f64(),
                        zone["zone_low"].as_f64(),
                        zone["start_idx"].as_u64(),
                        zone["end_idx"].as_u64(),
                        zone["detection_method"].as_str(),
                    ) {
                        let end_idx_usize = end_idx_u64 as usize;
                        if end_idx_usize >= candles.len() {
                            continue;
                        }

                        let zone_id = format!("demand-{}-{}", detection_method, start_idx_u64); // Prefix type
                        let pattern_type = format!("fifty_percent_demand_{}", detection_method);
                        log::debug!(
                            "Checking Demand Zone {} (ends idx {}), Proximal: {:.5}",
                            zone_id,
                            end_idx_usize,
                            proximal_line
                        );

                        let mut entered_this_zone = false; // Track entry per specific zone instance

                        for i in end_idx_usize..candles.len() {
                            // Start loop from zone end
                            let current_candle = &candles[i];
                            let current_zone_trades =
                                zone_trade_counts.entry(zone_id.clone()).or_insert(0);

                            if *current_zone_trades >= self.config.max_trades_per_pattern {
                                break;
                            }

                            // Entry condition: Low touches or crosses proximal line
                            if !entered_this_zone && current_candle.low <= proximal_line {
                                log::info!(
                                    "!!! DEMAND ZONE TOUCHED !!! Zone ID: {}, Candle Idx: {}",
                                    zone_id,
                                    i
                                );

                                // --- Set Entry Price to Proximal Line ---
                                let entry_price = proximal_line;
                                let pip_size = get_pip_size(entry_price);

                                // --- Calculate SL/TP based on entry_price (proximal_line) ---
                                // // SL: Below distal line + buffer
                                // let sl_buffer_pips = 2.0; // Example buffer
                                // let stop_loss = distal_line - sl_buffer_pips * pip_size;

                                // // TP: Based on Risk:Reward
                                // let risk_pips = (entry_price - stop_loss).abs() / pip_size;
                                // if risk_pips <= 0.0 {
                                //     log::warn!("Zone {}: Zero/negative risk (Entry: {:.5}, SL: {:.5}). Skipping trade.", zone_id, entry_price, stop_loss);
                                //     entered_this_zone = true; // Prevent re-entry attempts on this touch
                                //     continue; // Skip this iteration
                                // }
                                // let rr_ratio = self.config.default_take_profit_pips / self.config.default_stop_loss_pips; // Use config ratio
                                // let reward_pips = risk_pips * rr_ratio;
                                // let take_profit = entry_price + reward_pips * pip_size;
                                // // --- End SL/TP Calc ---

                                // --- CORRECTED SL/TP Calculation ---
                                // Use pip values directly from config
                                let stop_loss =
                                    entry_price - (self.config.default_stop_loss_pips * pip_size);
                                let take_profit =
                                    entry_price + (self.config.default_take_profit_pips * pip_size);
                                // --- End SL/TP ---

                                let trade = Trade::new(
                                    zone_id.clone(),
                                    pattern_type.clone(),
                                    TradeDirection::Long,
                                    current_candle, // Candle that touched
                                    entry_price,    // << Use proximal line price
                                    self.config.lot_size,
                                    stop_loss,
                                    take_profit,
                                    i, // Index of touching candle
                                );
                                log::info!(
                                    "EXECUTOR (50% Demand): Generated Long Trade ID {} at index {}, Entry: {:.5}, SL: {:.5}, TP: {:.5}",
                                    trade.id, i, entry_price, stop_loss, take_profit
                                );
                                initial_trades.push(trade);
                                entered_this_zone = true; // Mark entered for this zone touch
                                                          // Optional: break; // Use break if only ONE entry ever per zone instance is desired
                            }
                            // Optional: Logic to reset entered_this_zone if price moves far away?
                        } // End candle loop for zone
                    } else {
                        log::warn!("Skipping demand zone due to missing data: {:?}", zone);
                    }
                } // End loop through zones
            } // --- End Demand ---

            // --- Process SUPPLY zones ---
            if let Some(zones_array) = price_data
                .get("supply_zones")
                .and_then(|sz| sz.get("zones"))
                .and_then(|z| z.as_array())
            {
                for zone in zones_array {
                    if let (
                        Some(proximal_line), // zone_low for supply
                        Some(distal_line),   // zone_high for supply
                        Some(start_idx_u64),
                        Some(end_idx_u64),
                        Some(detection_method),
                    ) = (
                        zone["zone_low"].as_f64(),
                        zone["zone_high"].as_f64(),
                        zone["start_idx"].as_u64(),
                        zone["end_idx"].as_u64(),
                        zone["detection_method"].as_str(),
                    ) {
                        let end_idx_usize = end_idx_u64 as usize;
                        if end_idx_usize >= candles.len() {
                            continue;
                        }

                        let zone_id = format!("supply-{}-{}", detection_method, start_idx_u64); // Prefix type
                        let pattern_type = format!("fifty_percent_supply_{}", detection_method);
                        log::debug!(
                            "Checking Supply Zone {} (ends idx {}), Proximal: {:.5}",
                            zone_id,
                            end_idx_usize,
                            proximal_line
                        );

                        let mut entered_this_zone = false;

                        for i in end_idx_usize..candles.len() {
                            // Start loop from zone end
                            let current_candle = &candles[i];
                            let current_zone_trades =
                                zone_trade_counts.entry(zone_id.clone()).or_insert(0);

                            if *current_zone_trades >= self.config.max_trades_per_pattern {
                                break;
                            }

                            // Entry condition: High touches or crosses proximal line
                            if !entered_this_zone && current_candle.high >= proximal_line {
                                log::info!(
                                    "!!! SUPPLY ZONE TOUCHED !!! Zone ID: {}, Candle Idx: {}",
                                    zone_id,
                                    i
                                );

                                // --- Set Entry Price to Proximal Line ---
                                let entry_price = proximal_line;
                                let pip_size = get_pip_size(entry_price);

                                // --- Calculate SL/TP based on entry_price (proximal_line) ---
                                // SL: Above distal line + buffer
                                // let sl_buffer_pips = 2.0; // Example buffer
                                // let stop_loss = distal_line + sl_buffer_pips * pip_size;

                                // // TP: Based on Risk:Reward
                                // let risk_pips = (stop_loss - entry_price).abs() / pip_size;
                                // if risk_pips <= 0.0 {
                                //     log::warn!("Zone {}: Zero/negative risk (Entry: {:.5}, SL: {:.5}). Skipping trade.", zone_id, entry_price, stop_loss);
                                //     entered_this_zone = true; // Prevent re-entry attempts on this touch
                                //     continue; // Skip this iteration
                                // }
                                // let rr_ratio = self.config.default_take_profit_pips
                                //     / self.config.default_stop_loss_pips;
                                // let reward_pips = risk_pips * rr_ratio;
                                // let take_profit = entry_price - reward_pips * pip_size;
                                // // --- End SL/TP Calc ---
                                let entry_price = proximal_line;
                                let pip_size = get_pip_size(entry_price);
                        
                                // --- CORRECTED SL/TP Calculation ---
                                // Use pip values directly from config
                                let stop_loss = entry_price + (self.config.default_stop_loss_pips * pip_size);
                                let take_profit = entry_price - (self.config.default_take_profit_pips * pip_size);
                                // --- End SL/TP ---

                                let trade = Trade::new(
                                    zone_id.clone(),
                                    pattern_type.clone(),
                                    TradeDirection::Short,
                                    current_candle, // Candle that touched
                                    entry_price,    // << Use proximal line price
                                    self.config.lot_size,
                                    stop_loss,
                                    take_profit,
                                    i, // Index of touching candle
                                );
                                log::info!(
                                    "EXECUTOR (50% Supply): Generated Short Trade ID {} at index {}, Entry: {:.5}, SL: {:.5}, TP: {:.5}",
                                    trade.id, i, entry_price, stop_loss, take_profit
                                );
                                initial_trades.push(trade);
                                entered_this_zone = true; // Mark entered for this zone touch
                                                          // Optional: break; // Use break if only ONE entry ever per zone instance is desired
                            }
                            // Optional: Logic to reset entered_this_zone if price moves far away?
                        } // End candle loop
                    } else {
                        log::warn!("Skipping supply zone due to missing data: {:?}", zone);
                    }
                } // End loop through zones
            } // --- End Supply ---
        }
    }

    fn trade_specific_time_events(
        &self,
        pattern_data: &Value,
        candles: &[CandleData],
        trades: &mut Vec<Trade>,
    ) {
        log::info!("EXECUTOR (Specific Time): Processing events...");
        // Adjust JSON path based on detect output
        let signal_events = match pattern_data
            .get("data")
            .and_then(|d| d.get("events"))
            .and_then(|e| e.get("signals")) // Use "signals" key
            .and_then(|s| s.as_array())
        {
            Some(events) if !events.is_empty() => events,
            _ => {
                log::warn!("EXECUTOR (Specific Time): No signal events found.");
                return;
            }
        };
        log::info!(
            "EXECUTOR (Specific Time): Found {} events.",
            signal_events.len()
        );
        let pattern_name = "specific_time_entry";

        for event in signal_events {
            if let (Some(event_type), Some(idx_val)) =
                (event["type"].as_str(), event["candle_index"].as_u64())
            {
                let entry_candle_idx = idx_val as usize;
                if entry_candle_idx >= candles.len() {
                    continue;
                }
                let entry_candle = &candles[entry_candle_idx];
                let pip_size = if entry_candle.close > 20.0 {
                    0.01
                } else {
                    0.0001
                };

                let direction = match event_type {
                    "Buy Time" => Some(TradeDirection::Long),
                    "Sell Time" => Some(TradeDirection::Short),
                    _ => None,
                };

                if let Some(dir) = direction {
                    // Enter on CLOSE of the signal candle for simplicity here
                    let entry_price = entry_candle.close;
                    let (sl_price, tp_price) = match dir {
                        TradeDirection::Long => (
                            entry_price - self.config.default_stop_loss_pips * pip_size,
                            entry_price + self.config.default_take_profit_pips * pip_size,
                        ),
                        TradeDirection::Short => (
                            entry_price + self.config.default_stop_loss_pips * pip_size,
                            entry_price - self.config.default_take_profit_pips * pip_size,
                        ),
                    };
                    let pattern_id = format!("{}-{}", pattern_name, entry_candle.time);
                    let trade = Trade::new(
                        pattern_id,
                        pattern_name.to_string(),
                        dir,
                        entry_candle,
                        entry_price,
                        self.config.lot_size,
                        sl_price,
                        tp_price,
                        entry_candle_idx,
                    );
                    log::info!(
                        "EXECUTOR (Specific Time): Generated {:?} Trade ID {}",
                        trade.direction,
                        trade.id
                    );
                    trades.push(trade); // Add to initial_trades vector
                }
            } else {
                log::warn!(
                    "EXECUTOR (Specific Time): Failed to parse event details: {:?}",
                    event
                );
            }
        }
        log::info!("EXECUTOR (Specific Time): Finished generating initial trades from events.");
    }

    // Original processing logic renamed
    fn process_trades_with_hourly_data(&self, trades: &mut Vec<Trade>, candles: &[CandleData]) {
        // Your existing process_trades code here
        for trade in trades.iter_mut() {
            if trade.status != TradeStatus::Open {
                continue; // Skip already closed trades
            }

            // Start checking from the candle after entry
            let start_idx = trade.candlestick_idx_entry + 1;
            if start_idx >= candles.len() {
                continue; // No candles after entry
            }

            for i in start_idx..candles.len() {
                let candle = &candles[i];

                match trade.direction {
                    TradeDirection::Long => {
                        // Check if take profit hit (check this first)
                        if candle.high >= trade.take_profit {
                            trade.close(candle, "Take Profit", i);
                            break;
                        }

                        // Check if stop loss hit
                        if candle.low <= trade.stop_loss {
                            trade.close(candle, "Stop Loss", i);
                            break;
                        }
                    }
                    TradeDirection::Short => {
                        // Check if take profit hit (check this first)
                        if candle.low <= trade.take_profit {
                            trade.close(candle, "Take Profit", i);
                            break;
                        }

                        // Check if stop loss hit
                        if candle.high >= trade.stop_loss {
                            trade.close(candle, "Stop Loss", i);
                            break;
                        }
                    }
                }
            }

            // If trade is still open at the end of the data, close it at the last candle
            if trade.status == TradeStatus::Open && !candles.is_empty() {
                trade.close(
                    &candles[candles.len() - 1],
                    "End of Data",
                    candles.len() - 1,
                );
            }
        }
    }

    // New method for minute-level processing
    fn process_trades_with_minute_data(
        &self,
        trades: &mut Vec<Trade>,
        minute_candles: &[CandleData],
        hourly_candles: &[CandleData],
    ) {
        for trade in trades.iter_mut() {
            if trade.status != TradeStatus::Open {
                continue; // Skip already closed trades
            }

            // Map hourly entry time to minute data
            let hourly_entry_candle = &hourly_candles[trade.candlestick_idx_entry];
            let entry_time = &hourly_entry_candle.time;

            // Find corresponding minute candle index (first minute of the hour)
            let entry_minute_idx = minute_candles
                .iter()
                .position(|c| c.time >= entry_time.clone())
                .unwrap_or(0);

            if entry_minute_idx >= minute_candles.len() {
                continue; // Can't find corresponding minute data
            }

            // Start checking from the first minute candle after entry
            for i in (entry_minute_idx + 1)..minute_candles.len() {
                let minute_candle = &minute_candles[i];

                match trade.direction {
                    TradeDirection::Long => {
                        // Check if take profit hit
                        if minute_candle.high >= trade.take_profit {
                            trade.close(minute_candle, "Take Profit", i);
                            break;
                        }

                        // Check if stop loss hit
                        if minute_candle.low <= trade.stop_loss {
                            trade.close(minute_candle, "Stop Loss", i);
                            break;
                        }
                    }
                    TradeDirection::Short => {
                        // Check if take profit hit
                        if minute_candle.low <= trade.take_profit {
                            trade.close(minute_candle, "Take Profit", i);
                            break;
                        }

                        // Check if stop loss hit
                        if minute_candle.high >= trade.stop_loss {
                            trade.close(minute_candle, "Stop Loss", i);
                            break;
                        }
                    }
                }
            }

            // If still open at the end, close at the last minute candle
            if trade.status == TradeStatus::Open && !minute_candles.is_empty() {
                trade.close(
                    &minute_candles[minute_candles.len() - 1],
                    "End of Data",
                    minute_candles.len() - 1,
                );
            }
        }
    }
}
