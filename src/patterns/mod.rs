
// src/patterns/mod.rs
use crate::detect::CandleData;
use serde_json::Value;
// Trait for pattern recognizers
pub trait PatternRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Value; // Return a single JSON object

     // Optional method for trading - default implementation just uses the standard executor
     fn trade(&self, candles: &[CandleData], config: TradeConfig) -> (Vec<Trade>, TradeSummary) {
        let pattern_data = self.detect(candles);
        let pattern_type = pattern_data["pattern"].as_str().unwrap_or("unknown").to_string();
        
        let executor = TradeExecutor::new(config);
        let trades = executor.execute_trades_for_pattern(&pattern_type, &pattern_data, candles);
        let summary = TradeSummary::from_trades(&trades);
        
        (trades, summary)
    }
}

// Declare submodules
mod demand_zone;
mod bullish_engulfing;
mod supply_zone; // Add new module
mod pin_bar; // Add new module
mod big_bar; // Add new module
mod rally;
mod supply_demand;
mod supply_demand_zone_simple;
mod consolidation;
mod drop_base_rally;
mod demand_move_away;
mod combined_demand;
mod FiftyPercentBodyCandle;
mod FiftyPercentBeforeBigBar;

use crate::trades::{Trade, TradeConfig, TradeSummary};
use crate::trading::TradeExecutor;

// Export recognizers
pub use demand_zone::FlexibleDemandZoneRecognizer;
pub use bullish_engulfing::BullishEngulfingRecognizer;
pub use supply_zone::SupplyZoneRecognizer;
pub use pin_bar::PinBarRecognizer;
pub use big_bar::BigBarRecognizer;
pub use rally::RallyRecognizer;
pub use supply_demand::EnhancedSupplyDemandZoneRecognizer;
pub use supply_demand_zone_simple::SimpleSupplyDemandZoneRecognizer;
pub use consolidation::ConsolidationRecognizer;
pub use drop_base_rally::DropBaseRallyRecognizer;
pub use demand_move_away::DemandMoveAwayRecognizer;
pub use combined_demand::CombinedDemandRecognizer;
pub use FiftyPercentBodyCandle::FiftyPercentBodyCandleRecognizer;
pub use FiftyPercentBeforeBigBar::FiftyPercentBeforeBigBarRecognizer;

// Array of all recognizers - use later when we combine patterns
// pub const ALL_RECOGNIZERS: &[&dyn PatternRecognizer] = &[
//     &DemandZoneRecognizer,
//     &BullishEngulfingRecognizer,
//     &SupplyZoneRecognizer,
//     &PinBarRecognizer, 
//     &BigBarRecognizer,
// ];

