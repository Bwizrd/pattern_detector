// Test to determine exact cause of different zone IDs
// Real-time zone: bec9e3b8018aee3e
// Backtest zone: d59c80a4ceafc025

use sha2::{Digest, Sha256};

fn generate_deterministic_zone_id(
    symbol: &str,
    timeframe: &str,
    zone_type: &str,
    start_time: Option<&str>,
    zone_high: Option<f64>,
    zone_low: Option<f64>,
) -> String {
    const PRECISION: usize = 8;

    // Standardize symbol by removing _SB suffix
    let clean_symbol = symbol.replace("_SB", "");
    
    // STANDARDIZE TIMEFRAME: Always lowercase
    let clean_timeframe = timeframe.to_lowercase();
    
    // STANDARDIZE ZONE TYPE: Always lowercase
    let clean_zone_type = zone_type.to_lowercase();

    let high_str = zone_high.map_or("0.0".to_string(), |v| {
        if v.is_finite() {
            format!("{:.prec$}", v, prec = PRECISION)
        } else {
            "0.0".to_string()
        }
    });
    let low_str = zone_low.map_or("0.0".to_string(), |v| {
        if v.is_finite() {
            format!("{:.prec$}", v, prec = PRECISION)
        } else {
            "0.0".to_string()
        }
    });
    let clean_start_time = start_time.unwrap_or("unknown");

    // Create deterministic ID input
    let id_input = format!(
        "{}_{}_{}_{}_{}_{}",
        clean_symbol,
        clean_timeframe,
        clean_zone_type,
        clean_start_time,
        high_str,
        low_str
    );

    let mut hasher = Sha256::new();
    hasher.update(id_input.as_bytes());
    let result = hasher.finalize();
    let hex_id = format!("{:x}", result);

    // Use the first 16 chars of the hash for the ID
    let final_id = hex_id[..16].to_string();
    
    println!("ID_INPUT: {} -> ID: {}", id_input, final_id);
    
    final_id
}

