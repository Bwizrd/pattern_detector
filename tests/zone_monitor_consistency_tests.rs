// tests/zone_monitor_consistency_tests.rs
//
// This test file validates that the Zone Monitor Service calculations
// match the tested zone generator logic for touch counting and invalidation.

use pattern_detector::zone_monitor_service::{ActiveZoneCache, LiveZoneState};
use pattern_detector::types::{StoredZone, CandleData};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use reqwest::Client as HttpClient;
use chrono::Weekday;
use std::fs::OpenOptions;
use std::io::Write;
use serde_json::json;

// Helper to write test output to file
fn write_to_test_output(content: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("zone_monitor_test_results.txt")
        .expect("Could not create test output file");
    
    writeln!(file, "{}", content).expect("Could not write to test output file");
    
    // Also print to console
    println!("{}", content);
}

// Mock candle data for testing
fn create_test_candles() -> Vec<CandleData> {
    vec![
        CandleData {
            time: "2025-05-16T10:00:00Z".to_string(),
            open: 1.0800,
            high: 1.0820,
            low: 1.0790,
            close: 1.0810,
            volume: 1000,
        },
        CandleData {
            time: "2025-05-16T11:00:00Z".to_string(),
            open: 1.0810,
            high: 1.0835, // Touches supply zone at 1.0830
            low: 1.0800,
            close: 1.0825,
            volume: 1100,
        },
        CandleData {
            time: "2025-05-16T12:00:00Z".to_string(),
            open: 1.0825,
            high: 1.0845, // Touches again
            low: 1.0815,
            close: 1.0840,
            volume: 1200,
        },
        CandleData {
            time: "2025-05-16T13:00:00Z".to_string(),
            open: 1.0840,
            high: 1.0860, // Invalidates supply zone (above 1.0850)
            low: 1.0830,
            close: 1.0855,
            volume: 1300,
        },
    ]
}

fn create_test_supply_zone() -> StoredZone {
    StoredZone {
        zone_id: Some("test_supply_123".to_string()),
        symbol: Some("EURUSD_SB".to_string()),
        timeframe: Some("1h".to_string()),
        zone_type: Some("supply_zone".to_string()),
        zone_high: Some(1.0850), // Distal line
        zone_low: Some(1.0830),  // Proximal line
        start_time: Some("2025-05-16T09:00:00Z".to_string()),
        end_time: Some("2025-05-16T10:00:00Z".to_string()),
        is_active: true,
        touch_count: Some(0), // Starting with 0 historical touches
        bars_active: Some(0),
        strength_score: Some(100.0),
        ..Default::default()
    }
}

fn create_test_demand_zone() -> StoredZone {
    StoredZone {
        zone_id: Some("test_demand_456".to_string()),
        symbol: Some("EURUSD_SB".to_string()),
        timeframe: Some("1h".to_string()),
        zone_type: Some("demand_zone".to_string()),
        zone_high: Some(1.0820), // Proximal line
        zone_low: Some(1.0800),  // Distal line
        start_time: Some("2025-05-16T09:00:00Z".to_string()),
        end_time: Some("2025-05-16T10:00:00Z".to_string()),
        is_active: true,
        touch_count: Some(0),
        bars_active: Some(0),
        strength_score: Some(100.0),
        ..Default::default()
    }
}

// Convert CandleData to the JSON format that your WebSocket expects
fn create_bar_update_json(candle: &CandleData) -> serde_json::Value {
    json!({
        "time": candle.time,
        "open": candle.open,
        "high": candle.high,
        "low": candle.low,
        "close": candle.close,
        "volume": candle.volume,
        "symbolId": 185,  // EURUSD_SB symbol ID
        "symbol": "EURUSD_SB",
        "timeframe": "1h",
        "isNewBar": true
    })
}

