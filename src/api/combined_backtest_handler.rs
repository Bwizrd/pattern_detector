// src/api/combined_backtest_handler.rs

use crate::api::detect::CandleData;
use crate::trading::backtest::backtest_api::{
    IndividualTradeResult, OverallBacktestSummary, SymbolTimeframeSummary, 
    RejectedTradeResult, RejectionSummary, MultiBacktestResponse,
    TradingRulesSnapshot, EnvironmentVariablesSnapshot, RequestParametersSnapshot
};
use actix_web::{error::ErrorBadRequest, error::ErrorInternalServerError, web, HttpResponse, Responder};
use chrono::{DateTime, Utc, Datelike, Timelike};
use csv::ReaderBuilder;
use dotenv::dotenv;
use reqwest;
use serde::Deserialize;
use std::io::Cursor;
use std::collections::HashMap;
use log::{info, warn, debug};

// --- Imports from trading modules ---
use crate::zones::patterns::{
    FiftyPercentBeforeBigBarRecognizer, PriceSmaCrossRecognizer, SpecificTimeEntryRecognizer,
    PatternRecognizer,
};
use crate::trading::trading::TradeExecutor;
use crate::trading::trades::{Trade, TradeConfig};

// --- Request Structure for Combined Backtest ---
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CombinedBacktestParams {
    pub symbols: Vec<String>,  // List of currency pairs to test
    pub pattern_timeframes: Vec<String>,  // Multiple timeframes to test
    pub start_time: String,
    pub end_time: String,
    pub pattern: String,
    
    // Trading Parameters (optional - fall back to defaults or env vars)
    pub lot_size: Option<f64>,
    pub stop_loss_pips: Option<f64>,
    pub take_profit_pips: Option<f64>,
    
    // Trading Rules (optional - fall back to env vars)
    pub max_daily_trades: Option<u32>,
    pub max_trades_per_symbol: Option<u32>,
    pub max_trades_per_zone: Option<usize>,
    pub min_zone_strength: Option<f64>,
    pub max_touch_count_for_trading: Option<i64>,
    
    // Time/Day Filtering (optional - fall back to env vars)
    pub allowed_trade_days: Option<Vec<String>>,
    pub allowed_symbols: Option<Vec<String>>,
    pub allowed_timeframes: Option<Vec<String>>,
    pub trade_start_hour_utc: Option<u32>,
    pub trade_end_hour_utc: Option<u32>,

    // Rejected trades tracking
    pub show_rejected_trades: Option<bool>,
}

