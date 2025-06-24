// tests/backtest.rs
use dotenv::dotenv;
use pattern_detector::api::detect::CandleData;
use pattern_detector::zones::patterns::FiftyPercentBeforeBigBarRecognizer;
use pattern_detector::zones::patterns::PatternRecognizer;
use pattern_detector::trading::trades::TradeConfig;
use pattern_detector::trading::trading::TradeExecutor;
use chrono::{DateTime, Utc};

#[tokio::test]
async fn run_backtest() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenv().ok();
    // Parse command-line arguments
    // In your test file
    let lot_size = std::env::var("LOT_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.01);
    let take_profit_pips = std::env::var("TP_PIPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20.0);
    let stop_loss_pips = std::env::var("SL_PIPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10.0);
    // Default values

    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG must be set");
    let token = std::env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN must be set");
    let bucket = std::env::var("INFLUXDB_BUCKET").expect("INFLUXDB_BUCKET must be set");

    // Set the symbol and timeframe parameters
    // let symbol = "GBPUSD_SB";
    // let hourly_timeframe = "1h";
    // let minute_timeframe = "1m";
    // let start_time = "2025-02-17T00:00:00Z";
    // let end_time = "2025-03-21T23:59:59Z";
    let symbol = std::env::var("SYMBOL").expect("SYMBOL must be set");
    let hourly_timeframe = std::env::var("HOURLY_TIMEFRAME").expect("HOURLY_TIMEFRAME must be set");
    let minute_timeframe = std::env::var("MINUTE_TIMEFRAME").expect("MINUTE_TIMEFRAME must be set");
    let start_time = std::env::var("START_TIME").expect("START_TIME must be set");
    let end_time = std::env::var("END_TIME").expect("END_TIME must be set");


    // Load hourly data for pattern detection
    println!("Loading hourly data for pattern detection...");
    let hourly_candles = load_candles(
        &host,
        &org,
        &token,
        &bucket,
        &symbol,
        &hourly_timeframe,
        &start_time,
        &end_time,
    )
    .await?;
    println!("Loaded {} hourly candles", hourly_candles.len());

    // Load minute data for precise execution
    println!("Loading 1-minute data for precise execution...");
    let minute_candles = load_candles(
        &host,
        &org,
        &token,
        &bucket,
        &symbol,
        &minute_timeframe,
        &start_time,
        &end_time,
    )
    .await?;
    println!("Loaded {} minute candles", minute_candles.len());

    let start_datetime = DateTime::parse_from_rfc3339(&start_time)
        .unwrap_or_else(|_| DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap())
        .with_timezone(&Utc);

    let end_datetime = DateTime::parse_from_rfc3339(&end_time)
        .unwrap_or_else(|_| DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap())
        .with_timezone(&Utc);

    let duration = end_datetime - start_datetime;
    let total_days = duration.num_days();

    // Create trade config
    let trade_config = TradeConfig {
        enabled: true,
        lot_size: lot_size,
        default_stop_loss_pips: stop_loss_pips,
        default_take_profit_pips: take_profit_pips,
        risk_percent: 1.0,
        max_trades_per_zone: 2,
        ..Default::default()
    };

    // Create a trade executor with minute data
    let default_symbol_for_recognizer_trade = "UNKNOWN_SYMBOL_IN_RECOGNIZER"; 
    let mut trade_executor = TradeExecutor::new(trade_config, 
        default_symbol_for_recognizer_trade,
        None,
        None,
    );
    trade_executor.set_minute_candles(minute_candles); // Set minute data for precise execution

    // Run pattern detection
    let recognizer = FiftyPercentBeforeBigBarRecognizer::default();
    let pattern_data = recognizer.detect(&hourly_candles);

    // Execute trades using the executor
    let trades = trade_executor.execute_trades_for_pattern(
        "fifty_percent_before_big_bar",
        &pattern_data,
        &hourly_candles,
        true
    );

    // Calculate summary from trades
    let summary = pattern_detector::trading::trades::TradeSummary::from_trades(&trades);

    // Calculate total pips directly
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
    let net_pips = total_winning_pips - total_losing_pips;

    // Print results
    println!(
        "\nBacktest Results for 50% Body Before Big Bar Pattern on {} {}:",
        symbol, hourly_timeframe
    );
    println!("Total Trades: {}", summary.total_trades);
    println!("Win Rate: {:.2}%", summary.win_rate);
    println!("Total Profit/Loss: {:.2}", summary.total_profit_loss);
    println!("Total Net Pips: {:.1}", net_pips);
    println!("Profit Factor: {:.2}", summary.profit_factor);
    if summary.total_trades > 0 {
        let avg_profit_per_trade = summary.total_profit_loss / (summary.total_trades as f64);
        println!("Per Trade: {:.2}", avg_profit_per_trade);
    } else {
        println!("Per Trade: 0.00");
    }

    println!("Backtest Period: {} days (from {} to {})", 
         total_days, 
         start_datetime.format("%Y-%m-%d"),
         end_datetime.format("%Y-%m-%d"));

    // Print individual trade details
    println!(
        "\n{:<5} {:<12} {:<28} {:<10} {:<10} {:<15}",
        "No.", "Direction", "Entry Time", "Entry", "P/L pips", "Reason"
    );
    println!("{:-<85}", "");

    for (i, trade) in trades.iter().enumerate() {
        if let (Some(exit_price), Some(exit_time), Some(pl_pips), Some(exit_reason)) = (
            trade.exit_price,
            &trade.exit_time,
            trade.profit_loss_pips,
            &trade.exit_reason,
        ) {
            println!(
                "{:<5} {:<12} {:<28} {:<10.5} {:<10.1} {:<15}",
                i + 1,
                format!("{:?}", trade.direction),
                trade.entry_time,
                trade.entry_price,
                pl_pips,
                exit_reason
            );

            // Debug info for stops and targets
            let pip_size = 0.0001;
            let stop_distance = match trade.direction {
                pattern_detector::trading::trades::TradeDirection::Long => {
                    (trade.entry_price - trade.stop_loss) / pip_size
                }
                pattern_detector::trading::trades::TradeDirection::Short => {
                    (trade.stop_loss - trade.entry_price) / pip_size
                }
            };

            println!(
                "      Stop Loss: {} ({:.1} pips from entry)",
                trade.stop_loss, stop_distance
            );
            println!(
                "      Take Profit: {} ({:.1} pips from entry)",
                trade.take_profit,
                match trade.direction {
                    pattern_detector::trading::trades::TradeDirection::Long =>
                        (trade.take_profit - trade.entry_price) / pip_size,
                    pattern_detector::trading::trades::TradeDirection::Short =>
                        (trade.entry_price - trade.take_profit) / pip_size,
                }
            );
            println!("      Exit Time: {}", exit_time);
        }
    }

    Ok(())
}

// Helper function to load candles (implement this separately)
async fn load_candles(
    host: &str,
    org: &str,
    token: &str,
    bucket: &str,
    symbol: &str,
    timeframe: &str,
    start_time: &str,
    end_time: &str,
) -> Result<Vec<CandleData>, Box<dyn std::error::Error>> {
    use csv::ReaderBuilder;
    use reqwest;
    use serde_json::json;
    use std::io::Cursor;

    // Query construction
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

    // Execute query
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&json!({"query": flux_query, "type": "flux"}))
        .send()
        .await?
        .text()
        .await?;

    // Parse the data
    let mut candles = Vec::new();
    let cursor = Cursor::new(&response);
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(cursor);

    // Parse headers
    let headers = rdr.headers()?;

    // Find column indices
    let mut time_idx = None;
    let mut open_idx = None;
    let mut high_idx = None;
    let mut low_idx = None;
    let mut close_idx = None;
    let mut volume_idx = None;

    for (i, name) in headers.iter().enumerate() {
        match name {
            "_time" => time_idx = Some(i),
            "open" => open_idx = Some(i),
            "high" => high_idx = Some(i),
            "low" => low_idx = Some(i),
            "close" => close_idx = Some(i),
            "volume" => volume_idx = Some(i),
            _ => {}
        }
    }

    // Parse records
    for result in rdr.records() {
        let record = result?;
        if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) =
            (time_idx, open_idx, high_idx, low_idx, close_idx, volume_idx)
        {
            if let Some(time_val) = record.get(t_idx) {
                let open = record
                    .get(o_idx)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let high = record
                    .get(h_idx)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let low = record
                    .get(l_idx)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let close = record
                    .get(c_idx)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let volume = record
                    .get(v_idx)
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(0);

                candles.push(CandleData {
                    time: time_val.to_string(),
                    open,
                    high,
                    low,
                    close,
                    volume,
                });
            }
        }
    }

    Ok(candles)
}
