// fn trade_price_sma_cross_events(
//     &self,
//     pattern_data: &Value,
//     candles: &[CandleData],  // Primary timeframe candles
//     trades: &mut Vec<Trade>, // Add generated trades to this vector
// ) {
//     log::info!("TradeExecutor: Processing Price/SMA crossover events...");
//     // Adjust JSON path if your detect function puts events elsewhere
//     let crossover_events = match pattern_data
//         .get("data")
//         .and_then(|d| d.get("events"))
//         .and_then(|e| e.get("crossovers")) // Assuming key is 'crossovers'
//         .and_then(|c| c.as_array())
//     {
//         Some(events) if !events.is_empty() => events,
//         _ => {
//             log::warn!(
//                 "TradeExecutor (Price/SMA Cross): No crossover events found in pattern data."
//             );
//             return; // Exit if no events array found
//         }
//     };

//     log::info!(
//         "TradeExecutor (Price/SMA Cross): Found {} events.",
//         crossover_events.len()
//     );
//     let pattern_name = "price_sma_cross"; // Consistent name

//     for event in crossover_events {
//         log::debug!(
//             "TradeExecutor (Price/SMA Cross): Parsing event: {:?}",
//             event
//         );
//         // Extract necessary fields from the event JSON
//         if let (Some(event_type), Some(idx_val)) =
//             (event["type"].as_str(), event["candle_index"].as_u64())
//         {
//             let entry_candle_idx = idx_val as usize;
//             // Basic bounds check
//             if entry_candle_idx >= candles.len() {
//                 log::warn!("TradeExecutor (Price/SMA Cross): Event index {} out of bounds (candles len {}).", entry_candle_idx, candles.len());
//                 continue;
//             }
//             let entry_candle = &candles[entry_candle_idx];

//             // Determine pip size (basic check, refine if necessary)
//             let pip_size = if entry_candle.close > 20.0 {
//                 0.01
//             } else {
//                 0.0001
//             };

//             // Determine trade direction based on event type
//             let direction = match event_type {
//                 "Price Above SMA" => Some(TradeDirection::Long), // Match type from detect
//                 "Price Below SMA" => Some(TradeDirection::Short), // Match type from detect
//                 _ => None,
//             };

//             if let Some(dir) = direction {
//                 // Calculate entry, stop loss, and take profit
//                 let entry_price = entry_candle.close; // Enter on close of signal candle
//                 let (sl_price, tp_price) = match dir {
//                     TradeDirection::Long => (
//                         entry_price - self.config.default_stop_loss_pips * pip_size,
//                         entry_price + self.config.default_take_profit_pips * pip_size,
//                     ),
//                     TradeDirection::Short => (
//                         entry_price + self.config.default_stop_loss_pips * pip_size,
//                         entry_price - self.config.default_take_profit_pips * pip_size,
//                     ),
//                 };

//                 // Create a unique ID for the trade/pattern instance
//                 let pattern_id = format!("{}-{}", pattern_name, entry_candle.time);

//                 // Create the new Trade object
//                 let trade = Trade::new(
//                     pattern_id,               // ID for this specific signal instance
//                     pattern_name.to_string(), // General pattern type
//                     dir,                      // Long or Short
//                     entry_candle,             // Candle where signal occurred
//                     self.config.lot_size,     // Lot size from config
//                     sl_price,                 // Calculated Stop Loss
//                     tp_price,                 // Calculated Take Profit
//                     entry_candle_idx,         // Index of the entry candle
//                 );

//                 log::info!(
//                     "TradeExecutor (Price/SMA Cross): Generated {:?} Trade ID {}",
//                     trade.direction,
//                     trade.id
//                 );
//                 // Add the newly created trade to the output vector
//                 trades.push(trade);

//                 // Note: Max trades per pattern logic is currently omitted here for simplicity.
//                 // It would typically be handled during the process_trades simulation step.
//             }
//         } else {
//             log::warn!("TradeExecutor (Price/SMA Cross): Failed to parse 'type' or 'candle_index' from event details: {:?}", event);
//         }
//     } // End event loop
//     log::info!("TradeExecutor (Price/SMA Cross): Finished generating initial trades from events. Total generated in this call: {}", trades.len());
// }

// // --- NEW function trade_ma_crossover_events ---
// fn trade_ma_crossover_events(
//     &self,
//     pattern_data: &Value,
//     candles: &[CandleData],  // Primary timeframe candles
//     trades: &mut Vec<Trade>, // Add generated trades to this vector
// ) {
//     log::info!("TradeExecutor: Processing MA crossover events...");
//     let crossover_events = match pattern_data
//         .get("data")
//         .and_then(|d| d.get("events"))
//         .and_then(|e| e.get("crossovers"))
//         .and_then(|c| c.as_array())
//     {
//         Some(events) if !events.is_empty() => events,
//         _ => {
//             log::warn!("TradeExecutor (MA Cross): No crossover events found in pattern data.");
//             return;
//         }
//     };

