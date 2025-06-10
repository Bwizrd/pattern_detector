// src/trades.rs
use serde::{Deserialize, Serialize};
use crate::api::detect::CandleData;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeDirection {
    Long,
    Short,
}

#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub enum TradeStatus {
    Open,
    Closed,
    Cancelled,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct TradeConfig {
    pub enabled: bool,
    pub lot_size: f64,
    pub default_stop_loss_pips: f64,
    pub default_take_profit_pips: f64,
    pub risk_percent: f64,
    pub max_trades_per_pattern: usize,
    pub enable_trailing_stop: bool,
    pub trailing_stop_activation_pips: f64,
    pub trailing_stop_distance_pips: f64,
}

impl Default for TradeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            lot_size: 0.01,
            default_stop_loss_pips: 20.0,
            default_take_profit_pips: 40.0,
            risk_percent: 1.0,
            max_trades_per_pattern: 1,
            enable_trailing_stop: false,
            trailing_stop_activation_pips: 15.0,
            trailing_stop_distance_pips: 10.0,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct Trade {
    pub symbol: String,
    pub id: String,
    pub pattern_id: String,
    
    pub pattern_type: String,
    pub direction: TradeDirection,
    pub entry_time: String,
    pub entry_price: f64,
    pub exit_time: Option<String>,
    pub exit_price: Option<f64>,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub lot_size: f64,
    pub status: TradeStatus,
    pub profit_loss: Option<f64>,
    pub profit_loss_pips: Option<f64>,
    pub exit_reason: Option<String>,
    pub candlestick_idx_entry: usize,
    pub candlestick_idx_exit: Option<usize>,
}

impl Trade {
    pub fn new(
        symbol_arg: String,
        pattern_id: String,
        pattern_type: String,
        direction: TradeDirection,
        entry_candle: &CandleData,
        entry_price: f64,
        lot_size: f64,
        stop_loss: f64,
        take_profit: f64,
        candlestick_idx: usize,
    ) -> Self {
        
        let id = format!("{}-{}-{:?}", pattern_type, entry_candle.time, direction);
        
        Self {
            symbol: symbol_arg,
            id,
            pattern_id,
            pattern_type,
            direction,
            entry_time: entry_candle.time.clone(),
            entry_price,
            exit_time: None,
            exit_price: None,
            stop_loss,
            take_profit,
            lot_size,
            status: TradeStatus::Open,
            profit_loss: None,
            profit_loss_pips: None,
            exit_reason: None,
            candlestick_idx_entry: candlestick_idx,
            candlestick_idx_exit: None,
        }
    }
    
    pub fn close(&mut self, exit_candle: &CandleData, exit_reason: &str, candlestick_idx: usize) {
        if self.status == TradeStatus::Closed {
            // println!("Warning: Attempted to close an already closed trade: {}", self.id); // Simple console log for now
            return;
        }

        self.status = TradeStatus::Closed;
        self.exit_time = Some(exit_candle.time.clone());
        // self.exit_reason = Some(exit_reason.to_string()); // This will be set at the end
        // self.candlestick_idx_exit = Some(candlestick_idx); // This will be set at the end
        
        let effective_exit_price = if exit_reason == "Stop Loss" {
            self.stop_loss
        } else if exit_reason == "Take Profit" {
            self.take_profit
        } else {
            // For "End of Data" or any other/unexpected reason
            exit_candle.close
        };
        self.exit_price = Some(effective_exit_price);

        let identifier_to_check_for_symbol = &self.symbol;

        // ---- TEMPORARY DEBUG LOG ----
        if identifier_to_check_for_symbol.contains("JPY_SB") {
            println!("[DEBUG Trade::close] JPY PAIR DETECTED for ID: {}, pattern_id: {}, pattern_type: {}", self.id, self.pattern_id, self.pattern_type);
        } else if identifier_to_check_for_symbol.contains("USDJPY_SB") { // Be more specific for testing
             println!("[DEBUG Trade::close] USDJPY_SB PAIR DETECTED for ID: {}, pattern_id: {}, pattern_type: {}", self.id, self.pattern_id, self.pattern_type);
        } else {
             println!("[DEBUG Trade::close] NOT JPY PAIR for ID: {}, pattern_id: {}, pattern_type: {}", self.id, self.pattern_id, self.pattern_type);
        }
        // ---- END TEMPORARY DEBUG LOG ----
        
        // --- MINIMAL CHANGE TO DETERMINE pip_value ---
        let pip_value: f64;
        // Determine pip_value based on a field that reliably contains the symbol.
        // Assuming self.pattern_id is the best candidate (e.g., "EURUSD_SB-zone-123")
        // If pattern_id does NOT contain the symbol, this logic needs a reliable source for it.
        let identifier_to_check_for_symbol = &self.pattern_id; // Or self.id / self.pattern_type

        if identifier_to_check_for_symbol.contains("JPY_SB") {
            pip_value = 0.01; 
        } else if identifier_to_check_for_symbol.contains("NAS100_SB") || 
                    identifier_to_check_for_symbol.contains("US500_SB") || 
                    identifier_to_check_for_symbol.contains("XAU_SB") { // Assuming XAU_SB for gold
            pip_value = 1.0; // If SL/TP for these are in points/dollar values, and 1 point = 1.0 price change
        } else {
            pip_value = 0.0001; // Default for standard FX
        }
        // --- END MINIMAL CHANGE TO DETERMINE pip_value ---
        
        let profit_loss_price_diff = match self.direction {
            TradeDirection::Long => effective_exit_price - self.entry_price,
            TradeDirection::Short => self.entry_price - effective_exit_price,
        };

        // Calculate P/L in *true* pips/points using the determined pip_value
        let calculated_pnl_pips = if pip_value.abs() > 1e-12 {
            profit_loss_price_diff / pip_value
        } else {
            // This case should be logged more formally if log crate is used.
            println!("Error: pip_value is zero or too small for trade ID {} (pattern_id: {}). Pips set to 0.", self.id, self.pattern_id);
            0.0 
        };
        
        // Calculate monetary P/L using YOUR ORIGINAL FORMULA, but with the 
        // *newly calculated (and correctly scaled) pips*.
        // Your original formula: profit_loss_pips * 10.0 * self.lot_size
        // This implies that the "10.0" factor was designed when profit_loss_pips was always
        // based on a 0.0001 pip value.
        // To keep the monetary value's *intended scale* consistent with what it was before
        // for non-JPY pairs, while using the new true pips:
        // We need to find out what 1 true pip (from calculated_pnl_pips) is worth in terms of "0.0001 units"
        // and then apply the original 10.0 * self.lot_size scaling.
        // Number of "0.0001 units" in price_diff = profit_loss_price_diff / 0.0001
        let old_system_equivalent_pips = profit_loss_price_diff / 0.0001;
        let calculated_monetary_profit_loss = old_system_equivalent_pips * 10.0 * self.lot_size;
        
        // Assign all the values as per your original structure
        self.profit_loss = Some(calculated_monetary_profit_loss);
        self.profit_loss_pips = Some(calculated_pnl_pips); // THIS NOW HOLDS CORRECTLY SCALED PIPS
        self.exit_reason = Some(exit_reason.to_string());
        self.candlestick_idx_exit = Some(candlestick_idx);
    }
}

