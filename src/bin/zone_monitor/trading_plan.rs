use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OptimizedCombination {
    pub symbol: String,
    pub timeframe: String,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub profit_factor: Option<f64>,
    pub total_trades: i32,
    pub winning_trades: i32,
    pub losing_trades: i32,
    pub avg_duration: String,
    pub lot_size: f64,
    pub pattern: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DateRange {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Metadata {
    pub exported_at: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TradingPlan {
    pub optimization_date: String,
    pub total_combinations: i32,
    pub total_pnl: f64,
    pub date_range: DateRange,
    pub best_combinations: HashMap<String, OptimizedCombination>,
    pub metadata: Metadata,
}

impl TradingPlan {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Option<Self> {
        let data = fs::read_to_string(path).ok()?;
        match serde_json::from_str(&data) {
            Ok(plan) => {
                info!("📊 [TRADING_PLAN] Loaded optimization data:");
                info!("📊 [TRADING_PLAN]   - Date: {}", plan.optimization_date);
                info!("📊 [TRADING_PLAN]   - Total combinations: {}", plan.total_combinations);
                info!("📊 [TRADING_PLAN]   - Total PnL: {:.2} pips", plan.total_pnl);
                info!("📊 [TRADING_PLAN]   - Best combinations count: {}", plan.best_combinations.len());
                
                // Log a few examples
                for (key, combo) in plan.best_combinations.iter().take(3) {
                    info!("📊 [TRADING_PLAN]   - {}: SL={} TP={} PnL={:.2} pips", 
                          key, combo.stop_loss, combo.take_profit, combo.total_pnl);
                }
                
                Some(plan)
            },
            Err(e) => {
                warn!("📊 [TRADING_PLAN] Failed to parse trading plan: {}", e);
                None
            }
        }
    }

    /// Get optimized SL/TP for a specific symbol and timeframe
    pub fn get_optimized_params(&self, symbol: &str, timeframe: &str) -> Option<(f64, f64)> {
        let key = format!("{}_{}", symbol, timeframe);
        
        if let Some(combo) = self.best_combinations.get(&key) {
            info!("📊 [TRADING_PLAN] Found optimized params for {}: SL={} TP={}", 
                  key, combo.stop_loss, combo.take_profit);
            Some((combo.stop_loss, combo.take_profit))
        } else {
            warn!("📊 [TRADING_PLAN] No optimized params found for {}", key);
            None
        }
    }

    /// Check if a symbol/timeframe combination is in the trading plan
    pub fn is_combination_allowed(&self, symbol: &str, timeframe: &str) -> bool {
        let key = format!("{}_{}", symbol, timeframe);
        let allowed = self.best_combinations.contains_key(&key);
        
        if allowed {
            info!("📊 [TRADING_PLAN] Combination {} is ALLOWED by trading plan", key);
        } else {
            info!("📊 [TRADING_PLAN] Combination {} is NOT in trading plan", key);
        }
        
        allowed
    }

    /// Get all allowed symbol/timeframe combinations
    pub fn get_all_combinations(&self) -> Vec<(String, String)> {
        self.best_combinations.values()
            .map(|combo| (combo.symbol.clone(), combo.timeframe.clone()))
            .collect()
    }

    /// Get combination details for logging/debugging
    pub fn get_combination_details(&self, symbol: &str, timeframe: &str) -> Option<&OptimizedCombination> {
        let key = format!("{}_{}", symbol, timeframe);
        self.best_combinations.get(&key)
    }
} 