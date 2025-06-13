// src/multi_backtest_handler.rs

use actix_web::{web, HttpResponse, Error as ActixError};
use crate::trading::backtest::backtest_api::{
    MultiBacktestRequest, MultiBacktestResponse, OverallBacktestSummary,
    IndividualTradeResult, SymbolTimeframeSummary,
    // ⭐ ADD THESE NEW IMPORTS:
    TradingRulesSnapshot, EnvironmentVariablesSnapshot, RequestParametersSnapshot,
    RejectedTradeResult, RejectionSummary,
};
use crate::zones::patterns::{FiftyPercentBeforeBigBarRecognizer, PatternRecognizer};
use crate::zones::zone_detection::{ZoneDetectionEngine, ZoneDetectionRequest};
use crate::trading::trades::TradeConfig;
use crate::trading::trading::TradeExecutor;
use crate::types::CandleData;

use chrono::{DateTime, Utc, Timelike, Datelike, Weekday, Duration};
use std::collections::HashMap;
use futures::future::join_all;
use log::{info, error, warn, debug};
use serde_json::json;
use reqwest;
use csv::ReaderBuilder;
use std::io::Cursor;
use std::error::Error as StdError;

/// Calculate the same 90-day lookback period that the cache uses
fn calculate_90_day_window() -> (String, String) {
    let now = Utc::now();
    let ninety_days_ago = now - Duration::days(90);
    
    // Format to match: YYYY-MM-DDTHH:MM:SSZ
    let start_time = format!("{}T00:00:00Z", ninety_days_ago.format("%Y-%m-%d"));
    let end_time = format!("{}T23:59:59Z", now.format("%Y-%m-%d"));
    
    (start_time, end_time)
}

// ⭐ UPDATED: Helper functions to capture trading rules with request parameter resolution
fn capture_trading_rules_snapshot(trade_config: &TradeConfig, req: &MultiBacktestRequest) -> TradingRulesSnapshot {
    TradingRulesSnapshot {
        // From resolved TradeConfig
        trading_enabled: trade_config.enabled,
        lot_size: trade_config.lot_size,
        default_stop_loss_pips: trade_config.default_stop_loss_pips,
        default_take_profit_pips: trade_config.default_take_profit_pips,
        max_trades_per_zone: trade_config.max_trades_per_zone,
        
        // From resolved request parameters
        trading_max_daily_trades: Some(req.get_max_daily_trades()),
        trading_max_trades_per_symbol: Some(req.get_max_trades_per_symbol()),
        trading_min_zone_strength: Some(req.get_min_zone_strength()),
        max_touch_count_for_trading: Some(req.get_max_touch_count_for_trading()),
        trading_max_trades_per_zone_env: Some(req.get_max_trades_per_zone()),
        trading_allowed_symbols: req.get_allowed_symbols(),
        trading_allowed_timeframes: req.get_allowed_timeframes(),
        trading_allowed_weekdays: req.get_allowed_trade_days(),
        trading_start_hour_utc: req.get_trade_start_hour_utc(),
        trading_end_hour_utc: req.get_trade_end_hour_utc(),
    }
}

fn capture_environment_variables_snapshot() -> EnvironmentVariablesSnapshot {
    EnvironmentVariablesSnapshot {
        trading_max_daily_trades: std::env::var("TRADING_MAX_DAILY_TRADES").ok(),
        trading_max_trades_per_symbol: std::env::var("TRADING_MAX_TRADES_PER_SYMBOL").ok(),
        trading_max_trades_per_zone: std::env::var("TRADING_MAX_TRADES_PER_ZONE").ok(),
        trading_min_zone_strength: std::env::var("TRADING_MIN_ZONE_STRENGTH").ok(),
        max_touch_count_for_trading: std::env::var("MAX_TOUCH_COUNT_FOR_TRADING").ok(),
        trading_allowed_symbols: std::env::var("TRADING_ALLOWED_SYMBOLS").ok(),
        trading_allowed_timeframes: std::env::var("TRADING_ALLOWED_TIMEFRAMES").ok(),
        trading_allowed_weekdays: std::env::var("TRADING_ALLOWED_WEEKDAYS").ok(),
        trading_start_hour_utc: std::env::var("TRADING_START_HOUR_UTC").ok(),
        trading_end_hour_utc: std::env::var("TRADING_END_HOUR_UTC").ok(),
        trading_enabled: std::env::var("TRADING_ENABLED").ok(),
    }
}

