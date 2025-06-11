// src/optimize_handler.rs

use crate::zones::patterns::PatternRecognizer;
use actix_web::{error::ErrorInternalServerError, web, HttpResponse, Responder}; // Removed ErrorBadRequest
use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use csv::ReaderBuilder;
use futures::future::join_all;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::io::Cursor;

// --- Structs ---

#[derive(Deserialize, Debug, Clone)]
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

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SlTpRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BestCombinationEntry {
    pub symbol: String,
    #[serde(rename = "SL")]
    pub sl_pips: f64,
    #[serde(rename = "TP")]
    pub tp_pips: f64,
    #[serde(rename = "TotalPips")]
    pub total_pips: f64,
    pub total_trades: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SymbolPerformance {
    pub symbol: String,
    pub combinations: Vec<OptimizationResult>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TimeframePerformance {
    pub timeframe: String,
    pub symbols: Vec<SymbolPerformance>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationResponse {
    pub request_summary: RequestSummary,
    pub total_combinations_tested: usize,
    pub grouped_results: Vec<TimeframePerformance>,
    pub best_combinations: Vec<BestCombinationEntry>,
    pub best_result: OptimizationResult,
    pub worst_result: OptimizationResult,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummary {
    pub start_time: String,
    pub end_time: String,
    pub symbols: Vec<String>,
    pub timeframes: Vec<String>, // Corrected field name here
    pub sl_range: String,
    pub tp_range: String,
    pub total_combinations: usize,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CoreOptimizationOutput {
    pub request_summary: RequestSummary,
    pub total_combinations_tested: usize,
    pub grouped_results: Vec<TimeframePerformance>,
    pub best_combinations: Vec<BestCombinationEntry>,
    pub best_result: Option<OptimizationResult>,
    pub worst_result: Option<OptimizationResult>,
}

#[derive(Debug, Clone)]
pub struct MultiBacktestParams {
    pub start_time: String,
    pub end_time: String,
    pub symbols: Vec<String>,
    pub pattern_timeframes: Vec<String>,
    pub stop_loss_pips: f64,
    pub take_profit_pips: f64,
    pub lot_size: Option<f64>,
    pub allowed_trade_days: Option<Vec<String>>,
    pub trade_end_hour_utc: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct MultiBacktestResult {
    pub overall_summary: OverallSummary,
    pub all_trades: Vec<TaskTrade>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverallSummary {
    pub total_trades: usize,
    pub total_pnl_pips: f64,
    pub overall_win_rate_percent: f64,
    pub overall_profit_factor: Option<f64>,
    pub average_trade_duration_str: String,
    pub overall_winning_trades: usize,
    pub overall_losing_trades: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskTrade {
    pub symbol: String,
    pub timeframe: String,
    pub direction: String,
    pub entry_time: DateTime<Utc>,
    pub entry_price: f64,
    pub exit_time: Option<DateTime<Utc>>,
    pub exit_price: Option<f64>,
    pub pnl_pips: f64,
    pub exit_reason: Option<String>,
    pub entry_day_of_week: String,
    pub entry_hour_of_day: u32,
}

#[derive(Debug, Default, Clone)]
struct TradeTaskSummary {
    total_trades: usize,
    winning_trades: usize,
    losing_trades: usize,
    total_pnl_pips: f64,
    win_rate: f64,
    profit_factor: f64,
}

// --- Portfolio Meta-Optimization Structs ---
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioOptimizeParams {
    pub start_time: String,
    pub end_time: String,
    pub instruments: Vec<InstrumentToOptimize>,
    pub lot_size: f64,
    pub allowed_trade_days: Vec<String>,
    pub optimization_sl_range: SlTpRange,
    pub optimization_tp_range: SlTpRange,
    pub optimization_max_combinations: Option<usize>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentToOptimize {
    pub symbol: String,
    pub timeframe: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioOptimizationResultEntry {
    pub symbol: String,
    pub timeframe: String,
    pub optimal_sl_pips: f64,
    pub optimal_tp_pips: f64,
    pub backtest_pnl_pips: f64,
    pub backtest_total_trades: usize,
    pub backtest_win_rate: f64,
    pub backtest_profit_factor: Option<f64>,
    pub winning_trades: usize, // Added
    pub losing_trades: usize,  // Added
    pub trades: Vec<TaskTrade>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioOptimizedBacktestResponse {
    pub request_params: PortfolioOptimizeParamsSummary,
    pub overall_portfolio_pnl_pips: f64,
    pub overall_portfolio_total_trades: usize,
    pub overall_portfolio_win_rate: f64,
    pub overall_portfolio_profit_factor: Option<f64>,
    pub overall_portfolio_winning_trades: usize, // Added
    pub overall_portfolio_losing_trades: usize,  // Added
    pub individual_instrument_results: Vec<PortfolioOptimizationResultEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioOptimizeParamsSummary {
    pub start_time: String,
    pub end_time: String,
    pub instruments_queried: usize,
    pub lot_size: f64,
    pub optimization_sl_range: String,
    pub optimization_tp_range: String,
}

// --- Helper Functions ---
pub fn generate_range(min: f64, max: f64, step: f64) -> Vec<f64> {
    if step <= 1e-9 || min > max {
        return Vec::new();
    }
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

pub fn round_to_2_dp(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub fn calculate_sharpe_ratio(trades: &[TaskTrade]) -> Option<f64> {
    if trades.len() < 2 {
        return None;
    }
    let returns: Vec<f64> = trades.iter().map(|t| t.pnl_pips).collect();
    let mean_return = returns.iter().sum::<f64>() / returns.len() as f64;
    if returns.len() <= 1 {
        return None;
    }
    let variance = returns
        .iter()
        .map(|&r| (r - mean_return).powi(2))
        .sum::<f64>()
        / (returns.len() - 1) as f64;
    let std_dev = variance.sqrt();
    if std_dev.abs() < 1e-9 {
        None
    } else {
        Some(mean_return / std_dev)
    }
}

pub fn round_f64(value: f64, decimals: u32) -> f64 {
    if value.is_infinite() || value.is_nan() {
        return value;
    }
    let multiplier = 10f64.powi(decimals as i32);
    (value * multiplier).round() / multiplier
}

pub fn parse_allowed_days(day_strings: Option<Vec<String>>) -> Option<Vec<Weekday>> {
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
                }
            })
            .collect()
    })
}

pub async fn load_backtest_candles(
    host: &str,
    org: &str,
    token: &str,
    bucket: &str,
    symbol: &str,
    timeframe: &str,
    start_time: &str,
    end_time: &str,
) -> Result<Vec<crate::api::detect::CandleData>, Box<dyn std::error::Error + Send + Sync>> {
    let flux_query = format!(
        r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r["_measurement"] == "trendbar") |> filter(fn: (r) => r["symbol"] == "{}") |> filter(fn: (r) => r["timeframe"] == "{}") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
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
        let err_text = response
            .text()
            .await
            .unwrap_or_else(|e| format!("Unknown error reading error response body: {}", e));
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
            Err(e) => return Err(format!("Failed to read CSV headers from InfluxDB: {}", e).into()),
        };
        let get_idx = |name: &str| headers.iter().position(|h| h == name);
        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) = (
            get_idx("_time"),
            get_idx("open"),
            get_idx("high"),
            get_idx("low"),
            get_idx("close"),
            get_idx("volume"),
        ) {
            for result in rdr.records() {
                match result {
                    Ok(record) => {
                        let parse_f64 =
                            |idx: usize| record.get(idx).and_then(|v| v.trim().parse::<f64>().ok());
                        let parse_u32 =
                            |idx: usize| record.get(idx).and_then(|v| v.trim().parse::<u32>().ok());
                        let time_str = record.get(t_idx).unwrap_or("").trim();
                        if !time_str.is_empty() {
                            candles.push(crate::api::detect::CandleData {
                                time: time_str.to_string(),
                                open: parse_f64(o_idx).unwrap_or(0.0),
                                high: parse_f64(h_idx).unwrap_or(0.0),
                                low: parse_f64(l_idx).unwrap_or(0.0),
                                close: parse_f64(c_idx).unwrap_or(0.0),
                                volume: parse_u32(v_idx).unwrap_or(0),
                            });
                        }
                    }
                    Err(e) => warn!("Skipping malformed CSV record from InfluxDB: {}", e),
                }
            }
        } else {
            warn!(
                "Missing required columns in InfluxDB for {}/{}. Headers: {:?}",
                symbol, timeframe, headers
            );
        }
    } else if response_text.starts_with("Error:") {
        warn!("InfluxDB returned error in body: {}", response_text);
    }
    Ok(candles)
}

pub async fn run_multi_backtest_internal(
    req: MultiBacktestParams,
) -> Result<MultiBacktestResult, Box<dyn std::error::Error + Send + Sync>> {
    let parsed_allowed_days: Option<Vec<Weekday>> =
        parse_allowed_days(req.allowed_trade_days.clone());
    let parsed_trade_end_hour_utc: Option<u32> = req.trade_end_hour_utc;

    let host =
        std::env::var("INFLUXDB_HOST").unwrap_or_else(|_| "http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG")?;
    let token = std::env::var("INFLUXDB_TOKEN")?;
    let bucket = std::env::var("INFLUXDB_BUCKET")?;

    let pattern_timeframes = req.pattern_timeframes.clone();
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
                let pattern_candles = match load_backtest_candles(
                    &host_clone,
                    &org_clone,
                    &token_clone,
                    &bucket_clone,
                    &task_symbol,
                    &task_pattern_tf,
                    &task_start_time,
                    &task_end_time,
                )
                .await
                {
                    Ok(candles) => candles,
                    Err(e) => {
                        warn!(
                            "Failed to load pattern candles for {}/{}: {}",
                            task_symbol, task_pattern_tf, e
                        );
                        return (
                            task_symbol,
                            task_pattern_tf,
                            Vec::new(),
                            TradeTaskSummary::default(),
                        );
                    }
                };
                if pattern_candles.is_empty() {
                    return (
                        task_symbol,
                        task_pattern_tf,
                        Vec::new(),
                        TradeTaskSummary::default(),
                    );
                }
                let execution_candles = match load_backtest_candles(
                    &host_clone,
                    &org_clone,
                    &token_clone,
                    &bucket_clone,
                    &task_symbol,
                    "1m",
                    &task_start_time,
                    &task_end_time,
                )
                .await
                {
                    Ok(candles) => candles,
                    Err(e) => {
                        warn!(
                            "Failed to load execution candles for {}/{}: {}",
                            task_symbol, "1m", e
                        );
                        return (
                            task_symbol,
                            task_pattern_tf,
                            Vec::new(),
                            TradeTaskSummary::default(),
                        );
                    }
                };
                let recognizer = crate::zones::patterns::FiftyPercentBeforeBigBarRecognizer::default();
                let detected_value_json = recognizer.detect(&pattern_candles);
                let current_trade_config = crate::trading::trades::TradeConfig {
                    enabled: true,
                    lot_size: task_lot_size,
                    default_stop_loss_pips: task_sl_pips,
                    default_take_profit_pips: task_tp_pips,
                    risk_percent: 1.0,
                    max_trades_per_zone: 1,
                    ..Default::default()
                };
                let mut trade_executor = crate::trading::trading::TradeExecutor::new(
                    current_trade_config,
                    &task_symbol,
                    task_allowed_days,
                    task_trade_end_hour_utc,
                );
                if !execution_candles.is_empty() {
                    trade_executor.set_minute_candles(execution_candles);
                }
                let executed_trades = trade_executor.execute_trades_for_pattern(
                    "fifty_percent_before_big_bar",
                    &detected_value_json,
                    &pattern_candles,
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
                    let exit_dt = trade
                        .exit_time
                        .as_ref()
                        .and_then(|et_str| DateTime::parse_from_rfc3339(et_str).ok())
                        .map(|dt| dt.with_timezone(&Utc));
                    let mut pnl_raw = trade.profit_loss_pips.unwrap_or(0.0);
                    let main_pair_part = task_symbol.split('_').next().unwrap_or(&task_symbol);
                    if main_pair_part.to_uppercase().ends_with("JPY") {
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
                } else {
                    0.0
                };
                let profit_factor_val = if total_gross_loss_pips_raw > 1e-9 {
                    total_gross_profit_pips_raw / total_gross_loss_pips_raw
                } else if total_gross_profit_pips_raw > 1e-9 {
                    f64::INFINITY
                } else {
                    0.0
                };
                (
                    task_symbol,
                    task_pattern_tf,
                    current_task_trades,
                    TradeTaskSummary {
                        total_trades: total_trades_count,
                        winning_trades: winning_trades_count,
                        losing_trades: total_trades_count - winning_trades_count,
                        total_pnl_pips: round_f64(total_pnl_pips_raw, 1),
                        win_rate: win_rate_val,
                        profit_factor: profit_factor_val,
                    },
                )
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
            total_duration_seconds += exit_time
                .signed_duration_since(trade.entry_time)
                .num_seconds();
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
        let h = duration.num_hours();
        let m = duration.num_minutes() % 60;
        let s = duration.num_seconds() % 60;
        if h > 0 {
            format!("{}h {}m {}s", h, m, s)
        } else if m > 0 {
            format!("{}m {}s", m, s)
        } else {
            format!("{}s", s)
        }
    } else {
        "N/A".to_string()
    };
    let overall_win_rate = if overall_total_trades > 0 {
        (overall_winning_trades as f64 / overall_total_trades as f64) * 100.0
    } else {
        0.0
    };
    let overall_profit_factor_val = if overall_gross_loss_pips > 1e-9 {
        Some(overall_gross_profit_pips / overall_gross_loss_pips)
    } else if overall_gross_profit_pips > 1e-9 {
        None
    } else {
        Some(0.0)
    };
    Ok(MultiBacktestResult {
        overall_summary: OverallSummary {
            total_trades: overall_total_trades,
            total_pnl_pips: round_f64(overall_total_pnl_pips, 1),
            overall_win_rate_percent: round_f64(overall_win_rate, 2),
            overall_profit_factor: overall_profit_factor_val.map(|pf| round_f64(pf, 2)),
            average_trade_duration_str: avg_duration_str,
            overall_winning_trades,
            overall_losing_trades: overall_total_trades - overall_winning_trades,
        },
        all_trades,
    })
}

// --- CORE OPTIMIZATION LOGIC (Refactored) ---
pub async fn run_optimization_logic(
    // This function now contains the core logic
    params: OptimizeParams,
) -> Result<CoreOptimizationOutput, String> {
    info!("Core Logic: Starting parameter optimization: {:?}", params);

    // ... (parameter validation and SL/TP value generation as before)
    if params.sl_range.min >= params.sl_range.max
        || params.tp_range.min >= params.tp_range.max
        || params.sl_range.step <= 1e-9
        || params.tp_range.step <= 1e-9
    {
        return Err("Invalid SL/TP ranges or step values".to_string());
    }
    let sl_values = generate_range(
        params.sl_range.min,
        params.sl_range.max,
        params.sl_range.step,
    );
    let tp_values = generate_range(
        params.tp_range.min,
        params.tp_range.max,
        params.tp_range.step,
    );
    if sl_values.is_empty() || tp_values.is_empty() {
        return Err("SL/TP ranges generated no values".to_string());
    }

    let total_combinations_possible = sl_values.len() * tp_values.len();
    let combinations_to_test = params
        .max_combinations
        .map(|max| std::cmp::min(max, total_combinations_possible))
        .unwrap_or(total_combinations_possible);

    if combinations_to_test < total_combinations_possible {
        warn!(
            "Limiting combinations from {} to {} for {:?}/{:?}",
            total_combinations_possible,
            combinations_to_test,
            params.symbols,
            params.pattern_timeframes
        );
    }

    let mut all_instrument_results: Vec<OptimizationResult> = Vec::new();
    let mut tested_count = 0;

    'outer_loop: for &sl_pips in &sl_values {
        for &tp_pips in &tp_values {
            if tested_count >= combinations_to_test {
                break 'outer_loop;
            }

            let current_multi_backtest_params = MultiBacktestParams {
                start_time: params.start_time.clone(),
                end_time: params.end_time.clone(),
                symbols: params.symbols.clone(),
                pattern_timeframes: params.pattern_timeframes.clone(),
                stop_loss_pips: sl_pips,
                take_profit_pips: tp_pips,
                lot_size: Some(params.lot_size),
                allowed_trade_days: Some(params.allowed_trade_days.clone()),
                trade_end_hour_utc: None,
            };

            // Clone the values we need before the move
            let symbols_for_processing = current_multi_backtest_params.symbols.clone();
            let timeframes_for_processing =
                current_multi_backtest_params.pattern_timeframes.clone();

            match run_multi_backtest_internal(current_multi_backtest_params).await {
                Ok(backtest_result) => {
                    for symbol_to_process in symbols_for_processing {
                        for timeframe_to_process in timeframes_for_processing.iter() {
                            let instrument_trades: Vec<&TaskTrade> = backtest_result
                                .all_trades
                                .iter()
                                .filter(|t| {
                                    t.symbol.as_str() == symbol_to_process.as_str()
                                        && t.timeframe.as_str() == timeframe_to_process.as_str()
                                })
                                .collect();

                            if !instrument_trades.is_empty() {
                                let total_pips: f64 =
                                    instrument_trades.iter().map(|t| t.pnl_pips).sum();
                                let winning_trades = instrument_trades
                                    .iter()
                                    .filter(|t| t.pnl_pips > 0.0)
                                    .count();
                                let total_trades = instrument_trades.len();
                                let win_rate = if total_trades > 0 {
                                    (winning_trades as f64 / total_trades as f64) * 100.0
                                } else {
                                    0.0
                                };
                                let gross_profit: f64 = instrument_trades
                                    .iter()
                                    .filter(|t| t.pnl_pips > 0.0)
                                    .map(|t| t.pnl_pips)
                                    .sum();
                                let gross_loss: f64 = instrument_trades
                                    .iter()
                                    .filter(|t| t.pnl_pips < 0.0)
                                    .map(|t| t.pnl_pips.abs())
                                    .sum();
                                let profit_factor = if gross_loss > 1e-9 {
                                    Some(gross_profit / gross_loss)
                                } else if gross_profit > 1e-9 {
                                    None
                                } else {
                                    Some(0.0)
                                };
                                let durations: Vec<i64> = instrument_trades
                                    .iter()
                                    .filter_map(|t| {
                                        t.exit_time.map(|exit| {
                                            exit.signed_duration_since(t.entry_time).num_seconds()
                                        })
                                    })
                                    .collect();
                                let avg_duration_str = if !durations.is_empty() {
                                    let avg_seconds =
                                        durations.iter().sum::<i64>() / durations.len() as i64;
                                    let duration_obj = chrono::Duration::seconds(avg_seconds);
                                    let h = duration_obj.num_hours();
                                    let m = duration_obj.num_minutes() % 60;
                                    let s = duration_obj.num_seconds() % 60;
                                    if h > 0 {
                                        format!("{}h {}m {}s", h, m, s)
                                    } else if m > 0 {
                                        format!("{}m {}s", m, s)
                                    } else {
                                        format!("{}s", s)
                                    }
                                } else {
                                    "N/A".to_string()
                                };
                                let trades_owned: Vec<TaskTrade> =
                                    instrument_trades.iter().map(|&t| t.clone()).collect();
                                let risk_reward = if sl_pips.abs() > 1e-9 {
                                    tp_pips / sl_pips
                                } else {
                                    f64::NAN
                                };

                                all_instrument_results.push(OptimizationResult {
                                    symbol: symbol_to_process.clone(),
                                    timeframe: timeframe_to_process.clone(),
                                    sl_pips,
                                    tp_pips,
                                    total_pips: round_f64(total_pips, 1),
                                    total_trades,
                                    win_rate: round_f64(win_rate, 2),
                                    profit_factor: profit_factor.map(|pf| round_f64(pf, 2)),
                                    winning_trades,
                                    losing_trades: total_trades - winning_trades,
                                    avg_trade_duration: avg_duration_str,
                                    risk_reward_ratio: round_f64(risk_reward, 2),
                                    sharpe_ratio: calculate_sharpe_ratio(&trades_owned)
                                        .map(|sr| round_f64(sr, 4)),
                                });
                            }
                        }
                    }
                }
                Err(e) => warn!(
                    "Core Logic: Backtest error for SL={}, TP={}: {}",
                    sl_pips, tp_pips, e
                ),
            }
            tested_count += 1;
        }
    }

    if all_instrument_results.is_empty() {
        // Return early with an empty-ish CoreOptimizationOutput if no results
        // This allows the /optimize endpoint to still return a valid structure with no results
        warn!("No optimization results produced for params: {:?}", params);
        return Ok(CoreOptimizationOutput {
            request_summary: RequestSummary {
                start_time: params.start_time,
                end_time: params.end_time,
                symbols: params.symbols,
                timeframes: params.pattern_timeframes,
                sl_range: format!(
                    "{}-{}-{}",
                    params.sl_range.min, params.sl_range.max, params.sl_range.step
                ),
                tp_range: format!(
                    "{}-{}-{}",
                    params.tp_range.min, params.tp_range.max, params.tp_range.step
                ),
                total_combinations: tested_count,
            },
            total_combinations_tested: tested_count,
            grouped_results: Vec::new(),
            best_combinations: Vec::new(),
            best_result: None,
            worst_result: None,
        });
    }

    let mut sorted_for_overall = all_instrument_results.clone();
    sorted_for_overall.sort_by(|a, b| {
        b.total_pips
            .partial_cmp(&a.total_pips)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then_with(|| a.timeframe.cmp(&b.timeframe))
    });

    let best_res = sorted_for_overall.first().cloned();
    let worst_res = sorted_for_overall.last().cloned();

    let mut grouped_map: HashMap<String, HashMap<String, Vec<OptimizationResult>>> = HashMap::new();
    for res in &all_instrument_results {
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
                    combinations.sort_by(|a, b| {
                        b.total_pips
                            .partial_cmp(&a.total_pips)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
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

    let mut best_combinations_list: Vec<BestCombinationEntry> = all_instrument_results
        .iter()
        .map(|res| BestCombinationEntry {
            symbol: res.symbol.clone(),
            sl_pips: res.sl_pips,
            tp_pips: res.tp_pips,
            total_pips: res.total_pips,
            total_trades: res.total_trades,
        })
        .collect();
    best_combinations_list.sort_by(|a, b| {
        b.total_pips
            .partial_cmp(&a.total_pips)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(CoreOptimizationOutput {
        request_summary: RequestSummary {
            start_time: params.start_time,
            end_time: params.end_time,
            symbols: params.symbols,
            timeframes: params.pattern_timeframes,
            sl_range: format!(
                "{}-{}-{}",
                params.sl_range.min, params.sl_range.max, params.sl_range.step
            ),
            tp_range: format!(
                "{}-{}-{}",
                params.tp_range.min, params.tp_range.max, params.tp_range.step
            ),
            total_combinations: tested_count,
        },
        total_combinations_tested: tested_count,
        grouped_results: final_grouped_results,
        best_combinations: best_combinations_list,
        best_result: best_res,
        worst_result: worst_res,
    })
}
// --- Existing /optimize Endpoint Handler (Calls new core logic) ---
pub async fn run_parameter_optimization(
    params: web::Json<OptimizeParams>,
) -> Result<impl Responder, actix_web::Error> {
    info!("API Handler: /optimize received request: {:?}", params.0);
    match run_optimization_logic(params.into_inner()).await {
        Ok(core_output) => {
            let best_result = core_output.best_result.ok_or_else(|| {
                error!("Core optimization logic returned None for best_result.");
                ErrorInternalServerError(
                    "Optimization completed but failed to determine best result.",
                )
            })?;
            let worst_result = core_output.worst_result.ok_or_else(|| {
                error!("Core optimization logic returned None for worst_result.");
                ErrorInternalServerError(
                    "Optimization completed but failed to determine worst result.",
                )
            })?;
            let response = OptimizationResponse {
                request_summary: core_output.request_summary,
                total_combinations_tested: core_output.total_combinations_tested,
                grouped_results: core_output.grouped_results,
                best_combinations: core_output.best_combinations,
                best_result,
                worst_result,
            };
            Ok(HttpResponse::Ok().json(response))
        }
        Err(e) => {
            error!("Optimization process failed: {}", e);
            Err(ErrorInternalServerError(e))
        }
    }
}

// --- New Portfolio Meta-Optimized Backtest Handler ---
pub async fn run_portfolio_meta_optimized_backtest(
    params: web::Json<PortfolioOptimizeParams>,
) -> Result<impl Responder, actix_web::Error> {
    let portfolio_params = params.into_inner(); // These params define the REPORT period
    info!(
        "API Handler: /portfolio-meta-backtest for {} instruments. Report period: {} to {}",
        portfolio_params.instruments.len(),
        portfolio_params.start_time, // This is the start_time for the FINAL report
        portfolio_params.end_time    // This is the end_time for the FINAL report
    );

    let mut optimization_tasks = Vec::new();

    // --- Phase 1: Find optimal SL/TP for each instrument using its OPTIMIZATION period ---
    // Note: The `portfolio_params` for this endpoint define the *reporting* period.
    // The optimization itself should use potentially different, longer historical data.
    // For this example, we'll assume the optimization_sl_range etc. are for a separate,
    // longer historical period, and `portfolio_params.start_time` and `portfolio_params.end_time`
    // are for the *final report* using those optimized parameters.
    // If the optimization period is meant to be the same as the report period, that's fine too.

    for instrument in portfolio_params.instruments.iter() {
        // These OptimizeParams are for the *optimization phase* of each instrument.
        // They might use a different start/end time than the final report period.
        // For this example, let's assume the request 'portfolio_params' start_time/end_time
        // are for the optimization phase, and we'll need separate ones for the report phase,
        // OR that portfolio_params.start_time/end_time are used for BOTH.
        // Your JSON request suggests the latter, which means you optimize and test on the same data.
        // This is generally okay for finding "best performing on this data" but not true out-of-sample.
        // For a true "last week" report based on prior optimization, the optimization period would end *before* "last week".

        let single_instrument_opt_params = OptimizeParams {
            start_time: portfolio_params.start_time.clone(), // Using portfolio_params dates for optimization run
            end_time: portfolio_params.end_time.clone(),     // Using portfolio_params dates for optimization run
            symbols: vec![instrument.symbol.clone()],
            pattern_timeframes: vec![instrument.timeframe.clone()],
            lot_size: portfolio_params.lot_size,
            allowed_trade_days: portfolio_params.allowed_trade_days.clone(),
            sl_range: portfolio_params.optimization_sl_range.clone(),
            tp_range: portfolio_params.optimization_tp_range.clone(),
            max_combinations: portfolio_params.optimization_max_combinations,
        };
        
        let symbol_clone = instrument.symbol.clone();
        let timeframe_clone = instrument.timeframe.clone();

        optimization_tasks.push(tokio::spawn(async move {
            info!("Meta-Opt Phase 1: Optimizing for {}/{}", symbol_clone, timeframe_clone);
            match run_optimization_logic(single_instrument_opt_params).await { // Core logic
                Ok(core_opt_output) => {
                    if core_opt_output.best_result.is_none() {
                        warn!("Meta-Opt Phase 1: No best result from optimization logic for {}/{}", symbol_clone, timeframe_clone);
                    }
                    Ok((symbol_clone, timeframe_clone, core_opt_output.best_result))
                }
                Err(e) => {
                    error!("Meta-Opt Phase 1: Core optimization logic for {}/{} failed: {}", symbol_clone, timeframe_clone, e);
                    Err(format!("Core opt logic for {}/{} failed: {}", symbol_clone, timeframe_clone, e))
                }
            }
        }));
    }

    let optimization_phase_results = join_all(optimization_tasks).await;
    
    // This will store parameters for instruments that successfully optimized
    struct OptimalInstrumentParams {
        symbol: String,
        timeframe: String,
        sl: f64,
        tp: f64,
    }
    let mut instruments_for_final_backtest: Vec<OptimalInstrumentParams> = Vec::new();

    for res_result in optimization_phase_results {
        match res_result {
            Ok(Ok((symbol, timeframe, Some(best_opt_res)))) => {
                info!("Meta-Opt Phase 1: Optimal params found for {}/{}: SL={}, TP={}", symbol, timeframe, best_opt_res.sl_pips, best_opt_res.tp_pips);
                instruments_for_final_backtest.push(OptimalInstrumentParams {
                    symbol, timeframe, sl: best_opt_res.sl_pips, tp: best_opt_res.tp_pips
                });
            }
            Ok(Ok((symbol, timeframe, None))) => {
                 warn!("Meta-Opt Phase 1: No best optimization result found for {}/{}. Skipping for final backtest.", symbol, timeframe);
            }
            Ok(Err(e)) => error!("Meta-Opt Phase 1: Optimization task for an instrument returned an error: {}", e),
            Err(e) => error!("Meta-Opt Phase 1: Tokio task for optimization failed: {}", e), // JoinError
        }
    }

    if instruments_for_final_backtest.is_empty() {
        info!("Meta-Opt: No instruments yielded optimal parameters from Phase 1 for backtesting.");
        // Return a 200 OK with an empty/zeroed result set
        let summary = PortfolioOptimizeParamsSummary {
            start_time: portfolio_params.start_time.clone(),
            end_time: portfolio_params.end_time.clone(),
            instruments_queried: portfolio_params.instruments.len(),
            lot_size: portfolio_params.lot_size,
            optimization_sl_range: format!("{}-{} step {}", portfolio_params.optimization_sl_range.min, portfolio_params.optimization_sl_range.max, portfolio_params.optimization_sl_range.step),
            optimization_tp_range: format!("{}-{} step {}", portfolio_params.optimization_tp_range.min, portfolio_params.optimization_tp_range.max, portfolio_params.optimization_tp_range.step),
        };
        let empty_response = PortfolioOptimizedBacktestResponse {
            request_params: summary, overall_portfolio_pnl_pips: 0.0, overall_portfolio_total_trades: 0,
            overall_portfolio_win_rate: 0.0, overall_portfolio_profit_factor: None,
            overall_portfolio_winning_trades: 0, overall_portfolio_losing_trades: 0,
            individual_instrument_results: Vec::new(),
        };
        return Ok(HttpResponse::Ok().json(empty_response));
    }

    // --- Phase 2: Run backtest for each instrument WITH ITS OPTIMAL SL/TP over the REPORT PERIOD ---
    // The report period is defined by portfolio_params.start_time and portfolio_params.end_time
    let mut final_backtest_tasks = Vec::new();
    for opt_instrument in instruments_for_final_backtest {
        let backtest_params_for_report = MultiBacktestParams {
            start_time: portfolio_params.start_time.clone(), // Using the main request's start_time for the report
            end_time: portfolio_params.end_time.clone(),     // Using the main request's end_time for the report
            symbols: vec![opt_instrument.symbol.clone()],
            pattern_timeframes: vec![opt_instrument.timeframe.clone()],
            stop_loss_pips: opt_instrument.sl,
            take_profit_pips: opt_instrument.tp,
            lot_size: Some(portfolio_params.lot_size),
            allowed_trade_days: Some(portfolio_params.allowed_trade_days.clone()),
            trade_end_hour_utc: None, 
        };
        
        // These are captured for the PortfolioOptimizationResultEntry
        let symbol_for_entry = opt_instrument.symbol.clone();
        let timeframe_for_entry = opt_instrument.timeframe.clone();
        let optimal_sl_for_entry = opt_instrument.sl;
        let optimal_tp_for_entry = opt_instrument.tp;

        final_backtest_tasks.push(tokio::spawn(async move {
            info!("Meta-Opt Phase 2: Backtesting {}/{} with optimal SL={}, TP={}", symbol_for_entry, timeframe_for_entry, optimal_sl_for_entry, optimal_tp_for_entry);
            match run_multi_backtest_internal(backtest_params_for_report).await {
                Ok(bt_result) => {
                    // Calculate stats for this specific instrument's run during the report period
                    let (pnl, wins, losses, gross_profit_pips, gross_loss_pips) = 
                        bt_result.all_trades.iter().fold((0.0,0,0,0.0,0.0),|acc,t|
                        (acc.0+t.pnl_pips, acc.1+if t.pnl_pips>0.0 {1}else{0}, acc.2+if t.pnl_pips<0.0 {1}else{0},
                        acc.3+if t.pnl_pips>0.0 {t.pnl_pips}else{0.0}, acc.4+if t.pnl_pips<0.0 {t.pnl_pips.abs()}else{0.0}));
                    let total_t = bt_result.all_trades.len();
                    let win_r = if total_t > 0 { (wins as f64 / total_t as f64)*100.0 } else {0.0};
                    let pf = if gross_loss_pips > 1e-9 {Some(gross_profit_pips/gross_loss_pips)} else if gross_profit_pips > 1e-9 {None} else {Some(0.0)};
                    
                    Ok(PortfolioOptimizationResultEntry {
                        symbol: symbol_for_entry, timeframe: timeframe_for_entry, 
                        optimal_sl_pips: optimal_sl_for_entry, optimal_tp_pips: optimal_tp_for_entry,
                        backtest_pnl_pips: round_f64(pnl, 1), backtest_total_trades: total_t,
                        backtest_win_rate: round_f64(win_r, 2),
                        backtest_profit_factor: pf.map(|v| round_f64(v,2)),
                        winning_trades: wins, 
                        losing_trades: losses,
                        trades: bt_result.all_trades, // These are the trades from the report period
                    })
                }
                Err(e) => { 
                    warn!("Meta-Opt Phase 2: Backtest for {}/{} failed: {}", symbol_for_entry, timeframe_for_entry, e);
                    Err(format!("Backtest(optimal) for {}/{} failed: {}", symbol_for_entry, timeframe_for_entry, e))
                }
            }
        }));
    }

    let mut individual_instrument_results: Vec<PortfolioOptimizationResultEntry> = Vec::new();
    let final_backtest_run_results = join_all(final_backtest_tasks).await;

    for res in final_backtest_run_results {
        match res {
            Ok(Ok(entry)) => individual_instrument_results.push(entry),
            Ok(Err(e)) => error!("Meta-Opt Phase 2: A backtest task for an instrument returned an error: {}", e),
            Err(e) => error!("Meta-Opt Phase 2: Tokio task for backtest failed: {}", e), // JoinError
        }
    }
    
    // --- ACCURATE AGGREGATION for Overall Portfolio Stats (based on report period trades) ---
    let mut overall_portfolio_pnl_pips = 0.0;
    let mut overall_portfolio_total_trades = 0;
    let mut overall_portfolio_wins = 0;
    let mut overall_portfolio_losses = 0;
    let mut overall_portfolio_gross_profit = 0.0;
    let mut overall_portfolio_gross_loss = 0.0;

    for entry in &individual_instrument_results {
        // `entry.trades` now correctly refers to trades from the Phase 2 backtest (report period)
        for trade in &entry.trades { 
            overall_portfolio_pnl_pips += trade.pnl_pips;
            overall_portfolio_total_trades += 1;
            if trade.pnl_pips > 0.0 {
                overall_portfolio_wins += 1;
                overall_portfolio_gross_profit += trade.pnl_pips;
            } else if trade.pnl_pips < 0.0 {
                overall_portfolio_losses += 1; 
                overall_portfolio_gross_loss += trade.pnl_pips.abs();
            }
        }
    }
    // --- END OF ACCURATE AGGREGATION ---
    
    let overall_portfolio_win_rate = if overall_portfolio_total_trades > 0 { 
        (overall_portfolio_wins as f64 / overall_portfolio_total_trades as f64) * 100.0 
    } else {0.0};

    let overall_portfolio_pf = if overall_portfolio_gross_loss > 1e-9 { 
        Some(overall_portfolio_gross_profit / overall_portfolio_gross_loss) 
    } else if overall_portfolio_gross_profit > 1e-9 { Some(f64::INFINITY) } // Correctly handle infinity for PF
      else { Some(0.0) };


    let response = PortfolioOptimizedBacktestResponse {
        request_params: PortfolioOptimizeParamsSummary {
            start_time: portfolio_params.start_time, // This is the report period start
            end_time: portfolio_params.end_time,     // This is the report period end
            instruments_queried: portfolio_params.instruments.len(),
            lot_size: portfolio_params.lot_size,
            optimization_sl_range: format!("{}-{} step {}", portfolio_params.optimization_sl_range.min, portfolio_params.optimization_sl_range.max, portfolio_params.optimization_sl_range.step),
            optimization_tp_range: format!("{}-{} step {}", portfolio_params.optimization_tp_range.min, portfolio_params.optimization_tp_range.max, portfolio_params.optimization_tp_range.step),
        },
        overall_portfolio_pnl_pips: round_f64(overall_portfolio_pnl_pips, 1),
        overall_portfolio_total_trades,
        overall_portfolio_win_rate: round_f64(overall_portfolio_win_rate, 2),
        overall_portfolio_profit_factor: overall_portfolio_pf.map(|pf| if pf.is_infinite() { f64::NAN } else { round_f64(pf, 2) }), // Serialize NaN for Infinity
        overall_portfolio_winning_trades: overall_portfolio_wins,
        overall_portfolio_losing_trades: overall_portfolio_losses,
        individual_instrument_results,
    };
    Ok(HttpResponse::Ok().json(response))
}