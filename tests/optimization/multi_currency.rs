// tests/optimization/multi_currency.rs
use super::data_loader::load_candles;
use super::parameter_generator::generate_parameter_grid;
use crate::OptimizationResult;
use crate::ParameterSearchConfig;

use chrono::{DateTime, Utc};
use dotenv::dotenv;
use pattern_detector::api::detect::CandleData;
use pattern_detector::zones::patterns::FiftyPercentBeforeBigBarRecognizer;
use pattern_detector::zones::patterns::PatternRecognizer;
use pattern_detector::trading::trades::{TradeConfig, TradeSummary};
use pattern_detector::trading::trading::TradeExecutor;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::task;

// Define list of currency pairs to test
const CURRENCY_PAIRS: &[&str] = &[
    "EURUSD_SB",
    "GBPUSD_SB",
    "USDJPY_SB",
    "USDCHF_SB",
    "AUDUSD_SB",
    "USDCAD_SB",
    "NZDUSD_SB",
    "EURGBP_SB",
    "EURJPY_SB",
    "EURCHF_SB",
    "EURAUD_SB",
    "EURCAD_SB",
    "EURNZD_SB",
    "GBPJPY_SB",
    "GBPCHF_SB",
    "GBPAUD_SB",
    "GBPCAD_SB",
    "GBPNZD_SB",
    "AUDJPY_SB",
    "AUDNZD_SB",
    "AUDCAD_SB",
    "NZDJPY_SB",
    "CADJPY_SB",
    "CHFJPY_SB",
];

// Store best results for each currency
#[derive(Clone)]
struct BestResults {
    currency: String,
    best_by_pf: Option<OptimizationResult>,
    best_by_pips: Option<OptimizationResult>,
    best_by_winrate: Option<OptimizationResult>,
}

