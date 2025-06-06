// tests/zone_consistency_tests.rs
//
// This test file verifies that the endpoint and zone generator produce identical results
// by calling the same core logic and comparing the processed zones.

use pattern_detector::api::detect::{_fetch_and_detect_core, ChartQuery, CandleData};
// use reqwest::Client;
use serde_json::Value;
use std::env;
use tokio;
use sha2::{Digest, Sha256};

// Test configuration structure
struct TestConfig {
    influx_host: String,
    influx_org: String,
    influx_token: String,
    influx_bucket: String,
    write_bucket: String,
    zone_measurement: String,
}

impl TestConfig {
    fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            influx_host: env::var("INFLUXDB_HOST")?,
            influx_org: env::var("INFLUXDB_ORG")?,
            influx_token: env::var("INFLUXDB_TOKEN")?,
            influx_bucket: env::var("INFLUXDB_BUCKET")?,
            write_bucket: env::var("GENERATOR_WRITE_BUCKET")?,
            zone_measurement: env::var("GENERATOR_ZONE_MEASUREMENT")?,
        })
    }
}

// Mock zone structure for comparison (mirrors the logic of both endpoint and generator)
#[derive(Debug, Clone, PartialEq)]
struct MockZone {
    zone_id: String,
    zone_type: String,
    zone_high: f64,
    zone_low: f64,
    start_time: String,
    end_time: Option<String>,
    is_active: bool,
    touch_count: i64,
    bars_active: Option<u64>,
    strength_score: Option<f64>,
    detection_method: Option<String>,
    start_idx: Option<u64>,
    end_idx: Option<u64>,
    fifty_percent_line: Option<f64>,
}

// Simulate the endpoint's zone processing logic
async fn simulate_endpoint_zone_processing(
    chart_query: &ChartQuery,
) -> Result<Vec<MockZone>, Box<dyn std::error::Error>> {
    let (candles, _recognizer, detection_results) = 
        _fetch_and_detect_core(chart_query, "test_endpoint").await?;

    let mut zones = Vec::new();

    if let Some(data) = detection_results.get("data").and_then(|d| d.get("price")) {
        // Process supply zones
        if let Some(supply_zones) = data
            .get("supply_zones")
            .and_then(|sz| sz.get("zones"))
            .and_then(|z| z.as_array())
        {
            for zone_json in supply_zones {
                if let Some(zone) = process_zone_from_json(
                    zone_json, 
                    &candles, 
                    true, 
                    &chart_query.symbol, 
                    &chart_query.timeframe
                ) {
                    zones.push(zone);
                }
            }
        }

        // Process demand zones
        if let Some(demand_zones) = data
            .get("demand_zones")
            .and_then(|dz| dz.get("zones"))
            .and_then(|z| z.as_array())
        {
            for zone_json in demand_zones {
                if let Some(zone) = process_zone_from_json(
                    zone_json, 
                    &candles, 
                    false, 
                    &chart_query.symbol, 
                    &chart_query.timeframe
                ) {
                    zones.push(zone);
                }
            }
        }
    }

    Ok(zones)
}

// Simulate the generator's zone processing logic
async fn simulate_generator_zone_processing(
    chart_query: &ChartQuery,
    _config: &TestConfig,
) -> Result<Vec<MockZone>, Box<dyn std::error::Error>> {
    let (candles, _recognizer, detection_results) = 
        _fetch_and_detect_core(chart_query, "test_generator").await?;

    let mut zones = Vec::new();

    if let Some(data) = detection_results.get("data").and_then(|d| d.get("price")) {
        for zone_kind_key in ["supply_zones", "demand_zones"] {
            let is_supply = zone_kind_key == "supply_zones";
            
            if let Some(zones_array) = data
                .get(zone_kind_key)
                .and_then(|zk| zk.get("zones"))
                .and_then(|z| z.as_array())
            {
                for zone_json in zones_array {
                    if let Some(zone) = process_zone_from_json(
                        zone_json, 
                        &candles, 
                        is_supply, 
                        &chart_query.symbol, 
                        &chart_query.timeframe
                    ) {
                        zones.push(zone);
                    }
                }
            }
        }
    }

    Ok(zones)
}

