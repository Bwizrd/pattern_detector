// src/trading.rs
use crate::detect::CandleData;
use crate::trades::{Trade, TradeConfig, TradeDirection, TradeStatus};
use serde_json::Value;

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
        &self, // <-- Reverted signature
        pattern_type: &str,
        pattern_data: &Value,
        candles: &[CandleData], // Primary timeframe candles (e.g., 30m)
    ) -> Vec<Trade> {
        // Returns only the *closed* trades after simulation
        let mut initial_trades = Vec::new(); // Store trades generated from signals

        // Skip trading generation if not enabled or not enough candles
        if !self.config.enabled || candles.len() < 2 {
            log::warn!("TradeExecutor: Trading disabled or insufficient candles (<2).");
            return initial_trades; // Return empty vector
        }

        log::info!(
            "TradeExecutor: Generating initial trades for pattern type '{}'",
            pattern_type
        );

        // --- Generate initial trades based on pattern type ---
        match pattern_type {
            "supply_demand_zone" => {
                self.trade_supply_demand_zones(pattern_data, candles, &mut initial_trades);
            }
            "drop_base_rally" => {
                self.trade_dbr_pattern(pattern_data, candles, &mut initial_trades);
            }
            "fifty_percent_before_big_bar" => {
                self.trade_fifty_percent_before_big_bar(pattern_data, candles, &mut initial_trades);
            }
            "price_sma_cross" => {
                log::info!("EXECUTOR: Matched 'price_sma_cross', calling helper..."); // Confirm match
                self.trade_price_sma_cross_events(pattern_data, candles, &mut initial_trades);
            }
            "specific_time_entry" => {
                log::info!("EXECUTOR: Matched 'specific_time_entry', calling helper...");
                self.trade_specific_time_events(pattern_data, candles, &mut initial_trades);
            }
            // Add other pattern types as needed
            _ => {
                log::warn!(
                    "TradeExecutor: Unknown pattern type '{}', no trades generated.",
                    pattern_type
                );
            }
        }

        log::info!(
            "TradeExecutor: Generated {} initial trade signals.",
            initial_trades.len()
        );

        // --- Process ALL generated trades (simulate outcomes) ---
        if !initial_trades.is_empty() {
            log::info!(
                "TradeExecutor: Processing {} generated trades for exits...",
                initial_trades.len()
            );
            // process_trades also takes &self, correctly passing config implicitly via self
            self.process_trades(&mut initial_trades, candles); // Pass primary candles for context if needed by minute processor
        } else {
            log::info!("TradeExecutor: No initial trades to process.");
        }

        // Filter out any trades that might still be open (process_trades should handle this)
        let closed_trades: Vec<Trade> = initial_trades
            .into_iter()
            .filter(|t| matches!(t.status, TradeStatus::Closed))
            .collect();

        log::info!(
            "TradeExecutor: Returning {} closed trades.",
            closed_trades.len()
        );
        closed_trades // Return the vector of processed (mostly closed) trades
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

    fn trade_price_sma_cross_events(
        &self,
        pattern_data: &Value,
        candles: &[CandleData],  // Primary timeframe candles
        trades: &mut Vec<Trade>, // Add generated trades to this vector
    ) {
        log::info!("TradeExecutor: Processing Price/SMA crossover events...");
        // Adjust JSON path if your detect function puts events elsewhere
        let crossover_events = match pattern_data
            .get("data")
            .and_then(|d| d.get("events"))
            .and_then(|e| e.get("crossovers")) // Assuming key is 'crossovers'
            .and_then(|c| c.as_array())
        {
            Some(events) if !events.is_empty() => events,
            _ => {
                log::warn!(
                    "TradeExecutor (Price/SMA Cross): No crossover events found in pattern data."
                );
                return; // Exit if no events array found
            }
        };

        log::info!(
            "TradeExecutor (Price/SMA Cross): Found {} events.",
            crossover_events.len()
        );
        let pattern_name = "price_sma_cross"; // Consistent name

        for event in crossover_events {
            log::debug!(
                "TradeExecutor (Price/SMA Cross): Parsing event: {:?}",
                event
            );
            // Extract necessary fields from the event JSON
            if let (Some(event_type), Some(idx_val)) =
                (event["type"].as_str(), event["candle_index"].as_u64())
            {
                let entry_candle_idx = idx_val as usize;
                // Basic bounds check
                if entry_candle_idx >= candles.len() {
                    log::warn!("TradeExecutor (Price/SMA Cross): Event index {} out of bounds (candles len {}).", entry_candle_idx, candles.len());
                    continue;
                }
                let entry_candle = &candles[entry_candle_idx];

                // Determine pip size (basic check, refine if necessary)
                let pip_size = if entry_candle.close > 20.0 {
                    0.01
                } else {
                    0.0001
                };

                // Determine trade direction based on event type
                let direction = match event_type {
                    "Price Above SMA" => Some(TradeDirection::Long), // Match type from detect
                    "Price Below SMA" => Some(TradeDirection::Short), // Match type from detect
                    _ => None,
                };

                if let Some(dir) = direction {
                    // Calculate entry, stop loss, and take profit
                    let entry_price = entry_candle.close; // Enter on close of signal candle
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

                    // Create a unique ID for the trade/pattern instance
                    let pattern_id = format!("{}-{}", pattern_name, entry_candle.time);

                    // Create the new Trade object
                    let trade = Trade::new(
                        pattern_id,               // ID for this specific signal instance
                        pattern_name.to_string(), // General pattern type
                        dir,                      // Long or Short
                        entry_candle,             // Candle where signal occurred
                        self.config.lot_size,     // Lot size from config
                        sl_price,                 // Calculated Stop Loss
                        tp_price,                 // Calculated Take Profit
                        entry_candle_idx,         // Index of the entry candle
                    );

                    log::info!(
                        "TradeExecutor (Price/SMA Cross): Generated {:?} Trade ID {}",
                        trade.direction,
                        trade.id
                    );
                    // Add the newly created trade to the output vector
                    trades.push(trade);

                    // Note: Max trades per pattern logic is currently omitted here for simplicity.
                    // It would typically be handled during the process_trades simulation step.
                }
            } else {
                log::warn!("TradeExecutor (Price/SMA Cross): Failed to parse 'type' or 'candle_index' from event details: {:?}", event);
            }
        } // End event loop
        log::info!("TradeExecutor (Price/SMA Cross): Finished generating initial trades from events. Total generated in this call: {}", trades.len());
    }

    // --- NEW function trade_ma_crossover_events ---
    fn trade_ma_crossover_events(
        &self,
        pattern_data: &Value,
        candles: &[CandleData],  // Primary timeframe candles
        trades: &mut Vec<Trade>, // Add generated trades to this vector
    ) {
        log::info!("TradeExecutor: Processing MA crossover events...");
        let crossover_events = match pattern_data
            .get("data")
            .and_then(|d| d.get("events"))
            .and_then(|e| e.get("crossovers"))
            .and_then(|c| c.as_array())
        {
            Some(events) if !events.is_empty() => events,
            _ => {
                log::warn!("TradeExecutor (MA Cross): No crossover events found in pattern data.");
                return;
            }
        };

        log::info!(
            "TradeExecutor (MA Cross): Found {} events.",
            crossover_events.len()
        );

        let pattern_name = "ma_crossover"; // Consistent name

        for event in crossover_events {
            log::debug!("TradeExecutor (MA Cross): Parsing event: {:?}", event);
            if let (Some(event_type), Some(idx_val)) =
                (event["type"].as_str(), event["candle_index"].as_u64())
            {
                let entry_candle_idx = idx_val as usize;
                if entry_candle_idx >= candles.len() {
                    continue;
                } // Bounds check
                let entry_candle = &candles[entry_candle_idx];
                let pip_size = if entry_candle.close > 20.0 {
                    0.01
                } else {
                    0.0001
                };

                let direction = match event_type {
                    "Bullish Crossover" => Some(TradeDirection::Long),
                    "Bearish Crossover" => Some(TradeDirection::Short),
                    _ => None,
                };

                if let Some(dir) = direction {
                    let entry_price = entry_candle.close; // Enter on close
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
                        self.config.lot_size,
                        sl_price,
                        tp_price,
                        entry_candle_idx,
                    );
                    log::info!(
                        "TradeExecutor (MA Cross): Generated {:?} Trade ID {}",
                        trade.direction,
                        trade.id
                    );
                    trades.push(trade); // Add to the vector passed from execute_trades_for_pattern

                    // Note: Max trades per pattern needs more complex handling here or in process_trades
                    // Simple check would be: if trades.len() >= self.config.max_trades_per_pattern { break; }
                    // But this doesn't group by original signal source if multiple signals are close.
                }
            } else {
                log::warn!(
                    "TradeExecutor (MA Cross): Failed to parse event details: {:?}",
                    event
                );
            }
        } // End event loop
        log::info!("TradeExecutor (MA Cross): Finished generating initial trades from events.");
    }

    fn trade_supply_demand_zones(
        &self,
        pattern_data: &Value,
        candles: &[CandleData],
        trades: &mut Vec<Trade>,
    ) {
        // Extract demand zones
        if let Some(price_data) = pattern_data.get("data").and_then(|d| d.get("price")) {
            // Process demand zones for long trades
            if let Some(demand_zones) = price_data
                .get("demand_zones")
                .and_then(|dz| dz.get("zones"))
            {
                if let Some(zones) = demand_zones.as_array() {
                    for zone in zones {
                        self.trade_demand_zone(zone, candles, trades);
                    }
                }
            }

            // Process supply zones for short trades
            if let Some(supply_zones) = price_data
                .get("supply_zones")
                .and_then(|sz| sz.get("zones"))
            {
                if let Some(zones) = supply_zones.as_array() {
                    for zone in zones {
                        self.trade_supply_zone(zone, candles, trades);
                    }
                }
            }
        }
    }

    fn trade_demand_zone(&self, zone: &Value, candles: &[CandleData], trades: &mut Vec<Trade>) {
        // Extract zone information
        let zone_low = zone["zone_low"].as_f64().unwrap_or(0.0);
        let zone_high = zone["zone_high"].as_f64().unwrap_or(0.0);
        let start_idx = zone["start_idx"].as_u64().unwrap_or(0) as usize;
        let end_idx = zone["end_idx"].as_u64().unwrap_or(0) as usize;
        let zone_id = format!(
            "{}-{}",
            zone["detection_method"].as_str().unwrap_or("unknown"),
            start_idx
        );
        let pattern_type = format!(
            "demand_zone_{}",
            zone["detection_method"].as_str().unwrap_or("unknown")
        );

        // Skip if we don't have enough candles after the zone
        if end_idx + 1 >= candles.len() {
            return;
        }

        // Trading strategy: Enter when price returns to zone
        let mut entered = false;
        let mut trade_count = 0;

        for i in end_idx + 1..candles.len() {
            // Check if we've reached the maximum trades for this pattern
            if trade_count >= self.config.max_trades_per_pattern {
                break;
            }

            // Check if price enters the zone (low of candle is in the zone)
            if candles[i].low <= zone_high && candles[i].low >= zone_low {
                // Enter a long trade at the close of this candle
                if !entered {
                    entered = true;

                    // Calculate stop loss and take profit levels
                    let entry_price = candles[i].close;
                    let pip_size = 0.0001; // 4 decimal places for forex

                    // Place stop loss below the zone
                    let stop_loss = zone_low - (5.0 * pip_size); // 5 pips below zone_low

                    // Take profit is a multiple of the risk
                    let risk_pips = (entry_price - stop_loss) / pip_size;
                    let reward_pips = risk_pips * 2.0; // 1:2 risk:reward
                    let take_profit = entry_price + (reward_pips * pip_size);

                    // Create a new trade
                    let trade = Trade::new(
                        zone_id.clone(),
                        pattern_type.clone(),
                        TradeDirection::Long,
                        &candles[i],
                        self.config.lot_size,
                        stop_loss,
                        take_profit,
                        i,
                    );

                    trades.push(trade);
                    trade_count += 1;
                }
            } else if candles[i].low > zone_high {
                // Price moved above the zone, reset entered flag to allow for re-entry
                entered = false;
            }
        }
    }

    fn trade_supply_zone(&self, zone: &Value, candles: &[CandleData], trades: &mut Vec<Trade>) {
        // Extract zone information
        let zone_low = zone["zone_low"].as_f64().unwrap_or(0.0);
        let zone_high = zone["zone_high"].as_f64().unwrap_or(0.0);
        let start_idx = zone["start_idx"].as_u64().unwrap_or(0) as usize;
        let end_idx = zone["end_idx"].as_u64().unwrap_or(0) as usize;
        let zone_id = format!(
            "{}-{}",
            zone["detection_method"].as_str().unwrap_or("unknown"),
            start_idx
        );
        let pattern_type = format!(
            "supply_zone_{}",
            zone["detection_method"].as_str().unwrap_or("unknown")
        );

        // Skip if we don't have enough candles after the zone
        if end_idx + 1 >= candles.len() {
            return;
        }

        // Trading strategy: Enter when price returns to zone
        let mut entered = false;
        let mut trade_count = 0;

        for i in end_idx + 1..candles.len() {
            // Check if we've reached the maximum trades for this pattern
            if trade_count >= self.config.max_trades_per_pattern {
                break;
            }

            // Check if price enters the zone (high of candle is in the zone)
            if candles[i].high >= zone_low && candles[i].high <= zone_high {
                // Enter a short trade at the close of this candle
                if !entered {
                    entered = true;

                    // Calculate stop loss and take profit levels
                    let entry_price = candles[i].close;
                    let pip_size = 0.0001; // 4 decimal places for forex

                    // Place stop loss above the zone
                    let stop_loss = zone_high + (5.0 * pip_size); // 5 pips above zone_high

                    // Take profit is a multiple of the risk
                    let risk_pips = (stop_loss - entry_price) / pip_size;
                    let reward_pips = risk_pips * 2.0; // 1:2 risk:reward
                    let take_profit = entry_price - (reward_pips * pip_size);

                    // Create a new trade
                    let trade = Trade::new(
                        zone_id.clone(),
                        pattern_type.clone(),
                        TradeDirection::Short,
                        &candles[i],
                        self.config.lot_size,
                        stop_loss,
                        take_profit,
                        i,
                    );

                    trades.push(trade);
                    trade_count += 1;
                }
            } else if candles[i].high < zone_low {
                // Price moved below the zone, reset entered flag to allow for re-entry
                entered = false;
            }
        }
    }

    fn trade_dbr_pattern(
        &self,
        pattern_data: &Value,
        candles: &[CandleData],
        trades: &mut Vec<Trade>,
    ) {
        // Extract relevant data for Drop-Base-Rally pattern and create trades
        // Implementation would follow similar logic to trade_demand_zone
    }

    pub fn trade_fifty_percent_before_big_bar(
        &self,
        pattern_data: &Value,
        candles: &[CandleData],
        trades: &mut Vec<Trade>,
    ) {
        // Extract zones from the pattern data
        if let Some(price_data) = pattern_data.get("data").and_then(|d| d.get("price")) {
            // Process demand zones for long trades
            if let Some(demand_zones) = price_data
                .get("demand_zones")
                .and_then(|dz| dz.get("zones"))
            {
                if let Some(zones) = demand_zones.as_array() {
                    for zone in zones {
                        self.trade_fifty_percent_demand_zone(zone, candles, trades);
                    }
                }
            }

            // Process supply zones for short trades
            if let Some(supply_zones) = price_data
                .get("supply_zones")
                .and_then(|sz| sz.get("zones"))
            {
                if let Some(zones) = supply_zones.as_array() {
                    for zone in zones {
                        self.trade_fifty_percent_supply_zone(zone, candles, trades);
                    }
                }
            }
        }
    }

    fn trade_fifty_percent_demand_zone(
        &self,
        zone: &Value,           // Zone info from detect()
        candles: &[CandleData], // Primary timeframe candles
        trades: &mut Vec<Trade>,
    ) {
        // Extract zone boundaries and indices
        let zone_low = zone["zone_low"].as_f64().unwrap_or(f64::INFINITY);
        // The Proximal Line for Demand is the HIGH
        let proximal_line = zone["zone_high"].as_f64().unwrap_or(f64::NEG_INFINITY);
        let start_idx = zone["start_idx"].as_u64().unwrap_or(u64::MAX) as usize;
        // end_idx is the index where the zone drawing stops (candle j+1 from detect)
        let end_idx = zone["end_idx"].as_u64().unwrap_or(u64::MAX) as usize;

        // Validation
        if start_idx == usize::MAX
            || end_idx == usize::MAX
            || end_idx >= candles.len()
            || proximal_line == f64::NEG_INFINITY
            || zone_low == f64::INFINITY
        {
            log::warn!("Invalid zone data received for demand trading: {:?}", zone);
            return;
        }
        // Skip if no candles exist after the identified zone ends
        if end_idx >= candles.len() {
            log::debug!(
                "Demand zone end index {} is out of bounds or at last candle.",
                end_idx
            );
            return;
        }

        let mut trade_count = 0; // Track trades for this specific zone instance
        let detection_method = zone["detection_method"]
            .as_str()
            .unwrap_or("unknown_demand");
        let zone_id = format!("{}-{}", detection_method, start_idx);
        let pattern_type = format!("fifty_percent_demand_{}", detection_method);

        log::debug!(
            "Checking for demand entry starting AT index {}. Proximal Line (Zone High): {:.5}",
            end_idx,
            proximal_line
        );

        // Start checking from the candle AFTER the zone was fully formed
        for i in end_idx..candles.len() {
            if trade_count >= self.config.max_trades_per_pattern {
                log::debug!(
                    "Max trades ({}) reached for demand zone {}",
                    self.config.max_trades_per_pattern,
                    zone_id
                );
                break; // Stop checking this zone
            }

            let current_candle = &candles[i];
            log::trace!(
                "Checking Demand Candle idx {}: Low={}, High={}, Close={}",
                i,
                current_candle.low,
                current_candle.high,
                current_candle.close
            );

            // --- ENTRY CONDITION: Price Low touches or crosses BELOW Proximal Line (zone_high) ---
            if current_candle.low <= proximal_line {
                // Enter ONCE per zone signal instance
                if trade_count == 0 {
                    let entry_candle = current_candle;
                    let entry_price = entry_candle.close; // Enter at close
                                                          // Determine pip size (basic check)
                    let pip_size = if entry_price > 20.0 { 0.01 } else { 0.0001 };
                    // Calculate SL/TP based on config pips and entry price
                    let stop_loss = entry_price - (self.config.default_stop_loss_pips * pip_size);
                    let take_profit =
                        entry_price + (self.config.default_take_profit_pips * pip_size);

                    // Create the trade using all required arguments from Trade::new
                    let trade = Trade::new(
                        zone_id.clone(),
                        pattern_type.clone(),
                        TradeDirection::Long,
                        entry_candle,         // Pass reference to entry candle
                        self.config.lot_size, // Use config lot size
                        stop_loss,            // Calculated SL
                        take_profit,          // Calculated TP
                        i,                    // Index of entry candle
                    );
                    log::info!("EXECUTOR (50% Demand): Generated Long Trade ID {} at index {} (Candle Time: {}) based on touch of proximal line {:.5}",
                         trade.id, i, current_candle.time, proximal_line);
                    trades.push(trade); // Add to the main trades vector
                    trade_count += 1; // Increment count for this zone
                    break; // Exit loop after first entry for this zone
                }
            }
            // Optional: Add logic to stop checking if price moves far away?
            // if current_candle.low > proximal_line + (10.0 * pip_size) { break; }
        } // End loop checking for entry
    } // End trade_fifty_percent_demand_zone

    fn trade_fifty_percent_supply_zone(
        &self,
        zone: &Value,
        candles: &[CandleData],
        trades: &mut Vec<Trade>,
    ) {
        // The Proximal Line for Supply is the LOW (potentially extended)
        let proximal_line = zone["zone_low"].as_f64().unwrap_or(f64::INFINITY);
        let zone_high = zone["zone_high"].as_f64().unwrap_or(f64::NEG_INFINITY);
        let start_idx = zone["start_idx"].as_u64().unwrap_or(u64::MAX) as usize;
        let end_idx = zone["end_idx"].as_u64().unwrap_or(u64::MAX) as usize;

        // Validation
        if start_idx == usize::MAX
            || end_idx == usize::MAX
            || end_idx >= candles.len()
            || zone_high == f64::NEG_INFINITY
            || proximal_line == f64::INFINITY
        {
            log::warn!("Invalid zone data received for supply trading: {:?}", zone);
            return;
        }
        if end_idx >= candles.len() {
            log::debug!("Supply zone end index {} is out of bounds.", end_idx);
            return;
        }

        let mut trade_count = 0;
        let detection_method = zone["detection_method"]
            .as_str()
            .unwrap_or("unknown_supply");
        let zone_id = format!("{}-{}", detection_method, start_idx);
        let pattern_type = format!("fifty_percent_supply_{}", detection_method);

        log::debug!(
            "Checking for supply entry STARTING AT index {}. Proximal Line (Zone Low): {:.5}",
            end_idx,
            proximal_line
        );

        // Start checking from the candle AFTER the zone was fully formed
        for i in end_idx..candles.len() {
            if trade_count >= self.config.max_trades_per_pattern {
                log::debug!(
                    "Max trades ({}) reached for supply zone {}",
                    self.config.max_trades_per_pattern,
                    zone_id
                );
                break;
            }
            let current_candle = &candles[i];
            log::trace!(
                "Checking Supply Candle idx {}: Close={}, Low={}, High={}",
                i,
                current_candle.close,
                current_candle.low,
                current_candle.high
            );

            // --- ENTRY CONDITION: Price High touches or crosses ABOVE Proximal Line (zone_low) ---
            if current_candle.high >= proximal_line {
                if trade_count == 0 {
                    // Only first entry
                    let entry_candle = current_candle;
                    let entry_price = entry_candle.close;
                    let pip_size = if entry_price > 20.0 { 0.01 } else { 0.0001 };
                    let stop_loss = entry_price + (self.config.default_stop_loss_pips * pip_size);
                    let take_profit =
                        entry_price - (self.config.default_take_profit_pips * pip_size);

                    // Create the trade using all required arguments from Trade::new
                    let trade = Trade::new(
                        zone_id.clone(),
                        pattern_type.clone(),
                        TradeDirection::Short,
                        entry_candle,         // Pass reference to entry candle
                        self.config.lot_size, // Use config lot size
                        stop_loss,            // Calculated SL
                        take_profit,          // Calculated TP
                        i,                    // Index of entry candle
                    );
                    log::info!("EXECUTOR (50% Supply): Generated Short Trade ID {} at index {} (Candle Time: {}) based on touch of proximal line {:.5}",
                         trade.id, i, current_candle.time, proximal_line);
                    trades.push(trade); // Add to the main trades vector
                    trade_count += 1; // Increment count for this zone
                    break; // Exit loop after first entry
                }
            }
            // Optional: Stop checking if price moves far away?
            // if current_candle.high < proximal_line - (10.0 * pip_size) { break; }
        } // End loop
    } // End trade_fifty_percent_supply_zone

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