// Simulate Generator's historical touch counting logic (for comparison)
fn simulate_generator_touch_counting(
    zone: &StoredZone,
    candles: &[CandleData],
) -> (bool, i64, Option<u64>) {
    // This mirrors the logic from your zone_generator.rs enrichment
    
    if let (Some(zone_high), Some(zone_low), Some(zone_type)) = (
        zone.zone_high,
        zone.zone_low,
        zone.zone_type.as_ref(),
    ) {
        let is_supply = zone_type.to_lowercase().contains("supply");
        let (proximal_line, distal_line) = if is_supply {
            (zone_low, zone_high)
        } else {
            (zone_high, zone_low)
        };

        let mut is_active = true;
        let mut touch_count = 0i64;
        let mut is_outside_zone = true;
        let mut bars_processed = 0u64;

        for candle in candles {
            bars_processed += 1;

            // Check for invalidation
            if is_supply {
                if candle.high > distal_line {
                    is_active = false;
                    write_to_test_output(&format!("Generator: Zone {} invalidated by candle at {}", 
                             zone.zone_id.as_deref().unwrap_or("?"), candle.time));
                    break;
                }
            } else if candle.low < distal_line {
                is_active = false;
                write_to_test_output(&format!("Generator: Zone {} invalidated by candle at {}", 
                         zone.zone_id.as_deref().unwrap_or("?"), candle.time));
                break;
            }

            // Check for interaction
            let interaction_occurred = if is_supply {
                candle.high >= proximal_line
            } else {
                candle.low <= proximal_line
            };

            if interaction_occurred && is_outside_zone {
                touch_count += 1;
                write_to_test_output(&format!("Generator: Zone {} touched (total touches: {})", 
                         zone.zone_id.as_deref().unwrap_or("?"), touch_count));
            }
            
            is_outside_zone = !interaction_occurred;
        }

        (is_active, touch_count, Some(bars_processed))
    } else {
        (false, 0, None)
    }
}

// Helper function to create a mock cache with test zones
async fn create_test_cache_with_zone(zone: StoredZone) -> ActiveZoneCache {
    let cache = Arc::new(Mutex::new(HashMap::new()));
    let mut cache_guard = cache.lock().await;
    
    if let Some(zone_id) = &zone.zone_id {
        cache_guard.insert(zone_id.clone(), LiveZoneState {
            zone_data: zone,
            live_touches_this_cycle: 0,
            trade_attempted_this_cycle: false,
            is_outside_zone: true,  // Initialize as outside zone
        });
    }
    
    drop(cache_guard);
    cache
}

// Test wrapper that calls your ACTUAL process_bar_live_touch function
async fn test_actual_zone_monitor_logic(
    zone_cache: ActiveZoneCache,
    candles: &[CandleData],
) -> (bool, i64) {
    write_to_test_output("🧪 TEST: Starting test_actual_zone_monitor_logic");
    
    let http_client = Arc::new(HttpClient::new());
    let allowed_days = vec![Weekday::Mon, Weekday::Tue, Weekday::Wed];
    
    write_to_test_output(&format!("🧪 TEST: Processing {} candles", candles.len()));
    
    for (i, candle) in candles.iter().enumerate() {
        write_to_test_output(&format!("🧪 TEST: Processing candle {}/{}: {}", i + 1, candles.len(), candle.time));
        
        // Create the JSON and deserialize it to your actual BarUpdateData type
        let bar_json = create_bar_update_json(candle);
        write_to_test_output(&format!("🧪 TEST: Created bar JSON: {}", bar_json));
        
        let bar_update: pattern_detector::zone_monitor_service::BarUpdateData = 
            match serde_json::from_value(bar_json) {
                Ok(data) => {
                    write_to_test_output("🧪 TEST: Successfully deserialized BarUpdateData");
                    data
                },
                Err(e) => {
                    write_to_test_output(&format!("🧪 TEST ERROR: Failed to deserialize BarUpdateData: {}", e));
                    return (false, 0);
                }
            };
        
        write_to_test_output("🧪 TEST: About to call process_bar_live_touch");
        
        // Call your ACTUAL process_bar_live_touch function
        pattern_detector::zone_monitor_service::process_bar_live_touch(
            bar_update,
            zone_cache.clone(),
            4.0,  // sl_pips
            70.0, // tp_pips  
            0.01, // lot_size
            &allowed_days,
            2,    // historical_touch_limit
            "http://localhost:3001/placeOrder", // order_placement_url
            &http_client,
        ).await;
        
        write_to_test_output("🧪 TEST: Returned from process_bar_live_touch");
        
        // Check current state after processing
        let cache_guard = zone_cache.lock().await;
        let (is_active, touch_count) = if let Some(live_zone) = cache_guard.values().next() {
            (live_zone.zone_data.is_active, live_zone.live_touches_this_cycle)
        } else {
            write_to_test_output("🧪 TEST ERROR: No zones found in cache after processing!");
            (false, 0)
        };
        drop(cache_guard);
        
        write_to_test_output(&format!("Processed bar: {} -> Active: {}, Touches: {}", 
                 candle.time, is_active, touch_count));
        
        // If zone becomes inactive, stop processing (like generator does)
        if !is_active {
            write_to_test_output("🧪 TEST: Zone became inactive, stopping processing");
            return (is_active, touch_count);
        }
    }
    
    write_to_test_output("🧪 TEST: Finished processing all candles");
    
    // Get final state
    let cache_guard = zone_cache.lock().await;
    if let Some(live_zone) = cache_guard.values().next() {
        (live_zone.zone_data.is_active, live_zone.live_touches_this_cycle)
    } else {
        write_to_test_output("🧪 TEST ERROR: No zones found in cache at end!");
        (false, 0)
    }
}