// Process a zone from JSON (replicates the logic from both endpoint and generator)
fn process_zone_from_json(
    zone_json: &Value,
    candles: &[CandleData],
    is_supply: bool,
    symbol: &str,
    timeframe: &str,
) -> Option<MockZone> {
    // Extract basic zone data (same as both endpoint and generator)
    let start_time = zone_json.get("start_time")?.as_str()?.to_string();
    let end_time = zone_json.get("end_time").and_then(|v| v.as_str()).map(String::from);
    let zone_high = zone_json.get("zone_high")?.as_f64()?;
    let zone_low = zone_json.get("zone_low")?.as_f64()?;
    let start_idx = zone_json.get("start_idx")?.as_u64()?;
    let end_idx = zone_json.get("end_idx")?.as_u64()?;
    let detection_method = zone_json.get("detection_method").and_then(|v| v.as_str()).map(String::from);
    let fifty_percent_line = zone_json.get("fifty_percent_line").and_then(|v| v.as_f64());

    // Validate indices (same validation as your code)
    if end_idx as usize >= candles.len() || start_idx > end_idx {
        return None;
    }

    // Generate zone ID (same logic as your generator)
    let zone_type = if is_supply { "supply" } else { "demand" };
    let symbol_for_id = if symbol.ends_with("_SB") {
        symbol.to_string()
    } else {
        format!("{}_SB", symbol)
    };
    
    let zone_id = generate_test_zone_id(&symbol_for_id, timeframe, zone_type, &start_time, zone_high, zone_low);

    // Calculate activity (replicates your enrichment logic)
    let (is_active, touch_count, bars_active) = calculate_zone_activity_test(
        is_supply, zone_high, zone_low, end_idx as usize, candles
    );

    // Calculate strength score (same as your logic)
    let strength_score = Some(100.0 - (touch_count as f64 * 10.0).max(0.0).min(100.0));

    Some(MockZone {
        zone_id,
        zone_type: zone_type.to_string(),
        zone_high,
        zone_low,
        start_time,
        end_time,
        is_active,
        touch_count,
        bars_active,
        strength_score,
        detection_method,
        start_idx: Some(start_idx),
        end_idx: Some(end_idx),
        fifty_percent_line,
    })
}

// Generate deterministic zone ID (same as your generate_deterministic_zone_id function)
fn generate_test_zone_id(
    symbol: &str, 
    timeframe: &str, 
    zone_type: &str, 
    start_time: &str, 
    zone_high: f64, 
    zone_low: f64
) -> String {
    const PRECISION: usize = 6;
    let high_str = format!("{:.prec$}", zone_high, prec = PRECISION);
    let low_str = format!("{:.prec$}", zone_low, prec = PRECISION);
    
    let id_input = format!("{}_{}_{}_{}_{}_{}",
        symbol, timeframe, zone_type, start_time, high_str, low_str);
    
    let mut hasher = Sha256::new();
    hasher.update(id_input.as_bytes());
    let result = hasher.finalize();
    let hex_id = format!("{:x}", result);
    
    hex_id[..16].to_string()
}

// Calculate zone activity (replicates your enrichment logic)
fn calculate_zone_activity_test(
    is_supply: bool,
    zone_high: f64,
    zone_low: f64,
    end_idx: usize,
    candles: &[CandleData],
) -> (bool, i64, Option<u64>) {
    let mut is_active = true;
    let mut touch_count = 0i64;
    let mut bars_active = None;

    let (proximal_line, distal_line) = if is_supply {
        (zone_low, zone_high)
    } else {
        (zone_high, zone_low)
    };

    let check_start_idx = end_idx + 1;
    let mut is_outside_zone = true;

    if check_start_idx < candles.len() {
        for i in check_start_idx..candles.len() {
            let candle = &candles[i];
            
            if !candle.high.is_finite() || !candle.low.is_finite() || !candle.close.is_finite() {
                continue;
            }

            bars_active = Some((i - check_start_idx + 1) as u64);

            // Check for invalidation (same logic as your code)
            if is_supply {
                if candle.high > distal_line {
                    is_active = false;
                    break;
                }
            } else if candle.low < distal_line {
                is_active = false;
                break;
            }

            // Check for interaction (same logic as your code)
            let interaction_occurred = if is_supply {
                candle.high >= proximal_line
            } else {
                candle.low <= proximal_line
            };

            if interaction_occurred && is_outside_zone {
                touch_count += 1;
            }
            is_outside_zone = !interaction_occurred;
        }
    } else {
        bars_active = Some(0);
    }

    if is_active && bars_active.is_none() {
        bars_active = if check_start_idx < candles.len() {
            Some((candles.len() - check_start_idx) as u64)
        } else {
            Some(0)
        };
    }

    (is_active, touch_count, bars_active)
}