#[tokio::test]
async fn run_multi_currency_optimization() -> Result<(), Box<dyn std::error::Error>> {
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

    // Limit number of concurrent currency pairs to avoid overwhelming the system
    let max_concurrent = std::env::var("MAX_CONCURRENT_PAIRS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4); // Default to 4 concurrent currency pairs

    // Date range for reporting
    let start_datetime = DateTime::parse_from_rfc3339(&start_time)
        .unwrap_or_else(|_| DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap())
        .with_timezone(&Utc);

    let end_datetime = DateTime::parse_from_rfc3339(&end_time)
        .unwrap_or_else(|_| DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap())
        .with_timezone(&Utc);

    let total_days = (end_datetime - start_datetime).num_days();

    // Define timeframes, optionally excluding 1m
    let mut timeframes = vec![
        "5m".to_string(),
        "15m".to_string(),
        "30m".to_string(),
        "1h".to_string(),
        "4h".to_string(),
        "1d".to_string(),
    ];

    if !skip_1m {
        timeframes.push("1m".to_string());
    }

    // Store all currency results for the summary report
    let all_currency_results = Arc::new(TokioMutex::new(Vec::<BestResults>::new()));

    // Process currency pairs in batches to avoid overloading the system
    for chunk in CURRENCY_PAIRS.chunks(max_concurrent) {
        let mut tasks = Vec::new();

        // Process each currency pair in this batch concurrently
        for &currency in chunk {
            let currency_string = currency.to_string();
            let host = host.clone();
            let org = org.clone();
            let token = token.clone();
            let bucket = bucket.clone();
            let start_time = start_time.clone();
            let end_time = end_time.clone();
            let timeframes = timeframes.clone();
            let all_currency_results = Arc::clone(&all_currency_results);

            // Spawn a task for this currency pair
            let task = task::spawn(async move {
                println!("Starting optimization for {}", currency_string);

                // Define parameter search configuration for this currency
                let search_config = ParameterSearchConfig {
                    symbol: currency_string.clone(),
                    timeframes,
                    lot_size: 0.1,
                    take_profit_range: (5.0, 100.0),
                    stop_loss_range: (2.0, 50.0),
                    max_iterations: 10,
                };

                // Create storage for the results
                let currency_results = Arc::new(TokioMutex::new(Vec::<OptimizationResult>::new()));
                let mut timeframe_tasks = Vec::new();

                // Cache for loaded candles
                let candle_cache = Arc::new(TokioMutex::new(HashMap::<
                    (String, String),
                    Vec<CandleData>,
                >::new()));

                // Process each timeframe for this currency
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
                    let results = Arc::clone(&currency_results);
                    let candle_cache = Arc::clone(&candle_cache);
                    let search_config = search_config.clone();

                    // Spawn a task for this timeframe
                    let task = task::spawn(async move {
                        // Load timeframe candles
                        let hourly_candles = {
                            let cache = candle_cache.lock().await;
                            if let Some(candles) = cache.get(&(symbol.clone(), timeframe.clone())) {
                                candles.clone()
                            } else {
                                // Release the mutex before the await
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
                            let cache = candle_cache.lock().await;
                            if let Some(candles) =
                                cache.get(&(symbol.clone(), minute_timeframe.clone()))
                            {
                                candles.clone()
                            } else {
                                // Release the mutex before the await
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
                                cache.insert(
                                    (symbol.clone(), minute_timeframe.clone()),
                                    candles.clone(),
                                );
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

                        // Generate parameter combinations
                        let test_points = generate_parameter_grid(
                            &search_config.take_profit_range,
                            &search_config.stop_loss_range,
                            search_config.max_iterations,
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
                                max_trades_per_zone: 2,
                                ..Default::default()
                            };

                            // Create trade executor
                            let default_symbol_for_recognizer_trade = "UNKNOWN_SYMBOL_IN_RECOGNIZER"; 
                            let mut trade_executor = TradeExecutor::new(trade_config, 
                                default_symbol_for_recognizer_trade,
                                None, None
                            );
                            trade_executor.set_minute_candles(minute_candles.clone());

                            // Execute trades
                            let trades = trade_executor.execute_trades_for_pattern(
                                "fifty_percent_before_big_bar",
                                &pattern_data,
                                &hourly_candles,
                                true
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
                            // In your code where you calculate pips
                            let net_pips = if symbol.contains("JPY") {
                                // Adjust JPY pairs by dividing by 100
                                total_winning_pips / 100.0 - total_losing_pips / 100.0
                            } else {
                                total_winning_pips - total_losing_pips
                            };

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

                                let mut results_guard = results.lock().await;
                                results_guard.push(result);
                            }
                        }
                    });

                    timeframe_tasks.push(task);
                }

                // Wait for all timeframe tasks to complete
                for task in timeframe_tasks {
                    let _ = task.await;
                }

                let symbol = search_config.symbol.clone();

                // Generate individual currency report
                let results_guard = currency_results.lock().await;
                if !results_guard.is_empty() {
                    // Generate the HTML report for this currency
                    crate::html_reporter::generate_report(
                        &results_guard,
                        &search_config,
                        &symbol,
                        start_datetime,
                        end_datetime,
                        total_days,
                    )
                    .unwrap_or_else(|e| {
                        println!("Error generating report for {}: {}", currency_string, e)
                    });

                    // Sort results for summary report
                    let mut pf_sorted = results_guard.clone();
                    let mut pips_sorted = results_guard.clone();
                    let mut winrate_sorted = results_guard.clone();

                    pf_sorted
                        .sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap());
                    pips_sorted.sort_by(|a, b| b.net_pips.partial_cmp(&a.net_pips).unwrap());
                    winrate_sorted.sort_by(|a, b| b.win_rate.partial_cmp(&a.win_rate).unwrap());

                    // Save best results for this currency
                    let best_results = BestResults {
                        currency: currency_string.clone(),
                        best_by_pf: if !pf_sorted.is_empty() {
                            Some(pf_sorted[0].clone())
                        } else {
                            None
                        },
                        best_by_pips: if !pips_sorted.is_empty() {
                            Some(pips_sorted[0].clone())
                        } else {
                            None
                        },
                        best_by_winrate: if !winrate_sorted.is_empty() {
                            Some(winrate_sorted[0].clone())
                        } else {
                            None
                        },
                    };

                    // Add to global results
                    let mut all_results = all_currency_results.lock().await;
                    all_results.push(best_results);
                }

                println!("Completed optimization for {}", currency_string);
            });

            tasks.push(task);
        }

        // Wait for this batch of currency pairs to complete
        for task in tasks {
            let _ = task.await;
        }
    }

    // Generate summary report
    generate_summary_report(
        all_currency_results,
        start_datetime,
        end_datetime,
        total_days,
    )
    .await?;

    Ok(())
}

async fn generate_summary_report(
    all_results: Arc<TokioMutex<Vec<BestResults>>>,
    start_datetime: DateTime<Utc>,
    end_datetime: DateTime<Utc>,
    total_days: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let results = all_results.lock().await;

    if results.is_empty() {
        println!("No results to include in summary report");
        return Ok(());
    }

    let date_str = chrono::Utc::now().format("%Y%m%d").to_string();
    let report_dir = format!("reports/{}", date_str);
    std::fs::create_dir_all(&report_dir)?;

    let summary_filename = format!(
        "{}/currency_summary_report_{}.html",
        report_dir,
        chrono::Utc::now().format("%H%M%S")
    );
    let mut html_file = File::create(&summary_filename)?;

    // Write HTML header
    writeln!(
        html_file,
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Multi-Currency Trading Optimization Summary</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        h1, h2, h3 {{ color: #333366; }}
        table {{ border-collapse: collapse; width: 100%; margin-bottom: 20px; }}
        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
        th {{ background-color: #f2f2f2; }}
        tr:nth-child(even) {{ background-color: #f9f9f9; }}
        tr:hover {{ background-color: #f5f5f5; }}
        .summary {{ background-color: #e6f7ff; padding: 15px; border-radius: 5px; margin-bottom: 20px; }}
        .best {{ background-color: #e6ffe6; }}
        .profit-factor-section, .net-pips-section, .win-rate-section {{ margin-bottom: 30px; }}
    </style>
</head>
<body>
    <h1>Multi-Currency Trading Optimization Summary</h1>
    <div class="summary">
        <h2>Backtest Summary</h2>
        <p><strong>Pattern:</strong> Fifty Percent Before Big Bar</p>
        <p><strong>Period:</strong> {} days (from {} to {})</p>
        <p><strong>Currencies Tested:</strong> {}</p>
    </div>"#,
        total_days,
        start_datetime.format("%Y-%m-%d"),
        end_datetime.format("%Y-%m-%d"),
        results.len()
    )?;

    // --- Profit Factor section ---
    writeln!(
        html_file,
        r#"
    <div class="profit-factor-section">
        <h2>Best Settings by Profit Factor</h2>
        <p>Currency pairs ranked by highest profit factor (best risk management and consistency)</p>
        <table>
            <tr>
                <th>Rank</th>
                <th>Currency</th>
                <th>Timeframe</th>
                <th>TP/SL (pips)</th>
                <th>Win Rate</th>
                <th>Profit Factor</th>
                <th>Net Pips</th>
                <th>Trades</th>
            </tr>"#
    )?;

    // Sort by profit factor
    let mut pf_results: Vec<_> = results
        .iter()
        .filter_map(|r| r.best_by_pf.as_ref().map(|pf| (r.currency.clone(), pf)))
        .collect();
    pf_results.sort_by(|a, b| b.1.profit_factor.partial_cmp(&a.1.profit_factor).unwrap());

    for (i, (currency, result)) in pf_results.iter().enumerate() {
        writeln!(
            html_file,
            r#"
            <tr{}>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{:.1}/{:.1}</td>
                <td>{:.2}%</td>
                <td>{:.2}</td>
                <td>{:.1}</td>
                <td>{}</td>
            </tr>"#,
            if i == 0 { " class=\"best\"" } else { "" },
            i + 1,
            currency,
            result.timeframe,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.net_pips,
            result.total_trades
        )?;
    }

    writeln!(html_file, "        </table>\n    </div>")?;

    // --- Net Pips section ---
    writeln!(
        html_file,
        r#"
    <div class="net-pips-section">
        <h2>Best Settings by Net Pips</h2>
        <p>Currency pairs ranked by highest net pips (best total profitability)</p>
        <table>
            <tr>
                <th>Rank</th>
                <th>Currency</th>
                <th>Timeframe</th>
                <th>TP/SL (pips)</th>
                <th>Win Rate</th>
                <th>Profit Factor</th>
                <th>Net Pips</th>
                <th>Trades</th>
            </tr>"#
    )?;

    // Sort by net pips
    let mut pips_results: Vec<_> = results
        .iter()
        .filter_map(|r| {
            r.best_by_pips
                .as_ref()
                .map(|pips| (r.currency.clone(), pips))
        })
        .collect();
    pips_results.sort_by(|a, b| b.1.net_pips.partial_cmp(&a.1.net_pips).unwrap());

    for (i, (currency, result)) in pips_results.iter().enumerate() {
        writeln!(
            html_file,
            r#"
            <tr{}>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{:.1}/{:.1}</td>
                <td>{:.2}%</td>
                <td>{:.2}</td>
                <td>{:.1}</td>
                <td>{}</td>
            </tr>"#,
            if i == 0 { " class=\"best\"" } else { "" },
            i + 1,
            currency,
            result.timeframe,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.net_pips,
            result.total_trades
        )?;
    }

    writeln!(html_file, "        </table>\n    </div>")?;

    // --- Win Rate section ---
    writeln!(
        html_file,
        r#"
    <div class="win-rate-section">
        <h2>Best Settings by Win Rate</h2>
        <p>Currency pairs ranked by highest win rate (best psychological comfort)</p>
        <table>
            <tr>
                <th>Rank</th>
                <th>Currency</th>
                <th>Timeframe</th>
                <th>TP/SL (pips)</th>
                <th>Win Rate</th>
                <th>Profit Factor</th>
                <th>Net Pips</th>
                <th>Trades</th>
            </tr>"#
    )?;

    // Sort by win rate
    let mut winrate_results: Vec<_> = results
        .iter()
        .filter_map(|r| {
            r.best_by_winrate
                .as_ref()
                .map(|wr| (r.currency.clone(), wr))
        })
        .collect();
    winrate_results.sort_by(|a, b| b.1.win_rate.partial_cmp(&a.1.win_rate).unwrap());

    for (i, (currency, result)) in winrate_results.iter().enumerate() {
        writeln!(
            html_file,
            r#"
            <tr{}>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{:.1}/{:.1}</td>
                <td>{:.2}%</td>
                <td>{:.2}</td>
                <td>{:.1}</td>
                <td>{}</td>
            </tr>"#,
            if i == 0 { " class=\"best\"" } else { "" },
            i + 1,
            currency,
            result.timeframe,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.net_pips,
            result.total_trades
        )?;
    }

    writeln!(html_file, "        </table>\n    </div>")?;

    // Add conclusion
    writeln!(
        html_file,
        r#"
    <h2>Currency Performance Analysis</h2>
    <p>The table below shows the top performing currency for each optimization approach:</p>
    <table>
        <tr>
            <th>Optimization Approach</th>
            <th>Best Currency</th>
            <th>Timeframe</th>
            <th>TP/SL</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
        </tr>
        <tr>
            <td>Profit Factor (consistency)</td>
            <td>{}</td>
            <td>{}</td>
            <td>{:.1}/{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
        </tr>
        <tr>
            <td>Net Pips (total profit)</td>
            <td>{}</td>
            <td>{}</td>
            <td>{:.1}/{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
        </tr>
        <tr>
            <td>Win Rate (psychology)</td>
            <td>{}</td>
            <td>{}</td>
            <td>{:.1}/{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
        </tr>
    </table>
    
    <p>Generated on: {}</p>
</body>
</html>"#,
        // PF best
        if !pf_results.is_empty() {
            &pf_results[0].0
        } else {
            "N/A"
        },
        if !pf_results.is_empty() {
            &pf_results[0].1.timeframe
        } else {
            "N/A"
        },
        if !pf_results.is_empty() {
            pf_results[0].1.take_profit_pips
        } else {
            0.0
        },
        if !pf_results.is_empty() {
            pf_results[0].1.stop_loss_pips
        } else {
            0.0
        },
        if !pf_results.is_empty() {
            pf_results[0].1.win_rate
        } else {
            0.0
        },
        if !pf_results.is_empty() {
            pf_results[0].1.profit_factor
        } else {
            0.0
        },
        if !pf_results.is_empty() {
            pf_results[0].1.net_pips
        } else {
            0.0
        },
        // Pips best
        if !pips_results.is_empty() {
            &pips_results[0].0
        } else {
            "N/A"
        },
        if !pips_results.is_empty() {
            &pips_results[0].1.timeframe
        } else {
            "N/A"
        },
        if !pips_results.is_empty() {
            pips_results[0].1.take_profit_pips
        } else {
            0.0
        },
        if !pips_results.is_empty() {
            pips_results[0].1.stop_loss_pips
        } else {
            0.0
        },
        if !pips_results.is_empty() {
            pips_results[0].1.win_rate
        } else {
            0.0
        },
        if !pips_results.is_empty() {
            pips_results[0].1.profit_factor
        } else {
            0.0
        },
        if !pips_results.is_empty() {
            pips_results[0].1.net_pips
        } else {
            0.0
        },
        // Win rate best
        if !winrate_results.is_empty() {
            &winrate_results[0].0
        } else {
            "N/A"
        },
        if !winrate_results.is_empty() {
            &winrate_results[0].1.timeframe
        } else {
            "N/A"
        },
        if !winrate_results.is_empty() {
            winrate_results[0].1.take_profit_pips
        } else {
            0.0
        },
        if !winrate_results.is_empty() {
            winrate_results[0].1.stop_loss_pips
        } else {
            0.0
        },
        if !winrate_results.is_empty() {
            winrate_results[0].1.win_rate
        } else {
            0.0
        },
        if !winrate_results.is_empty() {
            winrate_results[0].1.profit_factor
        } else {
            0.0
        },
        if !winrate_results.is_empty() {
            winrate_results[0].1.net_pips
        } else {
            0.0
        },
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
    )?;

    println!("Summary report generated: {}", summary_filename);

    Ok(())
}