fn main() {
    println!("=== ZONE ID GENERATION DEBUGGING ===");
    println!("Real-time zone: bec9e3b8018aee3e");
    println!("Backtest zone:  d59c80a4ceafc025");
    println!("Symbol: GBPAUD, Timeframe: 30m, Zone type: supply_zone, Entry price: ~2.09849");
    println!();

    // Known real-time zone data
    let rt_start_time = "2025-06-23T16:30:00Z";
    let rt_high = 2.10238;
    let rt_low = 2.0984902180000002;
    
    println!("=== REAL-TIME ZONE PARAMETERS ===");
    let rt_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m",
        "supply",
        Some(rt_start_time),
        Some(rt_high),
        Some(rt_low),
    );
    println!("Real-time generated ID: {}", rt_id);
    println!("Expected real-time ID:  bec9e3b8018aee3e");
    println!("Match: {}", rt_id == "bec9e3b8018aee3e");
    println!();

    // Test potential variations that could lead to different IDs
    println!("=== TESTING POTENTIAL BACKTEST VARIATIONS ===");

    // Test 1: Different precision in zone_low
    println!("Test 1: Different zone_low precision");
    let test1_low = 2.09849022; // Rounded to different precision
    let test1_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m", 
        "supply",
        Some(rt_start_time),
        Some(rt_high),
        Some(test1_low),
    );
    println!("Test 1 ID: {} (expected backtest: d59c80a4ceafc025)", test1_id);
    println!("Match: {}", test1_id == "d59c80a4ceafc025");
    println!();

    // Test 2: Different zone_high precision
    println!("Test 2: Different zone_high precision");
    let test2_high = 2.1023800;
    let test2_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m",
        "supply", 
        Some(rt_start_time),
        Some(test2_high),
        Some(rt_low),
    );
    println!("Test 2 ID: {} (expected backtest: d59c80a4ceafc025)", test2_id);
    println!("Match: {}", test2_id == "d59c80a4ceafc025");
    println!();

    // Test 3: Different start_time (even slight difference)
    println!("Test 3: Different start_time");
    let test3_start = "2025-06-23T16:30:00.000Z"; // With microseconds
    let test3_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m",
        "supply",
        Some(test3_start),
        Some(rt_high),
        Some(rt_low),
    );
    println!("Test 3 ID: {} (expected backtest: d59c80a4ceafc025)", test3_id);
    println!("Match: {}", test3_id == "d59c80a4ceafc025");
    println!();

    // Test 4: Different zone_type format
    println!("Test 4: Different zone_type format");
    let test4_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m",
        "supply_zone", // Full zone type instead of just "supply"
        Some(rt_start_time),
        Some(rt_high),
        Some(rt_low),
    );
    println!("Test 4 ID: {} (expected backtest: d59c80a4ceafc025)", test4_id);
    println!("Match: {}", test4_id == "d59c80a4ceafc025");
    println!();

    // Test 5: Test with entry price values (~2.09849)
    println!("Test 5: Using entry price as zone boundaries");
    let entry_price = 2.09849;
    let test5_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m",
        "supply",
        Some(rt_start_time),
        Some(entry_price), // Using entry price as high
        Some(entry_price), // Using entry price as low
    );
    println!("Test 5 ID: {} (expected backtest: d59c80a4ceafc025)", test5_id);
    println!("Match: {}", test5_id == "d59c80a4ceafc025");
    println!();

    // Test 6: Test various precision levels for zone boundaries
    println!("Test 6: Different precision combinations");
    let test6_high = 2.09849000; // Entry price with high precision
    let test6_low = 2.09849000;
    let test6_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m",
        "supply",
        Some(rt_start_time),
        Some(test6_high),
        Some(test6_low),
    );
    println!("Test 6 ID: {} (expected backtest: d59c80a4ceafc025)", test6_id);
    println!("Match: {}", test6_id == "d59c80a4ceafc025");
    println!();

    // Test 7: Slight variations in start time format
    println!("Test 7: Start time without 'Z' suffix");
    let test7_start = "2025-06-23T16:30:00";
    let test7_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m",
        "supply",
        Some(test7_start),
        Some(rt_high),
        Some(rt_low),
    );
    println!("Test 7 ID: {} (expected backtest: d59c80a4ceafc025)", test7_id);
    println!("Match: {}", test7_id == "d59c80a4ceafc025");
    println!();

    // Test 8: Test with different start time that might be in backtest
    println!("Test 8: Different start time (backtest might use candle open time)");
    let test8_start = "2025-06-23T16:00:00Z"; // 30 minutes earlier
    let test8_id = generate_deterministic_zone_id(
        "GBPAUD",
        "30m",
        "supply",
        Some(test8_start),
        Some(rt_high),
        Some(rt_low),
    );
    println!("Test 8 ID: {} (expected backtest: d59c80a4ceafc025)", test8_id);
    println!("Match: {}", test8_id == "d59c80a4ceafc025");
    println!();

    println!("=== EXTENDED BRUTE FORCE SEARCH ===");
    println!("Searching for parameter combinations that generate d59c80a4ceafc025...");
    
    // Extended search with more possibilities
    let possible_times = vec![
        "2025-06-23T15:30:00Z",
        "2025-06-23T16:00:00Z",
        "2025-06-23T16:30:00Z", 
        "2025-06-23T16:30:00",
        "2025-06-23T17:00:00Z",
        "2025-06-23T17:30:00Z",
        // Different date possibilities
        "2025-06-22T16:30:00Z",
        "2025-06-24T16:30:00Z",
    ];
    
    // More extensive price variations
    let possible_highs = vec![
        2.098490, 2.09849, 2.09849000, 2.09850, 
        2.1023800, 2.10238, 2.10238000, 2.10239,
        2.10240, 2.10241,
    ];
    
    let possible_lows = vec![
        2.098490, 2.09849, 2.09849000, 2.09849001, 2.09849002,
        2.0984902180000002, 2.09849022, 2.09849021, 2.09849020,
        2.09848, 2.09850,
    ];

    let possible_types = vec!["supply", "supply_zone", "SUPPLY", "SUPPLY_ZONE"];
    let possible_symbols = vec!["GBPAUD", "gbpaud", "GBPAUD_SB"];
    let possible_timeframes = vec!["30m", "30M", "30min"];

    let mut found_match = false;
    let mut test_count = 0;

    for symbol in &possible_symbols {
        for timeframe in &possible_timeframes {
            for zone_type in &possible_types {
                for time in &possible_times {
                    for high in &possible_highs {
                        for low in &possible_lows {
                            test_count += 1;
                            let test_id = generate_deterministic_zone_id(
                                symbol,
                                timeframe,
                                zone_type,
                                Some(time),
                                Some(*high),
                                Some(*low),
                            );
                            if test_id == "d59c80a4ceafc025" {
                                println!("🎯 FOUND MATCH after {} tests!", test_count);
                                println!("   Symbol: {}", symbol);
                                println!("   Timeframe: {}", timeframe);
                                println!("   Zone Type: {}", zone_type);
                                println!("   Time: {}", time);
                                println!("   High: {}", high);
                                println!("   Low: {}", low);
                                println!("   ID: {}", test_id);
                                found_match = true;
                                break;
                            }
                            
                            // Show progress every 1000 tests
                            if test_count % 1000 == 0 {
                                println!("   Tested {} combinations...", test_count);
                            }
                        }
                        if found_match { break; }
                    }
                    if found_match { break; }
                }
                if found_match { break; }
            }
            if found_match { break; }
        }
        if found_match { break; }
    }
    
    if !found_match {
        println!("No exact match found in extended search after {} tests.", test_count);
        println!("The backtest zone likely uses significantly different parameters.");
        
        // Test if the backtest zone might be using a completely different approach
        println!("\n=== TESTING ALTERNATIVE APPROACHES ===");
        
        // Test with None values
        let none_test = generate_deterministic_zone_id(
            "GBPAUD", "30m", "supply", None, Some(2.09849), Some(2.09849)
        );
        println!("With None start_time: {}", none_test);
        
        // Test with "unknown" explicitly
        let unknown_test = generate_deterministic_zone_id(
            "GBPAUD", "30m", "supply", Some("unknown"), Some(2.09849), Some(2.09849)
        );
        println!("With 'unknown' start_time: {}", unknown_test);
        
        // Test with different precision formatting
        let precision_test = generate_deterministic_zone_id(
            "GBPAUD", "30m", "supply", Some("2025-06-23T16:30:00Z"), Some(2.098490), Some(2.098490)
        );
        println!("With 2.098490 precision: {}", precision_test);
    }
}