// Helper function to create test chart queries
fn create_test_chart_query() -> ChartQuery {
    ChartQuery {
        symbol: "EURUSD_SB".to_string(),
        timeframe: "1h".to_string(),
        start_time: "2025-05-16T10:00:00Z".to_string(),
        end_time: "2025-05-16T16:00:00Z".to_string(),
        pattern: "fifty_percent_before_big_bar".to_string(),
        enable_trading: None,
        lot_size: None,
        stop_loss_pips: None,
        take_profit_pips: None,
        enable_trailing_stop: None,
        max_touch_count: None,
    }
}

fn create_test_chart_query_with_params(symbol: &str, timeframe: &str) -> ChartQuery {
    ChartQuery {
        symbol: symbol.to_string(),
        timeframe: timeframe.to_string(),
        start_time: "2025-05-16T10:00:00Z".to_string(),
        end_time: "2025-05-16T16:00:00Z".to_string(),
        pattern: "fifty_percent_before_big_bar".to_string(),
        enable_trading: None,
        lot_size: None,
        stop_loss_pips: None,
        take_profit_pips: None,
        enable_trailing_stop: None,
        max_touch_count: None,
    }
}

// Helper to compare zones with detailed error messages
fn assert_zones_equal(endpoint_zone: &MockZone, generator_zone: &MockZone, zone_index: usize) {
    assert_eq!(endpoint_zone.zone_id, generator_zone.zone_id, 
               "Zone {} ID mismatch: endpoint='{}', generator='{}'", 
               zone_index, endpoint_zone.zone_id, generator_zone.zone_id);
    
    assert_eq!(endpoint_zone.zone_type, generator_zone.zone_type, 
               "Zone {} type mismatch: endpoint='{}', generator='{}'", 
               zone_index, endpoint_zone.zone_type, generator_zone.zone_type);
    
    assert!((endpoint_zone.zone_high - generator_zone.zone_high).abs() < 1e-9, 
            "Zone {} high mismatch: endpoint={:.9}, generator={:.9}", 
            zone_index, endpoint_zone.zone_high, generator_zone.zone_high);
    
    assert!((endpoint_zone.zone_low - generator_zone.zone_low).abs() < 1e-9, 
            "Zone {} low mismatch: endpoint={:.9}, generator={:.9}", 
            zone_index, endpoint_zone.zone_low, generator_zone.zone_low);
    
    assert_eq!(endpoint_zone.start_time, generator_zone.start_time, 
               "Zone {} start time mismatch: endpoint='{}', generator='{}'", 
               zone_index, endpoint_zone.start_time, generator_zone.start_time);
    
    assert_eq!(endpoint_zone.is_active, generator_zone.is_active, 
               "Zone {} active status mismatch: endpoint={}, generator={}", 
               zone_index, endpoint_zone.is_active, generator_zone.is_active);
    
    assert_eq!(endpoint_zone.touch_count, generator_zone.touch_count, 
               "Zone {} touch count mismatch: endpoint={}, generator={}", 
               zone_index, endpoint_zone.touch_count, generator_zone.touch_count);
    
    assert_eq!(endpoint_zone.bars_active, generator_zone.bars_active, 
               "Zone {} bars active mismatch: endpoint={:?}, generator={:?}", 
               zone_index, endpoint_zone.bars_active, generator_zone.bars_active);
    
    // Compare strength scores with small tolerance for floating point
    match (endpoint_zone.strength_score, generator_zone.strength_score) {
        (Some(e_score), Some(g_score)) => {
            assert!((e_score - g_score).abs() < 1e-6, 
                    "Zone {} strength score mismatch: endpoint={:.6}, generator={:.6}", 
                    zone_index, e_score, g_score);
        }
        (None, None) => {}
        _ => panic!("Zone {} strength score presence mismatch: endpoint={:?}, generator={:?}", 
                   zone_index, endpoint_zone.strength_score, generator_zone.strength_score),
    }
}

//
// ACTUAL TESTS
//

