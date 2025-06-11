// tests/optimization/performance_scanner.rs

use super::data_loader; // Assuming data_loader.rs is in tests/optimization/
use super::html_reporter_scanner; // Assuming html_reporter_scanner.rs is in tests/optimization/

use pattern_detector::zones::patterns::{FiftyPercentBeforeBigBarRecognizer, PatternRecognizer};
use pattern_detector::trading::trades::{TradeConfig, Trade}; // Trade comes from pattern_detector::trading::trades
use pattern_detector::trading::trading::TradeExecutor;
use pattern_detector::types::CandleData; // Assuming CandleData from pattern_detector::types

use chrono::{DateTime, Utc, Weekday, Timelike, Datelike}; // Add all chrono traits used
use dotenv::dotenv;
// use std::collections::HashMap; // Not strictly needed by PerformanceScanResult itself
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use futures::future::join_all;
use log::{info, error, warn, debug}; // Ensure all log levels are imported
use std::str::FromStr;

#[derive(Clone, Debug, serde::Serialize)]
pub struct PerformanceScanResult {
    pub symbol: String,
    pub timeframe: String, // Pattern timeframe
    pub stop_loss_pips: f64,
    pub take_profit_pips: f64,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate_percent: f64,
    pub net_pips: f64,
    pub profit_factor: f64,
    pub average_trade_duration_str: String,
}

// Helper function for rounding
fn round_f64_scan(value: f64, decimals: u32) -> f64 {
    if value.is_infinite() || value.is_nan() { return value; }
    let multiplier = 10f64.powi(decimals as i32);
    (value * multiplier).round() / multiplier
}

