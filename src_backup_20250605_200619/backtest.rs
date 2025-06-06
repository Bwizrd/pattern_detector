// src/backtest.rs

use crate::api::detect::CandleData; // Use CandleData defined in detect.rs
use actix_web::{
    error::ErrorBadRequest, error::ErrorInternalServerError, web, HttpResponse, Responder,
};
// Import necessary chrono types
use chrono::{DateTime, Utc};
use csv::ReaderBuilder;
use dotenv::dotenv;
use reqwest;
use serde::{Deserialize, Serialize};
// Remove BTreeMap if no longer needed elsewhere in this file
// use std::collections::BTreeMap;
use std::io::Cursor;

// --- Imports from your project ---
// Adjust paths if necessary based on your module structure (`crate::` assumes they are accessible from the root)
use crate::zones::patterns::{
    FiftyPercentBeforeBigBarRecognizer, PriceSmaCrossRecognizer, SpecificTimeEntryRecognizer,
    PatternRecognizer, // The trait
};
use crate::trading::trading::TradeExecutor; // Correct path for TradeExecutor
use crate::trading::trades::{Trade, TradeConfig, TradeSummary}; // Assuming these are in src/trades.rs

// --- Request Structure (BacktestParams) ---
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BacktestParams {
    pub symbol: String,
    pub timeframe: String,
    pub start_time: String,
    pub end_time: String,
    pub pattern: String,
    pub lot_size: f64,
    pub stop_loss_pips: f64,
    pub take_profit_pips: f64,
}

// --- Response Structures ---
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApiTradeSummary {
    pub total_trades: usize,
    pub win_rate: f64,
    pub total_profit_loss: f64, // Represents net pips in this simplified version
    pub profit_factor: f64,
    pub net_pips: f64,
    pub avg_pips_per_trade: f64,
    pub total_winning_pips: f64,
    pub total_losing_pips: f64,
    pub lot_size: f64,
    pub stop_loss_pips: f64,
    pub take_profit_pips: f64,
    pub winning_trades: usize,
    pub losing_trades: usize,
    // Added field for average pips per day
    pub avg_pips_per_day: f64,
    pub avg_total_pl_per_day: f64,
    // Removed: daily_profit_loss_pips field
}

// Updated From impl signature to accept total_days (i64)
// Tuple now: (&TradeSummary, net_pips, win_pips, lose_pips, lot_size, sl_pips, tp_pips, total_days)
impl From<(&TradeSummary, f64, f64, f64, f64, f64, f64, i64)> for ApiTradeSummary {
    fn from(data: (&TradeSummary, f64, f64, f64, f64, f64, f64, i64)) -> Self {
        // Destructure the tuple including total_days
        let (summary, net_pips, total_winning_pips, total_losing_pips, lot_size, sl_pips, tp_pips, total_days) = data;

        let avg_pips_per_trade = if summary.total_trades > 0 {
            net_pips / summary.total_trades as f64
        } else {
            0.0
        };

        // Calculate average pips per day
        let avg_pips_per_day = if total_days > 0 {
            net_pips / total_days as f64
        } else {
            0.0 // Avoid division by zero if backtest is less than a day (or total_days is 0)
        };

        // *** CALCULATE AVERAGE BASED ON summary.total_profit_loss ***
        // This uses the value directly from the TradeSummary struct, whatever it represents (pips or currency).
        let avg_total_pl_per_day = if total_days > 0 {
            // Use summary.total_profit_loss directly from the struct passed in
           summary.total_profit_loss / total_days as f64
       } else {
           0.0
       };

        ApiTradeSummary {
            total_trades: summary.total_trades,
            win_rate: round_dp(summary.win_rate, 2),
            // Setting total_profit_loss based on net_pips for consistency here
            total_profit_loss: round_dp(net_pips, 2),
            profit_factor: round_dp(summary.profit_factor, 2),
            net_pips: net_pips, // Already rounded in calculate_pips
            avg_pips_per_trade: round_dp(avg_pips_per_trade, 2),
            total_winning_pips: total_winning_pips, // Already rounded
            total_losing_pips: total_losing_pips, // Already rounded
            lot_size: lot_size,
            stop_loss_pips: sl_pips,
            take_profit_pips: tp_pips,
            winning_trades: summary.winning_trades,
            losing_trades: summary.losing_trades,
            // Include the calculated average pips per day, rounded
            avg_pips_per_day: round_dp(avg_pips_per_day, 2),
            avg_total_pl_per_day: round_dp(avg_total_pl_per_day, 2), // Add the new field
        }
    }
}