impl CombinedBacktestParams {
    /// Get stop loss pips with fallback to defaults
    pub fn get_stop_loss_pips(&self) -> f64 {
        self.stop_loss_pips.unwrap_or_else(|| {
            std::env::var("TRADING_STOP_LOSS_PIPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(20.0)
        })
    }
    
    /// Get take profit pips with fallback to defaults
    pub fn get_take_profit_pips(&self) -> f64 {
        self.take_profit_pips.unwrap_or_else(|| {
            std::env::var("TRADING_TAKE_PROFIT_PIPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10.0)
        })
    }
    
    /// Get lot size with fallback to defaults
    pub fn get_lot_size(&self) -> f64 {
        self.lot_size.unwrap_or_else(|| {
            std::env::var("TRADING_LOT_SIZE")
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|size| size / 1000.0) // Convert from 1000 to 1.0
                .unwrap_or(0.01)
        })
    }
    
    /// Get max trades per zone with fallback to defaults
    pub fn get_max_trades_per_zone(&self) -> usize {
        self.max_trades_per_zone.unwrap_or_else(|| {
            std::env::var("TRADING_MAX_TRADES_PER_ZONE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3)
        })
    }

    /// Get show rejected trades flag
    pub fn get_show_rejected_trades(&self) -> bool {
        self.show_rejected_trades.unwrap_or(false)
    }
}

// --- Default symbols list ---
const DEFAULT_CURRENCY_PAIRS: &[&str] = &[
    "EURUSD", "GBPUSD", "USDJPY", "USDCHF", 
    "AUDUSD", "USDCAD", "NZDUSD", "EURGBP",
    "EURJPY", "GBPJPY", "AUDJPY", "CHFJPY",
    "EURCHF", "EURAUD", "GBPCHF", "GBPAUD"
];

/// Main handler function for combined backtest
pub async fn run_combined_backtest(
    params: web::Json<CombinedBacktestParams>,
) -> Result<impl Responder, actix_web::Error> {
    let request_params = params.into_inner();
    info!("Received /combined-backtest request: {:?}", request_params);

    // --- Connection Setup & Env Vars ---
    dotenv().ok();
    let host = std::env::var("INFLUXDB_HOST")
        .map_err(|e| ErrorInternalServerError(format!("Server config error (HOST): {}", e)))?;
    let org = std::env::var("INFLUXDB_ORG")
        .map_err(|e| ErrorInternalServerError(format!("Server config error (ORG): {}", e)))?;
    let token = std::env::var("INFLUXDB_TOKEN")
        .map_err(|e| ErrorInternalServerError(format!("Server config error (TOKEN): {}", e)))?;
    let bucket = std::env::var("INFLUXDB_BUCKET")
        .map_err(|e| ErrorInternalServerError(format!("Server config error (BUCKET): {}", e)))?;

    // Use provided symbols or default to all major pairs
    let symbols_to_test = if request_params.symbols.is_empty() {
        DEFAULT_CURRENCY_PAIRS.iter().map(|s| s.to_string()).collect()
    } else {
        request_params.symbols.clone()
    };

    // Validate pattern timeframes
    if request_params.pattern_timeframes.is_empty() {
        return Err(ErrorBadRequest("pattern_timeframes cannot be empty"));
    }

    info!("Testing {} symbols on {} timeframes: {:?}", 
          symbols_to_test.len(), 
          request_params.pattern_timeframes.len(),
          request_params.pattern_timeframes);

    // Validate time range
    let start_dt = DateTime::parse_from_rfc3339(&request_params.start_time)
        .map_err(|e| ErrorBadRequest(format!("Invalid start_time format: {}", e)))?
        .with_timezone(&Utc);
    let end_dt = DateTime::parse_from_rfc3339(&request_params.end_time)
        .map_err(|e| ErrorBadRequest(format!("Invalid end_time format: {}", e)))?
        .with_timezone(&Utc);

    if start_dt >= end_dt {
        return Err(ErrorBadRequest("start_time must be before end_time"));
    }

    let duration = end_dt - start_dt;
    let total_days = duration.num_days().max(1);

    // Collect all results
    let mut all_trades: Vec<IndividualTradeResult> = Vec::new();
    let mut all_rejected_trades: Vec<RejectedTradeResult> = Vec::new();
    let mut detailed_summaries: Vec<SymbolTimeframeSummary> = Vec::new();

    // Process each symbol and timeframe combination using the reliable single backtest logic
    for symbol in &symbols_to_test {
        for timeframe in &request_params.pattern_timeframes {
            info!("Processing symbol: {} on timeframe: {}", symbol, timeframe);
            
            match run_single_symbol_timeframe_backtest(
                symbol,
                timeframe,
                &request_params,
                &host,
                &org,
                &token,
                &bucket,
                start_dt,
                end_dt,
                total_days,
            ).await {
                Ok((trades, rejected_trades, summary)) => {
                    all_trades.extend(trades);
                    all_rejected_trades.extend(rejected_trades);
                    detailed_summaries.push(summary);
                    info!("Successfully processed {} on {}", symbol, timeframe);
                }
                Err(e) => {
                    warn!("Failed to process symbol {} on {}: {}", symbol, timeframe, e);
                    // Add empty summary for failed symbol/timeframe combination
                    detailed_summaries.push(SymbolTimeframeSummary {
                        symbol: symbol.clone(),
                        timeframe: timeframe.clone(),
                        total_trades: 0,
                        winning_trades: 0,
                        losing_trades: 0,
                        win_rate_percent: 0.0,
                        total_pnl_pips: 0.0,
                        profit_factor: 0.0,
                    });
                }
            }
        }
    }

    // Calculate overall summary
    let overall_summary = calculate_overall_summary(&all_trades, total_days);
    
    // Calculate rejection summary if requested
    let (rejected_trades_response, rejection_summary) = if request_params.get_show_rejected_trades() {
        let rejection_summary = calculate_rejection_summary(&all_rejected_trades);
        (Some(all_rejected_trades), Some(rejection_summary))
    } else {
        (None, None)
    };

    // Create trading rules snapshot
    let trading_rules = create_trading_rules_snapshot(&request_params);
    let environment_vars = create_environment_snapshot();
    let request_snapshot = create_request_snapshot(&request_params);

    let response = MultiBacktestResponse {
        overall_summary,
        detailed_summaries,
        all_trades,
        rejected_trades: rejected_trades_response,
        rejection_summary,
        trading_rules_applied: trading_rules,
        environment_variables: environment_vars,
        request_parameters: request_snapshot,
    };

    info!(
        "Combined backtest completed: {} total trades, {:.2} total PnL pips, {:.2}% win rate",
        response.overall_summary.total_trades,
        response.overall_summary.total_pnl_pips,
        response.overall_summary.overall_win_rate_percent
    );

    Ok(HttpResponse::Ok().json(response))
}

/// Process a single symbol and timeframe using the reliable single backtest logic
async fn run_single_symbol_timeframe_backtest(
    symbol: &str,
    timeframe: &str,
    params: &CombinedBacktestParams,
    host: &str,
    org: &str,
    token: &str,
    bucket: &str,
    _start_dt: DateTime<Utc>,
    _end_dt: DateTime<Utc>,
    _total_days: i64,
) -> Result<(Vec<IndividualTradeResult>, Vec<RejectedTradeResult>, SymbolTimeframeSummary), Box<dyn std::error::Error>> {
    
    // Add _SB suffix if not present (for InfluxDB)
    let db_symbol = if symbol.ends_with("_SB") {
        symbol.to_string()
    } else {
        format!("{}_SB", symbol)
    };

    info!("Loading data for symbol: {} (DB: {})", symbol, db_symbol);

    // --- 1. Load Primary Timeframe Data ---
    let primary_candles = load_candles_from_influx(
        host, org, token, bucket,
        &db_symbol,
        timeframe,
        &params.start_time,
        &params.end_time,
    ).await?;

    if primary_candles.is_empty() {
        warn!("No primary candles found for {} on {}", symbol, timeframe);
        return Ok((Vec::new(), Vec::new(), SymbolTimeframeSummary {
            symbol: symbol.to_string(),
            timeframe: timeframe.to_string(),
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate_percent: 0.0,
            total_pnl_pips: 0.0,
            profit_factor: 0.0,
        }));
    }

    info!("Loaded {} primary candles for {}", primary_candles.len(), symbol);

    // --- 2. Load 1-minute Data for Precise Execution ---
    let minute_candles = load_candles_from_influx(
        host, org, token, bucket,
        &db_symbol,
        "1m",
        &params.start_time,
        &params.end_time,
    ).await.unwrap_or_else(|e| {
        warn!("Failed to load 1m candles for {}: {}", symbol, e);
        Vec::new()
    });

    if !minute_candles.is_empty() {
        info!("Loaded {} minute candles for {}", minute_candles.len(), symbol);
    }

    // --- 3. Pattern Recognition ---
    let pattern_name = &params.pattern;
    let recognizer: Box<dyn PatternRecognizer> = match pattern_name.as_str() {
        "fifty_percent_before_big_bar" => Box::new(FiftyPercentBeforeBigBarRecognizer::default()),
        "price_sma_cross" => Box::new(PriceSmaCrossRecognizer::default()),
        "specific_time_entry" => Box::new(SpecificTimeEntryRecognizer::default()),
        _ => return Err(format!("Unknown pattern: {}", pattern_name).into()),
    };

    let pattern_data = recognizer.detect(&primary_candles);
    info!("Pattern recognition complete for {}: found patterns", symbol);

    // --- 4. Trade Configuration (IDENTICAL TO SINGLE BACKTEST) ---
    let trade_config = TradeConfig {
        enabled: true,
        lot_size: params.get_lot_size(),
        default_stop_loss_pips: params.get_stop_loss_pips(),
        default_take_profit_pips: params.get_take_profit_pips(),
        risk_percent: 1.0,
        max_trades_per_zone: 1, // Same as single backtest default
        min_zone_strength: None,
        max_touch_count: None,
    };

    info!(
        "Trade config for {}: lot_size={}, sl={}, tp={}",
        symbol,
        trade_config.lot_size,
        trade_config.default_stop_loss_pips,
        trade_config.default_take_profit_pips,
    );

    // --- 5. Trade Execution (IDENTICAL TO SINGLE BACKTEST) ---
    let default_symbol_for_recognizer_trade = "UNKNOWN_SYMBOL_IN_RECOGNIZER"; 
    let mut trade_executor = TradeExecutor::new(
        trade_config,
        default_symbol_for_recognizer_trade, // Use same placeholder as single backtest
        None,   // allowed_days
        None,   // trade_end_hour
    );

    if !minute_candles.is_empty() {
        trade_executor.set_minute_candles(minute_candles);
        info!("TradeExecutor configured with 1m data for {}", symbol);
    }

    // Execute trades using the OLD METHOD (use_old_method = true) for consistency
    let trades = trade_executor.execute_trades_for_pattern(
        pattern_name,
        &pattern_data,
        &primary_candles,
        true, // use_old_method = true (same as single backtest)
    );

    info!("Trade execution complete for {}: {} trades found", symbol, trades.len());

    // --- 6. Convert Trades to Individual Results ---
    let mut api_trades = Vec::new();
    for trade in &trades {
        let mut api_trade = convert_trade_to_individual_result(trade);
        api_trade.symbol = symbol.to_string();
        api_trade.timeframe = timeframe.to_string();
        api_trades.push(api_trade);
    }

    // --- 7. Get Rejected Trades ---
    let rejected_trades = if params.get_show_rejected_trades() {
        trade_executor.get_rejected_trades()
    } else {
        Vec::new()
    };

    // --- 8. Calculate Summary ---
    let summary = calculate_symbol_summary(&trades, symbol, timeframe);

    Ok((api_trades, rejected_trades, summary))
}

/// Load candles from InfluxDB using the same logic as single backtest
async fn load_candles_from_influx(
    host: &str,
    org: &str,
    token: &str,
    bucket: &str,
    symbol: &str,
    timeframe: &str,
    start_time: &str,
    end_time: &str,
) -> Result<Vec<CandleData>, Box<dyn std::error::Error>> {
    
    let flux_query = format!(
        r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r["_measurement"] == "trendbar") |> filter(fn: (r) => r["symbol"] == "{}") |> filter(fn: (r) => r["timeframe"] == "{}") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
        bucket, start_time, end_time, symbol, timeframe
    );

    debug!("InfluxDB query for {}/{}: {}", symbol, timeframe, flux_query);

    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response_text = client
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "query": flux_query,
            "type": "flux"
        }))
        .send()
        .await?
        .text()
        .await?;

    debug!("InfluxDB response length for {}/{}: {}", symbol, timeframe, response_text.len());

    parse_influx_csv(&response_text)
}

