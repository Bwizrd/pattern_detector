// src/multi_backtest_handler.rs

use actix_web::{web, HttpResponse, Error as ActixError};
use crate::backtest_api::{
    MultiBacktestRequest, MultiBacktestResponse, OverallBacktestSummary,
    IndividualTradeResult, SymbolTimeframeSummary,
};
use crate::patterns::{FiftyPercentBeforeBigBarRecognizer, PatternRecognizer};
use crate::trades::TradeConfig;
use crate::trading::TradeExecutor;
use crate::types::CandleData;

use chrono::{DateTime, Utc, Timelike, Datelike};
use std::collections::HashMap;
use futures::future::join_all;
use log::{info, error, warn, debug};
use serde_json::json;
use reqwest;
use csv::ReaderBuilder;
use std::io::Cursor;
use std::error::Error as StdError;


// Helper function for rounding
fn round_f64(value: f64, decimals: u32) -> f64 {
    if value.is_infinite() || value.is_nan() {
        return value; // Return special numbers as is
    }
    let multiplier = 10f64.powi(decimals as i32);
    (value * multiplier).round() / multiplier
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
) -> Result<Vec<CandleData>, Box<dyn StdError>> {
    debug!(
        "[multi_backtest_handler::load_backtest_candles] Fetching: sym={}, tf={}, start={}, end={}",
        symbol, timeframe, start_time, end_time
    );

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
    debug!("[multi_backtest_handler::load_backtest_candles] Sending Query:\n{}", flux_query);

    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    
    let response = match client
        .post(&url)
        .bearer_auth(token)
        .json(&json!({"query": flux_query, "type": "flux"}))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("Request to InfluxDB failed for {}/{}: {}", symbol, timeframe, e);
            return Err(Box::new(e));
        }
    };

    let status = response.status();
    let response_text = match response.text().await {
        Ok(text) => text,
        Err(e) => {
            error!("Failed to read response text for {}/{}: {}", symbol, timeframe, e);
            return Err(Box::new(e));
        }
    };

    if !status.is_success() {
        error!("InfluxDB query failed for {}/{}: Status: {}, Body: {}", symbol, timeframe, status, response_text);
        return Err(format!("InfluxDB query error (status {}) for {}/{}", status, symbol, timeframe).into());
    }
    
    debug!("[multi_backtest_handler::load_backtest_candles] Response length for {}/{}: {}", symbol, timeframe, response_text.len());

    let mut candles = Vec::<CandleData>::new();
    if !response_text.trim().is_empty() {
        let cursor = Cursor::new(response_text.as_bytes());
        let mut rdr = ReaderBuilder::new().has_headers(true).flexible(true).comment(Some(b'#')).from_reader(cursor);
        
        let headers = match rdr.headers() {
            Ok(h) => h.clone(),
            Err(e) => {
                error!("Failed to read CSV headers for {}/{}: {}", symbol, timeframe, e);
                return Err(Box::new(e));
            }
        };
        
        let get_idx = |name: &str| headers.iter().position(|h| h == name);

        let time_idx_opt = get_idx("_time");
        let open_idx_opt = get_idx("open");
        let high_idx_opt = get_idx("high");
        let low_idx_opt = get_idx("low");
        let close_idx_opt = get_idx("close");
        let volume_idx_opt = get_idx("volume");

        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) =
            (time_idx_opt, open_idx_opt, high_idx_opt, low_idx_opt, close_idx_opt, volume_idx_opt)
        {
            debug!("[multi_backtest_handler::load_backtest_candles] Found required headers for {}/{}.", symbol, timeframe);
            for result in rdr.records() {
                 match result {
                     Ok(record) => {
                         let parse_f64 = |idx: usize| record.get(idx).and_then(|v| v.trim().parse::<f64>().ok());
                         let parse_u32 = |idx: usize| record.get(idx).and_then(|v| v.trim().parse::<u32>().ok());
                         let time_str = record.get(t_idx).unwrap_or("").trim();
                         if !time_str.is_empty() {
                             candles.push(CandleData {
                                 time: time_str.to_string(),
                                 open: parse_f64(o_idx).unwrap_or(0.0),
                                 high: parse_f64(h_idx).unwrap_or(0.0),
                                 low: parse_f64(l_idx).unwrap_or(0.0),
                                 close: parse_f64(c_idx).unwrap_or(0.0),
                                 volume: parse_u32(v_idx).unwrap_or(0),
                             });
                         } else {
                             warn!("[multi_backtest_handler::load_backtest_candles] Record with empty _time for {}/{}", symbol, timeframe);
                         }
                     },
                     Err(e) => warn!("[multi_backtest_handler::load_backtest_candles] CSV record parse error for {}/{}: {}", symbol, timeframe, e),
                 }
             }
        } else {
             let missing_headers: Vec<&str> = [
                ("_time", time_idx_opt), ("open", open_idx_opt), ("high", high_idx_opt),
                ("low", low_idx_opt), ("close", close_idx_opt), ("volume", volume_idx_opt),
            ].iter().filter_map(|(name, opt_idx)| if opt_idx.is_none() { Some(*name) } else { None }).collect();
            let error_msg = format!("CSV header mismatch in InfluxDB response for {}/{}. Missing: {:?}. Headers found: {:?}", symbol, timeframe, missing_headers, headers);
            error!("[multi_backtest_handler::load_backtest_candles] {}", error_msg);
            return Err(error_msg.into());
        }
    } else {
        info!("[multi_backtest_handler::load_backtest_candles] InfluxDB response text was empty for {}/{}.", symbol, timeframe);
    }
    info!("[multi_backtest_handler::load_backtest_candles] Parsed {} candles for {}/{}.", candles.len(), symbol, timeframe);
    Ok(candles)
}


