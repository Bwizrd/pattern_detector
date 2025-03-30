// src/backtest.rs

use crate::detect::CandleData; // Use CandleData defined in detect.rs
use actix_web::{
    error::ErrorBadRequest, error::ErrorInternalServerError, web, HttpResponse, Responder,
};
use chrono::{DateTime, Utc};
use csv::ReaderBuilder;
use dotenv::dotenv;
use reqwest;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

// --- Imports from your project ---
// Adjust paths if necessary based on your module structure (`crate::` assumes they are accessible from the root)
use crate::patterns::{
    FiftyPercentBeforeBigBarRecognizer,
    PriceSmaCrossRecognizer,
    SpecificTimeEntryRecognizer,
     // Example recognizer
    PatternRecognizer,                  // The trait
                                        // Add imports for other recognizers you want to support here
};
use crate::trading::TradeExecutor; // Correct path for TradeExecutor
use crate::trades::{Trade, TradeConfig, TradeSummary}; // Assuming these are in src/trades.rs

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
    pub total_profit_loss: f64,
    pub profit_factor: f64,
    pub net_pips: f64,
    pub avg_pips_per_trade: f64,
    pub total_winning_pips: f64,
    pub total_losing_pips: f64,
    // Add parameters used
    pub lot_size: f64,
    pub stop_loss_pips: f64,
    pub take_profit_pips: f64,
    pub winning_trades: usize,
    pub losing_trades: usize,
}

// Updated From impl to accept params
impl From<(&TradeSummary, f64, f64, f64, f64, f64, f64)> for ApiTradeSummary {
    fn from(data: (&TradeSummary, f64, f64, f64, f64, f64, f64)) -> Self {
        let (summary, net_pips, total_winning_pips, total_losing_pips, lot_size, sl_pips, tp_pips) = data;

        let avg_pips = if summary.total_trades > 0 {
            net_pips / summary.total_trades as f64
        } else {
            0.0
        };
        let total_profit_loss_rounded = round_dp(summary.total_profit_loss, 2);
        let win_rate_rounded = round_dp(summary.win_rate, 2);
        let profit_factor_rounded = round_dp(summary.profit_factor, 2);

        ApiTradeSummary {
            total_trades: summary.total_trades,
            win_rate: win_rate_rounded,
            total_profit_loss: total_profit_loss_rounded,
            profit_factor: profit_factor_rounded,
            net_pips: net_pips,                        // Already rounded
            avg_pips_per_trade: round_dp(avg_pips, 2), // Round avg pips
            total_winning_pips: total_winning_pips,    // Already rounded
            total_losing_pips: total_losing_pips,      // Already rounded
            // Include params
            lot_size: lot_size,
            stop_loss_pips: sl_pips,
            take_profit_pips: tp_pips,
            winning_trades: summary.winning_trades,
            losing_trades: summary.losing_trades,
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
    pub summary: ApiTradeSummary,
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

    // Validate times
    let start_dt = DateTime::parse_from_rfc3339(start_time_str)
        .map_err(|e| ErrorBadRequest(format!("Invalid start_time format: {}", e)))?
        .with_timezone(&Utc);
    let end_dt = DateTime::parse_from_rfc3339(end_time_str)
        .map_err(|e| ErrorBadRequest(format!("Invalid end_time format: {}", e)))?
        .with_timezone(&Utc);
    if start_dt >= end_dt {
        return Err(ErrorBadRequest("start_time must be before end_time"));
    }
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
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new(); // Create client once for reuse
    let response_text_primary = client
        .post(&url)
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

    if primary_candles.is_empty() {
        log::warn!("No primary candle data found for the specified parameters.");
        // Construct empty TradeSummary correctly
        let empty_summary = TradeSummary {
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            total_profit_loss: 0.0,
            win_rate: 0.0,
            average_win: 0.0,
            average_loss: 0.0,
            profit_factor: 0.0,
            max_drawdown: 0.0,
        };
        let (net_pips, win_pips, lose_pips) = (0.0, 0.0, 0.0);
        // Pass params to From impl
        let api_summary = ApiTradeSummary::from((
            &empty_summary,
            net_pips,
            win_pips,
            lose_pips,
            request_params.lot_size,
            request_params.stop_loss_pips,
            request_params.take_profit_pips,
        ));
        let response = BacktestResponse {
            summary: api_summary,
            trades: vec![],
            start_time: start_dt.to_rfc3339(),
            end_time: end_dt.to_rfc3339(),
            total_days: (end_dt - start_dt).num_days(),
        };
        return Ok(HttpResponse::Ok().json(response));
    }
    // --- End Primary Data Loading ---

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
        .post(&url)
        .bearer_auth(&token)
        .json(&serde_json::json!({"query": flux_query_minute, "type": "flux"}))
        .send()
        .await
        .map_err(|e| ErrorInternalServerError(format!("DB request failed (1m): {}", e)))? // Correct map_err
        .text()
        .await
        .map_err(|e| ErrorInternalServerError(format!("DB response read failed (1m): {}", e)))?; // Correct map_err

    // Use imported CandleData type
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
            Box::new(PriceSmaCrossRecognizer::default())
        },
        "specific_time_entry" => Box::new(SpecificTimeEntryRecognizer::default()),
        
        // Add other patterns here...
        _ => {
            let error_msg = format!("Unsupported pattern for backtesting: {}", pattern_name);
            log::error!("{}", error_msg);
            return Err(ErrorBadRequest(error_msg));
        }
    };