//     log::info!(
//         "TradeExecutor (MA Cross): Found {} events.",
//         crossover_events.len()
//     );

//     let pattern_name = "ma_crossover"; // Consistent name

//     for event in crossover_events {
//         log::debug!("TradeExecutor (MA Cross): Parsing event: {:?}", event);
//         if let (Some(event_type), Some(idx_val)) =
//             (event["type"].as_str(), event["candle_index"].as_u64())
//         {
//             let entry_candle_idx = idx_val as usize;
//             if entry_candle_idx >= candles.len() {
//                 continue;
//             } // Bounds check
//             let entry_candle = &candles[entry_candle_idx];
//             let pip_size = if entry_candle.close > 20.0 {
//                 0.01
//             } else {
//                 0.0001
//             };

//             let direction = match event_type {
//                 "Bullish Crossover" => Some(TradeDirection::Long),
//                 "Bearish Crossover" => Some(TradeDirection::Short),
//                 _ => None,
//             };

//             if let Some(dir) = direction {
//                 let entry_price = entry_candle.close; // Enter on close
//                 let (sl_price, tp_price) = match dir {
//                     TradeDirection::Long => (
//                         entry_price - self.config.default_stop_loss_pips * pip_size,
//                         entry_price + self.config.default_take_profit_pips * pip_size,
//                     ),
//                     TradeDirection::Short => (
//                         entry_price + self.config.default_stop_loss_pips * pip_size,
//                         entry_price - self.config.default_take_profit_pips * pip_size,
//                     ),
//                 };

//                 let pattern_id = format!("{}-{}", pattern_name, entry_candle.time);
//                 let trade = Trade::new(
//                     pattern_id,
//                     pattern_name.to_string(),
//                     dir,
//                     entry_candle,
//                     self.config.lot_size,
//                     sl_price,
//                     tp_price,
//                     entry_candle_idx,
//                 );
//                 log::info!(
//                     "TradeExecutor (MA Cross): Generated {:?} Trade ID {}",
//                     trade.direction,
//                     trade.id
//                 );
//                 trades.push(trade); // Add to the vector passed from execute_trades_for_pattern

//                 // Note: Max trades per pattern needs more complex handling here or in process_trades
//                 // Simple check would be: if trades.len() >= self.config.max_trades_per_pattern { break; }
//                 // But this doesn't group by original signal source if multiple signals are close.
//             }
//         } else {
//             log::warn!(
//                 "TradeExecutor (MA Cross): Failed to parse event details: {:?}",
//                 event
//             );
//         }
//     } // End event loop
//     log::info!("TradeExecutor (MA Cross): Finished generating initial trades from events.");
// }

// fn trade_supply_demand_zones(
//     &self,
//     pattern_data: &Value,
//     candles: &[CandleData],
//     trades: &mut Vec<Trade>,
// ) {
//     // Extract demand zones
//     if let Some(price_data) = pattern_data.get("data").and_then(|d| d.get("price")) {
//         // Process demand zones for long trades
//         if let Some(demand_zones) = price_data
//             .get("demand_zones")
//             .and_then(|dz| dz.get("zones"))
//         {
//             if let Some(zones) = demand_zones.as_array() {
//                 for zone in zones {
//                     self.trade_demand_zone(zone, candles, trades);
//                 }
//             }
//         }

//         // Process supply zones for short trades
//         if let Some(supply_zones) = price_data
//             .get("supply_zones")
//             .and_then(|sz| sz.get("zones"))
//         {
//             if let Some(zones) = supply_zones.as_array() {
//                 for zone in zones {
//                     self.trade_supply_zone(zone, candles, trades);
//                 }
//             }
//         }
//     }
// }

// fn trade_demand_zone(&self, zone: &Value, candles: &[CandleData], trades: &mut Vec<Trade>) {
//     // Extract zone information
//     let zone_low = zone["zone_low"].as_f64().unwrap_or(0.0);
//     let zone_high = zone["zone_high"].as_f64().unwrap_or(0.0);
//     let start_idx = zone["start_idx"].as_u64().unwrap_or(0) as usize;
//     let end_idx = zone["end_idx"].as_u64().unwrap_or(0) as usize;
//     let zone_id = format!(
//         "{}-{}",
//         zone["detection_method"].as_str().unwrap_or("unknown"),
//         start_idx
//     );
//     let pattern_type = format!(
//         "demand_zone_{}",
//         zone["detection_method"].as_str().unwrap_or("unknown")
//     );

//     // Skip if we don't have enough candles after the zone
//     if end_idx + 1 >= candles.len() {
//         return;
//     }

//     // Trading strategy: Enter when price returns to zone
//     let mut entered = false;
//     let mut trade_count = 0;

//     for i in end_idx + 1..candles.len() {
//         // Check if we've reached the maximum trades for this pattern
//         if trade_count >= self.config.max_trades_per_pattern {
//             break;
//         }