/// Parse InfluxDB CSV response (same logic as single backtest)
fn parse_influx_csv(response_text: &str) -> Result<Vec<CandleData>, Box<dyn std::error::Error>> {
    let mut candles = Vec::new();
    let cursor = Cursor::new(response_text);
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(cursor);

    let headers = match rdr.headers() {
        Ok(h) => h.clone(),
        Err(e) => {
            if response_text.trim().is_empty() || response_text.trim() == "\r\n\r\n" {
                debug!("Received empty response from InfluxDB");
                return Ok(vec![]);
            }
            return Err(format!("CSV header parsing error: {}", e).into());
        }
    };

    let find_idx = |name: &str| headers.iter().position(|h| h == name);
    let time_idx = find_idx("_time");
    let open_idx = find_idx("open");
    let high_idx = find_idx("high");
    let low_idx = find_idx("low");
    let close_idx = find_idx("close");
    let volume_idx = find_idx("volume");

    if time_idx.is_none() || open_idx.is_none() || high_idx.is_none() || low_idx.is_none() || close_idx.is_none() {
        return Err("Essential OHLC columns not found in InfluxDB response".into());
    }

    for result in rdr.records() {
        match result {
            Ok(record) => {
                let time_val = time_idx.and_then(|idx| record.get(idx));
                let open_val = open_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok()));
                let high_val = high_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok()));
                let low_val = low_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok()));
                let close_val = close_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok()));
                let volume_val = volume_idx
                    .and_then(|idx| record.get(idx).and_then(|v| v.parse::<u32>().ok()))
                    .unwrap_or(0);

                if let (Some(time), Some(open), Some(high), Some(low), Some(close)) =
                    (time_val, open_val, high_val, low_val, close_val)
                {
                    candles.push(CandleData {
                        time: time.to_string(),
                        open,
                        high,
                        low,
                        close,
                        volume: volume_val,
                    });
                }
            }
            Err(e) => {
                warn!("Error parsing CSV record: {}", e);
            }
        }
    }

    Ok(candles)
}


