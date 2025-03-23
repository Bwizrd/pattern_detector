use pattern_detector::detect::CandleData;
use pattern_detector::patterns::FiftyPercentBeforeBigBarRecognizer;
use pattern_detector::patterns::PatternRecognizer;
use pattern_detector::trades::TradeConfig;
use pattern_detector::trades::TradeDirection;
use dotenv::dotenv;
use reqwest;
use csv::ReaderBuilder;
use std::io::Cursor;
use serde_json::json;
use chrono::DateTime;

#[tokio::test]
async fn run_backtest() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenv().ok();
    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG must be set");
    let token = std::env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN must be set");
    let bucket = std::env::var("INFLUXDB_BUCKET").expect("INFLUXDB_BUCKET must be set");

    // Set the symbol and timeframe for backtesting
    let symbol = "GBPUSD_SB";
    let timeframe = "1h";
    let start_time = "2025-02-17T00:00:00Z";
    let end_time = "2025-03-21T23:59:59Z";

    // Query data
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

    // Get data from your database
    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .bearer_auth(&token)
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
                let open = record.get(o_idx).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let high = record.get(h_idx).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let low = record.get(l_idx).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let close = record.get(c_idx).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let volume = record.get(v_idx).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);

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

    println!("Loaded {} candles for {} {}", candles.len(), symbol, timeframe);

    // Create config
    let trade_config = TradeConfig {
        enabled: true,
        lot_size: 0.01,
        default_stop_loss_pips: 10.0,
        default_take_profit_pips: 20.0,
        risk_percent: 1.0,
        max_trades_per_pattern: 2,
        enable_trailing_stop: false,
        trailing_stop_activation_pips: 10.0,
        trailing_stop_distance_pips: 5.0,
        ..Default::default()
    };
    
    // Run backtest
    let recognizer = FiftyPercentBeforeBigBarRecognizer::default();
    let (trades, summary) = recognizer.trade(&candles, trade_config);
    
    // Print results
    println!("\nBacktest Results for 50% Body Before Big Bar Pattern on {} {}:", symbol, timeframe);
    println!("Total Trades: {}", summary.total_trades);
    println!("Win Rate: {:.2}%", summary.win_rate);
    println!("Total Profit/Loss: {:.2}", summary.total_profit_loss);
    println!("Profit Factor: {:.2}", summary.profit_factor);
    
    // Print trades with detailed information
    println!("\n{:<5} {:<12} {:<28} {:<10} {:<28} {:<10} {:<10} {:<15}", 
        "No.", "Direction", "Entry Time", "Entry", "Exit Time", "Exit", "P/L pips", "Reason");
    println!("{:-<120}", "");

    for (i, trade) in trades.iter().enumerate() {
        if let (Some(exit_price), Some(exit_time), Some(pl_pips), Some(exit_reason)) = 
            (trade.exit_price, &trade.exit_time, trade.profit_loss_pips, &trade.exit_reason)
        {
            // Format timestamps for human readability
            let entry_time = format_timestamp(&trade.entry_time);
            let exit_time_fmt = format_timestamp(exit_time);
            
            println!("{:<5} {:<12} {:<28} {:<10.5} {:<28} {:<10.5} {:<10.1} {:<15}", 
                i+1, 
                format!("{:?}", trade.direction),
                entry_time,
                trade.entry_price,
                exit_time_fmt,
                exit_price,
                pl_pips,
                exit_reason);
            
            // Debug info for stops and targets
            let pip_size = 0.0001;
            let stop_distance = match trade.direction {
                TradeDirection::Long => (trade.entry_price - trade.stop_loss) / pip_size,
                TradeDirection::Short => (trade.stop_loss - trade.entry_price) / pip_size,
            };
            
            println!("      Stop Loss: {} ({:.1} pips from entry)", 
                trade.stop_loss, stop_distance);
            
            // Special debug for large losses
            if pl_pips.abs() > 30.0 {
                println!("      *** WARNING: Large loss detected ***");
                println!("      Expected max loss based on stop: {:.1} pips", stop_distance);
                println!("      Actual loss: {:.1} pips", pl_pips.abs());
                println!("      Stop distance difference: {:.1} pips", pl_pips.abs() - stop_distance);
            }
        }
    }
    
    // Find Trade #5 for detailed investigation
    if trades.len() >= 5 {
        let trade5 = &trades[4]; // Zero-indexed, so Trade #5 is at index 4
        if let Some(exit_idx) = trade5.candlestick_idx_exit {
            println!("\n--- DETAILED ANALYSIS OF TRADE #5 ---");
            println!("Direction: {:?}", trade5.direction);
            println!("Entry: {} at {}", trade5.entry_price, format_timestamp(&trade5.entry_time));
            println!("Stop Loss: {}", trade5.stop_loss);
            
            // Look at candles around the exit
            let start_idx = trade5.candlestick_idx_entry;
            println!("\nCANDLES FROM ENTRY TO EXIT:");
            
            for idx in start_idx..=exit_idx {
                let candle = &candles[idx];
                let formatted_time = format_timestamp(&candle.time);
                
                // Mark significant candles
                let marker = if idx == start_idx { 
                    "ENTRY→" 
                } else if idx == exit_idx { 
                    "EXIT→" 
                } else { 
                    "     " 
                };
                
                println!("{}[{}] {}: O={:.5}, H={:.5}, L={:.5}, C={:.5}", 
                    marker, idx, formatted_time, candle.open, candle.high, candle.low, candle.close);
                
                // Check if this candle would hit stop
                if idx > start_idx && trade5.direction == TradeDirection::Short && candle.high >= trade5.stop_loss {
                    println!("      STOP LOSS HIT! High {:.5} > Stop {:.5}", candle.high, trade5.stop_loss);
                }
            }
        }
    }
    
    Ok(())
}

// Helper function to format ISO timestamps to human-readable format
fn format_timestamp(timestamp: &str) -> String {
    if let Ok(dt) = DateTime::parse_from_rfc3339(timestamp) {
        return dt.format("%Y-%m-%d %H:%M:%S").to_string();
    }
    timestamp.to_string()
}