#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApiTrade {
    pub entry_time: String,
    pub entry_price: f64,
    pub exit_time: Option<String>,
    pub exit_price: Option<f64>,
    pub direction: String,
    pub profit_loss_pips: Option<f64>,
    pub exit_reason: Option<String>,
    pub stop_loss: f64,
    pub take_profit: f64,
}

impl From<&Trade> for ApiTrade {
    fn from(trade: &Trade) -> Self {
        const PRICE_DP: u32 = 5; // Define decimal places for prices

        ApiTrade {
            entry_time: trade.entry_time.clone(), // Clone existing string
            entry_price: round_dp(trade.entry_price, PRICE_DP),
            exit_time: trade.exit_time.clone(), // Clone existing Option<String>
            exit_price: trade.exit_price.map(|price| round_dp(price, PRICE_DP)),
            direction: format!("{:?}", trade.direction),
            profit_loss_pips: trade.profit_loss_pips.map(|pips| round_dp(pips, 1)),
            exit_reason: trade.exit_reason.clone(),
            stop_loss: round_dp(trade.stop_loss, PRICE_DP),
            take_profit: round_dp(trade.take_profit, PRICE_DP),
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResponse {
    pub summary: ApiTradeSummary, // This struct now contains avg_pips_per_day
    pub trades: Vec<ApiTrade>,
    pub start_time: String,
    pub end_time: String,
    pub total_days: i64,
}
// --- End Response Structures ---

// --- Handler Function ---
pub async fn run_backtest(
    params: web::Json<BacktestParams>,
) -> Result<impl Responder, actix_web::Error> {
    let request_params = params.into_inner();
    log::info!("Received /api/backtest request: {:?}", request_params);

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

    let symbol = if request_params.symbol.ends_with("_SB") {
        request_params.symbol.clone()
    } else {
        format!("{}_SB", request_params.symbol)
    };

    let start_time_str = &request_params.start_time;
    let end_time_str = &request_params.end_time;

    // Validate times and calculate total_days
    let start_dt = DateTime::parse_from_rfc3339(start_time_str)
        .map_err(|e| ErrorBadRequest(format!("Invalid start_time format: {}", e)))?
        .with_timezone(&Utc);
    let end_dt = DateTime::parse_from_rfc3339(end_time_str)
        .map_err(|e| ErrorBadRequest(format!("Invalid end_time format: {}", e)))?
        .with_timezone(&Utc);
    if start_dt >= end_dt {
        return Err(ErrorBadRequest("start_time must be before end_time"));
    }
    // Calculate duration and ensure total_days is at least 1
    let duration = end_dt - start_dt;
    let total_days = duration.num_days().max(1); // Use .max(1) to prevent division by zero

    // Basic validation for trade params
    if request_params.lot_size <= 0.0
        || request_params.stop_loss_pips < 0.0
        || request_params.take_profit_pips < 0.0
    {
        return Err(ErrorBadRequest(
            "Lot size must be positive, SL/TP cannot be negative.".to_string(),
        ));
    }

    // --- Load Primary Timeframe Candles ---
    let primary_timeframe = &request_params.timeframe;
    log::info!("Loading primary candles ({})", primary_timeframe);
    let flux_query_primary = format!(
        r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == "trendbar") |> filter(fn: (r) => r.symbol == "{}") |> filter(fn: (r) => r.timeframe == "{}") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
        bucket, start_time_str, end_time_str, symbol, primary_timeframe
    );

    log::debug!("Executing Primary Flux query: {}", flux_query_primary);
    let client = reqwest::Client::new(); // Create client once for reuse
    let response_text_primary = client
        .post(&format!("{}/api/v2/query?org={}", host, org)) // Use client with URL directly
        .bearer_auth(&token)
        .json(&serde_json::json!({"query": flux_query_primary, "type": "flux"}))
        .send()
        .await
        .map_err(|e| ErrorInternalServerError(format!("DB request failed (primary): {}", e)))?
        .text()
        .await
        .map_err(|e| ErrorInternalServerError(format!("DB response read failed (primary): {}", e)))?;

    // Use imported CandleData type
    let primary_candles: Vec<CandleData> = parse_influx_csv(&response_text_primary)?;
    log::info!(
        "Parsed {} primary candles successfully.",
        primary_candles.len()
    );

    // --- Handle Case: No Primary Candles Found ---
    if primary_candles.is_empty() {
        log::warn!("No primary candle data found for the specified parameters.");
        // Construct empty TradeSummary
        let empty_summary_struct = TradeSummary::default(); // Use the default impl from trades.rs
        let (net_pips, win_pips, lose_pips) = (0.0, 0.0, 0.0);
        // Pass params and total_days (which is >= 1) to From impl
        let api_summary = ApiTradeSummary::from((
            &empty_summary_struct,
            net_pips,
            win_pips,
            lose_pips,
            request_params.lot_size,
            request_params.stop_loss_pips,
            request_params.take_profit_pips,
            total_days, // Pass total_days here
        ));
        let response = BacktestResponse {
            summary: api_summary,
            trades: vec![],
            start_time: start_dt.to_rfc3339(),
            end_time: end_dt.to_rfc3339(),
            total_days: total_days, // Include total_days in response
        };
        return Ok(HttpResponse::Ok().json(response));
    }
    // --- End Primary Data Loading & Empty Check ---


    // --- Load 1-Minute Candles ---
    let minute_timeframe = "1m";
    log::info!(
        "Loading minute candles ({}) for execution",
        minute_timeframe
    );
    let flux_query_minute = format!(
        r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == "trendbar") |> filter(fn: (r) => r.symbol == "{}") |> filter(fn: (r) => r.timeframe == "{}") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
        bucket, start_time_str, end_time_str, symbol, minute_timeframe
    );

    log::debug!("Executing Minute Flux query: {}", flux_query_minute);
    // Reuse the reqwest client
    let response_text_minute = client
        .post(&format!("{}/api/v2/query?org={}", host, org)) // Use client with URL directly
        .bearer_auth(&token)
        .json(&serde_json::json!({"query": flux_query_minute, "type": "flux"}))
        .send()
        .await
        .map_err(|e| ErrorInternalServerError(format!("DB request failed (1m): {}", e)))?
        .text()
        .await
        .map_err(|e| ErrorInternalServerError(format!("DB response read failed (1m): {}", e)))?;

    let minute_candles: Vec<CandleData> = parse_influx_csv(&response_text_minute)?;
    log::info!(
        "Parsed {} minute candles successfully.",
        minute_candles.len()
    );

    if minute_candles.is_empty() {
        log::error!("Execution data (1m candles) is missing for the requested range. Backtest accuracy will be affected.");
        // Proceeding with warning
    }
    // --- End 1-Minute Data Loading ---

    // --- 1. Select Pattern Recognizer ---
    let pattern_name = request_params.pattern.as_str();
    let recognizer: Box<dyn PatternRecognizer> = match pattern_name {
        "fifty_percent_before_big_bar" | "50% Body Before Big Bar" => {
            Box::new(FiftyPercentBeforeBigBarRecognizer::default())
        }
        "price_sma_cross" => {
            Box::new(PriceSmaCrossRecognizer::default()) // Assuming this exists
        },
        "specific_time_entry" => Box::new(SpecificTimeEntryRecognizer::default()), // Assuming this exists
        _ => {
            let error_msg = format!("Unsupported pattern for backtesting: {}", pattern_name);
            log::error!("{}", error_msg);
            return Err(ErrorBadRequest(error_msg));
        }
    };

    log::info!("Using pattern recognizer: {}", pattern_name);

    // --- 2. Run Pattern Detection ---
    let pattern_data = recognizer.detect(&primary_candles);
    log::debug!("Pattern detection complete.");

    // --- 3. Create TradeConfig ---
    let trade_config = TradeConfig {
        enabled: true,
        lot_size: request_params.lot_size,
        default_stop_loss_pips: request_params.stop_loss_pips,
        default_take_profit_pips: request_params.take_profit_pips,
        risk_percent: 1.0,
        // max_trades_per_pattern: 1, // This only works for now ommitted or set to 1, needs a fix in the zone recognizer if more than 1 is wanted
        enable_trailing_stop: false, // Example value, adjust as needed
        ..Default::default() // Use default for other fields
    };
    log::info!(
        "Trade config created: MaxTrades={}, SL={} pips, TP={} pips, Lot={}",
        trade_config.max_trades_per_pattern,
        trade_config.default_stop_loss_pips,
        trade_config.default_take_profit_pips,
        trade_config.lot_size
    );

    // --- 4. Instantiate and Execute Trades ---
    let default_symbol_for_recognizer_trade = "UNKNOWN_SYMBOL_IN_RECOGNIZER"; 
    let mut trade_executor = TradeExecutor::new(trade_config, 
        default_symbol_for_recognizer_trade,
        None, // Pass primary timeframe for context
        None// Pass the original timeframe for context
    );
    if !minute_candles.is_empty() {
        trade_executor.set_minute_candles(minute_candles);
        log::info!("TradeExecutor configured with 1m data for precise execution.");
    } else {
        log::warn!("Proceeding with trade execution based on primary timeframe ({}) candles due to missing 1m data.", primary_timeframe);
    }

    let trades = trade_executor.execute_trades_for_pattern(
        pattern_name,
        &pattern_data,
        &primary_candles,
    );
    log::info!(
        "Trade execution simulation complete. Found {} potential trades.",
        trades.len()
    );

    // --- 5. Calculate Results ---
    let summary_struct = TradeSummary::from_trades(&trades); // Get the underlying summary struct
    let (net_pips, total_winning_pips, total_losing_pips) = calculate_pips(&trades); // Calculate pip stats

    // --- 6. Format Response ---
    // Pass the struct reference, pip stats, params, AND total_days to the From impl
    let api_summary = ApiTradeSummary::from((
        &summary_struct,
        net_pips,
        total_winning_pips,
        total_losing_pips,
        request_params.lot_size,
        request_params.stop_loss_pips,
        request_params.take_profit_pips,
        total_days, // Pass the calculated total_days here
    ));

    let api_trades: Vec<ApiTrade> = trades.iter().map(ApiTrade::from).collect();

    let response = BacktestResponse {
        summary: api_summary, // This summary now includes avg_pips_per_day
        trades: api_trades,
        start_time: start_dt.to_rfc3339(),
        end_time: end_dt.to_rfc3339(),
        total_days: total_days, // Include total_days in response object as well
    };
    log::info!(
        "Backtest results calculated: Trades={}, Net Pips={:.1}, Avg Pips/Day={:.2}",
        response.summary.total_trades,
        response.summary.net_pips,
        response.summary.avg_pips_per_day // Log the newly calculated value
    );

    // --- 7. Return Final Response ---
    Ok(HttpResponse::Ok().json(response))
}

// --- Helper Function to Parse CSV ---
fn parse_influx_csv(response_text: &str) -> Result<Vec<CandleData>, actix_web::Error> {
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
            log::error!("CSV header parsing error: {}", e);
            if response_text.trim().is_empty() || response_text.trim() == "\r\n\r\n" {
                log::warn!("Received empty or minimal response from InfluxDB.");
                return Ok(vec![]);
            }
            return Err(ErrorInternalServerError(format!(
                "Invalid data format (headers): {}",
                e
            )));
        }
    };

    let find_idx = |name: &str| headers.iter().position(|h| h == name);
    let time_idx = find_idx("_time");
    let open_idx = find_idx("open");
    let high_idx = find_idx("high");
    let low_idx = find_idx("low");
    let close_idx = find_idx("close");
    let volume_idx = find_idx("volume");

    if time_idx.is_none()
        || open_idx.is_none()
        || high_idx.is_none()
        || low_idx.is_none()
        || close_idx.is_none()
    {
        log::error!(
            "Essential columns not found in InfluxDB response headers: {:?}",
            headers
        );
        // Check if volume is also missing, log if so, but don't require it
        if volume_idx.is_none() {
             log::warn!("'volume' column not found in InfluxDB response headers. Volume will be 0.");
        }
        return Err(ErrorInternalServerError(
            "Unexpected data format (missing essential OHLC columns).",
        ));
    }

    for result in rdr.records() {
        match result {
            Ok(record) => {
                let time_val = time_idx.and_then(|idx| record.get(idx));
                let open_val =
                    open_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok()));
                let high_val =
                    high_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok()));
                let low_val =
                    low_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok()));
                let close_val =
                    close_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok()));
                // Handle volume optionally - default to 0 if missing or parse fails
                let volume_val = volume_idx
                    .and_then(|idx| record.get(idx).and_then(|v| v.parse::<u32>().ok()))
                    .unwrap_or(0);

                if let (Some(time), Some(open), Some(high), Some(low), Some(close)) =
                    (time_val, open_val, high_val, low_val, close_val)
                {
                    candles.push(crate::detect::CandleData {
                        time: time.to_string(),
                        open,
                        high,
                        low,
                        close,
                        volume: volume_val,
                    });
                } else {
                    log::warn!(
                        "Skipping record due to missing essential OHLC fields or parse error: {:?}",
                        record
                    );
                }
            }
            Err(e) => {
                log::error!("Error parsing CSV record: {}", e);
                // Consider whether to continue or return an error
            }
        }
    }
    Ok(candles)
}