fn capture_request_parameters_snapshot(req: &MultiBacktestRequest) -> RequestParametersSnapshot {
    RequestParametersSnapshot {
        symbols: req.symbols.clone(),
        pattern_timeframes: req.pattern_timeframes.clone(),
        start_time: req.start_time.clone(),
        end_time: req.end_time.clone(),
        lot_size: req.lot_size,
        stop_loss_pips: req.stop_loss_pips,
        take_profit_pips: req.take_profit_pips,
        max_daily_trades: req.max_daily_trades,
        max_trades_per_symbol: req.max_trades_per_symbol,
        max_trades_per_zone: req.max_trades_per_zone,
        min_zone_strength: req.min_zone_strength,
        max_touch_count_for_trading: req.max_touch_count_for_trading,
        allowed_trade_days: req.allowed_trade_days.clone(),
        allowed_symbols: req.allowed_symbols.clone(),
        allowed_timeframes: req.allowed_timeframes.clone(),
        trade_start_hour_utc: req.trade_start_hour_utc,
        trade_end_hour_utc: req.trade_end_hour_utc,
    }
}

// ⭐ UPDATED: Logging function to show resolved values with request ➜ environment fallback
fn log_active_trading_rules(trade_config: &TradeConfig, req: &MultiBacktestRequest) {
    info!("🔧 ACTIVE TRADING RULES (Request ➜ Environment Fallback):");
    info!("   === Resolved Trading Parameters ===");
    info!("   trading_enabled: {}", trade_config.enabled);
    info!("   lot_size: {} (req: {:?} ➜ env: {})", 
          trade_config.lot_size, 
          req.lot_size,
          std::env::var("TRADING_LOT_SIZE").unwrap_or("NOT_SET".to_string()));
    info!("   stop_loss_pips: {} (req: {:?} ➜ env: {})", 
          trade_config.default_stop_loss_pips, 
          req.stop_loss_pips,
          std::env::var("TRADING_STOP_LOSS_PIPS").unwrap_or("NOT_SET".to_string()));
    info!("   take_profit_pips: {} (req: {:?} ➜ env: {})", 
          trade_config.default_take_profit_pips, 
          req.take_profit_pips,
          std::env::var("TRADING_TAKE_PROFIT_PIPS").unwrap_or("NOT_SET".to_string()));
    info!("   max_trades_per_zone: {} (req: {:?} ➜ env: {})", 
          trade_config.max_trades_per_zone, 
          req.max_trades_per_zone,
          std::env::var("TRADING_MAX_TRADES_PER_ZONE").unwrap_or("NOT_SET".to_string()));
    
    info!("   === Additional Trading Rules ===");
    info!("   max_daily_trades: {} (req: {:?} ➜ env: {})", 
          req.get_max_daily_trades(), 
          req.max_daily_trades,
          std::env::var("TRADING_MAX_DAILY_TRADES").unwrap_or("NOT_SET".to_string()));
    info!("   max_trades_per_symbol: {} (req: {:?} ➜ env: {})", 
          req.get_max_trades_per_symbol(), 
          req.max_trades_per_symbol,
          std::env::var("TRADING_MAX_TRADES_PER_SYMBOL").unwrap_or("NOT_SET".to_string()));
    info!("   min_zone_strength: {} (req: {:?} ➜ env: {})", 
          req.get_min_zone_strength(), 
          req.min_zone_strength,
          std::env::var("TRADING_MIN_ZONE_STRENGTH").unwrap_or("NOT_SET".to_string()));
    info!("   max_touch_count: {} (req: {:?} ➜ env: {})", 
          req.get_max_touch_count_for_trading(), 
          req.max_touch_count_for_trading,
          std::env::var("MAX_TOUCH_COUNT_FOR_TRADING").unwrap_or("NOT_SET".to_string()));
    
    info!("   === Time/Day Filtering ===");
    info!("   allowed_trade_days: {:?} (req: {:?} ➜ env: {})", 
          req.get_allowed_trade_days(), 
          req.allowed_trade_days,
          std::env::var("TRADING_ALLOWED_WEEKDAYS").unwrap_or("NOT_SET".to_string()));
    info!("   trade_start_hour_utc: {:?} (req: {:?} ➜ env: {})", 
          req.get_trade_start_hour_utc(), 
          req.trade_start_hour_utc,
          std::env::var("TRADING_START_HOUR").unwrap_or("NOT_SET".to_string()));
    info!("   trade_end_hour_utc: {:?} (req: {:?} ➜ env: {})", 
          req.get_trade_end_hour_utc(), 
          req.trade_end_hour_utc,
          std::env::var("TRADING_END_HOUR").unwrap_or("NOT_SET".to_string()));
    info!("   allowed_symbols: {:?} (req: {:?} ➜ env: {})", 
          req.get_allowed_symbols(), 
          req.allowed_symbols,
          std::env::var("TRADING_ALLOWED_SYMBOLS").unwrap_or("NOT_SET".to_string()));
    info!("   allowed_timeframes: {:?} (req: {:?} ➜ env: {})", 
          req.get_allowed_timeframes(), 
          req.allowed_timeframes,
          std::env::var("TRADING_ALLOWED_TIMEFRAMES").unwrap_or("NOT_SET".to_string()));
}

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
    
    // ⭐ LOG 90-DAY ZONE DETECTION WINDOW vs REQUESTED TRADE FILTERING WINDOW
    let (zone_window_start, zone_window_end) = calculate_90_day_window();
    info!("🕰️ Zone Detection Window (90 days): {} to {}", zone_window_start, zone_window_end);
    info!("🎯 Trade Filtering Window (requested): {} to {}", req.0.start_time, req.0.end_time);

    // ⭐ CAPTURE REQUEST PARAMETERS AND ENVIRONMENT AT THE START
    let request_params_snapshot = capture_request_parameters_snapshot(&req.0);
    let environment_snapshot = capture_environment_variables_snapshot();

    // ⭐ USE REQUEST HELPER METHODS FOR PARAMETER RESOLUTION
    let stop_loss_pips = req.0.get_stop_loss_pips();
    let take_profit_pips = req.0.get_take_profit_pips();
    let lot_size = req.0.get_lot_size();
    let max_trades_per_zone = req.0.get_max_trades_per_zone();
    let show_rejected_trades = req.0.get_show_rejected_trades();
    
    // Parse allowed days using the request helper method
    let parsed_allowed_days: Option<Vec<Weekday>> = parse_allowed_days(req.0.get_allowed_trade_days());
    let parsed_trade_end_hour_utc: Option<u32> = req.0.get_trade_end_hour_utc();

    if req.0.get_allowed_trade_days().is_some() && parsed_allowed_days.is_none() {
        warn!("Request included allowed_trade_days, but all entries were invalid or list was empty.");
    }

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

        // ⭐ CREATE THE TRADE CONFIG WITH RESOLVED PARAMETERS  
    // ⭐ CREATE A SIMPLE CONFIG FOR SNAPSHOT ONLY (before the task loop)
    let snapshot_trade_config = TradeConfig {
        enabled: true,
        lot_size,
        default_stop_loss_pips: stop_loss_pips,
        default_take_profit_pips: take_profit_pips,
        risk_percent: 1.0,
        max_trades_per_zone,
        min_zone_strength: Some(req.0.get_min_zone_strength()),
        max_touch_count: Some(req.0.get_max_touch_count_for_trading()),
        ..Default::default()
    };

    // ⭐ LOG THE ACTIVE RULES (now shows resolved values)
    log_active_trading_rules(&snapshot_trade_config, &req.0);

    // ⭐ CAPTURE TRADING RULES SNAPSHOT  
    let trading_rules_snapshot = capture_trading_rules_snapshot(&snapshot_trade_config, &req.0);

    let mut tasks = Vec::new();

    for symbol in req.0.symbols.iter() {
        for pattern_tf_str in pattern_timeframes.iter() {
            let task_symbol = symbol.clone();
            let task_pattern_tf = pattern_tf_str.clone(); 
            let task_execution_tf = execution_timeframe.to_string();
            let task_start_time = req.0.start_time.clone();
            let task_end_time = req.0.end_time.clone();
            let task_sl_pips = stop_loss_pips;
            let task_tp_pips = take_profit_pips;
            let task_lot_size = lot_size;
            let task_max_trades_per_zone = max_trades_per_zone;
            let task_allowed_days = parsed_allowed_days.clone();
            let task_trade_end_hour_utc = parsed_trade_end_hour_utc;
            // ⭐ ADD THESE LINES:
            let task_min_zone_strength = req.0.get_min_zone_strength();
            let task_max_touch_count = req.0.get_max_touch_count_for_trading();
            let task_show_rejected_trades = req.0.get_show_rejected_trades();
            let host_clone = host.clone();
            let org_clone = org.clone();
            let token_clone = token.clone();
            let bucket_clone = bucket.clone();
                
            tasks.push(tokio::spawn(async move {
                debug!("Starting backtest task for {} on {} (pattern TF), executing on {}", task_symbol, task_pattern_tf, task_execution_tf);

                // ⭐ ADD "_SB" SUFFIX FOR INFLUXDB QUERY (database stores symbols with _SB suffix)
                let db_symbol = if task_symbol.ends_with("_SB") { 
                    task_symbol.clone() 
                } else { 
                    format!("{}_SB", task_symbol) 
                };
                
                // ⭐ USE 90-DAY WINDOW FOR ZONE DETECTION (same as cache) but filter trades by requested dates
                let (zone_detection_start, zone_detection_end) = calculate_90_day_window();
                debug!("Using 90-day window for zone detection: {} to {}", zone_detection_start, zone_detection_end);
                debug!("Will filter trades to requested range: {} to {}", task_start_time, task_end_time);
                
                let pattern_candles = match load_backtest_candles( &host_clone, &org_clone, &token_clone, &bucket_clone, &db_symbol, &task_pattern_tf, &zone_detection_start, &zone_detection_end ).await {
                    Ok(candles) => candles,
                    Err(e) => { error!("Task Error: loading pattern candles for {}/{}: {}", task_symbol, task_pattern_tf, e); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary, Vec::new()); }
                };
                if pattern_candles.is_empty() { warn!("Task Info: No pattern candles found for {}/{}/{}", task_symbol, task_pattern_tf, task_start_time); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary, Vec::new()); }
                debug!("Task Info: Loaded {} pattern candles for {}/{}", pattern_candles.len(), task_symbol, task_pattern_tf);

                let execution_candles = match load_backtest_candles( &host_clone, &org_clone, &token_clone, &bucket_clone, &db_symbol, &task_execution_tf, &zone_detection_start, &zone_detection_end ).await {
                    Ok(candles) => candles,
                    Err(e) => { error!("Task Error: loading execution (1m) candles for {}/{}: {}", task_symbol, task_pattern_tf, e); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary, Vec::new()); }
                };
                if execution_candles.is_empty() { warn!("Task Info: No execution (1m) candles found for {}/{}/{}", task_symbol, task_pattern_tf, task_start_time); let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); return (task_symbol, task_pattern_tf, Vec::new(), summary, Vec::new()); }
                debug!("Task Info: Loaded {} execution candles for {}/{}", execution_candles.len(), task_symbol, task_execution_tf);

                // ⭐ USE ZONE DETECTION ENGINE (with enrichment) instead of raw pattern recognizer
                let zone_engine = ZoneDetectionEngine::new();
                let zone_request = ZoneDetectionRequest {
                    symbol: task_symbol.clone(),
                    timeframe: task_pattern_tf.clone(),
                    pattern: "fifty_percent_before_big_bar".to_string(),
                    candles: pattern_candles.clone(),
                    max_touch_count: Some(task_max_touch_count),
                };
                
                let zone_result = match zone_engine.detect_zones(zone_request) {
                    Ok(result) => result,
                    Err(e) => {
                        warn!("Task Error: Zone detection failed for {}/{}: {}", task_symbol, task_pattern_tf, e);
                        let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf);
                        return (task_symbol, task_pattern_tf, Vec::new(), summary, Vec::new());
                    }
                };
                
                let total_detected_zones_count = zone_result.supply_zones.len() + zone_result.demand_zones.len();
                if total_detected_zones_count == 0 { 
                    info!("Task Info: No enriched zones found for {} on {}", task_symbol, task_pattern_tf); 
                    let summary = SymbolTimeframeSummary::default_new(&task_symbol, &task_pattern_tf); 
                    return (task_symbol, task_pattern_tf, Vec::new(), summary, Vec::new()); 
                }
                debug!("Task Info: Total {} enriched zones found for {}/{} ({} supply, {} demand)", 
                    total_detected_zones_count, task_symbol, task_pattern_tf, 
                    zone_result.supply_zones.len(), zone_result.demand_zones.len());

                // ⭐ USE THE SAME SIMPLE CONFIG AS ORIGINAL
                let current_trade_config = TradeConfig {
                enabled: true,
                lot_size: task_lot_size,
                default_stop_loss_pips: task_sl_pips,
                default_take_profit_pips: task_tp_pips,
                risk_percent: 1.0,
                max_trades_per_zone: task_max_trades_per_zone,
                // ⭐ ADD THE FILTER PARAMETERS:
                min_zone_strength: Some(task_min_zone_strength),
                max_touch_count: Some(task_max_touch_count),
                ..Default::default()
            };

                // ⭐ ADD DEBUG LOGGING TO VERIFY CONFIG (inside the task)
                info!(
                    "🔧 CREATED TRADE CONFIG for {} - Min Strength: {:?}, Max Touch Count: {:?}",
                    task_symbol, current_trade_config.min_zone_strength, current_trade_config.max_touch_count
                );

                let mut trade_executor = TradeExecutor::new(
                    current_trade_config,
                    &task_symbol,
                    task_allowed_days, 
                    task_trade_end_hour_utc
                );
                trade_executor.set_minute_candles(execution_candles.clone());
                trade_executor.set_timeframe(task_pattern_tf.clone());

                // ⭐ Execute trades using enriched zones directly (no JSON conversion needed)
                let executed_trades_internal = trade_executor.execute_trades_for_pattern( "fifty_percent_before_big_bar", &serde_json::Value::Null, &pattern_candles );
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
                    
                    // ⭐ FILTER TRADES TO REQUESTED DATE RANGE
                    let requested_start = DateTime::parse_from_rfc3339(&task_start_time).unwrap().with_timezone(&Utc);
                    let requested_end = DateTime::parse_from_rfc3339(&task_end_time).unwrap().with_timezone(&Utc);
                    
                    if entry_dt < requested_start || entry_dt > requested_end {
                        debug!("Filtering out trade {} - entry time {} outside requested range {} to {}", 
                               trade_internal.pattern_id,
                               entry_dt, requested_start, requested_end);
                        continue;
                    } 
                    
                    let mut pnl_raw = trade_internal.profit_loss_pips.unwrap_or(0.0);

                    // --- JPY PIP ADJUSTMENT ---
                    // Get the main pair string, e.g., "USDJPY" from "USDJPY_SB"
                    let main_pair_part = task_symbol.split('_').next().unwrap_or(&task_symbol);
                    let is_jpy_pair = main_pair_part.to_uppercase().ends_with("JPY");

                    if is_jpy_pair {
                        // If profit_loss_pips was calculated assuming a 4-decimal pip (e.g., 0.0001 for EURUSD)
                        // for JPY pairs (which use 2-decimal pip, e.g., 0.01 for USDJPY),
                        // the reported pips will be 100 times too large.
                        // Example: price_change = 0.50 JPY (which is 50 JPY pips)
                        // Erroneous calculation: 0.50 / 0.0001 = 5000 "pips"
                        // Correct calculation: 0.50 / 0.01 = 50 pips
                        // To correct: 5000 * (0.0001 / 0.01) = 5000 * 0.01 = 50
                        pnl_raw *= 0.01; 
                        debug!("Adjusted PNL for JPY pair {}: original_pips_assuming_4dp={}, adjusted_pips={}", task_symbol, trade_internal.profit_loss_pips.unwrap_or(0.0), pnl_raw);
                    }
                    // --- END JPY PIP ADJUSTMENT ---
                    
                    let pnl_for_individual_trade_struct = round_f64(pnl_raw, 1);

                    total_pnl_pips_for_summary_raw += pnl_raw; // Use (potentially adjusted) pnl_raw
                    if pnl_raw > 1e-9 { // Use epsilon for float comparison
                        winning_trades_count += 1; 
                        total_gross_profit_pips_raw += pnl_raw; 
                    } else if pnl_raw < -1e-9 { // Use epsilon for float comparison
                        total_gross_loss_pips_raw += pnl_raw.abs(); 
                    }
                    
                    current_task_trades_result.push(IndividualTradeResult {
                        symbol: task_symbol.clone(), 
                        timeframe: task_pattern_tf.clone(),
                        zone_id: Some(trade_internal.pattern_id.clone()), // ADD THIS LINE - pattern_id contains the zone_id
                        direction: format!("{:?}", trade_internal.direction), 
                        entry_time: entry_dt,
                        entry_price: trade_internal.entry_price, 
                        exit_time: exit_dt,
                        exit_price: trade_internal.exit_price,
                        pnl_pips: Some(pnl_for_individual_trade_struct),
                        exit_reason: trade_internal.exit_reason.clone(),
                        zone_strength: trade_internal.zone_strength,        // ← Use from trade
                        touch_count: trade_internal.zone_touch_count,       // ← Use from trade
                        entry_day_of_week: Some(entry_dt.weekday().to_string()),
                        entry_hour_of_day: Some(entry_dt.hour()),
                    });
                }
                let total_trades_for_summary = executed_trades_internal.len();
                let win_rate = if total_trades_for_summary > 0 { (winning_trades_count as f64 / total_trades_for_summary as f64) * 100.0 } else { 0.0 };
                let profit_factor_val = if total_gross_loss_pips_raw > 1e-9 { total_gross_profit_pips_raw / total_gross_loss_pips_raw } else if total_gross_profit_pips_raw > 1e-9 { f64::INFINITY } else { 0.0 };
                
                let summary_for_task = SymbolTimeframeSummary {
                    symbol: task_symbol.clone(), timeframe: task_pattern_tf.clone(),
                    total_trades: total_trades_for_summary, winning_trades: winning_trades_count,
                    losing_trades: total_trades_for_summary - winning_trades_count,
                    win_rate_percent: round_f64(win_rate, 2),
                    total_pnl_pips: round_f64(total_pnl_pips_for_summary_raw, 1), // Uses sum of (potentially adjusted) raw pips
                    profit_factor: round_f64(profit_factor_val, 2),
                };
                // Collect rejected trades if requested
                let rejected_trades_for_task = if task_show_rejected_trades {
                    trade_executor.get_rejected_trades()
                } else {
                    Vec::new()
                };

                (task_symbol, task_pattern_tf, current_task_trades_result, summary_for_task, rejected_trades_for_task)
            }));
        }
    }

    let task_results_outcomes = join_all(tasks).await;
    let mut all_individual_trades: Vec<IndividualTradeResult> = Vec::new();
    let mut all_rejected_trades: Vec<RejectedTradeResult> = Vec::new();
    let mut detailed_summaries: Vec<SymbolTimeframeSummary> = Vec::new();

    for result_outcome in task_results_outcomes {
        match result_outcome {
            Ok((_symbol, _pattern_tf, individual_trades_from_task, summary_from_task, rejected_trades_from_task)) => {
                all_individual_trades.extend(individual_trades_from_task);
                all_rejected_trades.extend(rejected_trades_from_task);
                if summary_from_task.total_trades > 0 { // Only add summary if trades occurred
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
    let mut total_duration_seconds: i64 = 0;

    let mut trades_by_symbol_count: HashMap<String, usize> = HashMap::new();
    let mut trades_by_timeframe_count: HashMap<String, usize> = HashMap::new();
    let mut trades_by_day_count: HashMap<String, usize> = HashMap::new();
    let mut pnl_by_day_pips_sum_of_rounded: HashMap<String, f64> = HashMap::new();
    let mut trades_by_hour_count: HashMap<u32, usize> = HashMap::new();
    let mut pnl_by_hour_pips_sum_of_rounded: HashMap<u32, f64> = HashMap::new();

    for trade in &all_individual_trades { // `trade.pnl_pips` is now corrected for JPY pairs
        overall_total_trades += 1;
        let pnl_this_trade = trade.pnl_pips.unwrap_or(0.0); // This is already adjusted and rounded to 1 decimal
        
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
        if let Some(exit_t) = trade.exit_time {
            let entry_t = trade.entry_time;
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
        let hours = avg_duration.num_hours();
        let minutes = avg_duration.num_minutes() % 60;
        let seconds = avg_duration.num_seconds() % 60;
        if hours > 0 { average_trade_duration_str = format!("{}h {}m {}s", hours, minutes, seconds); }
        else if minutes > 0 { average_trade_duration_str = format!("{}m {}s", minutes, seconds); }
        else { average_trade_duration_str = format!("{}s", seconds); }
    } else { average_trade_duration_str = "N/A".to_string(); }
    debug!("[OverallSummary] Average trade duration: {}", average_trade_duration_str);

    let overall_win_rate_calc = if overall_total_trades > 0 { (overall_winning_trades as f64 / overall_total_trades as f64) * 100.0 } else { 0.0 };
    let overall_profit_factor_calc = if overall_gross_loss_pips_sum_of_rounded > 1e-9 { overall_gross_profit_pips_sum_of_rounded / overall_gross_loss_pips_sum_of_rounded } else if overall_gross_profit_pips_sum_of_rounded > 1e-9 { f64::INFINITY } else { 0.0 };
    
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
        overall_losing_trades: overall_total_trades - overall_winning_trades,
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

    // Create rejection summary if rejected trades were requested
    let (rejected_trades_response, rejection_summary_response) = if show_rejected_trades {
        // ⭐ FILTER REJECTED TRADES TO REQUESTED DATE RANGE
        let requested_start = DateTime::parse_from_rfc3339(&req.0.start_time).unwrap().with_timezone(&Utc);
        let requested_end = DateTime::parse_from_rfc3339(&req.0.end_time).unwrap().with_timezone(&Utc);
        
        let original_rejected_count = all_rejected_trades.len();
        let filtered_rejected_trades: Vec<_> = all_rejected_trades.into_iter()
            .filter(|rejected_trade| {
                rejected_trade.rejection_time >= requested_start && rejected_trade.rejection_time <= requested_end
            })
            .collect();
        debug!("Filtered rejected trades: {} total -> {} in requested date range", 
               original_rejected_count, filtered_rejected_trades.len());
        
        let mut rejections_by_reason = std::collections::HashMap::new();
        let mut rejections_by_symbol = std::collections::HashMap::new();
        
        for rejected_trade in &filtered_rejected_trades {
            *rejections_by_reason.entry(rejected_trade.rejection_reason.clone()).or_insert(0) += 1;
            *rejections_by_symbol.entry(rejected_trade.symbol.clone()).or_insert(0) += 1;
        }
        
        let rejection_summary = RejectionSummary {
            total_rejections: filtered_rejected_trades.len(),
            rejections_by_reason,
            rejections_by_symbol,
        };
        
        (Some(filtered_rejected_trades), Some(rejection_summary))
    } else {
        (None, None)
    };

    // ⭐ BUILD RESPONSE WITH TRADING RULES INCLUDED
    let response_payload = MultiBacktestResponse {
        overall_summary,
        detailed_summaries,
        all_trades: all_individual_trades,
        
        // Rejected trades tracking
        rejected_trades: rejected_trades_response,
        rejection_summary: rejection_summary_response,
        
        // ⭐ ADD THE NEW TRADING RULES FIELDS:
        trading_rules_applied: trading_rules_snapshot,
        environment_variables: environment_snapshot,
        request_parameters: request_params_snapshot,
    };

    Ok(HttpResponse::Ok().json(response_payload))
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
                    warn!("Invalid day string provided: {}", day_str);
                    None
                }
            })
            .collect::<Vec<Weekday>>() // Explicitly collect into Vec<Weekday>
            // Ensure an empty vector is returned if no valid days, rather than None, if that's preferred.
            // Or, if the vec is empty after filter_map, then map it to None if parsed_allowed_days must be None.
            // Current behavior: if day_strings is Some but all are invalid, parsed_allowed_days will be Some(empty_vec)
            // The check `parsed_allowed_days.is_none()` might need adjustment based on this.
            // For now, the given logic is fine; if all day_strings are invalid, it results in Some([]).
            // The warning "Request included allowed_trade_days, but all entries were invalid or list was empty."
            // should ideally check if the resulting vec is empty, not if parsed_allowed_days is None.
            // However, this is outside the scope of the JPY pip fix.
    })
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