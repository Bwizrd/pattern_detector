// src/patterns/ma_crossover.rs

use crate::api::detect::CandleData; // Assuming CandleData is in detect.rs
use crate::zones::patterns::PatternRecognizer;
use crate::trading::trades::{Trade, TradeConfig, TradeSummary}; // Keep for trade method stub
use crate::trading::trading::TradeExecutor; // Keep for trade method stub
use serde_json::{json, Value};

// Helper function to calculate Simple Moving Average (SMA)
fn calculate_sma(data: &[f64], period: usize) -> Vec<Option<f64>> {
    if period == 0 || data.is_empty() {
        return vec![None; data.len()];
    }
    let mut sma_values = vec![None; data.len()];
    let mut sum = 0.0;

    for i in 0..data.len() {
        sum += data[i];
        if i >= period {
            sum -= data[i - period]; // Subtract the value that falls out of the window
            sma_values[i] = Some(sum / period as f64);
        } else if i == period - 1 {
            // First calculation point
            sma_values[i] = Some(sum / period as f64);
        }
        // else: Not enough data yet, remains None
    }
    sma_values
}

// --- Simplified Recognizer Struct ---
pub struct PriceSmaCrossRecognizer {
    period: usize,
}

impl Default for PriceSmaCrossRecognizer {
    fn default() -> Self {
        Self { period: 20 } // Example: 20-period SMA
    }
}

// Custom constructor if needed
impl PriceSmaCrossRecognizer {
    pub fn new(period: usize) -> Self {
       assert!(period > 0, "SMA period must be positive");
       Self { period }
   }
}

impl PatternRecognizer for PriceSmaCrossRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Value {
        let total_bars = candles.len();
        let required_bars = self.period;

        if total_bars < required_bars {
            // ... (return error JSON as before, update pattern name) ...
             return json!({
                "pattern": "price_sma_cross", // New pattern name
                "error": "Not enough candles for detection", /* ... */
                "datasets": { "price": 0, "oscillators": 0, "lines": 1, "events": 1 }, // Only 1 line now
                "data": { "lines": [], "events": [] }
            });
        }

        let close_prices: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let sma = calculate_sma(&close_prices, self.period); // Calculate single SMA
        let mut crossover_events = Vec::new();

        // Detect Crossovers (Price vs SMA)
        for i in self.period..total_bars { // Start where SMA is valid
             if let Some(sma_val) = sma[i] { // Check current SMA is valid
                let prev_close = close_prices[i - 1];
                let curr_close = close_prices[i];
                let sma_prev = sma[i-1].unwrap_or(sma_val); // Use previous or current SMA value for comparison robustness

                if prev_close <= sma_prev && curr_close > sma_val {
                    // Bullish Crossover: Price closes above SMA
                     log::info!("Price/SMA DETECT: Bullish Cross at index {}, time {}", i, candles[i].time);
                    crossover_events.push(json!({
                        "time": candles[i].time.clone(),
                        "type": "Price Above SMA",
                        "price": curr_close,
                        "sma": sma_val,
                        "candle_index": i,
                    }));
                } else if prev_close >= sma_prev && curr_close < sma_val {
                    // Bearish Crossover: Price closes below SMA
                     log::info!("Price/SMA DETECT: Bearish Cross at index {}, time {}", i, candles[i].time);
                    crossover_events.push(json!({
                        "time": candles[i].time.clone(),
                        "type": "Price Below SMA",
                        "price": curr_close,
                        "sma": sma_val,
                        "candle_index": i,
                    }));
                }
            }
        }

        // Prepare SMA line data
        let sma_line_data: Vec<Value> = sma.iter().enumerate()
            .filter_map(|(i, val)| val.map(|v| json!({"time": candles[i].time, "value": v})))
            .collect();

         log::info!("Price/SMA detect finished. Found {} crossover events.", crossover_events.len());

        // Construct the final JSON response
        json!({
            "pattern": "price_sma_cross", // New pattern name
            "total_bars": total_bars,
            "total_detected": crossover_events.len(),
            "config": { "period": self.period },
             "datasets": { "price": 0, "oscillators": 0, "lines": 1, "events": 1 }, // Only 1 line
            "data": {
                "lines": [ // Array with one line object
                    { "name": format!("SMA({})", self.period), "values": sma_line_data }
                ],
                "events": {
                     "crossovers": crossover_events // Rename if desired
                 }
            }
        })
    } 

    // Basic trade implementation stub - Enters on crossover close
    fn trade(&self, candles: &[CandleData], config: TradeConfig) -> (Vec<Trade>, TradeSummary) {
        log::info!("Price/SMA TRADE: Entering trade method (delegating to executor).");

        // 1. Get pattern data (crossover events)
        let pattern_data = self.detect(candles);

        // 2. Create executor instance (or receive one if framework changes)
        let default_symbol_for_recognizer_trade = "UNKNOWN_SYMBOL_IN_RECOGNIZER";
        let executor = TradeExecutor::new(config, 
            default_symbol_for_recognizer_trade,
            None,
            None);
        log::warn!("Price/SMA TRADE: Creating new TradeExecutor. Minute candle context might be lost.");

        // 3. Call executor to handle trades based on this pattern's data
        log::info!("Price/SMA TRADE: Calling executor.execute_trades_for_pattern...");
        let trades = executor.execute_trades_for_pattern(
            "price_sma_cross", // Pass NEW pattern name
            &pattern_data,     // Pass data containing events
            candles
        );
        log::info!("Price/SMA TRADE: Executor finished. Found {} trades.", trades.len());

        // 4. Generate summary
        let summary = TradeSummary::from_trades(&trades);
        log::info!("Price/SMA TRADE: Summary calculated.");

        (trades, summary)
    }
}

impl Default for TradeSummary {
    fn default() -> Self {
       TradeSummary {
           total_trades: 0, winning_trades: 0, losing_trades: 0,
           total_profit_loss: 0.0, win_rate: 0.0, average_win: 0.0,
           average_loss: 0.0, profit_factor: 0.0, max_drawdown: 0.0,
       }
   }
}