    log::info!("Using pattern recognizer: {}", pattern_name);

    // --- 2. Run Pattern Detection ---
    let pattern_data = recognizer.detect(&primary_candles); // Use primary_candles
    log::debug!("Pattern detection complete.");

    // --- 3. Create TradeConfig ---
    let trade_config = TradeConfig {
        enabled: true,
        lot_size: request_params.lot_size,
        default_stop_loss_pips: request_params.stop_loss_pips,
        default_take_profit_pips: request_params.take_profit_pips,
        risk_percent: 1.0,           // Match test if relevant
        max_trades_per_pattern: 2,   // Match test value
        enable_trailing_stop: false, // Match test if relevant
        ..Default::default()
    };
    log::info!(
        "Trade config created: MaxTrades={}, SL={} pips, TP={} pips, Lot={}",
        trade_config.max_trades_per_pattern,
        trade_config.default_stop_loss_pips,
        trade_config.default_take_profit_pips,
        trade_config.lot_size
    );

    // --- 4. Instantiate and Execute Trades ---
    let mut trade_executor = TradeExecutor::new(trade_config);

    // Set the minute candles if available
    if !minute_candles.is_empty() {
        trade_executor.set_minute_candles(minute_candles);
        log::info!("TradeExecutor configured with 1m data for precise execution.");
    } else {
        log::warn!("Proceeding with trade execution based on primary timeframe ({}) candles due to missing 1m data.", primary_timeframe);
    }

    // Execute trades using primary candles for signals
    let trades = trade_executor.execute_trades_for_pattern(
        pattern_name,
        &pattern_data,
        &primary_candles, // Use primary_candles variable
    );
    log::info!(
        "Trade execution simulation complete. Found {} potential trades.",
        trades.len()
    );

    // --- 5. Calculate Results ---
    let summary = TradeSummary::from_trades(&trades);
    let (net_pips, total_winning_pips, total_losing_pips) = calculate_pips(&trades);

    // --- 6. Format Response ---
    let api_summary = ApiTradeSummary::from((
        &summary,
        net_pips,
        total_winning_pips,
        total_losing_pips,
        request_params.lot_size, // Pass params to From impl
        request_params.stop_loss_pips,
        request_params.take_profit_pips,
    ));
    let api_trades: Vec<ApiTrade> = trades.iter().map(ApiTrade::from).collect();
    let duration = end_dt - start_dt;
    let total_days = duration.num_days();

    let response = BacktestResponse {
        summary: api_summary,
        trades: api_trades,
        start_time: start_dt.to_rfc3339(),
        end_time: end_dt.to_rfc3339(),
        total_days,
    };
    log::info!(
        "Backtest results calculated: Trades={}, Net Pips={:.1}",
        response.summary.total_trades,
        response.summary.net_pips
    );

    // --- 7. Return Final Response ---
    Ok(HttpResponse::Ok().json(response))
}

// --- Helper Function to Parse CSV ---
// Ensures return type uses imported CandleData
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
        return Err(ErrorInternalServerError(
            "Unexpected data format (missing columns).",
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
                let volume_val = volume_idx
                    .and_then(|idx| record.get(idx).and_then(|v| v.parse::<u32>().ok()))
                    .unwrap_or(0);

                if let (Some(time), Some(open), Some(high), Some(low), Some(close)) =
                    (time_val, open_val, high_val, low_val, close_val)
                {
                    // Construct using the imported CandleData type
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
                        "Skipping record due to missing fields/parse error: {:?}",
                        record
                    );
                }
            }
            Err(e) => {
                log::error!("Error parsing CSV record: {}", e);
            }
        }
    }
    Ok(candles)
}

// Helper function to round f64 to a specific number of decimal places
fn round_dp(value: f64, dp: u32) -> f64 {
    let multiplier = 10f64.powi(dp as i32);
    (value * multiplier).round() / multiplier
}

// --- Helper Function to Calculate Pips ---
fn calculate_pips(trades: &[Trade]) -> (f64, f64, f64) {
    // (net, winning, losing)
    let mut total_winning_pips = 0.0;
    let mut total_losing_pips = 0.0;
    for trade in trades {
        if let Some(pl_pips) = trade.profit_loss_pips {
            if pl_pips > 0.0 {
                total_winning_pips += pl_pips;
            } else {
                total_losing_pips += pl_pips.abs();
            }
        }
    }
    let net_pips = total_winning_pips - total_losing_pips;
    (
        round_dp(net_pips, 1),
        round_dp(total_winning_pips, 1),
        round_dp(total_losing_pips, 1),
    )
}