#[derive(Serialize)]
pub struct TradeSummary {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub total_profit_loss: f64,
    pub win_rate: f64,
    pub average_win: f64,
    pub average_loss: f64,
    pub profit_factor: f64,
    pub max_drawdown: f64,
}

impl TradeSummary {
    pub fn from_trades(trades: &[Trade]) -> Self {
        let closed_trades: Vec<&Trade> = trades
            .iter()
            .filter(|t| matches!(t.status, TradeStatus::Closed))
            .collect();
        
        let total_trades = closed_trades.len();
        
        if total_trades == 0 {
            return TradeSummary {
                total_trades: 0,
                winning_trades: 0,
                losing_trades: 0,
                total_profit_loss: 0.0,
                win_rate: 0.0,
                average_win: 0.0,
                average_loss: 0.0,
                profit_factor: 0.0,
                max_drawdown: 0.0,
            };
        }
        
        let winning_trades = closed_trades.iter()
            .filter(|t| t.profit_loss.unwrap_or(0.0) > 0.0)
            .count();
            
        let losing_trades = closed_trades.iter()
            .filter(|t| t.profit_loss.unwrap_or(0.0) <= 0.0)
            .count();
            
        let total_profit_loss = closed_trades.iter()
            .map(|t| t.profit_loss.unwrap_or(0.0))
            .sum();
            
        let win_rate = if total_trades > 0 {
            winning_trades as f64 / total_trades as f64 * 100.0
        } else {
            0.0
        };
        
        let total_wins: f64 = closed_trades.iter()
            .filter(|t| t.profit_loss.unwrap_or(0.0) > 0.0)
            .map(|t| t.profit_loss.unwrap_or(0.0))
            .sum();
            
        let total_losses: f64 = closed_trades.iter()
            .filter(|t| t.profit_loss.unwrap_or(0.0) < 0.0)
            .map(|t| t.profit_loss.unwrap_or(0.0).abs())
            .sum();
            
        let average_win = if winning_trades > 0 {
            total_wins / winning_trades as f64
        } else {
            0.0
        };
        
        let average_loss = if losing_trades > 0 {
            total_losses / losing_trades as f64
        } else {
            0.0
        };
        
        let profit_factor = if total_losses > 0.0 {
            total_wins / total_losses
        } else {
            if total_wins > 0.0 { f64::INFINITY } else { 0.0 }
        };
        
        // Calculate max drawdown (simplified approach)
        let mut max_drawdown = 0.0;
        let mut peak = 0.0;
        let mut current_balance = 0.0;
        
        for trade in closed_trades {
            current_balance += trade.profit_loss.unwrap_or(0.0);
            
            if current_balance > peak {
                peak = current_balance;
            }
            
            let drawdown = peak - current_balance;
            if drawdown > max_drawdown {
                max_drawdown = drawdown;
            }
        }
        
        TradeSummary {
            total_trades,
            winning_trades,
            losing_trades,
            total_profit_loss,
            win_rate,
            average_win,
            average_loss,
            profit_factor,
            max_drawdown,
        }
    }
}