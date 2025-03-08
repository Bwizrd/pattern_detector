
// src/patterns/mod.rs
use crate::detect::CandleData;
use serde_json::Value;
// Trait for pattern recognizers
pub trait PatternRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Value; // Return a single JSON object
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

// Export recognizers
pub use demand_zone::FlexibleDemandZoneRecognizer;
pub use bullish_engulfing::BullishEngulfingRecognizer;
pub use supply_zone::SupplyZoneRecognizer;
pub use pin_bar::PinBarRecognizer;
pub use big_bar::BigBarRecognizer;
pub use rally::RallyRecognizer;
pub use supply_demand::SupplyDemandZoneRecognizer;
pub use supply_demand_zone_simple::SimpleSupplyDemandZoneRecognizer;
pub use consolidation::ConsolidationRecognizer;
pub use drop_base_rally::DropBaseRallyRecognizer;
pub use demand_move_away::DemandMoveAwayRecognizer;

// Array of all recognizers - use later when we combine patterns
// pub const ALL_RECOGNIZERS: &[&dyn PatternRecognizer] = &[
//     &DemandZoneRecognizer,
//     &BullishEngulfingRecognizer,
//     &SupplyZoneRecognizer,
//     &PinBarRecognizer, 
//     &BigBarRecognizer,
// ];