// Helper function to round f64 to a specific number of decimal places
fn round_dp(value: f64, dp: u32) -> f64 {
    // Handle NaN and Infinity to prevent propagation
    if value.is_nan() || value.is_infinite() {
        return 0.0; // Or return the original value, or handle as needed
    }
    let multiplier = 10f64.powi(dp as i32);
    (value * multiplier).round() / multiplier
}

// --- Helper Function to Calculate Pips ---
// Calculates net pips, total winning pips, and total losing pips (absolute value)
fn calculate_pips(trades: &[Trade]) -> (f64, f64, f64) {
    // Returns tuple: (net_pips, total_winning_pips, total_losing_pips)
    let mut total_winning_pips = 0.0;
    let mut total_losing_pips = 0.0; // Stores the sum of absolute losing pips

    for trade in trades {
        if let Some(pl_pips) = trade.profit_loss_pips {
            if pl_pips > 0.0 {
                total_winning_pips += pl_pips;
            } else if pl_pips < 0.0 {
                // Add the absolute value of the loss
                total_losing_pips += pl_pips.abs();
            }
            // Trades with exactly 0.0 pips are ignored in win/loss sums
        }
    }

    let net_pips = total_winning_pips - total_losing_pips;

    (
        round_dp(net_pips, 1),          // Round net pips
        round_dp(total_winning_pips, 1), // Round total winning pips
        round_dp(total_losing_pips, 1),  // Round total losing pips
    )
}