pub async fn run_multi_symbol_backtest(
    req: web::Json<MultiBacktestRequest>,
) -> Result<HttpResponse, ActixError> {
    info!("Received multi-symbol backtest request: {:?}", req.0);

    let host = std::env::var("INFLUXDB_HOST").unwrap_or_else(|_| "http://localhost:8086".to_string());
    let org = match std::env::var("INFLUXDB_ORG") {
        Ok(val) => val, Err(e) => { error!("INFLUXDB_ORG not set: {}", e); return Ok(HttpResponse::InternalServerError().json(json!({"error": "Server configuration error: INFLUXDB_ORG missing."}))); }
    };
    let token = match std::env::var("INFLUXDB_TOKEN") {
        Ok(val) => val, Err(e) => { error!("INFLUXDB_TOKEN not set: {}", e); return Ok(HttpResponse::InternalServerError().json(json!({"error": "Server configuration error: INFLUXDB_TOKEN missing."}))); }
    };
    let bucket = match std::env::var("INFLUXDB_BUCKET") {
        Ok(val) => val, Err(e) => { error!("INFLUXDB_BUCKET not set: {}", e); return Ok(HttpResponse::InternalServerError().json(json!({"error": "Server configuration error: INFLUXDB_BUCKET missing."}))); }
    };

    let pattern_timeframes = req.0.pattern_timeframes.clone();
    let execution_timeframe = "1m";
    let lot_size = req.0.lot_size.unwrap_or(0.01);

    let mut tasks = Vec::new();

    for symbol in req.0.symbols.iter() {
        for pattern_tf_str in pattern_timeframes.iter() { // Iterate over cloned Vec
            let task_symbol = symbol.clone();
            let task_pattern_tf = pattern_tf_str.clone(); 
            let task_execution_tf = execution_timeframe.to_string();
            let task_start_time = req.0.start_time.clone();
            let task_end_time = req.0.end_time.clone();
            let task_sl_pips = req.0.stop_loss_pips;
            let task_tp_pips = req.0.take_profit_pips;
            let task_lot_size = lot_size;

            let host_clone = host.clone();
            let org_clone = org.clone();
            let token_clone = token.clone();
            let bucket_clone = bucket.clone();
            
            tasks.push(tokio::spawn(async move {
                debug!("Starting backtest task for {} on {} (pattern TF), executing on {}", task_symbol, task_pattern_tf, task_execution_tf);

                let pattern_candles = match load_backtest_candles( &host_clone, &org_clone, &token_clone, &bucket_clone, &task_symbol, &task_pattern_tf, &task_start_time, &task_end_time ).await {
                    Ok(candles) => candles,
                    Err(e) => { error!("Task Error: loading pattern candles for {}/{}: {}", task_symbol, task_pattern_tf, e); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary); }
                };
                if pattern_candles.is_empty() { warn!("Task Info: No pattern candles found for {}/{}/{}", task_symbol, task_pattern_tf, task_start_time); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary); }
                debug!("Task Info: Loaded {} pattern candles for {}/{}", pattern_candles.len(), task_symbol, task_pattern_tf);

                let execution_candles = match load_backtest_candles( &host_clone, &org_clone, &token_clone, &bucket_clone, &task_symbol, &task_execution_tf, &task_start_time, &task_end_time ).await {
                    Ok(candles) => candles,
                    Err(e) => { error!("Task Error: loading execution (1m) candles for {}/{}: {}", task_symbol, task_pattern_tf, e); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary); }
                };
                if execution_candles.is_empty() { warn!("Task Info: No execution (1m) candles found for {}/{}/{}", task_symbol, task_pattern_tf, task_start_time); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary); }
                debug!("Task Info: Loaded {} execution candles for {}/{}", execution_candles.len(), task_symbol, task_execution_tf);

                let recognizer = FiftyPercentBeforeBigBarRecognizer::default();
                let detected_value_json: serde_json::Value = recognizer.detect(&pattern_candles);
                let mut total_detected_zones_count = 0;
                if let Some(data_val) = detected_value_json.get("data") { if let Some(price_val) = data_val.get("price") { if let Some(supply_zones_val) = price_val.get("supply_zones") { if let Some(zones_array) = supply_zones_val.get("zones").and_then(|z| z.as_array()) { total_detected_zones_count += zones_array.len(); } } if let Some(demand_zones_val) = price_val.get("demand_zones") { if let Some(zones_array) = demand_zones_val.get("zones").and_then(|z| z.as_array()) { total_detected_zones_count += zones_array.len(); } } } }
                if total_detected_zones_count == 0 { info!("Task Info: No pattern zones found in JSON for {} on {}", task_symbol, task_pattern_tf); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary); }
                debug!("Task Info: Total {} raw pattern zones found in JSON for {}/{}", total_detected_zones_count, task_symbol, task_pattern_tf);

                let trade_config = TradeConfig { enabled: true, lot_size: task_lot_size, default_stop_loss_pips: task_sl_pips, default_take_profit_pips: task_tp_pips, risk_percent: 1.0, max_trades_per_pattern: 1, ..Default::default() };
                let mut trade_executor = TradeExecutor::new(trade_config.clone());
                trade_executor.set_minute_candles(execution_candles.clone());
                let executed_trades_internal = trade_executor.execute_trades_for_pattern( "fifty_percent_before_big_bar", &detected_value_json, &pattern_candles );
                info!("Task Info: Executed {} trades for {}/{}", executed_trades_internal.len(), task_symbol, task_pattern_tf);
                
                let mut current_task_trades_result: Vec<IndividualTradeResult> = Vec::new();
                let mut winning_trades_count = 0;
                let mut total_pnl_pips_for_summary_raw: f64 = 0.0;
                let mut total_gross_profit_pips_raw: f64 = 0.0;
                let mut total_gross_loss_pips_raw: f64 = 0.0;

                for trade_internal in executed_trades_internal.iter() {
                    let entry_dt_res = DateTime::parse_from_rfc3339(&trade_internal.entry_time);
                    if entry_dt_res.is_err() { warn!("Task Warning: Failed to parse entry_time '{}' for {}/{}", &trade_internal.entry_time, task_symbol, task_pattern_tf); continue; }
                    let entry_dt = entry_dt_res.unwrap().with_timezone(&Utc);
                    let exit_dt = trade_internal.exit_time.as_ref().and_then(|et_str| DateTime::parse_from_rfc3339(et_str).ok() ).map(|dt| dt.with_timezone(&Utc)); 
                    
                    let pnl_raw = trade_internal.profit_loss_pips.unwrap_or(0.0);
                    let pnl_for_individual_trade_struct = round_f64(pnl_raw, 1);

                    total_pnl_pips_for_summary_raw += pnl_raw;
                    if pnl_raw > 0.0 { winning_trades_count += 1; total_gross_profit_pips_raw += pnl_raw; }
                    else if pnl_raw < 0.0 { total_gross_loss_pips_raw += pnl_raw.abs(); }
                    
                    current_task_trades_result.push(IndividualTradeResult {
                        symbol: task_symbol.clone(), timeframe: task_pattern_tf.clone(),
                        direction: format!("{:?}", trade_internal.direction), entry_time: entry_dt,
                        entry_price: trade_internal.entry_price, exit_time: exit_dt,
                        exit_price: trade_internal.exit_price,
                        pnl_pips: Some(pnl_for_individual_trade_struct),
                        exit_reason: trade_internal.exit_reason.clone(),
                        entry_day_of_week: Some(entry_dt.weekday().to_string()),
                        entry_hour_of_day: Some(entry_dt.hour()),
                    });
                }
                let total_trades_for_summary = executed_trades_internal.len();
                let win_rate = if total_trades_for_summary > 0 { (winning_trades_count as f64 / total_trades_for_summary as f64) * 100.0 } else { 0.0 };
                let profit_factor_val = if total_gross_loss_pips_raw > 0.0 { total_gross_profit_pips_raw / total_gross_loss_pips_raw } else if total_gross_profit_pips_raw > 0.0 { f64::INFINITY } else { 0.0 };
                
                let summary_for_task = SymbolTimeframeSummary {
                    symbol: task_symbol.clone(), timeframe: task_pattern_tf.clone(),
                    total_trades: total_trades_for_summary, winning_trades: winning_trades_count,
                    losing_trades: total_trades_for_summary - winning_trades_count,
                    win_rate_percent: round_f64(win_rate, 2),
                    total_pnl_pips: round_f64(total_pnl_pips_for_summary_raw, 1),
                    profit_factor: round_f64(profit_factor_val, 2),
                };
                (task_symbol, task_pattern_tf, current_task_trades_result, summary_for_task)
            }));
        }
    }

    let task_results_outcomes = join_all(tasks).await;
    let mut all_individual_trades: Vec<IndividualTradeResult> = Vec::new();
    let mut detailed_summaries: Vec<SymbolTimeframeSummary> = Vec::new();

    for result_outcome in task_results_outcomes {
        match result_outcome {
            Ok((_symbol, _pattern_tf, individual_trades_from_task, summary_from_task)) => {
                all_individual_trades.extend(individual_trades_from_task);
                if summary_from_task.total_trades > 0 {
                    detailed_summaries.push(summary_from_task);
                }
            }
            Err(e) => {
                error!("A backtest task panicked (join error): {:?}", e);
            }
        }
    }

    debug!("[OverallSummary] Starting calculation. all_individual_trades length: {}", all_individual_trades.len());
    let mut overall_total_trades = 0;
    let mut overall_total_pnl_pips_sum_of_rounded: f64 = 0.0; 
    let mut overall_winning_trades = 0;
    let mut overall_gross_profit_pips_sum_of_rounded: f64 = 0.0;
    let mut overall_gross_loss_pips_sum_of_rounded: f64 = 0.0;

    let mut trades_by_symbol_count: HashMap<String, usize> = HashMap::new();
    let mut trades_by_timeframe_count: HashMap<String, usize> = HashMap::new();
    let mut trades_by_day_count: HashMap<String, usize> = HashMap::new();
    let mut pnl_by_day_pips_sum_of_rounded: HashMap<String, f64> = HashMap::new();
    let mut trades_by_hour_count: HashMap<u32, usize> = HashMap::new();
    let mut pnl_by_hour_pips_sum_of_rounded: HashMap<u32, f64> = HashMap::new();

    let mut total_duration_seconds: i64 = 0;

    for trade in &all_individual_trades {
        overall_total_trades += 1;
        let pnl_this_trade = trade.pnl_pips.unwrap_or(0.0);
        
        overall_total_pnl_pips_sum_of_rounded += pnl_this_trade;

        if pnl_this_trade > 1e-9 { 
            overall_winning_trades += 1;
            overall_gross_profit_pips_sum_of_rounded += pnl_this_trade;
        } else if pnl_this_trade < -1e-9 {
            overall_gross_loss_pips_sum_of_rounded += pnl_this_trade.abs();
        }

        *trades_by_symbol_count.entry(trade.symbol.clone()).or_insert(0) += 1;
        *trades_by_timeframe_count.entry(trade.timeframe.clone()).or_insert(0) += 1;
        if let Some(day_str) = &trade.entry_day_of_week {
            *trades_by_day_count.entry(day_str.clone()).or_insert(0) += 1;
            *pnl_by_day_pips_sum_of_rounded.entry(day_str.clone()).or_insert(0.0) += pnl_this_trade;
        }
        if let Some(hour_val) = trade.entry_hour_of_day {
            *trades_by_hour_count.entry(hour_val).or_insert(0) += 1;
            *pnl_by_hour_pips_sum_of_rounded.entry(hour_val).or_insert(0.0) += pnl_this_trade;
        }

        // Calculate duration for this trade
        if let Some(exit_t) = trade.exit_time { // trade.exit_time is Option<DateTime<Utc>>
            let entry_t = trade.entry_time;     // trade.entry_time is DateTime<Utc>
            let duration_for_this_trade = exit_t.signed_duration_since(entry_t);
            total_duration_seconds += duration_for_this_trade.num_seconds();
        }
    }
    debug!("[OverallSummary] After loop. overall_total_trades: {}", overall_total_trades);
    debug!("[OverallSummary] After loop. overall_total_pnl_pips_sum_of_rounded: {}", overall_total_pnl_pips_sum_of_rounded);
    
    let average_trade_duration_str: String;
    if overall_total_trades > 0 && total_duration_seconds > 0 {
        let avg_seconds_per_trade = total_duration_seconds / overall_total_trades as i64;
        let avg_duration = chrono::Duration::seconds(avg_seconds_per_trade);
        
        // Format the duration
        let hours = avg_duration.num_hours();
        let minutes = avg_duration.num_minutes() % 60;
        let seconds = avg_duration.num_seconds() % 60;
        if hours > 0 {
            average_trade_duration_str = format!("{}h {}m {}s", hours, minutes, seconds);
        } else if minutes > 0 {
            average_trade_duration_str = format!("{}m {}s", minutes, seconds);
        } else {
            average_trade_duration_str = format!("{}s", seconds);
        }
    } else {
        average_trade_duration_str = "N/A".to_string();
    }
    debug!("[OverallSummary] Average trade duration: {}", average_trade_duration_str);
    let overall_losing_trades = overall_total_trades - overall_winning_trades; 
    
    let overall_win_rate_calc = if overall_total_trades > 0 { (overall_winning_trades as f64 / overall_total_trades as f64) * 100.0 } else { 0.0 };
    let overall_profit_factor_calc = if overall_gross_loss_pips_sum_of_rounded > 0.0 { overall_gross_profit_pips_sum_of_rounded / overall_gross_loss_pips_sum_of_rounded } else if overall_gross_profit_pips_sum_of_rounded > 0.0 { f64::INFINITY } else { 0.0 };
    
    let calculate_percent = |count: usize, total: usize| {
        round_f64(if total > 0 { (count as f64 / total as f64) * 100.0 } else { 0.0 }, 2)
    };

    let trades_by_symbol_percent: HashMap<String, f64> = trades_by_symbol_count.iter().map(|(k, v)| (k.clone(), calculate_percent(*v, overall_total_trades))).collect();
    let trades_by_timeframe_percent: HashMap<String, f64> = trades_by_timeframe_count.iter().map(|(k, v)| (k.clone(), calculate_percent(*v, overall_total_trades))).collect();
    let trades_by_day_of_week_percent: HashMap<String, f64> = trades_by_day_count.iter().map(|(k, v)| (k.clone(), calculate_percent(*v, overall_total_trades))).collect();
    let trades_by_hour_of_day_percent: HashMap<u32, f64> = trades_by_hour_count.iter().map(|(k, v)| (*k, calculate_percent(*v, overall_total_trades))).collect();

    let pnl_by_day_pips_final: HashMap<String, f64> = pnl_by_day_pips_sum_of_rounded.iter().map(|(k,v)| (k.clone(), round_f64(*v, 1))).collect();
    let pnl_by_hour_pips_final: HashMap<u32, f64> = pnl_by_hour_pips_sum_of_rounded.iter().map(|(k,v)| (*k, round_f64(*v, 1))).collect();

    let overall_summary = OverallBacktestSummary {
        total_trades: overall_total_trades,
        overall_winning_trades,
        overall_losing_trades, 
        total_pnl_pips: round_f64(overall_total_pnl_pips_sum_of_rounded, 1),
        overall_win_rate_percent: round_f64(overall_win_rate_calc, 2),
        overall_profit_factor: round_f64(overall_profit_factor_calc, 2),
        average_trade_duration_str,
        trades_by_symbol_percent,
        trades_by_timeframe_percent,
        trades_by_day_of_week_percent,
        pnl_by_day_of_week_pips: pnl_by_day_pips_final,
        trades_by_hour_of_day_percent,
        pnl_by_hour_of_day_pips: pnl_by_hour_pips_final,
    };

    let response_payload = MultiBacktestResponse {
        overall_summary,
        detailed_summaries,
        all_trades: all_individual_trades,
    };

    Ok(HttpResponse::Ok().json(response_payload))
}

impl SymbolTimeframeSummary {
    fn default_new(symbol: &str, timeframe: &str) -> Self {
        SymbolTimeframeSummary {
            symbol: symbol.to_string(),
            timeframe: timeframe.to_string(),
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate_percent: 0.0,
            total_pnl_pips: 0.0,
            profit_factor: 0.0,
        }
    }
}