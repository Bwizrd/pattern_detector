
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

// Export recognizers
pub use demand_zone::DemandZoneRecognizer;
pub use bullish_engulfing::BullishEngulfingRecognizer;
pub use supply_zone::SupplyZoneRecognizer;
pub use pin_bar::PinBarRecognizer;
pub use big_bar::BigBarRecognizer;

// Array of all recognizers - use later when we combine patterns
// pub const ALL_RECOGNIZERS: &[&dyn PatternRecognizer] = &[
//     &DemandZoneRecognizer,
//     &BullishEngulfingRecognizer,
//     &SupplyZoneRecognizer,
//     &PinBarRecognizer, 
//     &BigBarRecognizer,
// ];