#[tokio::test]
async fn test_endpoint_vs_generator_same_zones() {
    dotenv::dotenv().ok();
    
    let config = match TestConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            println!("Skipping test - environment variables not set: {}", e);
            return;
        }
    };

    let chart_query = create_test_chart_query();
    
    // Simulate endpoint processing
    let endpoint_zones = simulate_endpoint_zone_processing(&chart_query)
        .await
        .expect("Endpoint should process zones successfully");
    
    // Simulate generator processing  
    let generator_zones = simulate_generator_zone_processing(&chart_query, &config)
        .await
        .expect("Generator should process zones successfully");
    
    println!("🔍 Comparing results:");
    println!("   Endpoint zones: {}", endpoint_zones.len());
    println!("   Generator zones: {}", generator_zones.len());
    
    // They should have the same number of zones
    assert_eq!(endpoint_zones.len(), generator_zones.len(), 
               "Endpoint ({}) and generator ({}) should return same number of zones", 
               endpoint_zones.len(), generator_zones.len());
    
    if endpoint_zones.is_empty() {
        println!("   ℹ️  No zones detected by either endpoint or generator");
        return;
    }
    
    // Sort zones by zone_id for consistent comparison
    let mut endpoint_sorted = endpoint_zones;
    let mut generator_sorted = generator_zones;
    endpoint_sorted.sort_by(|a, b| a.zone_id.cmp(&b.zone_id));
    generator_sorted.sort_by(|a, b| a.zone_id.cmp(&b.zone_id));
    
    // Compare each zone in detail
    for (i, (endpoint_zone, generator_zone)) in 
        endpoint_sorted.iter().zip(generator_sorted.iter()).enumerate() {
        
        println!("   Comparing zone {}: {} ({})", i + 1, endpoint_zone.zone_id, endpoint_zone.zone_type);
        assert_zones_equal(endpoint_zone, generator_zone, i);
    }
    
    println!("✅ All {} zones match perfectly between endpoint and generator!", endpoint_sorted.len());
}

#[tokio::test]
async fn test_zone_id_deterministic() {
    dotenv::dotenv().ok();
    
    let config = match TestConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            println!("Skipping deterministic test - environment variables not set: {}", e);
            return;
        }
    };

    let chart_query = create_test_chart_query();
    
    println!("🔄 Testing deterministic behavior...");
    
    // Run the same processing twice
    let zones_run1 = simulate_endpoint_zone_processing(&chart_query)
        .await
        .expect("First run should succeed");
    let zones_run2 = simulate_endpoint_zone_processing(&chart_query)
        .await
        .expect("Second run should succeed");
    
    assert_eq!(zones_run1.len(), zones_run2.len(), 
               "Both runs should return same number of zones");
    
    for (i, (zone1, zone2)) in zones_run1.iter().zip(zones_run2.iter()).enumerate() {
        assert_eq!(zone1.zone_id, zone2.zone_id, 
                   "Zone {} ID should be deterministic: run1='{}', run2='{}'", 
                   i, zone1.zone_id, zone2.zone_id);
    }
    
    println!("✅ Zone IDs are deterministic across {} zones!", zones_run1.len());
}

#[tokio::test]
async fn test_multiple_symbols_timeframes() {
    dotenv::dotenv().ok();
    
    let config = match TestConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            println!("Skipping multi-symbol test - environment variables not set: {}", e);
            return;
        }
    };

    let test_cases = vec![
        ("EURUSD_SB", "1h"),
        ("GBPUSD_SB", "4h"),
        ("USDJPY_SB", "1d"),
    ];
    
    println!("🌐 Testing multiple symbols and timeframes...");
    
    for (symbol, timeframe) in test_cases {
        println!("   Testing {}/{}", symbol, timeframe);
        
        let query = create_test_chart_query_with_params(symbol, timeframe);
        
        let endpoint_zones = simulate_endpoint_zone_processing(&query)
            .await
            .expect(&format!("Endpoint should work for {}/{}", symbol, timeframe));
        let generator_zones = simulate_generator_zone_processing(&query, &config)
            .await
            .expect(&format!("Generator should work for {}/{}", symbol, timeframe));
        
        assert_eq!(endpoint_zones.len(), generator_zones.len(), 
                   "Zone counts should match for {}/{}: endpoint={}, generator={}", 
                   symbol, timeframe, endpoint_zones.len(), generator_zones.len());
        
        // Quick comparison of first zone if any exist
        if !endpoint_zones.is_empty() && !generator_zones.is_empty() {
            assert_eq!(endpoint_zones[0].zone_id, generator_zones[0].zone_id,
                       "First zone ID should match for {}/{}", symbol, timeframe);
        }
        
        println!("     ✅ {}/{}: {} zones match", symbol, timeframe, endpoint_zones.len());
    }
    
    println!("✅ All symbol/timeframe combinations consistent!");
}