/// Convert a Trade to IndividualTradeResult
fn convert_trade_to_individual_result(trade: &Trade) -> IndividualTradeResult {
    let entry_dt = DateTime::parse_from_rfc3339(&trade.entry_time)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    
    let exit_dt = trade.exit_time.as_ref()
        .and_then(|et| DateTime::parse_from_rfc3339(et).ok())
        .map(|dt| dt.with_timezone(&Utc));

    IndividualTradeResult {
        symbol: "UNKNOWN".to_string(), // Will be set by caller
        timeframe: "UNKNOWN".to_string(), // Will be set by caller
        zone_id: Some(trade.pattern_id.clone()),
        direction: format!("{:?}", trade.direction),
        entry_time: entry_dt,
        entry_price: trade.entry_price,
        exit_time: exit_dt,
        exit_price: trade.exit_price,
        pnl_pips: trade.profit_loss_pips,
        exit_reason: trade.exit_reason.clone(),
        zone_strength: trade.zone_strength,
        touch_count: trade.zone_touch_count,
        entry_day_of_week: Some(entry_dt.weekday().to_string()),
        entry_hour_of_day: Some(entry_dt.hour()),
    }
}

/// Calculate symbol-specific summary
fn calculate_symbol_summary(trades: &[Trade], symbol: &str, timeframe: &str) -> SymbolTimeframeSummary {
    let total_trades = trades.len();
    let winning_trades = trades.iter().filter(|t| {
        t.profit_loss_pips.map_or(false, |pnl| pnl > 0.0)
    }).count();
    let losing_trades = total_trades - winning_trades;
    
    let win_rate = if total_trades > 0 {
        (winning_trades as f64 / total_trades as f64) * 100.0
    } else {
        0.0
    };
    
    let total_pnl = trades.iter()
        .filter_map(|t| t.profit_loss_pips)
        .sum::<f64>();
        
    let total_wins = trades.iter()
        .filter_map(|t| t.profit_loss_pips)
        .filter(|&pnl| pnl > 0.0)
        .sum::<f64>();
        
    let total_losses = trades.iter()
        .filter_map(|t| t.profit_loss_pips)
        .filter(|&pnl| pnl < 0.0)
        .map(|pnl| pnl.abs())
        .sum::<f64>();
    
    let profit_factor = if total_losses > 0.0 {
        total_wins / total_losses
    } else if total_wins > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };

    SymbolTimeframeSummary {
        symbol: symbol.to_string(),
        timeframe: timeframe.to_string(),
        total_trades,
        winning_trades,
        losing_trades,
        win_rate_percent: round_f64(win_rate, 2),
        total_pnl_pips: round_f64(total_pnl, 1),
        profit_factor: round_f64(profit_factor, 2),
    }
}

