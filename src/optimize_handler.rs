// src/optimize_handler.rs

use actix_web::{web, HttpResponse, Responder, error::ErrorInternalServerError, error::ErrorBadRequest};
use serde::{Deserialize, Serialize};
use log::{info, warn};
use chrono::{DateTime, Utc, Weekday, Timelike, Datelike};
use crate::patterns::PatternRecognizer;
use std::collections::HashMap;
use futures::future::join_all;
use serde_json::json;
use csv::ReaderBuilder;
use std::io::Cursor;

// Request structure
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OptimizeParams {
    pub start_time: String,
    pub end_time: String,
    pub symbols: Vec<String>,
    pub pattern_timeframes: Vec<String>,
    pub lot_size: f64,
    pub allowed_trade_days: Vec<String>,
    pub sl_range: SlTpRange,
    pub tp_range: SlTpRange,
    pub max_combinations: Option<usize>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SlTpRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

// --- Response structures ---

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationResult {
    pub symbol: String,
    pub timeframe: String,
    pub sl_pips: f64,
    pub tp_pips: f64,
    pub total_pips: f64,
    pub total_trades: usize,
    pub win_rate: f64,
    pub profit_factor: Option<f64>,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub avg_trade_duration: String,
    pub risk_reward_ratio: f64,
    pub sharpe_ratio: Option<f64>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BestCombinationEntry {
    pub symbol: String, // Will be raw e.g., GBPUSD_SB
    #[serde(rename = "SL")]
    pub sl_pips: f64,
    #[serde(rename = "TP")]
    pub tp_pips: f64,
    #[serde(rename = "TotalPips")]
    pub total_pips: f64,
    pub total_trades: usize, // Added
    // Removed: #[serde(rename = "Duration")] pub avg_trade_duration: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SymbolPerformance {
    pub symbol: String, 
    pub combinations: Vec<OptimizationResult>, 
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TimeframePerformance {
    pub timeframe: String,
    pub symbols: Vec<SymbolPerformance>, 
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationResponse {
    pub request_summary: RequestSummary,
    pub total_combinations_tested: usize,
    pub grouped_results: Vec<TimeframePerformance>, 
    pub best_combinations: Vec<BestCombinationEntry>, 
    pub best_result: OptimizationResult, 
    pub worst_result: OptimizationResult, 
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummary {
    pub start_time: String,
    pub end_time: String,
    pub symbols: Vec<String>,
    pub timeframes: Vec<String>,
    pub sl_range: String,
    pub tp_range: String,
    pub total_combinations: usize,
}

// Removed format_symbol_for_display function

// Helper function to format UTC hour (0-23) to AM/PM string - NOT USED ANYMORE, but kept for potential future use
#[allow(dead_code)]
fn format_hour_to_ampm(hour_utc: u32) -> String {
    match hour_utc {
        0 => "12am".to_string(),
        1..=11 => format!("{}am", hour_utc),
        12 => "12pm".to_string(),
        13..=23 => format!("{}pm", hour_utc - 12),
        _ => format!("{}h_utc", hour_utc), 
    }
}


pub async fn run_parameter_optimization(
    params: web::Json<OptimizeParams>,
) -> Result<impl Responder, actix_web::Error> {
    let request_params = params.into_inner();
    info!("Starting parameter optimization: {:?}", request_params);

    if request_params.sl_range.min >= request_params.sl_range.max ||
       request_params.tp_range.min >= request_params.tp_range.max ||
       request_params.sl_range.step <= 0.0 || 
       request_params.tp_range.step <= 0.0 {
        return Err(ErrorBadRequest("Invalid SL/TP ranges or step values"));
    }

    let sl_values = generate_range(
        request_params.sl_range.min,
        request_params.sl_range.max,
        request_params.sl_range.step,
    );
    
    let tp_values = generate_range(
        request_params.tp_range.min,
        request_params.tp_range.max,
        request_params.tp_range.step,
    );

    if sl_values.is_empty() || tp_values.is_empty() {
        return Err(ErrorBadRequest("SL/TP ranges generated no values. Check min, max, step."));
    }

    let total_combinations_possible = sl_values.len() * tp_values.len();
    info!("Testing {} SL values × {} TP values = {} potential combinations", 
          sl_values.len(), tp_values.len(), total_combinations_possible);

    let combinations_to_test = request_params.max_combinations
        .map(|max| std::cmp::min(max, total_combinations_possible))
        .unwrap_or(total_combinations_possible);
    
    if combinations_to_test < total_combinations_possible {
        warn!("Limiting combinations from {} to {}", total_combinations_possible, combinations_to_test);
    }

    let mut all_optimization_results = Vec::new();
    let mut tested_count = 0;

    'outer_loop: for (_sl_idx, &sl_pips) in sl_values.iter().enumerate() {
        for (_tp_idx, &tp_pips) in tp_values.iter().enumerate() {
            if tested_count >= combinations_to_test {
                break 'outer_loop; 
            }

            info!("Testing combination {}/{}: SL={}, TP={}", 
                  tested_count + 1, combinations_to_test, sl_pips, tp_pips);

            let multi_backtest_params = MultiBacktestParams {
                start_time: request_params.start_time.clone(),
                end_time: request_params.end_time.clone(),
                symbols: request_params.symbols.clone(),
                pattern_timeframes: request_params.pattern_timeframes.clone(),
                stop_loss_pips: sl_pips,
                take_profit_pips: tp_pips,
                lot_size: Some(request_params.lot_size),
                allowed_trade_days: Some(request_params.allowed_trade_days.clone()),
                trade_end_hour_utc: None,
            };

            match run_multi_backtest_internal(multi_backtest_params).await {
                Ok(backtest_result) => {
                    for symbol in &request_params.symbols {
                        for timeframe in &request_params.pattern_timeframes {
                            let symbol_trades: Vec<&TaskTrade> = backtest_result.all_trades
                                .iter()
                                .filter(|t| t.symbol == *symbol && t.timeframe == *timeframe)
                                .collect();

                            if !symbol_trades.is_empty() {
                                let symbol_total_pips: f64 = symbol_trades.iter().map(|t| t.pnl_pips).sum();
                                let symbol_winning_trades = symbol_trades.iter().filter(|t| t.pnl_pips > 0.0).count();
                                let symbol_total_trades = symbol_trades.len();
                                let symbol_win_rate = if symbol_total_trades > 0 {
                                    (symbol_winning_trades as f64 / symbol_total_trades as f64) * 100.0
                                } else { 0.0 };

                                let symbol_gross_profit: f64 = symbol_trades.iter().filter(|t| t.pnl_pips > 0.0).map(|t| t.pnl_pips).sum();
                                let symbol_gross_loss: f64 = symbol_trades.iter().filter(|t| t.pnl_pips < 0.0).map(|t| t.pnl_pips.abs()).sum();
                                let symbol_profit_factor = if symbol_gross_loss > 1e-9 { 
                                    Some(symbol_gross_profit / symbol_gross_loss)
                                } else if symbol_gross_profit > 1e-9 { None  } else { Some(0.0) };

                                let symbol_durations: Vec<i64> = symbol_trades.iter()
                                    .filter_map(|t| t.exit_time.map(|exit| exit.signed_duration_since(t.entry_time).num_seconds()))
                                    .collect();
                                let avg_duration_str = if !symbol_durations.is_empty() {
                                    let avg_seconds = symbol_durations.iter().sum::<i64>() / symbol_durations.len() as i64;
                                    let duration = chrono::Duration::seconds(avg_seconds);
                                    let hours = duration.num_hours();
                                    let minutes = duration.num_minutes() % 60;
                                    let seconds = duration.num_seconds() % 60;
                                    if hours > 0 { format!("{}h {}m {}s", hours, minutes, seconds) }
                                    else if minutes > 0 { format!("{}m {}s", minutes, seconds) }
                                    else { format!("{}s", seconds) }
                                } else { "N/A".to_string() };

                                let symbol_trades_owned: Vec<TaskTrade> = symbol_trades.iter().map(|&t| t.clone()).collect();
                                
                                let risk_reward = if sl_pips.abs() > 1e-9 { tp_pips / sl_pips } else { f64::NAN }; 

                                all_optimization_results.push(OptimizationResult {
                                    symbol: symbol.clone(),
                                    timeframe: timeframe.clone(),
                                    sl_pips,
                                    tp_pips,
                                    total_pips: round_f64(symbol_total_pips, 1),
                                    total_trades: symbol_total_trades,
                                    win_rate: round_f64(symbol_win_rate, 2),
                                    profit_factor: symbol_profit_factor.map(|pf| round_f64(pf, 2)),
                                    winning_trades: symbol_winning_trades,
                                    losing_trades: symbol_total_trades - symbol_winning_trades,
                                    avg_trade_duration: avg_duration_str,
                                    risk_reward_ratio: round_f64(risk_reward, 2),
                                    sharpe_ratio: calculate_sharpe_ratio(&symbol_trades_owned).map(|sr| round_f64(sr, 4)),
                                });
                            }
                        }
                    }
                }
                Err(e) => warn!("Backtest failed for SL={}, TP={}: {}", sl_pips, tp_pips, e),
            }
            tested_count += 1;
        }
    }

    if all_optimization_results.is_empty() {
        return Err(ErrorInternalServerError("No successful backtests completed or no trades found for any combination."));
    }

    let mut sorted_for_overall = all_optimization_results.clone();
    sorted_for_overall.sort_by(|a, b| {
        match b.total_pips.partial_cmp(&a.total_pips).unwrap_or(std::cmp::Ordering::Equal) {
            std::cmp::Ordering::Equal => {
                match a.symbol.cmp(&b.symbol) {
                    std::cmp::Ordering::Equal => a.timeframe.cmp(&b.timeframe),
                    other => other,
                }
            }
            other => other,
        }
    });
    let best_overall_result = sorted_for_overall.first().cloned()
        .ok_or_else(|| ErrorInternalServerError("Failed to determine best result"))?;
    let worst_overall_result = sorted_for_overall.last().cloned()
        .ok_or_else(|| ErrorInternalServerError("Failed to determine worst result"))?;

    let mut grouped_map: HashMap<String, HashMap<String, Vec<OptimizationResult>>> = HashMap::new();
    for res in all_optimization_results.iter() {
        grouped_map
            .entry(res.timeframe.clone())
            .or_default()
            .entry(res.symbol.clone())
            .or_default()
            .push(res.clone());
    }

    let mut final_grouped_results: Vec<TimeframePerformance> = Vec::new();
    let mut timeframes_sorted: Vec<String> = grouped_map.keys().cloned().collect();
    timeframes_sorted.sort_unstable(); 

    for tf_key in timeframes_sorted {
        if let Some(symbols_map_for_tf) = grouped_map.get_mut(&tf_key) {
            let mut symbol_performances: Vec<SymbolPerformance> = Vec::new();
            let mut symbol_keys_sorted: Vec<String> = symbols_map_for_tf.keys().cloned().collect();
            symbol_keys_sorted.sort_unstable(); 

            for sym_key in symbol_keys_sorted {
                if let Some(combinations) = symbols_map_for_tf.get_mut(&sym_key) {
                    combinations.sort_by(|a, b| b.total_pips.partial_cmp(&a.total_pips).unwrap_or(std::cmp::Ordering::Equal));
                    symbol_performances.push(SymbolPerformance {
                        symbol: sym_key.clone(), 
                        combinations: combinations.clone(),
                    });
                }
            }
            final_grouped_results.push(TimeframePerformance {
                timeframe: tf_key.clone(),
                symbols: symbol_performances,
            });
        }
    }
    
    let mut best_combinations_list: Vec<BestCombinationEntry> = all_optimization_results.iter().map(|res| {
        BestCombinationEntry {
            symbol: res.symbol.clone(), // Use raw symbol
            sl_pips: res.sl_pips,
            tp_pips: res.tp_pips,
            total_pips: res.total_pips,
            total_trades: res.total_trades, // Added
            // Removed: avg_trade_duration: res.avg_trade_duration.clone(),
        }
    }).collect();

    best_combinations_list.sort_by(|a, b| b.total_pips.partial_cmp(&a.total_pips).unwrap_or(std::cmp::Ordering::Equal));

    let response = OptimizationResponse {
        request_summary: RequestSummary {
            start_time: request_params.start_time,
            end_time: request_params.end_time,
            symbols: request_params.symbols,
            timeframes: request_params.pattern_timeframes,
            sl_range: format!("{}-{} step {}", request_params.sl_range.min, request_params.sl_range.max, request_params.sl_range.step),
            tp_range: format!("{}-{} step {}", request_params.tp_range.min, request_params.tp_range.max, request_params.tp_range.step),
            total_combinations: tested_count, 
        },
        total_combinations_tested: tested_count,
        grouped_results: final_grouped_results,
        best_combinations: best_combinations_list,
        best_result: best_overall_result.clone(), 
        worst_result: worst_overall_result.clone(), 
    };

    info!("Optimization complete. Best overall: Symbol={}, TF={}, SL={}, TP={}, Pips={}",
          best_overall_result.symbol, best_overall_result.timeframe,
          best_overall_result.sl_pips, best_overall_result.tp_pips, best_overall_result.total_pips);

    Ok(HttpResponse::Ok().json(response))
}

fn generate_range(min: f64, max: f64, step: f64) -> Vec<f64> {
    if step <= 1e-9 || min > max { return Vec::new(); } 
    let mut values = Vec::new();
    let mut current = min;
    while current <= max + (step * 0.0001) { 
        values.push(round_to_2_dp(current));
        current += step;
        if values.len() > 1_000_000 { 
             warn!("generate_range exceeded 1,000,000 values, breaking. Check parameters min={}, max={}, step={}", min, max, step);
             break;
        }
    }
    values
}

fn round_to_2_dp(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn calculate_sharpe_ratio(trades: &[TaskTrade]) -> Option<f64> {
    if trades.len() < 2 { return None; }
    let returns: Vec<f64> = trades.iter().map(|t| t.pnl_pips).collect();
    let mean_return = returns.iter().sum::<f64>() / returns.len() as f64;
    if returns.len() <= 1 { return None; } 
    let variance = returns.iter().map(|&r| (r - mean_return).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
    let std_dev = variance.sqrt();
    if std_dev.abs() < 1e-9 { None } else { Some(mean_return / std_dev) } 
}

// --- Internal backtest structures and functions ---
#[derive(Debug)]
struct MultiBacktestParams {
    start_time: String,
    end_time: String,
    symbols: Vec<String>,
    pattern_timeframes: Vec<String>,
    stop_loss_pips: f64,
    take_profit_pips: f64,
    lot_size: Option<f64>,
    allowed_trade_days: Option<Vec<String>>,
    trade_end_hour_utc: Option<u32>,
}

#[derive(Debug)]
struct MultiBacktestResult {
    overall_summary: OverallSummary,
    all_trades: Vec<TaskTrade>,
}

#[derive(Debug)]
struct OverallSummary {
    total_trades: usize,
    total_pnl_pips: f64,
    overall_win_rate_percent: f64,
    overall_profit_factor: Option<f64>,
    average_trade_duration_str: String,
    overall_winning_trades: usize,
    overall_losing_trades: usize,
}

#[derive(Debug, Clone)] 
struct TaskTrade {
    symbol: String,
    timeframe: String,
    direction: String,
    entry_time: DateTime<Utc>,
    entry_price: f64,
    exit_time: Option<DateTime<Utc>>,
    exit_price: Option<f64>,
    pnl_pips: f64,
    exit_reason: Option<String>,
    entry_day_of_week: String, 
    entry_hour_of_day: u32,    
}

#[derive(Debug, Default)]
struct TradeTaskSummary {
    total_trades: usize,
    winning_trades: usize,
    losing_trades: usize,
    total_pnl_pips: f64,
    win_rate: f64,
    profit_factor: f64, 
}

async fn run_multi_backtest_internal(
    req: MultiBacktestParams
) -> Result<MultiBacktestResult, Box<dyn std::error::Error + Send + Sync>> {
    let parsed_allowed_days: Option<Vec<Weekday>> = parse_allowed_days(req.allowed_trade_days.clone());
    let parsed_trade_end_hour_utc: Option<u32> = req.trade_end_hour_utc;

    let host = std::env::var("INFLUXDB_HOST").unwrap_or_else(|_| "http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG")?;
    let token = std::env::var("INFLUXDB_TOKEN")?;
    let bucket = std::env::var("INFLUXDB_BUCKET")?;

    let pattern_timeframes = req.pattern_timeframes.clone();
    let _execution_timeframe = "1m"; 
    let lot_size = req.lot_size.unwrap_or(0.01);

    let mut tasks = Vec::new();

    for symbol in req.symbols.iter() {
        for pattern_tf_str in pattern_timeframes.iter() {
            let task_symbol = symbol.clone();
            let task_pattern_tf = pattern_tf_str.clone(); 
            let task_start_time = req.start_time.clone();
            let task_end_time = req.end_time.clone();
            let task_sl_pips = req.stop_loss_pips;
            let task_tp_pips = req.take_profit_pips;
            let task_lot_size = lot_size;
            let task_allowed_days = parsed_allowed_days.clone();
            let task_trade_end_hour_utc = parsed_trade_end_hour_utc;
            let host_clone = host.clone();
            let org_clone = org.clone();
            let token_clone = token.clone();
            let bucket_clone = bucket.clone();
            
            tasks.push(tokio::spawn(async move {
                let pattern_candles = match load_backtest_candles(&host_clone, &org_clone, &token_clone, &bucket_clone, &task_symbol, &task_pattern_tf, &task_start_time, &task_end_time).await {
                    Ok(candles) => candles,
                    Err(e) => {
                        warn!("Failed to load pattern candles for {}/{}: {}", task_symbol, task_pattern_tf, e);
                        return (task_symbol, task_pattern_tf, Vec::new(), TradeTaskSummary::default());
                    }
                };
                
                if pattern_candles.is_empty() {
                    return (task_symbol, task_pattern_tf, Vec::new(), TradeTaskSummary::default());
                }

                let execution_candles = match load_backtest_candles(&host_clone, &org_clone, &token_clone, &bucket_clone, &task_symbol, "1m", &task_start_time, &task_end_time).await {
                    Ok(candles) => candles,
                     Err(e) => {
                        warn!("Failed to load execution candles for {}/{}: {}", task_symbol, "1m", e);
                        return (task_symbol, task_pattern_tf, Vec::new(), TradeTaskSummary::default());
                    }
                };

                let recognizer = crate::patterns::FiftyPercentBeforeBigBarRecognizer::default();
                let detected_value_json = recognizer.detect(&pattern_candles);

                let current_trade_config = crate::trades::TradeConfig {
                    enabled: true,
                    lot_size: task_lot_size,
                    default_stop_loss_pips: task_sl_pips,
                    default_take_profit_pips: task_tp_pips,
                    risk_percent: 1.0,
                    max_trades_per_pattern: 1,
                    ..Default::default()
                };

                let mut trade_executor = crate::trading::TradeExecutor::new(
                    current_trade_config,
                    &task_symbol,
                    task_allowed_days,
                    task_trade_end_hour_utc
                );
                
                if !execution_candles.is_empty() {
                    trade_executor.set_minute_candles(execution_candles);
                }

                let executed_trades = trade_executor.execute_trades_for_pattern(
                    "fifty_percent_before_big_bar",
                    &detected_value_json,
                    &pattern_candles
                );

                let mut current_task_trades = Vec::new();
                let mut winning_trades_count = 0;
                let mut total_pnl_pips_raw = 0.0;
                let mut total_gross_profit_pips_raw = 0.0;
                let mut total_gross_loss_pips_raw = 0.0;

                for trade in executed_trades.iter() {
                    let entry_dt = match DateTime::parse_from_rfc3339(&trade.entry_time) {
                        Ok(dt) => dt.with_timezone(&Utc),
                        Err(_) => continue,
                    };
                    
                    let exit_dt = trade.exit_time.as_ref()
                        .and_then(|et_str| DateTime::parse_from_rfc3339(et_str).ok())
                        .map(|dt| dt.with_timezone(&Utc));

                    let mut pnl_raw = trade.profit_loss_pips.unwrap_or(0.0);

                    let main_pair_part = task_symbol.split('_').next().unwrap_or(&task_symbol);
                    let is_jpy_pair = main_pair_part.to_uppercase().ends_with("JPY");
                    if is_jpy_pair {
                        pnl_raw *= 0.01;
                    }

                    let pnl_rounded = round_f64(pnl_raw, 1);
                    total_pnl_pips_raw += pnl_raw;
                    
                    if pnl_raw > 1e-9 {
                        winning_trades_count += 1;
                        total_gross_profit_pips_raw += pnl_raw;
                    } else if pnl_raw < -1e-9 {
                        total_gross_loss_pips_raw += pnl_raw.abs();
                    }

                    current_task_trades.push(TaskTrade {
                        symbol: task_symbol.clone(),
                        timeframe: task_pattern_tf.clone(),
                        direction: format!("{:?}", trade.direction), 
                        entry_time: entry_dt,
                        entry_price: trade.entry_price,
                        exit_time: exit_dt,
                        exit_price: trade.exit_price,
                        pnl_pips: pnl_rounded,
                        exit_reason: trade.exit_reason.clone(),
                        entry_day_of_week: entry_dt.weekday().to_string(),
                        entry_hour_of_day: entry_dt.hour(),
                    });
                }

                let total_trades_count = executed_trades.len(); 
                let win_rate_val = if total_trades_count > 0 { 
                    (winning_trades_count as f64 / total_trades_count as f64) * 100.0 
                } else { 0.0 };
                
                let profit_factor_val = if total_gross_loss_pips_raw > 1e-9 {
                    total_gross_profit_pips_raw / total_gross_loss_pips_raw
                } else if total_gross_profit_pips_raw > 1e-9 {
                    f64::INFINITY 
                } else { 0.0 };

                let summary = TradeTaskSummary {
                    total_trades: total_trades_count,
                    winning_trades: winning_trades_count,
                    losing_trades: total_trades_count - winning_trades_count,
                    total_pnl_pips: round_f64(total_pnl_pips_raw, 1),
                    win_rate: win_rate_val,
                    profit_factor: profit_factor_val,
                };
                (task_symbol, task_pattern_tf, current_task_trades, summary)
            }));
        }
    }

    let task_results = join_all(tasks).await;
    let mut all_trades = Vec::new();
    let mut overall_total_trades = 0;
    let mut overall_total_pnl_pips = 0.0;
    let mut overall_winning_trades = 0;
    let mut overall_gross_profit_pips = 0.0;
    let mut overall_gross_loss_pips = 0.0; 
    let mut total_duration_seconds = 0i64;

    for result in task_results {
        match result {
            Ok((_symbol, _pattern_tf, trades, summary)) => {
                all_trades.extend(trades); 
                overall_total_trades += summary.total_trades;
                overall_total_pnl_pips += summary.total_pnl_pips; 
                overall_winning_trades += summary.winning_trades;
            }
            Err(e) => {
                warn!("A backtest task failed: {:?}", e); 
                continue;
            }
        }
    }

    for trade in &all_trades { 
        if let Some(exit_time) = trade.exit_time {
            let duration = exit_time.signed_duration_since(trade.entry_time);
            total_duration_seconds += duration.num_seconds();
        }
        if trade.pnl_pips > 1e-9 { 
            overall_gross_profit_pips += trade.pnl_pips;
        } else if trade.pnl_pips < -1e-9 { 
            overall_gross_loss_pips += trade.pnl_pips.abs();
        }
    }
    
    let avg_duration_str = if overall_total_trades > 0 && total_duration_seconds > 0 {
        let avg_seconds = total_duration_seconds / overall_total_trades as i64;
        let duration = chrono::Duration::seconds(avg_seconds);
        let hours = duration.num_hours();
        let minutes = duration.num_minutes() % 60;
        let seconds = duration.num_seconds() % 60;
        if hours > 0 { format!("{}h {}m {}s", hours, minutes, seconds) }
        else if minutes > 0 { format!("{}m {}s", minutes, seconds) }
        else { format!("{}s", seconds) }
    } else { "N/A".to_string() };

    let overall_win_rate = if overall_total_trades > 0 {
        (overall_winning_trades as f64 / overall_total_trades as f64) * 100.0
    } else { 0.0 };

    let overall_profit_factor_val = if overall_gross_loss_pips > 1e-9 { 
        Some(overall_gross_profit_pips / overall_gross_loss_pips)
    } else if overall_gross_profit_pips > 1e-9 { 
        None 
    } else { Some(0.0) };

    Ok(MultiBacktestResult {
        overall_summary: OverallSummary {
            total_trades: overall_total_trades,
            total_pnl_pips: round_f64(overall_total_pnl_pips, 1), 
            overall_win_rate_percent: round_f64(overall_win_rate, 2),
            overall_profit_factor: overall_profit_factor_val.map(|pf| round_f64(pf,2)),
            average_trade_duration_str: avg_duration_str,
            overall_winning_trades,
            overall_losing_trades: overall_total_trades - overall_winning_trades,
        },
        all_trades,
    })
}

async fn load_backtest_candles(
    host: &str,
    org: &str,
    token: &str,
    bucket: &str,
    symbol: &str,
    timeframe: &str,
    start_time: &str,
    end_time: &str,
) -> Result<Vec<crate::detect::CandleData>, Box<dyn std::error::Error + Send + Sync>> {
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

    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&json!({"query": flux_query, "type": "flux"}))
        .send()
        .await?;

    let status = response.status();

    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_else(|e| format!("Unknown error reading error response body: {}", e));
        return Err(format!("InfluxDB query failed: {} - {}", status, err_text).into());
    }

    let response_text = response.text().await?;
    
    let mut candles = Vec::new();
    if !response_text.trim().is_empty() && !response_text.starts_with("Error:") { 
        let cursor = Cursor::new(response_text.as_bytes());
        let mut rdr = ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .comment(Some(b'#'))
            .from_reader(cursor);
        
        let headers = match rdr.headers() {
            Ok(h) => h.clone(),
            Err(e) => return Err(format!("Failed to read CSV headers from InfluxDB response: {}", e).into()),
        };
        let get_idx = |name: &str| headers.iter().position(|h| h == name);

        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) = (
            get_idx("_time"), get_idx("open"), get_idx("high"), 
            get_idx("low"), get_idx("close"), get_idx("volume")
        ) {
            for result in rdr.records() {
                match result {
                    Ok(record) => {
                        let parse_f64 = |idx: usize| record.get(idx).and_then(|v| v.trim().parse::<f64>().ok());
                        let parse_u32 = |idx: usize| record.get(idx).and_then(|v| v.trim().parse::<u32>().ok());
                        let time_str = record.get(t_idx).unwrap_or("").trim();
                        
                        if !time_str.is_empty() {
                            candles.push(crate::detect::CandleData {
                                time: time_str.to_string(),
                                open: parse_f64(o_idx).unwrap_or(0.0),
                                high: parse_f64(h_idx).unwrap_or(0.0),
                                low: parse_f64(l_idx).unwrap_or(0.0),
                                close: parse_f64(c_idx).unwrap_or(0.0),
                                volume: parse_u32(v_idx).unwrap_or(0),
                            });
                        }
                    }
                    Err(e) => {
                        warn!("Skipping malformed CSV record from InfluxDB: {}", e);
                    }
                }
            }
        } else {
             warn!("Missing one or more required columns (_time, open, high, low, close, volume) in InfluxDB response for {}/{}. Headers: {:?}", symbol, timeframe, headers);
        }
    } else if response_text.starts_with("Error:") { 
        warn!("InfluxDB returned an error in response body even with success status: {}", response_text);
    }
    
    Ok(candles)
}

fn parse_allowed_days(day_strings: Option<Vec<String>>) -> Option<Vec<Weekday>> {
    day_strings.map(|days| {
        days.iter()
            .filter_map(|day_str| match day_str.to_lowercase().as_str() {
                "mon" | "monday" => Some(Weekday::Mon),
                "tue" | "tuesday" => Some(Weekday::Tue),
                "wed" | "wednesday" => Some(Weekday::Wed),
                "thu" | "thursday" => Some(Weekday::Thu),
                "fri" | "friday" => Some(Weekday::Fri),
                "sat" | "saturday" => Some(Weekday::Sat),
                "sun" | "sunday" => Some(Weekday::Sun),
                _ => {
                    warn!("Unrecognized day string: {}", day_str);
                    None
                },
            })
            .collect()
    })
}

fn round_f64(value: f64, decimals: u32) -> f64 {
    if value.is_infinite() || value.is_nan() { return value; }
    let multiplier = 10f64.powi(decimals as i32);
    (value * multiplier).round() / multiplier
}