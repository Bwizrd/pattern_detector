use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

pub struct SupplyDemandZoneRecognizer {
    // Configuration parameters
    min_imbalance_pips: f64,    // Minimum imbalance size in pips
    zone_lookback: usize,       // Number of candles to lookback for zone creation
    pip_value: f64,             // Value of 1 pip for the instrument
}

impl SupplyDemandZoneRecognizer {
    pub fn new(min_imbalance_pips: f64, zone_lookback: usize, pip_value: f64) -> Self {
        Self {
            min_imbalance_pips,
            zone_lookback,
            pip_value,
        }
    }
    
    pub fn default() -> Self {
        // Default configuration
        Self {
            min_imbalance_pips: 0.0010, // 10 pips minimum imbalance
            zone_lookback: 20,          // Look back 20 candles
            pip_value: 0.0001,          // Standard pip value for forex
        }
    }

    // Helper function to detect supply zones (buy-then-sell pattern)
    fn detect_supply_zones(&self, candles: &[CandleData]) -> Vec<serde_json::Value> {
        let mut supply_zones = Vec::new();
        
        // Need at least 3 candles to detect the pattern
        if candles.len() < 3 {
            return supply_zones;
        }
        
        for i in 1..candles.len() - 1 {
            // Check for a strong bullish candle (green)
            let bullish_candle = &candles[i-1];
            if bullish_candle.close <= bullish_candle.open {
                continue; // Not a bullish candle
            }
            
            // Calculate candle size
            let candle_size = bullish_candle.high - bullish_candle.low;
            if candle_size < self.min_imbalance_pips {
                continue; // Not strong enough
            }
            
            // Check for a bearish imbalance (red candle)
            let bearish_candle = &candles[i];
            if bearish_candle.close >= bearish_candle.open {
                continue; // Not a bearish candle
            }
            
            // Check for imbalance from wick to wick
            let imbalance = bearish_candle.high < bullish_candle.high && 
                            bearish_candle.low < bullish_candle.low;
            
            if !imbalance {
                continue; // No imbalance detected
            }
            
            // Calculate the supply zone
            let zone_high = bullish_candle.high;
            let zone_low = (bullish_candle.high + bullish_candle.low) / 2.0; // 50% zone line
            
            // Parse timestamps
            let start_time = DateTime::parse_from_rfc3339(&bullish_candle.time)
                .unwrap_or(Utc::now().into())
                .with_timezone(&Utc);
            let end_time = DateTime::parse_from_rfc3339(&bearish_candle.time)
                .unwrap_or(Utc::now().into())
                .with_timezone(&Utc) + Duration::minutes(1);
                
            supply_zones.push(json!({
                "type": "supply_zone",
                "start_time": bullish_candle.time.clone(),
                "end_time": end_time.to_rfc3339(),
                "zone_high": zone_high,
                "zone_low": zone_low,
                "fifty_percent_line": zone_low,
                "strength": candle_size / self.pip_value, // Strength in pips
            }));
        }
        
        supply_zones
    }
    
    // Helper function to detect demand zones (sell-then-buy pattern)
    fn detect_demand_zones(&self, candles: &[CandleData]) -> Vec<serde_json::Value> {
        let mut demand_zones = Vec::new();
        
        // Need at least 3 candles to detect the pattern
        if candles.len() < 3 {
            return demand_zones;
        }
        
        for i in 1..candles.len() - 1 {
            // Check for a strong bearish candle (red)
            let bearish_candle = &candles[i-1];
            if bearish_candle.close >= bearish_candle.open {
                continue; // Not a bearish candle
            }
            
            // Calculate candle size
            let candle_size = bearish_candle.high - bearish_candle.low;
            if candle_size < self.min_imbalance_pips {
                continue; // Not strong enough
            }
            
            // Check for a bullish imbalance (green candle)
            let bullish_candle = &candles[i];
            if bullish_candle.close <= bullish_candle.open {
                continue; // Not a bullish candle
            }
            
            // Check for imbalance from wick to wick
            let imbalance = bullish_candle.low > bearish_candle.low && 
                           bullish_candle.high > bearish_candle.high;
            
            if !imbalance {
                continue; // No imbalance detected
            }
            
            // Calculate the demand zone
            let zone_low = bearish_candle.low;
            let zone_high = (bearish_candle.high + bearish_candle.low) / 2.0; // 50% zone line
            
            // Parse timestamps
            let start_time = DateTime::parse_from_rfc3339(&bearish_candle.time)
                .unwrap_or(Utc::now().into())
                .with_timezone(&Utc);
            let end_time = DateTime::parse_from_rfc3339(&bullish_candle.time)
                .unwrap_or(Utc::now().into())
                .with_timezone(&Utc) + Duration::minutes(1);
                
            demand_zones.push(json!({
                "type": "demand_zone",
                "start_time": bearish_candle.time.clone(),
                "end_time": end_time.to_rfc3339(),
                "zone_high": zone_high,
                "zone_low": zone_low,
                "fifty_percent_line": zone_high,
                "strength": candle_size / self.pip_value, // Strength in pips
            }));
        }
        
        demand_zones
    }
}