/// Calculate overall summary across all symbols
fn calculate_overall_summary(trades: &[IndividualTradeResult], _total_days: i64) -> OverallBacktestSummary {
    let total_trades = trades.len();
    let winning_trades = trades.iter().filter(|t| {
        t.pnl_pips.map_or(false, |pnl| pnl > 0.0)
    }).count();
    let losing_trades = total_trades - winning_trades;
    
    let win_rate = if total_trades > 0 {
        (winning_trades as f64 / total_trades as f64) * 100.0
    } else {
        0.0
    };
    
    let total_pnl = trades.iter()
        .filter_map(|t| t.pnl_pips)
        .sum::<f64>();
        
    let total_wins = trades.iter()
        .filter_map(|t| t.pnl_pips)
        .filter(|&pnl| pnl > 0.0)
        .sum::<f64>();
        
    let total_losses = trades.iter()
        .filter_map(|t| t.pnl_pips)
        .filter(|&pnl| pnl < 0.0)
        .map(|pnl| pnl.abs())
        .sum::<f64>();
    
    let profit_factor = if total_losses > 0.0 {
        total_wins / total_losses
    } else if total_wins > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };

    // Calculate average trade duration
    let mut durations_hours = Vec::new();
    for trade in trades {
        if let Some(exit_time) = &trade.exit_time {
            if let Ok(duration) = exit_time.signed_duration_since(trade.entry_time).to_std() {
                durations_hours.push(duration.as_secs_f64() / 3600.0);
            }
        }
    }
    
    let avg_duration_hours = if !durations_hours.is_empty() {
        durations_hours.iter().sum::<f64>() / durations_hours.len() as f64
    } else {
        0.0
    };
    
    let avg_duration_str = if avg_duration_hours >= 24.0 {
        format!("{:.1} days", avg_duration_hours / 24.0)
    } else {
        format!("{:.1} hours", avg_duration_hours)
    };

    // Calculate distribution statistics
    let mut trades_by_symbol = HashMap::new();
    let mut trades_by_timeframe = HashMap::new();
    let mut trades_by_day_of_week = HashMap::new();
    let mut trades_by_hour_of_day = HashMap::new();
    let mut pnl_by_day_of_week = HashMap::new();
    let mut pnl_by_hour_of_day = HashMap::new();

    for trade in trades {
        // Symbol distribution
        *trades_by_symbol.entry(trade.symbol.clone()).or_insert(0.0) += 1.0;
        
        // Timeframe distribution
        *trades_by_timeframe.entry(trade.timeframe.clone()).or_insert(0.0) += 1.0;
        
        // Day of week distribution
        if let Some(day) = &trade.entry_day_of_week {
            *trades_by_day_of_week.entry(day.clone()).or_insert(0.0) += 1.0;
            *pnl_by_day_of_week.entry(day.clone()).or_insert(0.0) += trade.pnl_pips.unwrap_or(0.0);
        }
        
        // Hour of day distribution
        if let Some(hour) = trade.entry_hour_of_day {
            *trades_by_hour_of_day.entry(hour).or_insert(0.0) += 1.0;
            *pnl_by_hour_of_day.entry(hour).or_insert(0.0) += trade.pnl_pips.unwrap_or(0.0);
        }
    }

    // Convert counts to percentages
    let total_trades_f64 = total_trades as f64;
    for (_, count) in trades_by_symbol.iter_mut() {
        *count = (*count / total_trades_f64) * 100.0;
    }
    for (_, count) in trades_by_timeframe.iter_mut() {
        *count = (*count / total_trades_f64) * 100.0;
    }
    for (_, count) in trades_by_day_of_week.iter_mut() {
        *count = (*count / total_trades_f64) * 100.0;
    }
    for (_, count) in trades_by_hour_of_day.iter_mut() {
        *count = (*count / total_trades_f64) * 100.0;
    }

    OverallBacktestSummary {
        total_trades,
        total_pnl_pips: round_f64(total_pnl, 2),
        overall_win_rate_percent: round_f64(win_rate, 2),
        overall_profit_factor: round_f64(profit_factor, 2),
        average_trade_duration_str: avg_duration_str,
        overall_winning_trades: winning_trades,
        overall_losing_trades: losing_trades,
        trades_by_symbol_percent: trades_by_symbol,
        trades_by_timeframe_percent: trades_by_timeframe,
        trades_by_day_of_week_percent: trades_by_day_of_week,
        pnl_by_day_of_week_pips: pnl_by_day_of_week,
        trades_by_hour_of_day_percent: trades_by_hour_of_day,
        pnl_by_hour_of_day_pips: pnl_by_hour_of_day,
    }
}

