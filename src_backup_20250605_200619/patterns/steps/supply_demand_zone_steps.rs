use cucumber::{given, when, then, World};
use crate::api::detect::CandleData;
use crate::zones::patterns::SupplyDemandZoneRecognizer;
use serde_json::Value;

#[derive(World, Debug, Default)]
pub struct SupplyDemandZoneWorld {
    pub candles: Vec<CandleData>,
    pub result: Option<Value>,
}

#[given("a series of candles with a strong bullish candle")]
fn a_series_with_bullish_candle(world: &mut SupplyDemandZoneWorld) {
    world.candles.push(CandleData {
        time: "2023-10-01T00:00:00Z".to_string(),
        high: 1.3050,
        low: 1.3000,
        open: 1.3005,
        close: 1.3045, // Bullish candle
    });
}

#[given("followed by a bearish imbalance candle")]
fn followed_by_bearish_imbalance(world: &mut SupplyDemandZoneWorld) {
    world.candles.push(CandleData {
        time: "2023-10-01T00:01:00Z".to_string(),
        high: 1.3030,
        low: 1.2980,
        open: 1.3025,
        close: 1.2985, // Bearish candle with imbalance
    });
    
    // Add a third candle for context
    world.candles.push(CandleData {
        time: "2023-10-01T00:02:00Z".to_string(),
        high: 1.2970,
        low: 1.2940,
        open: 1.2965,
        close: 1.2945, // Continuation
    });
}

#[given("a series of candles with a strong bearish candle")]
fn a_series_with_bearish_candle(world: &mut SupplyDemandZoneWorld) {
    world.candles.push(CandleData {
        time: "2023-10-01T00:03:00Z".to_string(),
        high: 1.2960,
        low: 1.2900,
        open: 1.2955,
        close: 1.2905, // Bearish candle
    });
}

#[given("followed by a bullish imbalance candle")]
fn followed_by_bullish_imbalance(world: &mut SupplyDemandZoneWorld) {
    world.candles.push(CandleData {
        time: "2023-10-01T00:04:00Z".to_string(),
        high: 1.2990,
        low: 1.2950,
        open: 1.2955,
        close: 1.2985, // Bullish candle with imbalance
    });
    
    // Add a third candle for context
    world.candles.push(CandleData {
        time: "2023-10-01T00:05:00Z".to_string(),
        high: 1.3010,
        low: 1.2980,
        open: 1.2985,
        close: 1.3005, // Continuation
    });
}

#[when("the recognizer is run")]
fn recognizer_is_run(world: &mut SupplyDemandZoneWorld) {
    let recognizer = SupplyDemandZoneRecognizer::default();
    world.result = Some(recognizer.detect(&world.candles));
}

#[then("a supply zone should be identified")]
fn supply_zone_identified(world: &mut SupplyDemandZoneWorld) {
    let result = world.result.as_ref().unwrap();
    let supply_zones = result.get("supply_zones").unwrap().as_u64().unwrap();
    assert!(supply_zones > 0, "Should detect at least one supply zone");
}

#[then("the zone should be marked from the high of the bullish candle")]
fn zone_marked_from_high(world: &mut SupplyDemandZoneWorld) {
    let result = world.result.as_ref().unwrap();
    let data = result.get("data").unwrap().as_array().unwrap();
    
    let supply_zone = data.iter()
        .find(|z| z["type"] == "supply_zone")
        .expect("Supply zone not found");
    
    let zone_high = supply_zone["zone_high"].as_f64().unwrap();
    
    // The high of the first candle is 1.3050
    assert_eq!(zone_high, 1.3050, "Zone high should be the high of the bullish candle");
}

#[then("the zone should have a 50% line as the lower boundary")]
fn zone_has_fifty_percent_line_lower(world: &mut SupplyDemandZoneWorld) {
    let result = world.result.as_ref().unwrap();
    let data = result.get("data").unwrap().as_array().unwrap();
    
    let supply_zone = data.iter()
        .find(|z| z["type"] == "supply_zone")
        .expect("Supply zone not found");
    
    let zone_low = supply_zone["zone_low"].as_f64().unwrap();
    let fifty_percent_line = supply_zone["fifty_percent_line"].as_f64().unwrap();
    
    // The 50% calculation from high to low is (1.3050 + 1.3000) / 2 = 1.3025
    assert_eq!(zone_low, 1.3025, "Zone low should be at the 50% line");
    assert_eq!(fifty_percent_line, zone_low, "50% line should match zone low");
}

#[then("a demand zone should be identified")]
fn demand_zone_identified(world: &mut SupplyDemandZoneWorld) {
    let result = world.result.as_ref().unwrap();
    let demand_zones = result.get("demand_zones").unwrap().as_u64().unwrap();
    assert!(demand_zones > 0, "Should detect at least one demand zone");
}

#[then("the zone should be marked from the low of the bearish candle")]
fn zone_marked_from_low(world: &mut SupplyDemandZoneWorld) {
    let result = world.result.as_ref().unwrap();
    let data = result.get("data").unwrap().as_array().unwrap();
    
    let demand_zone = data.iter()
        .find(|z| z["type"] == "demand_zone")
        .expect("Demand zone not found");
    
    let zone_low = demand_zone["zone_low"].as_f64().unwrap();
    
    // The low of the bearish candle is 1.2900
    assert_eq!(zone_low, 1.2900, "Zone low should be the low of the bearish candle");
}

#[then("the zone should have a 50% line as the upper boundary")]
fn zone_has_fifty_percent_line_upper(world: &mut SupplyDemandZoneWorld) {
    let result = world.result.as_ref().unwrap();
    let data = result.get("data").unwrap().as_array().unwrap();
    
    let demand_zone = data.iter()
        .find(|z| z["type"] == "demand_zone")
        .expect("Demand zone not found");
    
    let zone_high = demand_zone["zone_high"].as_f64().unwrap();
    let fifty_percent_line = demand_zone["fifty_percent_line"].as_f64().unwrap();
    
    // The 50% calculation from high to low is (1.2960 + 1.2900) / 2 = 1.2930
    assert_eq!(zone_high, 1.2930, "Zone high should be at the 50% line");
    assert_eq!(fifty_percent_line, zone_high, "50% line should match zone high");
}