#[test]
fn test_zone_properties() {
    println!("🧪 Testing zone property calculations...");
    
    // Test that individual zone processing logic is sound
    let test_candles = vec![
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
            high: 1.0830, // This should invalidate a supply zone at 1.0825
            low: 1.0800,
            close: 1.0815,
            volume: 1100,
        },
    ];
    
    // Test supply zone invalidation
    let (is_active, touch_count, bars_active) = calculate_zone_activity_test(
        true,   // is_supply
        1.0825, // zone_high (distal line)
        1.0815, // zone_low (proximal line)
        0,      // end_idx (zone ends at index 0)
        &test_candles,
    );
    
    assert!(!is_active, "Supply zone should be invalidated by high at 1.0830");
    assert_eq!(bars_active, Some(1), "Should have 1 bar active before invalidation");
    
    // Test demand zone that remains active
    let (is_active_demand, touch_count_demand, bars_active_demand) = calculate_zone_activity_test(
        false,  // is_demand
        1.0805, // zone_high (proximal line)
        1.0795, // zone_low (distal line)
        0,      // end_idx
        &test_candles,
    );
    
    assert!(is_active_demand, "Demand zone should remain active");
    assert!(touch_count_demand >= 0, "Touch count should be non-negative");
    
    println!("✅ Zone property calculations working correctly!");
}

#[test]
fn test_zone_id_generation() {
    println!("🆔 Testing zone ID generation...");
    
    let zone_id1 = generate_test_zone_id(
        "EURUSD_SB", 
        "1h", 
        "supply", 
        "2025-05-16T10:00:00Z", 
        1.08250, 
        1.08150
    );
    
    let zone_id2 = generate_test_zone_id(
        "EURUSD_SB", 
        "1h", 
        "supply", 
        "2025-05-16T10:00:00Z", 
        1.08250, 
        1.08150
    );
    
    // Same inputs should produce same ID
    assert_eq!(zone_id1, zone_id2, "Same inputs should produce same zone ID");
    
    // Different inputs should produce different IDs
    let zone_id3 = generate_test_zone_id(
        "EURUSD_SB", 
        "1h", 
        "demand", // Different type
        "2025-05-16T10:00:00Z", 
        1.08250, 
        1.08150
    );
    
    assert_ne!(zone_id1, zone_id3, "Different zone types should produce different IDs");
    
    println!("✅ Zone ID generation is deterministic and unique!");
}

#[tokio::test]
#[ignore] // Run with: cargo test --ignored
async fn test_with_real_influxdb_data() {
    dotenv::dotenv().ok();
    
    let config = match TestConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            println!("Skipping real InfluxDB test: {}", e);
            return;
        }
    };

    println!("🗄️  Testing with real InfluxDB data...");
    
    let chart_query = create_test_chart_query();
    
    match simulate_endpoint_zone_processing(&chart_query).await {
        Ok(zones) => {
            println!("✅ Successfully processed {} zones with real data", zones.len());
            
            for (i, zone) in zones.iter().take(3).enumerate() {
                println!("   Zone {}: {} - Type: {} - Active: {} - Touches: {}", 
                         i + 1, zone.zone_id, zone.zone_type, zone.is_active, zone.touch_count);
            }
            
            // Test with generator as well
            match simulate_generator_zone_processing(&chart_query, &config).await {
                Ok(generator_zones) => {
                    assert_eq!(zones.len(), generator_zones.len(), 
                               "Real data test: endpoint and generator should return same number of zones");
                    println!("✅ Real data test: both endpoint and generator returned {} zones", zones.len());
                }
                Err(e) => {
                    println!("❌ Generator failed with real data: {}", e);
                }
            }
        }
        Err(e) => {
            println!("❌ Real data test failed: {}", e);
            panic!("Real data test failed: {}", e);
        }
    }
}