/// Calculate rejection summary
fn calculate_rejection_summary(rejected_trades: &[RejectedTradeResult]) -> RejectionSummary {
    let total_rejections = rejected_trades.len();
    let mut rejections_by_reason = HashMap::new();
    let mut rejections_by_symbol = HashMap::new();

    for rejection in rejected_trades {
        *rejections_by_reason.entry(rejection.rejection_reason.clone()).or_insert(0) += 1;
        *rejections_by_symbol.entry(rejection.symbol.clone()).or_insert(0) += 1;
    }

    RejectionSummary {
        total_rejections,
        rejections_by_reason,
        rejections_by_symbol,
    }
}

/// Create trading rules snapshot
fn create_trading_rules_snapshot(params: &CombinedBacktestParams) -> TradingRulesSnapshot {
    TradingRulesSnapshot {
        trading_enabled: true,
        lot_size: params.get_lot_size(),
        default_stop_loss_pips: params.get_stop_loss_pips(),
        default_take_profit_pips: params.get_take_profit_pips(),
        max_trades_per_zone: params.get_max_trades_per_zone(),
        trading_max_daily_trades: None,
        trading_max_trades_per_symbol: None,
        trading_min_zone_strength: None,
        max_touch_count_for_trading: None,
        trading_max_trades_per_zone_env: None,
        trading_allowed_symbols: None,
        trading_allowed_timeframes: None,
        trading_allowed_weekdays: None,
        trading_start_hour_utc: None,
        trading_end_hour_utc: None,
    }
}

