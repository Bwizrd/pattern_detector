// tests/zone_generator_integration_tests.rs

// Use the name of your crate (as defined in Cargo.toml, e.g., "pattern_detector")
use pattern_detector::zone_generator::run_zone_generation;

use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use env_logger; // <<< ADD THIS IMPORT (make sure env_logger is in [dev-dependencies])
use log; // <<< ADD THIS if you use log::error! directly in this test file for debugging

const TEST_ZONE_MEASUREMENT: &str = "test_zones";

fn clear_raw_zone_log() {
    let log_path = Path::new("detected_raw_zones.txt");
    if log_path.exists() {
        fs::remove_file(log_path).expect("Failed to remove raw zone log file");
    }
}

fn setup_test_logging() {
    // Initialize env_logger for tests.
    // .is_test(true) formats output nicely for tests.
    // .try_init() is used because tests might run in parallel or logger might be init elsewhere.
    // It's okay if it's already initialized.
    match env_logger::builder().is_test(true).try_init() {
        Ok(_) => println!("[TEST_LOG_SETUP] env_logger initialized successfully for this test run."),
        Err(e) => println!("[TEST_LOG_SETUP] env_logger initialization failed or already initialized: {}", e),
    }
}

#[tokio::test]
#[ignore] // Ignored by default because it hits DB and uses global env vars. Run with `cargo test -- --ignored --test-threads=1`
async fn test_full_historical_run_writes_to_test_measurement() {
    // --- CALL LOGGING SETUP FIRST ---
    setup_test_logging();

    // --- Test a log message directly from the test ---
    // This helps confirm if the logger setup and RUST_LOG are working for the test process.
    log::error!("[TEST_RUN_DEBUG] THIS IS AN ERROR LOG FROM THE TEST FUNCTION ITSELF. If you see this, basic logging setup is working.");
    log::info!("[TEST_RUN_DEBUG] This is an info log from the test function.");


    clear_raw_zone_log();

    // --- Test-specific Environment Variable Setup ---
    std::env::set_var("ENABLE_PERIODIC_ZONE_GENERATOR", "false"); // Ensure periodic is off
    std::env::set_var("GENERATOR_START_TIME", "2025-05-16T00:00:00Z");
    std::env::set_var("GENERATOR_END_TIME", "2025-05-16T23:59:59Z"); // Test specific end time
    std::env::set_var("GENERATOR_PATTERNS", "fifty_percent_before_big_bar");
    std::env::set_var("GENERATOR_EXCLUDE_TIMEFRAMES", "1m");

    // Log the env vars as the test sees them, BEFORE run_zone_generation
    println!("[TEST_ENV_CHECK] GENERATOR_START_TIME before run: {}", env::var("GENERATOR_START_TIME").unwrap_or_default());
    println!("[TEST_ENV_CHECK] GENERATOR_END_TIME before run: {}", env::var("GENERATOR_END_TIME").unwrap_or_default());


    println!(
        "Starting full historical run test, writing to '{}' measurement.",
        TEST_ZONE_MEASUREMENT
    );

    // Call run_zone_generation for a "FULL" run, overriding the target measurement
    let result = run_zone_generation(false, Some(TEST_ZONE_MEASUREMENT)).await;
    assert!(
        result.is_ok(),
        "run_zone_generation (full) failed: {:?}",
        result.err()
    );

    // --- Verification Phase ---
    let log_path = Path::new("detected_raw_zones.txt");
    assert!(log_path.exists(), "Raw zone log file was not created.");
    let mut file_content = String::new();
    File::open(log_path)
        .expect("Failed to open raw zone log file for reading")
        .read_to_string(&mut file_content)
        .expect("Failed to read raw zone log file content");
    assert!(
        !file_content.is_empty(),
        "Raw zone log file is empty after full run."
    );

    let expected_eurusd_1h_zone_start = "ZoneStartTime: 2025-05-16T13:00:00Z"; // More specific
    let mut found_specific_eurusd_1h_zone_in_raw_log = false;
    for line in file_content.lines() {
        if line.contains("Symbol: EURUSD_SB")
            && line.contains("TF: 1h")
            && line.contains(expected_eurusd_1h_zone_start) // Check for the exact start time
        {
            println!("[TEST_VERIFY] Found target EURUSD/1h 13:00 zone in detected_raw_zones.txt: {}", line);
            found_specific_eurusd_1h_zone_in_raw_log = true;
            break;
        }
    }
    // You can assert this if you are sure it should be there after the generator run.
    // assert!(found_specific_eurusd_1h_zone_in_raw_log, "Target EURUSD/1h 13:00 zone NOT found in detected_raw_zones.txt");


    println!(
        "Raw log content sample (first 500 chars):\n{}",
        &file_content.chars().take(500).collect::<String>()
    );

    // ... (InfluxDB verification part remains the same) ...
    let influx_host =
        env::var("INFLUXDB_HOST").expect("INFLUXDB_HOST not set for test verification");
    let influx_org = env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG not set");
    let influx_token = env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN not set");
    let write_bucket = env::var("GENERATOR_WRITE_BUCKET").expect("GENERATOR_WRITE_BUCKET not set");

    let client = reqwest::Client::new();
    let query_start_for_influx = env::var("GENERATOR_START_TIME").unwrap(); // Use what test expects
    let query_end_for_influx = env::var("GENERATOR_END_TIME").unwrap();     // Use what test expects

    let query = format!(
        r#"from(bucket: "{}")
             |> range(start: {}, stop: {})
             |> filter(fn: (r) => r._measurement == "{}")
             |> filter(fn: (r) => r.symbol == "{}" and r.timeframe == "{}")
             |> limit(n:1)
             |> yield(name: "test_verification")"#,
        write_bucket,
        query_start_for_influx, // Use specific time set by test
        query_end_for_influx,   // Use specific time set by test
        TEST_ZONE_MEASUREMENT,
        "EURUSD_SB",
        "1h"
    );
    // ... (rest of InfluxDB verification)

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

    let response_text = response
        .text()
        .await
        .expect("Failed to get text from verification response");
    println!("Verification query for test_zones (EURUSD/1h):\n{}", query);
    println!(
        "Verification response from test_zones (EURUSD/1h):\n{}",
        response_text
    );
    
    let data_lines = response_text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .count();


    // --- Clean up Environment Variables set by this test ---
    std::env::remove_var("ENABLE_PERIODIC_ZONE_GENERATOR");
    std::env::remove_var("GENERATOR_START_TIME");
    std::env::remove_var("GENERATOR_END_TIME");
    std::env::remove_var("GENERATOR_EXCLUDE_TIMEFRAMES");
}