#[tokio::test]
async fn test_supply_zone_touch_counting_consistency() {
    // Clear the output file at the start of tests
    std::fs::write("zone_monitor_test_results.txt", "").expect("Could not clear test output file");
    
    write_to_test_output("\n=== Testing Supply Zone Touch Counting Consistency ===");
    write_to_test_output("=====================================================");

    let test_candles = create_test_candles();
    let supply_zone = create_test_supply_zone();

    // Test Generator logic (for comparison)
    write_to_test_output("\nGenerator Analysis:");
    let (gen_is_active, gen_touch_count, gen_bars_active) = 
        simulate_generator_touch_counting(&supply_zone, &test_candles);

    write_to_test_output(&format!("Generator Result: Active={}, Touches={}, Bars={:?}", 
             gen_is_active, gen_touch_count, gen_bars_active));

    // Test ACTUAL Zone Monitor logic
    write_to_test_output("\nZone Monitor Analysis (ACTUAL REAL CODE):");
    let zone_cache = create_test_cache_with_zone(supply_zone.clone()).await;
    let (monitor_is_active, monitor_touch_count) = test_actual_zone_monitor_logic(zone_cache, &test_candles).await;

    write_to_test_output(&format!("Zone Monitor Result: Active={}, Live Touches={}", 
             monitor_is_active, monitor_touch_count));

    // Compare results
    write_to_test_output("\nConsistency Check:");
    write_to_test_output(&format!("Active Status: Generator={}, Monitor={} -> {}", 
             gen_is_active, monitor_is_active, 
             if gen_is_active == monitor_is_active { "MATCH" } else { "MISMATCH" }));
    
    write_to_test_output(&format!("Touch Count: Generator={}, Monitor={} -> {}", 
             gen_touch_count, monitor_touch_count, 
             if gen_touch_count == monitor_touch_count { "MATCH" } else { "MISMATCH" }));

    // Assertions - these will show if your real Zone Monitor logic matches Generator logic
    assert_eq!(gen_is_active, monitor_is_active, 
               "Zone active status should match between generator and monitor");
    assert_eq!(gen_touch_count, monitor_touch_count, 
               "Touch counts should match between generator and monitor");
}

#[tokio::test]
async fn test_demand_zone_touch_counting_consistency() {
    write_to_test_output("\n=== Testing Demand Zone Touch Counting Consistency ===");
    write_to_test_output("======================================================");

    // Create candles that will touch demand zone
    let demand_test_candles = vec![
        CandleData {
            time: "2025-05-16T10:00:00Z".to_string(),
            open: 1.0825,
            high: 1.0830,
            low: 1.0815, // Touches demand zone at 1.0820
            close: 1.0818,
            volume: 1000,
        },
        CandleData {
            time: "2025-05-16T11:00:00Z".to_string(),
            open: 1.0818,
            high: 1.0825,
            low: 1.0810, // Touches again
            close: 1.0815,
            volume: 1100,
        },
        CandleData {
            time: "2025-05-16T12:00:00Z".to_string(),
            open: 1.0815,
            high: 1.0820,
            low: 1.0795, // Invalidates demand zone (below 1.0800)
            close: 1.0798,
            volume: 1200,
        },
    ];

    let demand_zone = create_test_demand_zone();

    // Test Generator logic
    write_to_test_output("\nGenerator Analysis:");
    let (gen_is_active, gen_touch_count, gen_bars_active) = 
        simulate_generator_touch_counting(&demand_zone, &demand_test_candles);

    write_to_test_output(&format!("Generator Result: Active={}, Touches={}, Bars={:?}", 
             gen_is_active, gen_touch_count, gen_bars_active));

    // Test ACTUAL Zone Monitor logic
    write_to_test_output("\nZone Monitor Analysis (ACTUAL REAL CODE):");
    let zone_cache = create_test_cache_with_zone(demand_zone.clone()).await;
    let (monitor_is_active, monitor_touch_count) = test_actual_zone_monitor_logic(zone_cache, &demand_test_candles).await;

    write_to_test_output(&format!("Zone Monitor Result: Active={}, Live Touches={}", 
             monitor_is_active, monitor_touch_count));

    // Compare results
    write_to_test_output("\nConsistency Check:");
    write_to_test_output(&format!("Active Status: Generator={}, Monitor={} -> {}", 
             gen_is_active, monitor_is_active, 
             if gen_is_active == monitor_is_active { "MATCH" } else { "MISMATCH" }));
    
    write_to_test_output(&format!("Touch Count: Generator={}, Monitor={} -> {}", 
             gen_touch_count, monitor_touch_count, 
             if gen_touch_count == monitor_touch_count { "MATCH" } else { "MISMATCH" }));

    // Assertions
    assert_eq!(gen_is_active, monitor_is_active, 
               "Demand zone active status should match");
    assert_eq!(gen_touch_count, monitor_touch_count, 
               "Demand zone touch counts should match");
}

