// src/trading.rs
use serde_json::Value;
use crate::detect::CandleData;
use crate::trades::{Trade, TradeConfig, TradeDirection, TradeStatus, TradeSummary};

pub struct TradeExecutor {
    pub config: TradeConfig,
}

impl TradeExecutor {
    pub fn new(config: TradeConfig) -> Self {
        Self { config }
    }
    
    pub fn execute_trades_for_pattern(&self, 
                                     pattern_type: &str, 
                                     pattern_data: &Value, 
                                     candles: &[CandleData]) -> Vec<Trade> {
        let mut trades = Vec::new();
        
        // Skip trading if it's not enabled
        if !self.config.enabled || candles.len() < 2 {
            return trades;
        }
        
        match pattern_type {
            "supply_demand_zone" => self.trade_supply_demand_zones(pattern_data, candles, &mut trades),
            "drop_base_rally" => self.trade_dbr_pattern(pattern_data, candles, &mut trades),
            // Add other pattern types as needed
            _ => {} // Unknown pattern, no trades
        }
        
        // Process all trades to simulate their outcomes
        self.process_trades(&mut trades, candles);
        
        trades
    }
    
    fn trade_supply_demand_zones(&self, pattern_data: &Value, candles: &[CandleData], trades: &mut Vec<Trade>) {
        // Extract demand zones
        if let Some(price_data) = pattern_data.get("data").and_then(|d| d.get("price")) {
            // Process demand zones for long trades
            if let Some(demand_zones) = price_data.get("demand_zones").and_then(|dz| dz.get("zones")) {
                if let Some(zones) = demand_zones.as_array() {
                    for zone in zones {
                        self.trade_demand_zone(zone, candles, trades);
                    }
                }
            }
            
            // Process supply zones for short trades
            if let Some(supply_zones) = price_data.get("supply_zones").and_then(|sz| sz.get("zones")) {
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
        let zone_id = format!("{}-{}", zone["detection_method"].as_str().unwrap_or("unknown"), start_idx);
        let pattern_type = format!("demand_zone_{}", zone["detection_method"].as_str().unwrap_or("unknown"));
        
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
                        i
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
        let zone_id = format!("{}-{}", zone["detection_method"].as_str().unwrap_or("unknown"), start_idx);
        let pattern_type = format!("supply_zone_{}", zone["detection_method"].as_str().unwrap_or("unknown"));
        
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
                        i
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
    
    fn trade_dbr_pattern(&self, pattern_data: &Value, candles: &[CandleData], trades: &mut Vec<Trade>) {
        // Extract relevant data for Drop-Base-Rally pattern and create trades
        // Implementation would follow similar logic to trade_demand_zone
    }
    
    fn process_trades(&self, trades: &mut Vec<Trade>, candles: &[CandleData]) {
        for trade in trades.iter_mut() {
            if trade.status != TradeStatus::Open {
                continue; // Skip already closed trades
            }
            
            // Start checking from the candle after entry
            let start_idx = trade.candlestick_idx_entry + 1;
            if start_idx >= candles.len() {
                continue; // No candles after entry
            }
            
            let mut highest_price = trade.entry_price;
            let mut trailing_stop = trade.stop_loss;
            let mut trailing_activated = false;
            
            for i in start_idx..candles.len() {
                let candle = &candles[i];
                
                match trade.direction {
                    TradeDirection::Long => {
                        // Check if stop loss hit
                        if candle.low <= trade.stop_loss {
                            trade.close(candle, "Stop Loss", i);
                            break;
                        }
                        
                        // Check if take profit hit
                        if candle.high >= trade.take_profit {
                            trade.close(candle, "Take Profit", i);
                            break;
                        }
                        
                        // Trailing stop logic
                        if self.config.enable_trailing_stop {
                            let pip_size = 0.0001;
                            let activation_threshold = trade.entry_price + 
                                (self.config.trailing_stop_activation_pips * pip_size);
                            
                            // Check if price moved enough to activate trailing stop
                            if candle.high > activation_threshold && !trailing_activated {
                                trailing_activated = true;
                            }
                            
                            // Update trailing stop if it's activated and price makes new high
                            if trailing_activated && candle.high > highest_price {
                                highest_price = candle.high;
                                trailing_stop = highest_price - (self.config.trailing_stop_distance_pips * pip_size);
                                
                                // Only move stop loss up, never down
                                if trailing_stop > trade.stop_loss {
                                    trade.stop_loss = trailing_stop;
                                }
                            }
                        }
                    },
                    TradeDirection::Short => {
                        // Check if stop loss hit
                        if candle.high >= trade.stop_loss {
                            trade.close(candle, "Stop Loss", i);
                            break;
                        }
                        
                        // Check if take profit hit
                        if candle.low <= trade.take_profit {
                            trade.close(candle, "Take Profit", i);
                            break;
                        }
                        
                        // Trailing stop logic
                        if self.config.enable_trailing_stop {
                            let pip_size = 0.0001;
                            let activation_threshold = trade.entry_price - 
                                (self.config.trailing_stop_activation_pips * pip_size);
                            
                            // Check if price moved enough to activate trailing stop
                            if candle.low < activation_threshold && !trailing_activated {
                                trailing_activated = true;
                                highest_price = candle.low; // For shorts, track lowest price
                            }
                            
                            // Update trailing stop if it's activated and price makes new low
                            if trailing_activated && candle.low < highest_price {
                                highest_price = candle.low;
                                trailing_stop = highest_price + (self.config.trailing_stop_distance_pips * pip_size);
                                
                                // Only move stop loss down, never up
                                if trailing_stop < trade.stop_loss {
                                    trade.stop_loss = trailing_stop;
                                }
                            }
                        }
                    }
                }
            }
            
            // If trade is still open at the end of the data, close it at the last candle
            if trade.status == TradeStatus::Open && !candles.is_empty() {
                trade.close(&candles[candles.len() - 1], "End of Data", candles.len() - 1);
            }
        }
    }
}