#[tokio::test]
async fn run_performance_scan() -> Result<(), Box<dyn std::error::Error>> {
    let _ = env_logger::builder().is_test(true).try_init();
    dotenv().ok();
    info!("[PERF_SCAN] Starting Performance Scan.");

    // --- Configuration for the Scan ---
    let host = std::env::var("INFLUXDB_HOST").unwrap_or_else(|_| "http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG must be set");
    let token = std::env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN must be set");
    let bucket = std::env::var("INFLUXDB_BUCKET").expect("INFLUXDB_BUCKET must be set");
    
    let start_time_str = std::env::var("START_TIME").expect("START_TIME from .env must be set");
    let end_time_str = std::env::var("END_TIME").expect("END_TIME from .env must be set");
    info!("[PERF_SCAN] Using date range: {} to {}", start_time_str, end_time_str);

    let start_datetime_obj = DateTime::parse_from_rfc3339(&start_time_str)?.with_timezone(&Utc);
    let end_datetime_obj = DateTime::parse_from_rfc3339(&end_time_str)?.with_timezone(&Utc);
    let total_days_in_period = (end_datetime_obj - start_datetime_obj).num_days();

    let symbols_to_scan: Vec<String> = vec![ 
        "EURUSD_SB", "GBPUSD_SB", "USDCHF_SB", "AUDUSD_SB", "USDCAD_SB", 
        "NZDUSD_SB", "EURGBP_SB", "EURCHF_SB", "EURAUD_SB", "EURCAD_SB", 
        "EURNZD_SB", "GBPCHF_SB", "GBPAUD_SB", "GBPCAD_SB", "GBPNZD_SB", 
        "AUDNZD_SB", "AUDCAD_SB",
        "USDJPY_SB", "EURJPY_SB", "GBPJPY_SB", "AUDJPY_SB", 
        "NZDJPY_SB", "CADJPY_SB", "CHFJPY_SB",
        // Add Indices if you have data and want to test them with the correction
        // "NAS100_SB", "US500_SB", "XAU_SB" 
    ].into_iter().map(String::from).collect();

    let exclude_lower_tfs = std::env::var("SCANNER_EXCLUDE_LOWER_TFS")
        .map(|s| s.to_lowercase() == "true" || s == "1") 
        .unwrap_or(true); // Default to EXCLUDING 5m, 15m if var not set or not "true"

    let mut pattern_timeframes_to_scan: Vec<String> = vec![
        "5m".to_string(), "15m".to_string(), "30m".to_string(), 
        "1h".to_string(), "4h".to_string(), "1d".to_string()
    ];
    if exclude_lower_tfs {
        info!("[PERF_SCAN] SCANNER_EXCLUDE_LOWER_TFS is true. Excluding 5m and 15m timeframes from scan.");
        pattern_timeframes_to_scan.retain(|tf| tf != "5m" && tf != "15m");
    }
    info!("[PERF_SCAN] Scanning symbols: {:?}", symbols_to_scan);
    info!("[PERF_SCAN] Scanning pattern timeframes: {:?}", pattern_timeframes_to_scan);

    // let fixed_sl_pips = 10.0; 
    // let fixed_tp_pips = 50.0; 
    let fixed_tp_pips_str = std::env::var("SCANNER_TP_PIPS").unwrap_or_else(|_| "50.0".to_string());
    let fixed_sl_pips_str = std::env::var("SCANNER_SL_PIPS").unwrap_or_else(|_| "10.0".to_string());

    let fixed_tp_pips = f64::from_str(&fixed_tp_pips_str)
        .map_err(|e| format!("Invalid SCANNER_TP_PIPS value '{}': {}", fixed_tp_pips_str, e))?;
    let fixed_sl_pips = f64::from_str(&fixed_sl_pips_str)
        .map_err(|e| format!("Invalid SCANNER_SL_PIPS value '{}': {}", fixed_sl_pips_str, e))?;
    
    info!("[PERF_SCAN] Using Fixed SL: {} pips, Fixed TP: {} pips", fixed_sl_pips, fixed_tp_pips);
    let lot_size_to_use = 0.01;

    let all_scan_results_arc = Arc::new(TokioMutex::new(Vec::<PerformanceScanResult>::new()));
    let mut tasks = Vec::new();

    if symbols_to_scan.is_empty() || pattern_timeframes_to_scan.is_empty() {
        warn!("[PERF_SCAN] No symbols or timeframes selected for scan. Exiting.");
        return Ok(());
    }

    for symbol_str_ref in &symbols_to_scan {
        for pattern_tf_str_ref in &pattern_timeframes_to_scan {
            let task_symbol = symbol_str_ref.clone();
            let task_pattern_tf = pattern_tf_str_ref.clone();
            
            let host_c = host.clone(); 
            let org_c = org.clone(); 
            let token_c = token.clone(); 
            let bucket_c = bucket.clone();
            let start_time_c = start_time_str.clone(); 
            let end_time_c = end_time_str.clone();
            let lot_size_c = lot_size_to_use;
            let sl_c = fixed_sl_pips; 
            let tp_c = fixed_tp_pips;

            let results_arc_c = Arc::clone(&all_scan_results_arc);

            tasks.push(tokio::spawn(async move {
                info!("[SCAN_TASK] Scanning: {} on Pattern TF: {}", task_symbol, task_pattern_tf);

                let pattern_candles = match data_loader::load_candles(&host_c, &org_c, &token_c, &bucket_c, &task_symbol, &task_pattern_tf, &start_time_c, &end_time_c).await {
                    Ok(c) => c, Err(e) => { error!("[SCAN_TASK] Failed to load pattern candles for {}/{}: {}", task_symbol, task_pattern_tf, e); return; }
                };
                if pattern_candles.is_empty() { warn!("[SCAN_TASK] No pattern candles for {}/{}", task_symbol, task_pattern_tf); return; }
                debug!("[SCAN_TASK] Loaded {} pattern_candles for {}/{}", pattern_candles.len(), task_symbol, task_pattern_tf);

                let execution_candles = match data_loader::load_candles(&host_c, &org_c, &token_c, &bucket_c, &task_symbol, "1m", &start_time_c, &end_time_c).await {
                    Ok(c) => c, Err(e) => { error!("[SCAN_TASK] Failed to load 1m exec candles for {}/{}: {}", task_symbol, task_pattern_tf, e); return; }
                };
                if execution_candles.is_empty() { warn!("[SCAN_TASK] No 1m exec candles for {}/{}", task_symbol, task_pattern_tf); return; }
                debug!("[SCAN_TASK] Loaded {} execution_candles for {}/{}", execution_candles.len(), task_symbol, task_pattern_tf);

                let recognizer = FiftyPercentBeforeBigBarRecognizer::default();
                let detected_value_json: serde_json::Value = recognizer.detect(&pattern_candles);
                
                let mut initial_zones_count = 0;
                if let Some(data_val) = detected_value_json.get("data") {
                    if let Some(price_val) = data_val.get("price") {
                        if let Some(supply_zones_val) = price_val.get("supply_zones") {
                            if let Some(zones_array) = supply_zones_val.get("zones").and_then(|z| z.as_array()) {
                                initial_zones_count += zones_array.len();
                            }
                        }
                        if let Some(demand_zones_val) = price_val.get("demand_zones") {
                            if let Some(zones_array) = demand_zones_val.get("zones").and_then(|z| z.as_array()) {
                                initial_zones_count += zones_array.len();
                            }
                        }
                    }
                }

                if initial_zones_count == 0 { 
                    info!("[SCAN_TASK] No initial patterns found in JSON for {}/{}", task_symbol, task_pattern_tf); 
                    return; 
                }
                info!("[SCAN_TASK] {}/{}: Initial zones count from JSON: {}", task_symbol, task_pattern_tf, initial_zones_count);

                let trade_config = TradeConfig {
                    enabled: true, lot_size: lot_size_c,
                    default_stop_loss_pips: sl_c, default_take_profit_pips: tp_c,
                    risk_percent: 1.0, max_trades_per_zone: 1, 
                    ..Default::default()
                };

                let mut executor = TradeExecutor::new(trade_config, &task_symbol, None, None ); 
                executor.set_minute_candles(execution_candles.clone());

                let trades_from_executor: Vec<Trade> = executor.execute_trades_for_pattern(
                    "fifty_percent_before_big_bar", &detected_value_json, &pattern_candles
                );

                if trades_from_executor.is_empty() { 
                    info!("[SCAN_TASK] No trades executed by TradeExecutor for {}/{}", task_symbol, task_pattern_tf); 
                    return; 
                }
                info!("[SCAN_TASK] {}/{}: Trades from executor: {}", task_symbol, task_pattern_tf, trades_from_executor.len());

                let mut winning_trades = 0;
                let mut net_pips_raw:f64 = 0.0;
                let mut gross_profit_raw:f64 = 0.0;
                let mut gross_loss_raw:f64 = 0.0;
                let mut total_duration_seconds: i64 = 0;

                for trade_obj in &trades_from_executor {
                    let pnl_as_reported_by_trade_struct = trade_obj.profit_loss_pips.unwrap_or(0.0); 
                    let mut pnl_corrected_for_scan_report = pnl_as_reported_by_trade_struct;

                    // --- JPY/INDEX PIP CORRECTION FOR THIS SCANNER'S REPORTING ---
                    if task_symbol.contains("JPY_SB") {
                        pnl_corrected_for_scan_report /= 100.0; 
                        debug!("[SCAN_TASK_JPY_CORRECT] {}/{} | RawPips: {}, CorrectedPips: {}", task_symbol, task_pattern_tf, pnl_as_reported_by_trade_struct, pnl_corrected_for_scan_report);
                    } else if task_symbol.contains("NAS100_SB") || task_symbol.contains("US500_SB") || task_symbol.contains("XAU_SB") {
                        pnl_corrected_for_scan_report /= 10000.0;
                         debug!("[SCAN_TASK_IDX_CORRECT] {}/{} | RawPips: {}, CorrectedPips: {}", task_symbol, task_pattern_tf, pnl_as_reported_by_trade_struct, pnl_corrected_for_scan_report);
                    }
                    // --- END JPY/INDEX PIP CORRECTION ---
                        
                    net_pips_raw += pnl_corrected_for_scan_report; 
                    if pnl_corrected_for_scan_report > 1e-9 { 
                        winning_trades += 1; 
                        gross_profit_raw += pnl_corrected_for_scan_report; 
                    } else if pnl_corrected_for_scan_report < -1e-9 { 
                        gross_loss_raw += pnl_corrected_for_scan_report.abs(); 
                    }

                    if let (Some(exit_t_str), Ok(entry_t)) = (&trade_obj.exit_time, DateTime::parse_from_rfc3339(&trade_obj.entry_time)) {
                        if let Ok(exit_t) = DateTime::parse_from_rfc3339(exit_t_str) {
                            total_duration_seconds += exit_t.signed_duration_since(entry_t).num_seconds();
                        }
                    }
                }

                let total_trades = trades_from_executor.len();
                let losing_trades = total_trades - winning_trades;
                let win_rate = if total_trades > 0 { (winning_trades as f64 / total_trades as f64) * 100.0 } else { 0.0 };
                let pf = if gross_loss_raw > 1e-9 { gross_profit_raw / gross_loss_raw } else if gross_profit_raw > 1e-9 { f64::INFINITY } else { 0.0 };
                let avg_duration_str = if total_trades > 0 && total_duration_seconds != 0 {
                    let avg_s = total_duration_seconds / total_trades as i64;
                    let avg_d = chrono::Duration::seconds(avg_s); 
                    let h = avg_d.num_hours(); 
                    let m = avg_d.num_minutes() % 60; 
                    let s = avg_d.num_seconds() % 60;
                    if h > 0 { format!("{}h {}m {}s", h,m,s) } 
                    else if m > 0 { format!("{}m {}s", m,s) } 
                    else { format!("{}s",s) }
                } else { "N/A".to_string() };

                let result = PerformanceScanResult {
                    symbol: task_symbol.clone(), timeframe: task_pattern_tf.clone(),
                    stop_loss_pips: sl_c, take_profit_pips: tp_c,
                    total_trades, winning_trades, losing_trades,
                    win_rate_percent: round_f64_scan(win_rate, 2),
                    net_pips: round_f64_scan(net_pips_raw, 1),
                    profit_factor: round_f64_scan(pf, 2),
                    average_trade_duration_str: avg_duration_str,
                };
                results_arc_c.lock().await.push(result);
                info!("[SCAN_TASK] Finished scan and stored result for: {} on Pattern TF: {} -> {} trades, CORRECTED NET PIPS: {:.1}", task_symbol, task_pattern_tf, total_trades, net_pips_raw);
            }));
        }
    }

    join_all(tasks).await;
    let final_scan_results = all_scan_results_arc.lock().await.clone();
    info!("[PERF_SCAN] SCAN COMPLETE. Total Symbol/Timeframe combinations producing results: {}", final_scan_results.len());

    if !final_scan_results.is_empty() {
        let report_id = format!(
            "PerfScan_SL{}_TP{}_{}", 
            fixed_sl_pips, 
            fixed_tp_pips,
            if symbols_to_scan.len() == 1 { symbols_to_scan[0].split('_').next().unwrap_or(&symbols_to_scan[0]).to_string() }
            else if symbols_to_scan.len() <= 3 { symbols_to_scan.iter().map(|s| s.split('_').next().unwrap_or(s)).collect::<Vec<&str>>().join("-") }
            else { "MultiSymbol".to_string() }
        );

        info!("[PERF_SCAN] Generating performance scan report with ID: {}", report_id);
        match html_reporter_scanner::generate_performance_scan_report(
            &final_scan_results,
            &report_id,
            start_datetime_obj,
            end_datetime_obj,
            total_days_in_period,
            fixed_sl_pips, 
            fixed_tp_pips  
        ) {
            Ok(_) => info!("[PERF_SCAN] Performance Scan report generated successfully."),
            Err(e) => error!("[PERF_SCAN] Failed to generate Performance Scan report: {}", e),
        }
    } else {
        info!("[PERF_SCAN] No results from scan to generate a report (all combinations might have produced no trades).");
    }
    Ok(())
}