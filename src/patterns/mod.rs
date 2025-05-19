// src/patterns/mod.rs
use crate::detect::CandleData;
use serde_json::Value;
// Trait for pattern recognizers
pub trait PatternRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Value; // Return a single JSON object

    // Optional method for trading - default implementation just uses the standard executor
    fn trade(&self, candles: &[CandleData], config: TradeConfig) -> (Vec<Trade>, TradeSummary) {
        let pattern_data = self.detect(candles);
        let pattern_type = pattern_data["pattern"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        let default_symbol_for_recognizer_trade = "UNKNOWN_SYMBOL_IN_RECOGNIZER";
        let executor = TradeExecutor::new(config, 
            default_symbol_for_recognizer_trade,
            None,
            None);
        let trades = executor.execute_trades_for_pattern(&pattern_type, &pattern_data, candles);
        let summary = TradeSummary::from_trades(&trades);

        (trades, summary)
    }
    // New method that allows passing in a pre-configured executor
    fn trade_with_executor(
        &self,
        candles: &[CandleData],
        executor: &TradeExecutor,
    ) -> (Vec<Trade>, TradeSummary) {
        let pattern_data = self.detect(candles);
        let pattern_type = pattern_data["pattern"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        let trades = executor.execute_trades_for_pattern(&pattern_type, &pattern_data, candles);
        let summary = TradeSummary::from_trades(&trades);

        (trades, summary)
    }
}

// Declare submodules
mod FiftyPercentBeforeBigBar;
mod FiftyPercentBodyCandle;
mod big_bar; // Add new module
mod bullish_engulfing;
mod combined_demand;
mod consolidation;
mod demand_move_away;
mod demand_zone;
mod drop_base_rally;
mod pin_bar; // Add new module
mod rally;
mod supply_demand;
mod supply_demand_zone_simple;
mod supply_zone; // Add new module
mod ma_crossover; // Add new module
mod specific_time_entry; // Add declaration
// ... other modules ...


use crate::trades::{Trade, TradeConfig, TradeSummary};
use crate::trading::TradeExecutor;

// Export recognizers
pub use big_bar::BigBarRecognizer;
pub use bullish_engulfing::BullishEngulfingRecognizer;
pub use combined_demand::CombinedDemandRecognizer;
pub use consolidation::ConsolidationRecognizer;
pub use demand_move_away::DemandMoveAwayRecognizer;
pub use demand_zone::FlexibleDemandZoneRecognizer;
pub use drop_base_rally::DropBaseRallyRecognizer;
pub use pin_bar::PinBarRecognizer;
pub use rally::RallyRecognizer;
pub use supply_demand::EnhancedSupplyDemandZoneRecognizer;
pub use supply_demand_zone_simple::SimpleSupplyDemandZoneRecognizer;
pub use supply_zone::SupplyZoneRecognizer;
pub use FiftyPercentBeforeBigBar::FiftyPercentBeforeBigBarRecognizer;
pub use FiftyPercentBodyCandle::FiftyPercentBodyCandleRecognizer;
pub use ma_crossover::PriceSmaCrossRecognizer;
pub use specific_time_entry::SpecificTimeEntryRecognizer; 
// Array of all recognizers - use later when we combine patterns
// pub const ALL_RECOGNIZERS: &[&dyn PatternRecognizer] = &[
//     &DemandZoneRecognizer,
//     &BullishEngulfingRecognizer,
//     &SupplyZoneRecognizer,
//     &PinBarRecognizer,
//     &BigBarRecognizer,
// ];