impl PatternRecognizer for SupplyDemandZoneRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        let supply_zones = self.detect_supply_zones(candles);
        let demand_zones = self.detect_demand_zones(candles);
        
        let total_zones = supply_zones.len() + demand_zones.len();
        
        // Combine the zones
        let mut all_zones = Vec::new();
        all_zones.extend(supply_zones);
        all_zones.extend(demand_zones);
        
        // Construct the response object
        json!({
            "pattern": "supply_demand_zone",
            "config": {
                "min_imbalance_pips": self.min_imbalance_pips,
                "zone_lookback": self.zone_lookback,
                "pip_value": self.pip_value,
            },
            "total_bars": candles.len(),
            "total_detected": total_zones,
            "supply_zones": all_zones.iter().filter(|z| z["type"] == "supply_zone").collect::<Vec<_>>().len(),
            "demand_zones": all_zones.iter().filter(|z| z["type"] == "demand_zone").collect::<Vec<_>>().len(),
            "data": all_zones,
        })
    }
}

#[test]
fn test_supply_demand_zone_recognizer() {
    let recognizer = SupplyDemandZoneRecognizer::default();
    
    // Test data that should trigger both supply and demand zones
    let candles = vec![
        // Supply zone pattern (buy-then-sell)
        CandleData {
            time: "2023-10-01T00:00:00Z".to_string(),
            high: 1.3050,
            low: 1.3000,
            open: 1.3005,
            close: 1.3045, // Bullish candle
        },
        CandleData {
            time: "2023-10-01T00:01:00Z".to_string(),
            high: 1.3030,
            low: 1.2980,
            open: 1.3025,
            close: 1.2985, // Bearish candle with imbalance
        },
        CandleData {
            time: "2023-10-01T00:02:00Z".to_string(),
            high: 1.2970,
            low: 1.2940,
            open: 1.2965,
            close: 1.2945, // Continuation
        },
        
        // Demand zone pattern (sell-then-buy)
        CandleData {
            time: "2023-10-01T00:03:00Z".to_string(),
            high: 1.2960,
            low: 1.2900,
            open: 1.2955,
            close: 1.2905, // Bearish candle
        },
        CandleData {
            time: "2023-10-01T00:04:00Z".to_string(),
            high: 1.2990,
            low: 1.2950,
            open: 1.2955,
            close: 1.2985, // Bullish candle with imbalance
        },
        CandleData {
            time: "2023-10-01T00:05:00Z".to_string(),
            high: 1.3010,
            low: 1.2980,
            open: 1.2985,
            close: 1.3005, // Continuation
        },
    ];

    let result = recognizer.detect(&candles);
    println!("Detection result: {:?}", result);
    
    // Check if we detected at least one supply zone and one demand zone
    let data = result.get("data").unwrap().as_array().unwrap();
    let supply_zones = result.get("supply_zones").unwrap().as_u64().unwrap();
    let demand_zones = result.get("demand_zones").unwrap().as_u64().unwrap();
    
    assert!(supply_zones > 0, "Should detect at least one supply zone");
    assert!(demand_zones > 0, "Should detect at least one demand zone");
}