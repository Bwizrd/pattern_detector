// tests/optimization/advanced_optimizer.rs

use super::data_loader; 
use super::parameter_generator;

use super::html_reporter_advanced;

use pattern_detector::zones::patterns::{FiftyPercentBeforeBigBarRecognizer, PatternRecognizer};
use pattern_detector::trading::trades::{TradeConfig, Trade}; 
use pattern_detector::trading::trading::TradeExecutor;

use chrono::{DateTime, Utc, Weekday, Timelike, Datelike}; 
use dotenv::dotenv;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
// use tokio::task; // Removed
use futures::future::join_all; 
use log::{info, error, warn, debug}; 
use std::str::FromStr;

#[derive(Clone, Debug, serde::Serialize)] 
pub struct AdvancedOptimizationResult {
    pub symbol: String,
    pub timeframe: String,
    pub lot_size: f64,
    pub take_profit_pips: f64,
    pub stop_loss_pips: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub total_trades: usize,
    pub net_pips: f64,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub pnl_by_day_of_week: HashMap<String, f64>, 
    pub trades_by_day_of_week: HashMap<String, usize>,
    pub pnl_by_hour_of_day: HashMap<u32, f64>,
    pub trades_by_hour_of_day: HashMap<u32, usize>,
    pub average_trade_duration_str: String,
}

fn round_f64_adv(value: f64, decimals: u32) -> f64 {
    if value.is_infinite() || value.is_nan() { return value; }
    let multiplier = 10f64.powi(decimals as i32);
    (value * multiplier).round() / multiplier
}