/// Create environment variables snapshot
fn create_environment_snapshot() -> EnvironmentVariablesSnapshot {
    EnvironmentVariablesSnapshot {
        trading_max_daily_trades: std::env::var("TRADING_MAX_DAILY_TRADES").ok(),
        trading_max_trades_per_symbol: std::env::var("TRADING_MAX_TRADES_PER_SYMBOL").ok(),
        trading_max_trades_per_zone: std::env::var("TRADING_MAX_TRADES_PER_ZONE").ok(),
        trading_min_zone_strength: std::env::var("TRADING_MIN_ZONE_STRENGTH").ok(),
        max_touch_count_for_trading: std::env::var("MAX_TOUCH_COUNT_FOR_TRADING").ok(),
        trading_allowed_symbols: std::env::var("TRADING_ALLOWED_SYMBOLS").ok(),
        trading_allowed_timeframes: std::env::var("TRADING_ALLOWED_TIMEFRAMES").ok(),
        trading_allowed_weekdays: std::env::var("TRADING_ALLOWED_WEEKDAYS").ok(),
        trading_start_hour_utc: std::env::var("TRADING_START_HOUR").ok(),
        trading_end_hour_utc: std::env::var("TRADING_END_HOUR").ok(),
        trading_enabled: std::env::var("TRADING_ENABLED").ok(),
    }
}

/// Create request parameters snapshot
fn create_request_snapshot(params: &CombinedBacktestParams) -> RequestParametersSnapshot {
    RequestParametersSnapshot {
        symbols: params.symbols.clone(),
        pattern_timeframes: params.pattern_timeframes.clone(),
        start_time: params.start_time.clone(),
        end_time: params.end_time.clone(),
        lot_size: params.lot_size,
        stop_loss_pips: params.stop_loss_pips,
        take_profit_pips: params.take_profit_pips,
        max_daily_trades: params.max_daily_trades,
        max_trades_per_symbol: params.max_trades_per_symbol,
        max_trades_per_zone: params.max_trades_per_zone,
        min_zone_strength: params.min_zone_strength,
        max_touch_count_for_trading: params.max_touch_count_for_trading,
        allowed_trade_days: params.allowed_trade_days.clone(),
        allowed_symbols: params.allowed_symbols.clone(),
        allowed_timeframes: params.allowed_timeframes.clone(),
        trade_start_hour_utc: params.trade_start_hour_utc,
        trade_end_hour_utc: params.trade_end_hour_utc,
    }
}

/// Round f64 to specified decimal places
fn round_f64(value: f64, dp: u32) -> f64 {
    if value.is_nan() || value.is_infinite() {
        return 0.0;
    }
    let multiplier = 10f64.powi(dp as i32);
    (value * multiplier).round() / multiplier
}