//         // Check if price enters the zone (low of candle is in the zone)
//         if candles[i].low <= zone_high && candles[i].low >= zone_low {
//             // Enter a long trade at the close of this candle
//             if !entered {
//                 entered = true;

//                 // Calculate stop loss and take profit levels
//                 let entry_price = candles[i].close;
//                 let pip_size = 0.0001; // 4 decimal places for forex

//                 // Place stop loss below the zone
//                 let stop_loss = zone_low - (5.0 * pip_size); // 5 pips below zone_low

//                 // Take profit is a multiple of the risk
//                 let risk_pips = (entry_price - stop_loss) / pip_size;
//                 let reward_pips = risk_pips * 2.0; // 1:2 risk:reward
//                 let take_profit = entry_price + (reward_pips * pip_size);

//                 // Create a new trade
//                 let trade = Trade::new(
//                     zone_id.clone(),
//                     pattern_type.clone(),
//                     TradeDirection::Long,
//                     &candles[i],
//                     self.config.lot_size,
//                     stop_loss,
//                     take_profit,
//                     i,
//                 );

//                 trades.push(trade);
//                 trade_count += 1;
//             }
//         } else if candles[i].low > zone_high {
//             // Price moved above the zone, reset entered flag to allow for re-entry
//             entered = false;
//         }
//     }
// }

// fn trade_supply_zone(&self, zone: &Value, candles: &[CandleData], trades: &mut Vec<Trade>) {
//     // Extract zone information
//     let zone_low = zone["zone_low"].as_f64().unwrap_or(0.0);
//     let zone_high = zone["zone_high"].as_f64().unwrap_or(0.0);
//     let start_idx = zone["start_idx"].as_u64().unwrap_or(0) as usize;
//     let end_idx = zone["end_idx"].as_u64().unwrap_or(0) as usize;
//     let zone_id = format!(
//         "{}-{}",
//         zone["detection_method"].as_str().unwrap_or("unknown"),
//         start_idx
//     );
//     let pattern_type = format!(
//         "supply_zone_{}",
//         zone["detection_method"].as_str().unwrap_or("unknown")
//     );

//     // Skip if we don't have enough candles after the zone
//     if end_idx + 1 >= candles.len() {
//         return;
//     }

//     // Trading strategy: Enter when price returns to zone
//     let mut entered = false;
//     let mut trade_count = 0;

//     for i in end_idx + 1..candles.len() {
//         // Check if we've reached the maximum trades for this pattern
//         if trade_count >= self.config.max_trades_per_pattern {
//             break;
//         }

//         // Check if price enters the zone (high of candle is in the zone)
//         if candles[i].high >= zone_low && candles[i].high <= zone_high {
//             // Enter a short trade at the close of this candle
//             if !entered {
//                 entered = true;

//                 // Calculate stop loss and take profit levels
//                 let entry_price = candles[i].close;
//                 let pip_size = 0.0001; // 4 decimal places for forex

//                 // Place stop loss above the zone
//                 let stop_loss = zone_high + (5.0 * pip_size); // 5 pips above zone_high

//                 // Take profit is a multiple of the risk
//                 let risk_pips = (stop_loss - entry_price) / pip_size;
//                 let reward_pips = risk_pips * 2.0; // 1:2 risk:reward
//                 let take_profit = entry_price - (reward_pips * pip_size);

//                 // Create a new trade
//                 let trade = Trade::new(
//                     zone_id.clone(),
//                     pattern_type.clone(),
//                     TradeDirection::Short,
//                     &candles[i],
//                     self.config.lot_size,
//                     stop_loss,
//                     take_profit,
//                     i,
//                 );

//                 trades.push(trade);
//                 trade_count += 1;
//             }
//         } else if candles[i].high < zone_low {
//             // Price moved below the zone, reset entered flag to allow for re-entry
//             entered = false;
//         }
//     }
// }

// fn trade_dbr_pattern(
//     &self,
//     _pattern_data: &Value,
//     _candles: &[CandleData],
//     _trades: &mut Vec<Trade>,
// ) {
//     // Extract relevant data for Drop-Base-Rally pattern and create trades
//     // Implementation would follow similar logic to trade_demand_zone
// }

// pub fn trade_fifty_percent_before_big_bar(
//     &self,
//     pattern_data: &Value,
//     candles: &[CandleData],
//     trades: &mut Vec<Trade>, // This vector will be populated by the helper
// ) {
//     // This function now just acts as a wrapper calling the specific generator
//     self.generate_fifty_percent_zone_trades(pattern_data, candles, trades);
// }

// // Modify process_trades to use minute data when available
// fn process_trades(&self, trades: &mut Vec<Trade>, hourly_candles: &[CandleData]) {
//     // If we have minute data, use it for more precise execution
//     if let Some(minute_candles) = &self.minute_candles {
//         self.process_trades_with_minute_data(trades, minute_candles, hourly_candles);
//     } else {
//         // Use regular hourly processing if no minute data available
//         self.process_trades_with_hourly_data(trades, hourly_candles);
//     }
// }