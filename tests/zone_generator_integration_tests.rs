// tests/zone_generator_integration_tests.rs

// Use the name of your crate (as defined in Cargo.toml, e.g., "pattern_detector")
use pattern_detector::zone_generator::run_zone_generation;
// Add other necessary imports from your crate if helpers are needed (e.g., for CandleData setup)
// use pattern_detector::types::CandleData;

use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
// If you need tokio for the test runtime directly:
// use tokio;

// You'll need your InfluxDB client for verification if you choose to query after test.
// For now, we'll focus on the raw text log.
// use influxdb2::Client as InfluxClient; // Example, if you use this crate

const TEST_ZONE_MEASUREMENT: &str = "test_zones";

fn clear_raw_zone_log() {
    let log_path = Path::new("detected_raw_zones.txt");
    if log_path.exists() {
        fs::remove_file(log_path).expect("Failed to remove raw zone log file");
    }
}

fn setup_test_logging() {
    // Initialize your logger for tests if needed to see output.
    // Example using env_logger (add to [dev-dependencies]):
    // let _ = env_logger::builder().is_test(true).try_init();
}

#[tokio::test]
#[ignore] // Ignored by default because it hits DB and uses global env vars. Run with `cargo test -- --ignored --test-threads=1`
async fn test_full_historical_run_writes_to_test_measurement() {
    setup_test_logging();
    clear_raw_zone_log();

    // --- Test-specific Environment Variable Setup ---
    // These will be read by run_zone_generation via dotenv() or if already in environment
    // For a historical period you KNOW has EURUSD/1h patterns:
    std::env::set_var("GENERATOR_START_TIME", "2025-05-16T00:00:00Z"); // *** ADJUST TO YOUR DATA ***
    std::env::set_var("GENERATOR_END_TIME", "2025-05-16T23:59:59Z");   // *** ADJUST TO YOUR DATA ***
    std::env::set_var("GENERATOR_PATTERNS", "fifty_percent_before_big_bar");
    std::env::set_var("GENERATOR_EXCLUDE_TIMEFRAMES", "1m"); // Allow 1h, 5m, 15m, 30m, etc.
    // Ensure INFLUXDB_HOST, ORG, TOKEN, BUCKET, GENERATOR_TRENDBAR_MEASUREMENT are set
    // The GENERATOR_ZONE_MEASUREMENT env var will be IGNORED due to the override.

    println!("Starting full historical run test, writing to '{}' measurement.", TEST_ZONE_MEASUREMENT);

    // Call run_zone_generation for a "FULL" run, overriding the target measurement
    let result = run_zone_generation(false, Some(TEST_ZONE_MEASUREMENT)).await;
    assert!(result.is_ok(), "run_zone_generation (full) failed: {:?}", result.err());

    // --- Verification Phase ---

    // 1. Check the raw text log
    let log_path = Path::new("detected_raw_zones.txt");
    assert!(log_path.exists(), "Raw zone log file was not created.");
    let mut file_content = String::new();
    File::open(log_path)
        .expect("Failed to open raw zone log file for reading")
        .read_to_string(&mut file_content)
        .expect("Failed to read raw zone log file content");
    assert!(!file_content.is_empty(), "Raw zone log file is empty after full run.");

    // Add specific assertions about file_content for your chosen historical period.
    // Example: if you expect EURUSD/1h patterns on 2025-05-16
    let expected_eurusd_1h_zone_start = "ZoneStartTime: 2025-05-16T"; // Partial match
    let found_eurusd_1h = file_content.lines().any(|line| {
        line.contains("Symbol: EURUSD_SB") && line.contains("TF: 1h") && line.contains(expected_eurusd_1h_zone_start)
    });
    // THIS ASSERTION WILL FAIL IF NO SUCH PATTERN EXISTS IN YOUR DATA FOR THAT DAY
    // assert!(found_eurusd_1h, "Expected EURUSD/1h zones from 2025-05-16 not found in raw log. Check test data range and patterns.");
    println!("Raw log content sample (first 500 chars):\n{}", &file_content.chars().take(500).collect::<String>());


    // 2. OPTIONAL: Query InfluxDB's "test_zones" measurement to verify writes
    // This requires InfluxDB client setup and async query execution here.
    // Example conceptual verification:
    let influx_host = env::var("INFLUXDB_HOST").expect("INFLUXDB_HOST not set for test verification");
    let influx_org = env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG not set");
    let influx_token = env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN not set");
    let write_bucket = env::var("GENERATOR_WRITE_BUCKET").expect("GENERATOR_WRITE_BUCKET not set"); // Should be "market_data"

    let client = reqwest::Client::new();
    let query = format!(
        r#"from(bucket: "{}")
             |> range(start: {}, stop: {})
             |> filter(fn: (r) => r._measurement == "{}")
             |> filter(fn: (r) => r.symbol == "{}" and r.timeframe == "{}")
             |> limit(n:1)
             |> yield(name: "test_verification")"#,
        write_bucket, // Querying the bucket where test_zones measurement resides
        env::var("GENERATOR_START_TIME").unwrap(),
        env::var("GENERATOR_END_TIME").unwrap(),
        TEST_ZONE_MEASUREMENT, // Verify data in the test measurement
        "EURUSD_SB", // Example symbol
        "1h"         // Example timeframe
    );

    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);
    let response = client
        .post(&query_url)
        .bearer_auth(&influx_token)
        .header("Accept", "application/csv")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "query": query, "type": "flux" }))
        .send()
        .await
        .expect("InfluxDB query for test verification failed to send");

    let response_text = response.text().await.expect("Failed to get text from verification response");
    println!("Verification query for test_zones (EURUSD/1h):\n{}", query);
    println!("Verification response from test_zones (EURUSD/1h):\n{}", response_text);
    
    // A simple check: if we expect data, the response shouldn't just be headers/empty.
    // A more robust check would parse the CSV and count rows or check specific values.
    let data_lines = response_text.lines().filter(|line| !line.starts_with('#') && !line.trim().is_empty()).count();
    // If you expect EURUSD/1h zones to be written to test_zones:
    // assert!(data_lines > 1, "Expected data for EURUSD/1h in '{}' measurement, but found none or only headers. Response: {}", TEST_ZONE_MEASUREMENT, response_text);


    // --- Clean up Environment Variables set by this test ---
    std::env::remove_var("GENERATOR_START_TIME");
    std::env::remove_var("GENERATOR_END_TIME");
    std::env::remove_var("GENERATOR_EXCLUDE_TIMEFRAMES");
}

// You can create a similar test for periodic runs:
// test_periodic_run_writes_to_test_measurement, calling run_zone_generation(true, Some(TEST_ZONE_MEASUREMENT))
// and adjusting assertions for recent ZoneStartTimes.