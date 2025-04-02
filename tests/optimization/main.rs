// tests/optimization/main.rs
mod data_loader;
mod html_reporter;
mod parameter_generator;

use dotenv::dotenv;
use pattern_detector::detect::CandleData;
use pattern_detector::patterns::FiftyPercentBeforeBigBarRecognizer;
use pattern_detector::patterns::PatternRecognizer;
use pattern_detector::trades::{TradeConfig, TradeSummary};
use pattern_detector::trading::TradeExecutor;
use chrono::{DateTime, Utc};
use tokio::sync::Mutex as TokioMutex;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::task;

use crate::data_loader::load_candles;
use crate::html_reporter::generate_report;
use crate::parameter_generator::generate_parameter_grid;

mod multi_currency;

// Parameter search configuration
#[derive(Clone)]
pub struct ParameterSearchConfig {
    pub symbol: String,
    pub timeframes: Vec<String>,
    pub lot_size: f64,
    pub take_profit_range: (f64, f64),
    pub stop_loss_range: (f64, f64),
    pub max_iterations: usize,
}

// Result struct for optimization
#[derive(Clone)]
pub struct OptimizationResult {
    pub symbol: String,
    pub timeframe: String,
    pub lot_size: f64,
    pub take_profit_pips: f64,
    pub stop_loss_pips: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub total_profit_loss: f64,
    pub total_trades: usize,
    pub net_pips: f64,
}