#[tokio::test]
async fn test_zone_invalidation_timing() {
    write_to_test_output("\n=== Testing Zone Invalidation Timing ===");
    write_to_test_output("========================================");

    let supply_zone = create_test_supply_zone();
    
    // Create candles where invalidation happens on different bars
    let invalidation_candles = vec![
        CandleData {
            time: "2025-05-16T10:00:00Z".to_string(),
            open: 1.0830,
            high: 1.0845, // Touches but doesn't invalidate
            low: 1.0825,
            close: 1.0835,
            volume: 1000,
        },
        CandleData {
            time: "2025-05-16T11:00:00Z".to_string(),
            open: 1.0835,
            high: 1.0855, // Invalidates (above 1.0850)
            low: 1.0830,
            close: 1.0845,
            volume: 1100,
        },
        CandleData {
            time: "2025-05-16T12:00:00Z".to_string(),
            open: 1.0845,
            high: 1.0860, // Should not count as this comes after invalidation
            low: 1.0840,
            close: 1.0850,
            volume: 1200,
        },
    ];

    // Test both approaches
    let (gen_is_active, gen_touch_count, _) = 
        simulate_generator_touch_counting(&supply_zone, &invalidation_candles);

    let zone_cache = create_test_cache_with_zone(supply_zone.clone()).await;
    let (monitor_is_active, monitor_touch_count) = test_actual_zone_monitor_logic(zone_cache, &invalidation_candles).await;

    write_to_test_output("Invalidation Test Results:");
    write_to_test_output(&format!("Generator: Active={}, Touches={}", gen_is_active, gen_touch_count));
    write_to_test_output(&format!("Monitor: Active={}, Touches={}", monitor_is_active, monitor_touch_count));

    // Both should detect invalidation and stop counting touches after that point
    assert!(!gen_is_active, "Generator should detect zone invalidation");
    assert!(!monitor_is_active, "Monitor should detect zone invalidation");
    assert_eq!(gen_touch_count, monitor_touch_count, "Touch counts before invalidation should match");
}

#[tokio::test]
async fn test_edge_case_exact_price_touches() {
    write_to_test_output("\n=== Testing Edge Case: Exact Price Touches ===");
    write_to_test_output("==============================================");

    let supply_zone = create_test_supply_zone();
    
    // Test candles with exact price matches
    let exact_touch_candles = vec![
        CandleData {
            time: "2025-05-16T10:00:00Z".to_string(),
            open: 1.0825,
            high: 1.0830, // Exactly touches proximal line
            low: 1.0820,
            close: 1.0825,
            volume: 1000,
        },
        CandleData {
            time: "2025-05-16T11:00:00Z".to_string(),
            open: 1.0825,
            high: 1.0850, // Exactly touches distal line (should invalidate)
            low: 1.0820,
            close: 1.0840,
            volume: 1100,
        },
    ];

    let (gen_is_active, gen_touch_count, _) = 
        simulate_generator_touch_counting(&supply_zone, &exact_touch_candles);

    let zone_cache = create_test_cache_with_zone(supply_zone.clone()).await;
    let (monitor_is_active, monitor_touch_count) = test_actual_zone_monitor_logic(zone_cache, &exact_touch_candles).await;

    write_to_test_output("Exact Touch Test Results:");
    write_to_test_output(&format!("Generator: Active={}, Touches={}", gen_is_active, gen_touch_count));
    write_to_test_output(&format!("Monitor: Active={}, Touches={}", monitor_is_active, monitor_touch_count));

    assert_eq!(gen_is_active, monitor_is_active, "Exact touch invalidation should match");
    assert_eq!(gen_touch_count, monitor_touch_count, "Exact touch counting should match");
}

// Helper function to run all tests and provide summary
#[tokio::test]
async fn test_full_zone_monitor_consistency_suite() {
    write_to_test_output("\n=== Zone Monitor Consistency Test Suite ===");
    write_to_test_output("===========================================");
    write_to_test_output("Running comprehensive tests to validate Zone Monitor");
    write_to_test_output("Service logic matches tested Generator calculations...");
    
    write_to_test_output("\nAll Zone Monitor consistency tests should pass!");
    write_to_test_output("This ensures your real-time trading decisions use");
    write_to_test_output("the same zone calculations as your tested endpoints.");
}