#[tokio::test]
async fn run_advanced_optimization() -> Result<(), Box<dyn std::error::Error>> {
    let _ = env_logger::builder().is_test(true).try_init();
    dotenv().ok();

    let is_debug_run = std::env::var("OPTIMIZATION_DEBUG")
        .map(|s| s.to_lowercase() == "true" || s == "1")
        .unwrap_or(false);

    let debug_symbol: Option<String> = std::env::var("OPTIMIZATION_DEBUG_SYMBOL").ok();
    let debug_pattern_tf: Option<String> = std::env::var("OPTIMIZATION_DEBUG_PATTERN_TIMEFRAME").ok();
    let debug_tp: Option<f64> = std::env::var("OPTIMIZATION_DEBUG_TP").ok().and_then(|s| f64::from_str(&s).ok());
    let debug_sl: Option<f64> = std::env::var("OPTIMIZATION_DEBUG_SL").ok().and_then(|s| f64::from_str(&s).ok());

    if is_debug_run {
        info!("[DEBUG_MODE] OPTIMIZATION_DEBUG is true.");
        if let Some(ref sym) = debug_symbol { info!("[DEBUG_MODE] Targeting Symbol: {}", sym); }
        if let Some(ref tf) = debug_pattern_tf { info!("[DEBUG_MODE] Targeting Pattern TF: {}", tf); }
        if let Some(tp) = debug_tp { info!("[DEBUG_MODE] Targeting TP: {}", tp); }
        if let Some(sl) = debug_sl { info!("[DEBUG_MODE] Targeting SL: {}", sl); }
    }
    
    let host = std::env::var("INFLUXDB_HOST").unwrap_or_else(|_| "http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG must be set");
    let token = std::env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN must be set");
    let bucket = std::env::var("INFLUXDB_BUCKET").expect("INFLUXDB_BUCKET must be set");
    
    let start_time_str = if is_debug_run && debug_symbol.is_some() { // Use specific dates for focused debug
        "2025-05-11T22:00:00Z".to_string() 
    } else {
        std::env::var("START_TIME").expect("START_TIME must be set")
    };
    let end_time_str = if is_debug_run && debug_symbol.is_some() {
        "2025-05-16T22:00:00Z".to_string() 
    } else {
        std::env::var("END_TIME").expect("END_TIME must be set")
    };

    let start_datetime_obj = DateTime::parse_from_rfc3339(&start_time_str)?.with_timezone(&Utc);
    let end_datetime_obj = DateTime::parse_from_rfc3339(&end_time_str)?.with_timezone(&Utc);
    let total_days_in_period = (end_datetime_obj - start_datetime_obj).num_days();

    let symbols_to_optimize: Vec<String> = if let Some(ref sym) = debug_symbol {
        vec![sym.clone()]
    } else {
        vec![ 
            "EURUSD_SB".to_string(), "GBPUSD_SB".to_string(), "AUDUSD_SB".to_string(),
        ]
    };
    let pattern_timeframes_to_optimize: Vec<String> = if let Some(ref tf) = debug_pattern_tf {
        vec![tf.clone()]
    } else {
        // vec!["5m".to_string(), "15m".to_string(), "30m".to_string(), "1h".to_string()]
        vec!["30m".to_string(), "1h".to_string(), "4h".to_string(), "1d".to_string()] 
    };
    let lot_size_to_use = 0.01; 
    let sl_tp_config_default = ( (5.0, 100.0), (2.0, 50.0), 10 ); 

    let all_results_arc = Arc::new(TokioMutex::new(Vec::<AdvancedOptimizationResult>::new()));
    let mut outer_tasks = Vec::new();

    for symbol_str_ref in &symbols_to_optimize { 
        for pattern_tf_str_ref in &pattern_timeframes_to_optimize { 
            let task_symbol = symbol_str_ref.clone(); 
            let task_pattern_tf = pattern_tf_str_ref.clone(); 
            
            let host_c = host.clone(); let org_c = org.clone(); let token_c = token.clone(); let bucket_c = bucket.clone();
            let start_time_c = start_time_str.clone(); let end_time_c = end_time_str.clone();
            let lot_size_c = lot_size_to_use; 
            
            let all_results_arc_c = Arc::clone(&all_results_arc);

            outer_tasks.push(tokio::spawn(async move {
                let log_prefix = if is_debug_run { "[DEBUG_RUN]" } else { "" };
                info!("{} Optimizing: {} on Pattern TF: {}", log_prefix, task_symbol, task_pattern_tf);

                let pattern_candles = match data_loader::load_candles( &host_c, &org_c, &token_c, &bucket_c, &task_symbol, &task_pattern_tf, &start_time_c, &end_time_c ).await {
                    Ok(c) => c, Err(e) => { error!("{} Failed to load pattern candles for {}/{}: {}", log_prefix, task_symbol, task_pattern_tf, e); return; }
                };
                if pattern_candles.is_empty() { warn!("{} No pattern candles for {}/{}", log_prefix, task_symbol, task_pattern_tf); return; }
                if is_debug_run {
                    info!("{} Loaded {} pattern_candles for {}/{}. First: {:?}, Last: {:?}",
                        log_prefix, pattern_candles.len(), task_symbol, task_pattern_tf,
                        pattern_candles.first().map(|c|&c.time), pattern_candles.last().map(|c|&c.time));
                }

                let execution_candles = match data_loader::load_candles( &host_c, &org_c, &token_c, &bucket_c, &task_symbol, "1m", &start_time_c, &end_time_c ).await {
                     Ok(c) => c, Err(e) => { error!("{} Failed to load 1m exec candles for {}/{}: {}", log_prefix, task_symbol, task_pattern_tf, e); return; }
                };
                if execution_candles.is_empty() { warn!("{} No 1m exec candles for {}/{}", log_prefix, task_symbol, task_pattern_tf); return; }
                if is_debug_run {
                     info!("{} Loaded {} execution_candles for {}/{}", log_prefix, execution_candles.len(), task_symbol, task_pattern_tf);
                }

                let recognizer = FiftyPercentBeforeBigBarRecognizer::default();
                let detected_value_json: serde_json::Value = recognizer.detect(&pattern_candles);
                
                if is_debug_run {
                    info!("{} Full detected_value_json for {}/{}:\n{}", log_prefix, task_symbol, task_pattern_tf, serde_json::to_string_pretty(&detected_value_json).unwrap_or_else(|e| format!("Error serializing JSON: {}", e)));
                }
                
                let mut initial_zones_count = 0;
                if let Some(data) = detected_value_json.get("data").and_then(|d|d.get("price")) {
                    if let Some(sz) = data.get("supply_zones").and_then(|s|s.get("zones")).and_then(|z|z.as_array()) { initial_zones_count += sz.len(); }
                    if let Some(dz) = data.get("demand_zones").and_then(|d|d.get("zones")).and_then(|z|z.as_array()) { initial_zones_count += dz.len(); }
                }
                if initial_zones_count == 0 { info!("{} No initial patterns in JSON for {}/{}", log_prefix, task_symbol, task_pattern_tf); return; }
                info!("{} Initial zones count from JSON: {}", log_prefix, initial_zones_count);

                let test_points: Vec<(f64, f64)> = if is_debug_run && debug_tp.is_some() && debug_sl.is_some() {
                    info!("{} Using specific debug TP/SL: TP={}, SL={}", log_prefix, debug_tp.unwrap(), debug_sl.unwrap());
                    vec![(debug_tp.unwrap(), debug_sl.unwrap())]
                } else {
                    parameter_generator::generate_parameter_grid(
                        &sl_tp_config_default.0, 
                        &sl_tp_config_default.1, 
                        sl_tp_config_default.2   
                    )
                };
                
                debug!("{} Testing {} SL/TP combinations for {}/{}", log_prefix, test_points.len(), task_symbol, task_pattern_tf);
                let mut task_local_results = Vec::new();

                for (tp_pips, sl_pips) in test_points {
                    if tp_pips <= sl_pips { continue; }
                    if is_debug_run {
                        info!("{} Testing SL: {}, TP: {} for {}/{}", log_prefix, sl_pips, tp_pips, task_symbol, task_pattern_tf);
                    }

                    let trade_config = TradeConfig {
                        enabled: true, lot_size: lot_size_c,
                        default_stop_loss_pips: sl_pips, default_take_profit_pips: tp_pips,
                        risk_percent: 1.0, max_trades_per_pattern: 1, 
                        ..Default::default()
                    };

                    let mut executor = TradeExecutor::new(
                        trade_config,
                        &task_symbol, 
                        None,         
                        None          
                    );
                    executor.set_minute_candles(execution_candles.clone());

                    if is_debug_run {
                        info!("{} Calling execute_trades_for_pattern for {}/{} with SL:{}, TP:{}", log_prefix, task_symbol, task_pattern_tf, sl_pips, tp_pips);
                    }
                    let trades_from_executor: Vec<Trade> = executor.execute_trades_for_pattern( "fifty_percent_before_big_bar", &detected_value_json, &pattern_candles );
                    info!("{} Trades from executor for SL:{}/TP:{}: {}", log_prefix, sl_pips, tp_pips, trades_from_executor.len());
                    
                    // if !is_debug_run && trades_from_executor.len() < 5 { 
                    //     debug!("{} Skipping SL:{}/TP:{} due to < 5 trades ({})", log_prefix, sl_pips, tp_pips, trades_from_executor.len());
                    //     continue; 
                    // } 
                    if is_debug_run && trades_from_executor.is_empty() && initial_zones_count > 0 {
                        warn!("{} DEBUG: SL:{}/TP:{} - Initial zones detected but 0 trades executed.", log_prefix, sl_pips, tp_pips);
                    }


                    let mut pnl_by_day: HashMap<Weekday, f64> = HashMap::new();
                    let mut trades_by_day: HashMap<Weekday, usize> = HashMap::new();
                    let mut pnl_by_hour: HashMap<u32, f64> = HashMap::new();
                    let mut trades_by_hour: HashMap<u32, usize> = HashMap::new();
                    let mut total_duration_seconds_for_avg: i64 = 0;
                    let mut current_run_winning_trades = 0;
                    let mut current_run_net_pips_raw:f64 = 0.0;
                    let mut current_run_gross_profit_raw:f64 = 0.0;
                    let mut current_run_gross_loss_raw:f64 = 0.0;

                    for (trade_idx, trade_obj) in trades_from_executor.iter().enumerate() {
                        let pnl_raw = trade_obj.profit_loss_pips.unwrap_or(0.0);
                        
                        current_run_net_pips_raw += pnl_raw; 
                        if pnl_raw > 1e-9 {
                            current_run_winning_trades += 1;
                            current_run_gross_profit_raw += pnl_raw;
                        } else if pnl_raw < -1e-9 {
                            current_run_gross_loss_raw += pnl_raw.abs();
                        }

                        if let Ok(entry_dt) = DateTime::parse_from_rfc3339(&trade_obj.entry_time).map(|dt| dt.with_timezone(&Utc)) {
                            let day = entry_dt.weekday();
                            let hour = entry_dt.hour();
                            *pnl_by_day.entry(day).or_insert(0.0) += pnl_raw; 
                            *trades_by_day.entry(day).or_insert(0) += 1;
                            *pnl_by_hour.entry(hour).or_insert(0.0) += pnl_raw; 
                            *trades_by_hour.entry(hour).or_insert(0) += 1;
                            if is_debug_run && day == Weekday::Fri {
                                info!("{} DEBUG FRIDAY TRADE [{}]: Entry: {}, PNL: {:.1}, Exit: {:?}, Reason: {:?}", log_prefix, trade_idx, trade_obj.entry_time, pnl_raw, trade_obj.exit_time, trade_obj.exit_reason);
                            }
                        }
                        if let Some(exit_t_str) = &trade_obj.exit_time {
                            if let Ok(exit_t) = DateTime::parse_from_rfc3339(exit_t_str) {
                                if let Ok(entry_t) = DateTime::parse_from_rfc3339(&trade_obj.entry_time) {
                                     total_duration_seconds_for_avg += exit_t.signed_duration_since(entry_t).num_seconds();
                                }
                            }
                        }
                    }
                    
                    if is_debug_run {
                        info!("{} SL{}/TP{}: Total Trades: {}, Net Pips Raw: {:.1}", log_prefix, sl_pips, tp_pips, trades_from_executor.len(), current_run_net_pips_raw);
                        info!("{} SL{}/TP{}: Trades by Day: {:?}", log_prefix, sl_pips, tp_pips, trades_by_day);
                        info!("{} SL{}/TP{}: PNL by Day: {:?}", log_prefix, sl_pips, tp_pips, pnl_by_day);
                    }

                    let total_trades_this_run = trades_from_executor.len();
                    // Ensure we only add to results if there were trades, even in debug mode, to prevent division by zero later if needed
                    if total_trades_this_run == 0 && is_debug_run {
                         info!("{} No trades executed for SL {} / TP {} for {}/{}, skipping result addition.", log_prefix, sl_pips, tp_pips, task_symbol, task_pattern_tf);
                         continue; // Skip adding this to task_local_results if no trades
                    }


                    let win_rate_this_run = if total_trades_this_run > 0 { (current_run_winning_trades as f64 / total_trades_this_run as f64) * 100.0 } else { 0.0 };
                    let pf_this_run = if current_run_gross_loss_raw > 1e-9 { current_run_gross_profit_raw / current_run_gross_loss_raw } else if current_run_gross_profit_raw > 1e-9 { f64::INFINITY } else { 0.0 };
                    
                    let avg_duration_str_this_run = if total_trades_this_run > 0 && total_duration_seconds_for_avg != 0 { 
                        let avg_s = total_duration_seconds_for_avg / total_trades_this_run as i64;
                        let avg_duration = chrono::Duration::seconds(avg_s);
                        let h = avg_duration.num_hours();
                        let m = avg_duration.num_minutes() % 60;
                        let s = avg_duration.num_seconds() % 60;
                        if h > 0 { format!("{}h {}m {}s", h, m, s) }
                        else if m > 0 { format!("{}m {}s", m, s) }
                        else { format!("{}s", s) }
                    } else { "N/A".to_string() };

                    let pnl_by_day_of_week_str_keys: HashMap<String, f64> = pnl_by_day.iter().map(|(k,v)| (k.to_string(), round_f64_adv(*v,1))).collect();
                    let trades_by_day_of_week_str_keys: HashMap<String, usize> = trades_by_day.iter().map(|(k,v)| (k.to_string(), *v)).collect();
                    let pnl_by_hour_of_day_rounded: HashMap<u32, f64> = pnl_by_hour.iter().map(|(k,v)| (*k, round_f64_adv(*v,1))).collect();

                    task_local_results.push(AdvancedOptimizationResult {
                        symbol: task_symbol.clone(), timeframe: task_pattern_tf.clone(),
                        lot_size: lot_size_c, take_profit_pips: tp_pips, stop_loss_pips: sl_pips,
                        win_rate: round_f64_adv(win_rate_this_run, 2),
                        profit_factor: round_f64_adv(pf_this_run, 2),
                        total_trades: total_trades_this_run,
                        net_pips: round_f64_adv(current_run_net_pips_raw, 1),
                        winning_trades: current_run_winning_trades,
                        losing_trades: total_trades_this_run - current_run_winning_trades,
                        pnl_by_day_of_week: pnl_by_day_of_week_str_keys,
                        trades_by_day_of_week: trades_by_day_of_week_str_keys,
                        pnl_by_hour_of_day: pnl_by_hour_of_day_rounded,
                        trades_by_hour_of_day: trades_by_hour.clone(), 
                        average_trade_duration_str: avg_duration_str_this_run,
                    });
                } 

                if !task_local_results.is_empty() {
                    all_results_arc_c.lock().await.extend(task_local_results);
                }
                info!("{} Finished optimizing: {} on Pattern TF: {}", log_prefix, task_symbol, task_pattern_tf);
            }));
        } 
    } 

    join_all(outer_tasks).await; 

    let final_results = all_results_arc.lock().await.clone();
    info!("[ADV_OPT] ADVANCED OPTIMIZATION COMPLETE. Total unique parameter sets producing results: {}", final_results.len());

    if !final_results.is_empty() {
        let report_id_str = if is_debug_run && debug_symbol.is_some() {
             format!("{}_{}_debug_adv_opt", 
                debug_symbol.as_ref().unwrap_or(&"UNK".to_string()), 
                debug_pattern_tf.as_ref().map_or("allTF".to_string(), |tf| tf.replace(":", "_"))) // Sanitize timeframe for filename
        } else if symbols_to_optimize.len() == 1 {
            format!("{}_advanced_opt", symbols_to_optimize[0])
        } else if symbols_to_optimize.len() <= 3 { 
            format!("{}_adv_opt", symbols_to_optimize.iter().map(|s| s.split('_').next().unwrap_or(s)).collect::<Vec<&str>>().join("_"))
        } else {
            "multi_symbol_advanced_opt".to_string()
        };
        
        log::info!("[ADV_OPT] Generating advanced HTML report with ID: {}", report_id_str);
        match html_reporter_advanced::generate_advanced_report(
            &final_results,
            &report_id_str,
            start_datetime_obj,     
            end_datetime_obj,       
            total_days_in_period,
            if is_debug_run { 1 } else { 5 } // Show only 1 top config detail in debug mode, 5 otherwise
        ) {
            Ok(_) => info!("[ADV_OPT] Advanced optimization report generated successfully."),
            Err(e) => error!("[ADV_OPT] Failed to generate advanced optimization report: {}", e),
        }
    } else {
        info!("[ADV_OPT] No optimization results to generate an advanced report.");
    }

    Ok(())
}