#[tokio::test]
async fn run_optimization() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenv().ok();

    // Common parameters
    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG must be set");
    let token = std::env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN must be set");
    let bucket = std::env::var("INFLUXDB_BUCKET").expect("INFLUXDB_BUCKET must be set");
    let start_time = std::env::var("START_TIME").expect("START_TIME must be set");
    let end_time = std::env::var("END_TIME").expect("END_TIME must be set");
    
    // Check if 1m timeframe should be skipped
    let skip_1m = std::env::var("SKIP_1M_TIMEFRAME")
        .map(|v| v.to_lowercase() == "true" || v == "1")
        .unwrap_or(false);

    // Date range for reporting
    let start_datetime = DateTime::parse_from_rfc3339(&start_time)
        .unwrap_or_else(|_| DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap())
        .with_timezone(&Utc);

    let end_datetime = DateTime::parse_from_rfc3339(&end_time)
        .unwrap_or_else(|_| DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap())
        .with_timezone(&Utc);

    let total_days = (end_datetime - start_datetime).num_days();

    // Define timeframes, optionally excluding 1m
    let mut timeframes = vec!["5m".to_string(), "15m".to_string(), 
                         "30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()];
                         
    if !skip_1m {
        timeframes.push("1m".to_string());
    }

    // Define parameter search configuration
    let search_config = ParameterSearchConfig {
        symbol: "EURUSD_SB".to_string(),
        timeframes,
        lot_size: 0.1,
        take_profit_range: (5.0, 100.0),
        stop_loss_range: (2.0, 50.0),
        max_iterations: 10,
    };

    // Create shared result storage
    let results = Arc::new(TokioMutex::new(Vec::<OptimizationResult>::new()));
    let mut tasks = Vec::new();

    // Cache for loaded candles
    let candle_cache = Arc::new(TokioMutex::new(HashMap::<(String, String), Vec<CandleData>>::new()));

    println!("Starting optimization for {} across {} timeframes", 
             search_config.symbol, search_config.timeframes.len());

    // Spawn tasks for each timeframe
    for timeframe in &search_config.timeframes {
        let symbol = search_config.symbol.clone();
        let timeframe = timeframe.clone();
        let minute_timeframe = "1m".to_string();
        let host = host.clone();
        let org = org.clone();
        let token = token.clone();
        let bucket = bucket.clone();
        let start_time = start_time.clone();
        let end_time = end_time.clone();
        let results = Arc::clone(&results);
        let candle_cache = Arc::clone(&candle_cache);
        let search_config = search_config.clone();

        // Spawn a task for this timeframe
        let task = task::spawn(async move {
            println!("Processing timeframe: {}", timeframe);
            
            // Safely load timeframe candles
            let hourly_candles = {
                let mut cache = candle_cache.lock().await;
                if let Some(candles) = cache.get(&(symbol.clone(), timeframe.clone())) {
                    candles.clone()
                } else {
                    drop(cache);
                    
                    let candles = load_candles(
                        &host,
                        &org,
                        &token,
                        &bucket,
                        &symbol,
                        &timeframe,
                        &start_time,
                        &end_time,
                    )
                    .await
                    .unwrap_or_default();
                    
                    let mut cache = candle_cache.lock().await;
                    cache.insert((symbol.clone(), timeframe.clone()), candles.clone());
                    candles
                }
            };

            // Load minute candles for execution
            let minute_candles = {
                let mut cache = candle_cache.lock().await;
                if let Some(candles) = cache.get(&(symbol.clone(), minute_timeframe.clone())) {
                    candles.clone()
                } else {
                    drop(cache);
                    
                    let candles = load_candles(
                        &host,
                        &org,
                        &token,
                        &bucket,
                        &symbol,
                        &minute_timeframe,
                        &start_time,
                        &end_time,
                    )
                    .await
                    .unwrap_or_default();
                    
                    let mut cache = candle_cache.lock().await;
                    cache.insert((symbol.clone(), minute_timeframe.clone()), candles.clone());
                    candles
                }
            };

            if hourly_candles.is_empty() || minute_candles.is_empty() {
                println!("Warning: No data found for {} {}", symbol, timeframe);
                return;
            }

            // Run pattern detection
            let recognizer = FiftyPercentBeforeBigBarRecognizer::default();
            let pattern_data = recognizer.detect(&hourly_candles);

            // Track best parameters
            let mut best_profit_factor = 0.0;
            let mut best_net_pips = 0.0;
            let mut best_pf_params = (0.0, 0.0);
            let mut best_pips_params = (0.0, 0.0);
            
            // Generate parameter combinations
            let test_points = generate_parameter_grid(
                &search_config.take_profit_range,
                &search_config.stop_loss_range,
                search_config.max_iterations
            );
            
            println!("Testing {} parameter combinations for {}", test_points.len(), timeframe);
            
            // Test each parameter combination
            for (take_profit_pips, stop_loss_pips) in test_points {
                // Skip invalid risk:reward ratios
                if take_profit_pips <= stop_loss_pips {
                    continue;
                }
                
                // Create trade config
                let trade_config = TradeConfig {
                    enabled: true,
                    lot_size: search_config.lot_size,
                    default_stop_loss_pips: stop_loss_pips,
                    default_take_profit_pips: take_profit_pips,
                    risk_percent: 1.0,
                    max_trades_per_pattern: 2,
                    enable_trailing_stop: false,
                    trailing_stop_activation_pips: 10.0,
                    trailing_stop_distance_pips: 5.0,
                    ..Default::default()
                };

                // Create trade executor
                let mut trade_executor = TradeExecutor::new(trade_config);
                trade_executor.set_minute_candles(minute_candles.clone());

                // Execute trades
                let trades = trade_executor.execute_trades_for_pattern(
                    "fifty_percent_before_big_bar",
                    &pattern_data,
                    &hourly_candles,
                );

                // Calculate summary
                let summary = TradeSummary::from_trades(&trades);

                // Calculate pips
                let mut total_winning_pips = 0.0;
                let mut total_losing_pips = 0.0;
                for trade in &trades {
                    if let Some(pl_pips) = trade.profit_loss_pips {
                        if pl_pips > 0.0 {
                            total_winning_pips += pl_pips;
                        } else {
                            total_losing_pips += pl_pips.abs();
                        }
                    }
                }
                let net_pips = total_winning_pips - total_losing_pips;

                // Add result if we have enough trades
                if summary.total_trades >= 5 {
                    let result = OptimizationResult {
                        symbol: symbol.clone(),
                        timeframe: timeframe.clone(),
                        lot_size: search_config.lot_size,
                        take_profit_pips,
                        stop_loss_pips,
                        win_rate: summary.win_rate,
                        profit_factor: summary.profit_factor,
                        total_profit_loss: summary.total_profit_loss,
                        total_trades: summary.total_trades,
                        net_pips,
                    };
                    
                    // Update best parameters for profit factor
                    if summary.profit_factor > best_profit_factor {
                        best_profit_factor = summary.profit_factor;
                        best_pf_params = (take_profit_pips, stop_loss_pips);
                    }
                    
                    // Update best parameters for net pips
                    if net_pips > best_net_pips {
                        best_net_pips = net_pips;
                        best_pips_params = (take_profit_pips, stop_loss_pips);
                    }

                    let mut results_guard = results.lock().await;
                    results_guard.push(result);
                }
            }
            
            println!("Results for {}:", timeframe);
            println!("  Best PF: {:.2} (TP: {:.1}, SL: {:.1})", 
                     best_profit_factor, best_pf_params.0, best_pf_params.1);
            println!("  Best Pips: {:.1} (TP: {:.1}, SL: {:.1})", 
                     best_net_pips, best_pips_params.0, best_pips_params.1);
        });

        tasks.push(task);
    }

    // Wait for all tasks to complete
    for task in tasks {
        let _ = task.await;
    }

    // Get results and create report
    let results_guard = results.lock().await;
    
    // Clone for sorting
    let results_vec = results_guard.clone();
    
    // Generate report
    generate_report(
        &results_vec, 
        &search_config,
        &search_config.symbol, 
        start_datetime,
        end_datetime,
        total_days,
    )?;

    Ok(())
}