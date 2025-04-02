// tests/multithreaded_optimization.rs
// tests/enhanced_optimization.rs
use chrono::{DateTime, Utc};
use dotenv::dotenv;
use pattern_detector::detect::CandleData;
use pattern_detector::patterns::FiftyPercentBeforeBigBarRecognizer;
use pattern_detector::patterns::PatternRecognizer;
use pattern_detector::trades::{TradeConfig, TradeSummary};
use pattern_detector::trading::TradeExecutor;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::task;

// Parameter search configuration
#[derive(Clone)]
struct ParameterSearchConfig {
    symbol: String,
    timeframes: Vec<String>,
    lot_size: f64,
    take_profit_range: (f64, f64),
    stop_loss_range: (f64, f64),
    max_iterations: usize,
}

// Result struct for optimization
#[derive(Clone)]
struct OptimizationResult {
    symbol: String,
    timeframe: String,
    lot_size: f64,
    take_profit_pips: f64,
    stop_loss_pips: f64,
    win_rate: f64,
    profit_factor: f64,
    total_profit_loss: f64,
    total_trades: usize,
    net_pips: f64,
}

#[tokio::test]
async fn run_enhanced_optimization() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenv().ok();

    // Common parameters
    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG must be set");
    let token = std::env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN must be set");
    let bucket = std::env::var("INFLUXDB_BUCKET").expect("INFLUXDB_BUCKET must be set");
    let start_time = std::env::var("START_TIME").expect("START_TIME must be set");
    let end_time = std::env::var("END_TIME").expect("END_TIME must be set");
    let symbol = std::env::var("SYMBOL").unwrap_or("EURUSD_SB".to_string());

    // Date range for reporting
    let start_datetime = DateTime::parse_from_rfc3339(&start_time)
        .unwrap_or_else(|_| DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap())
        .with_timezone(&Utc);

    let end_datetime = DateTime::parse_from_rfc3339(&end_time)
        .unwrap_or_else(|_| DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap())
        .with_timezone(&Utc);

    let total_days = (end_datetime - start_datetime).num_days();

    let skip_1m_timeframe = std::env::var("SKIP_1M_TIMEFRAME")
        .map(|val| val.eq_ignore_ascii_case("true")) // Check if string is "true" (case-insensitive)
        .unwrap_or(false); // Default to false if not set or not "true"

    if skip_1m_timeframe {
        println!("SKIP_1M_TIMEFRAME=true detected. Skipping 1m timeframe for optimization.");
    }

    // Define parameter search configuration - EURUSD only with fixed lot size
    let mut search_config = ParameterSearchConfig {
        symbol: symbol.clone(),
        timeframes: vec![
            "1m".to_string(),
            "5m".to_string(),
            "15m".to_string(),
            "30m".to_string(),
            "1h".to_string(),
            "4h".to_string(),
            "1d".to_string(),
        ],
        lot_size: 0.1,                   // Fixed lot size
        take_profit_range: (5.0, 100.0), // Min and max take profit pips
        stop_loss_range: (2.0, 50.0),    // Min and max stop loss pips
        max_iterations: 10,              // Maximum search iterations
    };

    if skip_1m_timeframe {
        search_config.timeframes.retain(|tf| tf != "1m");
        println!("Removed '1m' from timeframes list.");
    }

    // Create shared result storage using a tokio mutex
    let results = Arc::new(TokioMutex::new(Vec::<OptimizationResult>::new()));
    let mut tasks = Vec::new();

    // Cache for loaded candles to avoid redundant loading
    let candle_cache = Arc::new(TokioMutex::new(
        HashMap::<(String, String), Vec<CandleData>>::new(),
    ));

    println!(
        "Starting optimization for {} across {} timeframes: {:?}", // Log the actual list
        search_config.symbol,
        search_config.timeframes.len(),
        search_config.timeframes // Log the actual list being used
    );

    // Spawn tasks for each timeframe
    for timeframe in &search_config.timeframes {
        let symbol = search_config.symbol.clone();
        let timeframe = timeframe.clone();
        let minute_timeframe = "1m".to_string(); // Always use 1m for execution
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

            // Safely load hourly candles with tokio mutex
            let hourly_candles = {
                let mut cache = candle_cache.lock().await;
                if let Some(candles) = cache.get(&(symbol.clone(), timeframe.clone())) {
                    candles.clone()
                } else {
                    // Safe to drop mutex while awaiting
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

                    // Re-acquire mutex
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
                    // Safe to drop mutex while awaiting
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

                    // Re-acquire mutex
                    let mut cache = candle_cache.lock().await;
                    cache.insert((symbol.clone(), minute_timeframe.clone()), candles.clone());
                    candles
                }
            };

            if hourly_candles.is_empty() || minute_candles.is_empty() {
                println!("Warning: No data found for {} {}", symbol, timeframe);
                return;
            }

            // Run pattern detection once to get pattern data
            let recognizer = FiftyPercentBeforeBigBarRecognizer::default();
            let pattern_data = recognizer.detect(&hourly_candles);

            // Track best parameters and results
            let mut best_profit_factor = 0.0;
            let mut best_net_pips = 0.0;
            let mut best_pf_params = (0.0, 0.0);
            let mut best_pips_params = (0.0, 0.0);

            // Generate parameter combinations systematically
            let test_points = generate_parameter_grid(
                &search_config.take_profit_range,
                &search_config.stop_loss_range,
                search_config.max_iterations,
            );

            println!(
                "Testing {} parameter combinations for {}",
                test_points.len(),
                timeframe
            );

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

                // Create a trade executor with minute data
                let mut trade_executor = TradeExecutor::new(trade_config);
                trade_executor.set_minute_candles(minute_candles.clone());

                // Execute trades using the executor
                let trades = trade_executor.execute_trades_for_pattern(
                    "fifty_percent_before_big_bar",
                    &pattern_data,
                    &hourly_candles,
                );

                // Calculate summary from trades
                let summary = TradeSummary::from_trades(&trades);

                // Calculate total pips
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

                // Add result to the shared results if we have enough trades
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

                    // Safely store results with tokio mutex
                    let mut results_guard = results.lock().await;
                    results_guard.push(result);
                }
            }

            println!("Results for {}:", timeframe);
            println!(
                "  Best PF: {:.2} (TP: {:.1}, SL: {:.1})",
                best_profit_factor, best_pf_params.0, best_pf_params.1
            );
            println!(
                "  Best Pips: {:.1} (TP: {:.1}, SL: {:.1})",
                best_net_pips, best_pips_params.0, best_pips_params.1
            );
        });

        tasks.push(task);
    }

    // Wait for all tasks to complete
    for task in tasks {
        let _ = task.await;
    }

    // Generate report
    let results_guard = results.lock().await;

    // Convert to standard Vec for sorting
    let mut sorted_results = results_guard.clone();

    // Sort by profit factor (descending)
    sorted_results.sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap());

    // Create a copy sorted by net pips
    let mut pips_sorted_results = sorted_results.clone();
    pips_sorted_results.sort_by(|a, b| b.net_pips.partial_cmp(&a.net_pips).unwrap());

    let date_str = chrono::Utc::now().format("%Y%m%d").to_string();
    let report_dir = format!("reports/{}", date_str);
    std::fs::create_dir_all(&report_dir)?;

    // Create CSV report
    let csv_filename = format!(
        "{}/optimization_results_{}.csv",
        report_dir,
        chrono::Utc::now().format("%H%M%S")
    );
    let mut csv_file = File::create(&csv_filename)?;

    // Write CSV header
    writeln!(csv_file, "Symbol,Timeframe,Lot Size,TP Pips,SL Pips,Win Rate,Profit Factor,Total P/L,Total Trades,Net Pips")?;

    // Write data
    for result in &sorted_results {
        writeln!(
            csv_file,
            "{},{},{:.2},{:.1},{:.1},{:.2}%,{:.2},{:.2},{},{}",
            result.symbol,
            result.timeframe,
            result.lot_size,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.total_profit_loss,
            result.total_trades,
            result.net_pips
        )?;
    }

    // Generate HTML report
    let html_filename = format!(
        "{}/optimization_report_{}.html",
        report_dir,
        chrono::Utc::now().format("%Y%m%d_%H%M%S")
    );
    let mut html_file = File::create(&html_filename)?;

    // Write HTML header
    write!(
        html_file,
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Forex Trading Strategy Optimization Report</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        h1, h2 {{ color: #333366; }}
        table {{ border-collapse: collapse; width: 100%; margin-bottom: 20px; }}
        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
        th {{ background-color: #f2f2f2; }}
        tr:nth-child(even) {{ background-color: #f9f9f9; }}
        tr:hover {{ background-color: #f5f5f5; }}
        .summary {{ background-color: #e6f7ff; padding: 15px; border-radius: 5px; margin-bottom: 20px; }}
        .best {{ background-color: #e6ffe6; }}
        .tabs {{ display: flex; margin-bottom: 10px; }}
        .tab {{ padding: 8px 16px; cursor: pointer; background-color: #f1f1f1; border: 1px solid #ccc; }}
        .tab.active {{ background-color: #e6f7ff; border-bottom: none; }}
        .tab-content {{ display: none; }}
        .tab-content.active {{ display: block; }}
    </style>
</head>
<body>
    <h1>Forex Trading Strategy Optimization Report</h1>
    <div class="summary">
        <h2>Backtest Summary</h2>
        <p><strong>Pattern:</strong> Fifty Percent Before Big Bar</p>
        <p><strong>Symbol:</strong> {}</p>
        <p><strong>Period:</strong> {} days (from {} to {})</p>
        <p><strong>Total Combinations Tested:</strong> {}</p>
    </div>
"#,
        search_config.symbol,
        total_days,
        start_datetime.format("%Y-%m-%d"),
        end_datetime.format("%Y-%m-%d"),
        sorted_results.len()
    )?;

    // Write top 10 results by profit factor
    write!(
        html_file,
        r#"
    <h2>Top 10 Results by Profit Factor</h2>
    <p>Optimizing for risk-adjusted returns (consistency and risk management)</p>
    <table>
        <tr>
            <th>Rank</th>
            <th>Timeframe</th>
            <th>TP (pips)</th>
            <th>SL (pips)</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
            <th>Trades</th>
        </tr>
"#
    )?;

    // Get top 10 or all if less than 10
    let top_count = std::cmp::min(10, sorted_results.len());
    for i in 0..top_count {
        let result = &sorted_results[i];
        write!(
            html_file,
            r#"
        <tr{}>
            <td>{}</td>
            <td>{}</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>
"#,
            if i == 0 { " class=\"best\"" } else { "" },
            i + 1,
            result.timeframe,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.net_pips,
            result.total_trades
        )?;
    }

    write!(html_file, "    </table>\n")?;

    // Write top 10 results by net pips
    write!(
        html_file,
        r#"
    <h2>Top 10 Results by Net Pips</h2>
    <p>Optimizing for total profit (potentially higher returns with higher risk)</p>
    <table>
        <tr>
            <th>Rank</th>
            <th>Timeframe</th>
            <th>TP (pips)</th>
            <th>SL (pips)</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
            <th>Trades</th>
        </tr>
"#
    )?;

    // Get top 10 by pips or all if less than 10
    let pips_top_count = std::cmp::min(10, pips_sorted_results.len());
    for i in 0..pips_top_count {
        let result = &pips_sorted_results[i];
        write!(
            html_file,
            r#"
        <tr{}>
            <td>{}</td>
            <td>{}</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>
"#,
            if i == 0 { " class=\"best\"" } else { "" },
            i + 1,
            result.timeframe,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.net_pips,
            result.total_trades
        )?;
    }

    write!(html_file, "    </table>\n")?;

    // Add results by timeframe
    write!(
        html_file,
        r#"
    <h2>Results by Timeframe</h2>
"#
    )?;

    // Group by timeframe
    let mut timeframe_groups = HashMap::new();
    for result in &sorted_results {
        let key = result.timeframe.clone();
        timeframe_groups
            .entry(key)
            .or_insert_with(Vec::new)
            .push(result.clone());
    }

    // For each timeframe, show best result by PF and best by pips
    for (timeframe, mut group) in timeframe_groups {
        if group.is_empty() {
            continue;
        }

        // Sort by profit factor
        group.sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap());
        let best_pf = &group[0];

        // Sort by net pips
        let mut pips_group = group.clone();
        pips_group.sort_by(|a, b| b.net_pips.partial_cmp(&a.net_pips).unwrap());
        let best_pips = &pips_group[0];

        write!(
            html_file,
            r#"
    <h3>Timeframe: {}</h3>
    <table>
        <tr>
            <th>Optimization</th>
            <th>TP (pips)</th>
            <th>SL (pips)</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
            <th>Trades</th>
        </tr>
        <tr class="best">
            <td>Best Profit Factor</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>
        <tr>
            <td>Best Net Pips</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>
    </table>
"#,
            timeframe,
            best_pf.take_profit_pips,
            best_pf.stop_loss_pips,
            best_pf.win_rate,
            best_pf.profit_factor,
            best_pf.net_pips,
            best_pf.total_trades,
            best_pips.take_profit_pips,
            best_pips.stop_loss_pips,
            best_pips.win_rate,
            best_pips.profit_factor,
            best_pips.net_pips,
            best_pips.total_trades
        )?;
    }

    // Add comparison of optimization approaches
    write!(
        html_file,
        r#"
    <h2>Conclusion</h2>
    <p>The optimization process tested {} different parameter combinations across {} timeframes.</p>
"#,
        sorted_results.len(),
        search_config.timeframes.len()
    )?;

    if !sorted_results.is_empty() && !pips_sorted_results.is_empty() {
        write!(
            html_file,
            r#"
    <h3>Optimization Approaches Comparison</h3>
    <table>
        <tr>
            <th>Approach</th>
            <th>Timeframe</th>
            <th>TP (pips)</th>
            <th>SL (pips)</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
            <th>Trades</th>
            <th>Description</th>
        </tr>
        <tr class="best">
            <td>Profit Factor</td>
            <td>{}</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
            <td>Prioritizes consistency and risk management</td>
        </tr>
        <tr>
            <td>Net Pips</td>
            <td>{}</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
            <td>Prioritizes total profit</td>
        </tr>
    </table>
    
    <p><strong>Profit Factor Approach:</strong> More consistent returns with better risk management. Typically has lower drawdowns.</p>
    <p><strong>Net Pips Approach:</strong> Higher total profit but potentially higher risk and larger drawdowns.</p>
"#,
            sorted_results[0].timeframe,
            sorted_results[0].take_profit_pips,
            sorted_results[0].stop_loss_pips,
            sorted_results[0].win_rate,
            sorted_results[0].profit_factor,
            sorted_results[0].net_pips,
            sorted_results[0].total_trades,
            pips_sorted_results[0].timeframe,
            pips_sorted_results[0].take_profit_pips,
            pips_sorted_results[0].stop_loss_pips,
            pips_sorted_results[0].win_rate,
            pips_sorted_results[0].profit_factor,
            pips_sorted_results[0].net_pips,
            pips_sorted_results[0].total_trades
        )?;
    }

    write!(
        html_file,
        r#"
    <p>Generated on: {}</p>
</body>
</html>
"#,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
    )?;

    // Print summary to console
    println!("\nOptimization Complete:");
    println!(
        "Total parameter combinations tested: {}",
        sorted_results.len()
    );
    println!("Results saved to {} and {}", csv_filename, html_filename);

    if !sorted_results.is_empty() && !pips_sorted_results.is_empty() {
        println!("\nOptimization Approaches Comparison:");
        println!("----------------------------------");
        println!("Best by Profit Factor (consistency):");
        println!("  Timeframe: {}", sorted_results[0].timeframe);
        println!(
            "  Take Profit: {:.1} pips",
            sorted_results[0].take_profit_pips
        );
        println!("  Stop Loss: {:.1} pips", sorted_results[0].stop_loss_pips);
        println!("  Win Rate: {:.2}%", sorted_results[0].win_rate);
        println!("  Profit Factor: {:.2}", sorted_results[0].profit_factor);
        println!("  Net Pips: {:.1}", sorted_results[0].net_pips);
        println!("  Total Trades: {}", sorted_results[0].total_trades);

        println!("\nBest by Net Pips (total profit):");
        println!("  Timeframe: {}", pips_sorted_results[0].timeframe);
        println!(
            "  Take Profit: {:.1} pips",
            pips_sorted_results[0].take_profit_pips
        );
        println!(
            "  Stop Loss: {:.1} pips",
            pips_sorted_results[0].stop_loss_pips
        );
        println!("  Win Rate: {:.2}%", pips_sorted_results[0].win_rate);
        println!(
            "  Profit Factor: {:.2}",
            pips_sorted_results[0].profit_factor
        );
        println!("  Net Pips: {:.1}", pips_sorted_results[0].net_pips);
        println!("  Total Trades: {}", pips_sorted_results[0].total_trades);
    }

    Ok(())
}

// Generate a grid of parameter combinations
fn generate_parameter_grid(
    tp_range: &(f64, f64),
    sl_range: &(f64, f64),
    max_points: usize,
) -> Vec<(f64, f64)> {
    let mut results = Vec::new();

    // Calculate how many points to sample in each dimension
    let dim_size = (max_points as f64).sqrt().ceil() as usize;

    // Generate TP points
    let mut tp_values = Vec::new();
    for i in 0..dim_size {
        let tp_value = tp_range.0 + (tp_range.1 - tp_range.0) * (i as f64 / (dim_size - 1) as f64);
        tp_values.push((tp_value * 10.0).round() / 10.0);
    }

    // Generate SL points
    let mut sl_values = Vec::new();
    for i in 0..dim_size {
        let sl_value = sl_range.0 + (sl_range.1 - sl_range.0) * (i as f64 / (dim_size - 1) as f64);
        sl_values.push((sl_value * 10.0).round() / 10.0);
    }

    // Add some specific values that are frequently good
    tp_values.push(5.0);
    tp_values.push(10.0);
    tp_values.push(15.0);
    tp_values.push(20.0);

    sl_values.push(2.0);
    sl_values.push(5.0);
    sl_values.push(10.0);

    // Generate combinations
    for &tp in &tp_values {
        for &sl in &sl_values {
            // Only include reasonable risk:reward ratios
            if tp > sl {
                results.push((tp, sl));
            }
        }
    }

    // Remove duplicates
    results.sort_by(|a, b| {
        let cmp = a.0.partial_cmp(&b.0).unwrap();
        if cmp == std::cmp::Ordering::Equal {
            a.1.partial_cmp(&b.1).unwrap()
        } else {
            cmp
        }
    });
    results.dedup();

    results
}

// Helper function to load candles (reused from the original code)
async fn load_candles(
    host: &str,
    org: &str,
    token: &str,
    bucket: &str,
    symbol: &str,
    timeframe: &str,
    start_time: &str,
    end_time: &str,
) -> Result<Vec<CandleData>, Box<dyn std::error::Error>> {
    use csv::ReaderBuilder;
    use reqwest;
    use serde_json::json;
    use std::io::Cursor;

    // Query construction
    let flux_query = format!(
        r#"from(bucket: "{}")
        |> range(start: {}, stop: {})
        |> filter(fn: (r) => r["_measurement"] == "trendbar")
        |> filter(fn: (r) => r["symbol"] == "{}")
        |> filter(fn: (r) => r["timeframe"] == "{}")
        |> toFloat()
        |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value")
        |> sort(columns: ["_time"])"#,
        bucket, start_time, end_time, symbol, timeframe
    );

    // Execute query
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&json!({"query": flux_query, "type": "flux"}))
        .send()
        .await?
        .text()
        .await?;

    // Parse the data
    let mut candles = Vec::new();
    let cursor = Cursor::new(&response);
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(cursor);

    // Parse headers
    let headers = rdr.headers()?;

    // Find column indices
    let mut time_idx = None;
    let mut open_idx = None;
    let mut high_idx = None;
    let mut low_idx = None;
    let mut close_idx = None;
    let mut volume_idx = None;

    for (i, name) in headers.iter().enumerate() {
        match name {
            "_time" => time_idx = Some(i),
            "open" => open_idx = Some(i),
            "high" => high_idx = Some(i),
            "low" => low_idx = Some(i),
            "close" => close_idx = Some(i),
            "volume" => volume_idx = Some(i),
            _ => {}
        }
    }

    // Parse records
    for result in rdr.records() {
        let record = result?;
        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) =
            (time_idx, open_idx, high_idx, low_idx, close_idx, volume_idx)
        {
            if let Some(time_val) = record.get(t_idx) {
                let open = record
                    .get(o_idx)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let high = record
                    .get(h_idx)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let low = record
                    .get(l_idx)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let close = record
                    .get(c_idx)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let volume = record
                    .get(v_idx)
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(0);

                candles.push(CandleData {
                    time: time_val.to_string(),
                    open,
                    high,
                    low,
                    close,
                    volume,
                });
            }
        }
    }

    Ok(candles)
}
