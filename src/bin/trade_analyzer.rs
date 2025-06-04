// src/bin/trade_analyzer.rs - New binary for historical trade analysis
use chrono::{DateTime, Utc};
use clap::Parser;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use reqwest::Client;

#[derive(Parser, Debug)]
#[command(name = "trade-analyzer")]
#[command(about = "Analyze what trades would have been booked for a given period")]
struct Args {
    /// Start time for analysis (RFC3339 format, default: today 00:00 UTC)
    #[arg(short, long)]
    start_time: Option<String>,
    
    /// End time for analysis (RFC3339 format, default: now)
    #[arg(short, long)]
    end_time: Option<String>,
    
    /// Main application URL
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    app_url: String,
    
    /// Output CSV file path
    #[arg(short, long, default_value = "analyzed_trades.csv")]
    output: String,
    
    /// Minimum timeframe to analyze (default: 30m)
    #[arg(long, default_value = "30m")]
    min_timeframe: String,
    
    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ZoneData {
    zone_id: String,
    symbol: String,
    timeframe: String,
    zone_high: f64,
    zone_low: f64,
    zone_type: String, // supply/demand
    strength_score: f64,
    is_active: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct PriceCandle {
    timestamp: DateTime<Utc>,
    symbol: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Debug, Serialize)]
struct TradeResult {
    entry_time: DateTime<Utc>,
    symbol: String,
    timeframe: String,
    zone_id: String,
    action: String, // BUY/SELL
    entry_price: f64,
    exit_time: Option<DateTime<Utc>>,
    exit_price: Option<f64>,
    exit_reason: String, // TP_HIT, SL_HIT, ZONE_INVALIDATED, STILL_OPEN
    pnl_pips: Option<f64>,
    duration_minutes: Option<i64>,
    zone_strength: f64,
}

#[derive(Debug)]
struct AnalysisConfig {
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    app_url: String,
    output_file: String,
    min_timeframe: String,
    debug: bool,
}

#[derive(Debug, Clone)]
struct OpenTrade {
    zone_id: String,
    symbol: String,
    timeframe: String,
    action: String,
    entry_time: DateTime<Utc>,
    entry_price: f64,
    take_profit: f64,
    stop_loss: f64,
    zone_strength: f64,
}

#[derive(Debug)]
struct TradeAnalyzer {
    config: AnalysisConfig,
    http_client: Client,
    zones: Vec<ZoneData>,
    price_data: HashMap<String, Vec<PriceCandle>>, // symbol -> candles
}

impl TradeAnalyzer {
    fn new(config: AnalysisConfig) -> Self {
        Self {
            config,
            http_client: Client::new(),
            zones: Vec::new(),
            price_data: HashMap::new(),
        }
    }

    async fn fetch_zones_from_main_app(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("📊 Fetching zones from main application...");
        
        let url = format!("{}/debug/minimal-cache-zones", self.config.app_url);
        let response = self.http_client.get(&url).send().await?;
            
        if !response.status().is_success() {
            return Err(format!("Failed to fetch zones: HTTP {}", response.status()).into());
        }
        
        let zones_json: serde_json::Value = response.json().await?;
        
        let zones_array = zones_json.get("retrieved_zones")
            .and_then(|v| v.as_array())
            .ok_or("Expected 'retrieved_zones' field containing an array")?;
            
        let mut filtered_zones = Vec::new();
        
        for zone_value in zones_array {
            if let Ok(zone) = self.parse_zone_from_json(zone_value) {
                if self.is_timeframe_eligible(&zone.timeframe) && zone.is_active {
                    filtered_zones.push(zone);
                }
            }
        }
        
        self.zones = filtered_zones;
        info!("✅ Loaded {} eligible zones", self.zones.len());
        Ok(())
    }
    
    fn parse_zone_from_json(&self, zone_json: &serde_json::Value) -> Result<ZoneData, Box<dyn std::error::Error>> {
        let zone_id = zone_json.get("zone_id").and_then(|v| v.as_str()).ok_or("Missing zone_id")?.to_string();
        let symbol = zone_json.get("symbol").and_then(|v| v.as_str()).ok_or("Missing symbol")?.to_string();
        let timeframe = zone_json.get("timeframe").and_then(|v| v.as_str()).ok_or("Missing timeframe")?.to_string();
        let zone_high = zone_json.get("zone_high").and_then(|v| v.as_f64()).ok_or("Missing zone_high")?;
        let zone_low = zone_json.get("zone_low").and_then(|v| v.as_f64()).ok_or("Missing zone_low")?;
        let zone_type = zone_json.get("type").or_else(|| zone_json.get("zone_type")).and_then(|v| v.as_str()).ok_or("Missing type")?.to_string();
        let strength_score = zone_json.get("strength_score").and_then(|v| v.as_f64()).unwrap_or(50.0);
        let is_active = zone_json.get("is_active").and_then(|v| v.as_bool()).unwrap_or(false);
            
        Ok(ZoneData { zone_id, symbol, timeframe, zone_high, zone_low, zone_type, strength_score, is_active })
    }
    
    fn is_timeframe_eligible(&self, timeframe: &str) -> bool {
        matches!(timeframe, "30m" | "1h" | "4h" | "1d")
    }

    async fn fetch_price_data(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("📈 Fetching 1-minute price data...");
        
        let symbols: std::collections::HashSet<String> = self.zones.iter().map(|zone| zone.symbol.clone()).collect();
        
        let host = std::env::var("INFLUXDB_HOST").map_err(|_| "INFLUXDB_HOST not set")?;
        let org = std::env::var("INFLUXDB_ORG").map_err(|_| "INFLUXDB_ORG not set")?;
        let token = std::env::var("INFLUXDB_TOKEN").map_err(|_| "INFLUXDB_TOKEN not set")?;
        let bucket = std::env::var("INFLUXDB_BUCKET").map_err(|_| "INFLUXDB_BUCKET not set")?;
            
        let start_time_str = self.config.start_time.to_rfc3339();
        let end_time_str = self.config.end_time.to_rfc3339();
        
        for symbol in symbols {
            let db_symbol = if symbol.ends_with("_SB") { symbol.clone() } else { format!("{}_SB", symbol) };
            
            let flux_query = format!(
                r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == "trendbar") |> filter(fn: (r) => r.symbol == "{}") |> filter(fn: (r) => r.timeframe == "1m") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
                bucket, start_time_str, end_time_str, db_symbol
            );
            
            let response = self.http_client
                .post(&format!("{}/api/v2/query?org={}", host, org))
                .bearer_auth(&token)
                .json(&serde_json::json!({"query": flux_query, "type": "flux"}))
                .send().await?;
                
            if !response.status().is_success() {
                warn!("Failed to fetch 1m data for {}: HTTP {}", db_symbol, response.status());
                continue;
            }
            
            let response_text = response.text().await?;
            
            if response_text.trim().is_empty() {
                warn!("⚠️ Empty response for {}, skipping", symbol);
                continue;
            }
            
            let mut candles = match self.parse_influx_csv(&response_text) {
                Ok(candles) => candles,
                Err(e) => {
                    warn!("⚠️ Failed to parse 1m data for {}: {}", symbol, e);
                    continue;
                }
            };
            
            for candle in &mut candles {
                candle.symbol = symbol.clone();
            }
            
            info!("✅ Loaded {} 1m candles for {}", candles.len(), symbol);
            self.price_data.insert(symbol, candles);
        }
        
        info!("✅ Price data loading complete for {} symbols", self.price_data.len());
        Ok(())
    }
    
    fn parse_influx_csv(&self, response_text: &str) -> Result<Vec<PriceCandle>, Box<dyn std::error::Error>> {
        use csv::ReaderBuilder;
        use std::io::Cursor;
        
        let mut candles = Vec::new();
        let cursor = Cursor::new(response_text);
        let mut rdr = ReaderBuilder::new().has_headers(true).flexible(true).comment(Some(b'#')).trim(csv::Trim::All).from_reader(cursor);

        let headers = rdr.headers()?.clone();
        if headers.is_empty() { return Ok(vec![]); }

        let find_idx = |name: &str| headers.iter().position(|h| h == name);
        let time_idx = find_idx("_time");
        let open_idx = find_idx("open");
        let high_idx = find_idx("high");
        let low_idx = find_idx("low");
        let close_idx = find_idx("close");

        if time_idx.is_none() || open_idx.is_none() || high_idx.is_none() || low_idx.is_none() || close_idx.is_none() {
            return Err("Essential OHLC columns not found".into());
        }

        for result in rdr.records() {
            let record = result?;
            if let (Some(time_str), Some(open), Some(high), Some(low), Some(close)) = (
                time_idx.and_then(|idx| record.get(idx)),
                open_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok())),
                high_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok())),
                low_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok())),
                close_idx.and_then(|idx| record.get(idx).and_then(|v| v.parse::<f64>().ok())),
            ) {
                let timestamp = DateTime::parse_from_rfc3339(time_str)?.with_timezone(&Utc);
                candles.push(PriceCandle { timestamp, symbol: "TEMP".to_string(), open, high, low, close });
            }
        }
        
        Ok(candles)
    }

    async fn analyze_trades(&self) -> Result<Vec<TradeResult>, Box<dyn std::error::Error>> {
        info!("🔍 Starting minute-by-minute trade analysis...");
        
        let mut trade_results = Vec::new();
        let mut open_trades: Vec<OpenTrade> = Vec::new();
        
        let mut all_timestamps = std::collections::BTreeSet::new();
        for candles in self.price_data.values() {
            for candle in candles {
                all_timestamps.insert(candle.timestamp);
            }
        }
        
        let total_minutes = all_timestamps.len();
        info!("📊 Analyzing {} minutes of price data", total_minutes);
        
        let mut minute_count = 0;
        
        for current_time in all_timestamps {
            minute_count += 1;
            
            if minute_count % 100 == 0 {
                info!("⏳ Processing minute {}/{} ({})", minute_count, total_minutes, current_time.format("%H:%M"));
            }
            
            let current_prices = self.get_prices_at_timestamp(current_time);
            
            // Check for new trade entries
            for zone in &self.zones {
                if let Some(current_price) = current_prices.get(&zone.symbol) {
                    if open_trades.iter().any(|t| t.zone_id == zone.zone_id) { continue; }
                    
                    if self.check_zone_entry(zone, *current_price) && self.validate_trade_signal(zone, current_time) {
                        let (tp_price, sl_price) = self.calculate_tp_sl_levels(zone, *current_price);
                        
                        let open_trade = OpenTrade {
                            zone_id: zone.zone_id.clone(),
                            symbol: zone.symbol.clone(),
                            timeframe: zone.timeframe.clone(),
                            action: if zone.zone_type.contains("supply") { "SELL".to_string() } else { "BUY".to_string() },
                            entry_time: current_time,
                            entry_price: *current_price,
                            take_profit: tp_price,
                            stop_loss: sl_price,
                            zone_strength: zone.strength_score,
                        };
                        
                        info!("📈 NEW TRADE: {} {} {} @ {:.5}", open_trade.action, open_trade.symbol, open_trade.timeframe, open_trade.entry_price);
                        open_trades.push(open_trade);
                    }
                }
            }
            
            // Check existing trades for exits
            let mut trades_to_close = Vec::new();
            
            for (index, open_trade) in open_trades.iter().enumerate() {
                if let Some(current_price) = current_prices.get(&open_trade.symbol) {
                    if let Some(exit_reason) = self.check_trade_exit_with_price(open_trade, *current_price) {
                        let duration = current_time - open_trade.entry_time;
                        let duration_minutes = duration.num_minutes();
                        let pnl_pips = self.calculate_pnl_pips(open_trade, *current_price);
                        
                        let completed_trade = TradeResult {
                            entry_time: open_trade.entry_time,
                            symbol: open_trade.symbol.clone(),
                            timeframe: open_trade.timeframe.clone(),
                            zone_id: open_trade.zone_id.clone(),
                            action: open_trade.action.clone(),
                            entry_price: open_trade.entry_price,
                            exit_time: Some(current_time),
                            exit_price: Some(*current_price),
                            exit_reason,
                            pnl_pips: Some(pnl_pips),
                            duration_minutes: Some(duration_minutes),
                            zone_strength: open_trade.zone_strength,
                        };
                        
                        info!("🎯 TRADE CLOSED: {} {} @ {:.5} -> {:.5} | {:.1} pips | {} | {}min", 
                              completed_trade.action, completed_trade.symbol, completed_trade.entry_price, 
                              current_price, pnl_pips, completed_trade.exit_reason, duration_minutes);
                        
                        trade_results.push(completed_trade);
                        trades_to_close.push(index);
                    }
                    else if let Some(zone) = self.zones.iter().find(|z| z.zone_id == open_trade.zone_id) {
                        if self.check_zone_invalidation(zone, *current_price) {
                            let duration = current_time - open_trade.entry_time;
                            let duration_minutes = duration.num_minutes();
                            let pnl_pips = self.calculate_pnl_pips(open_trade, *current_price);
                            
                            let completed_trade = TradeResult {
                                entry_time: open_trade.entry_time,
                                symbol: open_trade.symbol.clone(),
                                timeframe: open_trade.timeframe.clone(),
                                zone_id: open_trade.zone_id.clone(),
                                action: open_trade.action.clone(),
                                entry_price: open_trade.entry_price,
                                exit_time: Some(current_time),
                                exit_price: Some(*current_price),
                                exit_reason: "ZONE_INVALIDATED".to_string(),
                                pnl_pips: Some(pnl_pips),
                                duration_minutes: Some(duration_minutes),
                                zone_strength: open_trade.zone_strength,
                            };
                            
                            info!("❌ ZONE INVALIDATED: {} {} @ {:.5} | {:.1} pips | {}min", 
                                  completed_trade.action, completed_trade.symbol, current_price, pnl_pips, duration_minutes);
                            
                            trade_results.push(completed_trade);
                            trades_to_close.push(index);
                        }
                    }
                }
            }
            
            for &index in trades_to_close.iter().rev() {
                open_trades.remove(index);
            }
        }
        
        // Handle remaining open trades
        for open_trade in open_trades {
            let final_price = self.price_data.get(&open_trade.symbol)
                .and_then(|candles| candles.last())
                .map(|candle| candle.close)
                .unwrap_or(open_trade.entry_price);
                
            let duration = self.config.end_time - open_trade.entry_time;
            let duration_minutes = duration.num_minutes();
            let pnl_pips = self.calculate_pnl_pips(&open_trade, final_price);
            
            let still_open_trade = TradeResult {
                entry_time: open_trade.entry_time,
                symbol: open_trade.symbol.clone(),
                timeframe: open_trade.timeframe.clone(),
                zone_id: open_trade.zone_id.clone(),
                action: open_trade.action.clone(),
                entry_price: open_trade.entry_price,
                exit_time: None,
                exit_price: None,
                exit_reason: "STILL_OPEN".to_string(),
                pnl_pips: Some(pnl_pips),
                duration_minutes: Some(duration_minutes),
                zone_strength: open_trade.zone_strength,
            };
            
            trade_results.push(still_open_trade);
        }
        
        info!("✅ Analysis complete: {} total trades identified", trade_results.len());
        Ok(trade_results)
    }
    
    fn get_prices_at_timestamp(&self, timestamp: DateTime<Utc>) -> HashMap<String, f64> {
        let mut prices = HashMap::new();
        for (symbol, candles) in &self.price_data {
            if let Some(candle) = candles.iter().find(|c| c.timestamp == timestamp) {
                prices.insert(symbol.clone(), candle.close);
            }
        }
        prices
    }
    
    fn check_trade_exit_with_price(&self, open_trade: &OpenTrade, current_price: f64) -> Option<String> {
        let is_buy = open_trade.action == "BUY";
        
        let tp_hit = if is_buy { current_price >= open_trade.take_profit } else { current_price <= open_trade.take_profit };
        if tp_hit { return Some("TP_HIT".to_string()); }
        
        let sl_hit = if is_buy { current_price <= open_trade.stop_loss } else { current_price >= open_trade.stop_loss };
        if sl_hit { return Some("SL_HIT".to_string()); }
        
        None
    }
    
    fn calculate_pnl_pips(&self, open_trade: &OpenTrade, current_price: f64) -> f64 {
        let is_buy = open_trade.action == "BUY";
        let pip_value = self.get_pip_value(&open_trade.symbol);
        let price_diff = if is_buy { current_price - open_trade.entry_price } else { open_trade.entry_price - current_price };
        price_diff / pip_value
    }
    
    fn get_pip_value(&self, symbol: &str) -> f64 {
        if symbol.contains("JPY") { 0.01 } else { 0.0001 }
    }

    fn check_zone_entry(&self, zone: &ZoneData, current_price: f64) -> bool {
        let zone_height = zone.zone_high - zone.zone_low;
        let pip_threshold: f64 = 0.0002;
        let percent_threshold: f64 = zone_height * 0.02;
        let proximity_threshold = pip_threshold.min(percent_threshold);
        
        let is_supply = zone.zone_type.contains("supply");
        if is_supply {
            let proximal_line = zone.zone_low;
            current_price >= (proximal_line - proximity_threshold) && current_price <= (proximal_line + proximity_threshold)
        } else {
            let proximal_line = zone.zone_high;
            current_price >= (proximal_line - proximity_threshold) && current_price <= (proximal_line + proximity_threshold)
        }
    }

    fn check_zone_invalidation(&self, zone: &ZoneData, current_price: f64) -> bool {
        let is_supply = zone.zone_type.contains("supply");
        if is_supply { current_price > zone.zone_high } else { current_price < zone.zone_low }
    }

    fn validate_trade_signal(&self, _zone: &ZoneData, _current_time: DateTime<Utc>) -> bool {
        true
    }

    fn calculate_tp_sl_levels(&self, zone: &ZoneData, entry_price: f64) -> (f64, f64) {
        let pip_value = self.get_pip_value(&zone.symbol);
        let is_supply = zone.zone_type.contains("supply");
        let tp_pips = 50.0;
        let sl_pips = 25.0;
        
        if is_supply {
            let take_profit = entry_price - (tp_pips * pip_value);
            let stop_loss = entry_price + (sl_pips * pip_value);
            (take_profit, stop_loss)
        } else {
            let take_profit = entry_price + (tp_pips * pip_value);
            let stop_loss = entry_price - (sl_pips * pip_value);
            (take_profit, stop_loss)
        }
    }

    async fn write_results_to_csv(&self, trades: &[TradeResult]) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all("trades")?;
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("trades/trade_analysis_{}.csv", timestamp);
        
        info!("📝 Writing {} trades to CSV: {}", trades.len(), filename);
        
        use std::fs::File;
        use csv::Writer;
        
        let file = File::create(&filename)?;
        let mut writer = Writer::from_writer(file);
        
        writer.write_record(&["entry_time", "symbol", "timeframe", "zone_id", "action", "entry_price", "exit_time", "exit_price", "exit_reason", "pnl_pips", "duration_minutes", "zone_strength"])?;
        
        for trade in trades {
            let record = vec![
                trade.entry_time.to_rfc3339(),
                trade.symbol.clone(),
                trade.timeframe.clone(),
                trade.zone_id.clone(),
                trade.action.clone(),
                trade.entry_price.to_string(),
                trade.exit_time.map_or("".to_string(), |t| t.to_rfc3339()),
                trade.exit_price.map_or("".to_string(), |p| p.to_string()),
                trade.exit_reason.clone(),
                trade.pnl_pips.map_or("".to_string(), |p| format!("{:.1}", p)),
                trade.duration_minutes.map_or("".to_string(), |d| d.to_string()),
                format!("{:.1}", trade.zone_strength),
            ];
            writer.write_record(&record)?;
        }
        
        writer.flush()?;
        info!("✅ CSV file written successfully: {}", filename);
        println!("📄 Results saved to: {}", filename);
        Ok(())
    }

    fn print_summary(&self, _trades: &[TradeResult]) {
        info!("📊 Analysis summary printing not yet implemented");
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("🚀 Starting trade analysis for {} to {}", 
              self.config.start_time.format("%Y-%m-%d %H:%M:%S"),
              self.config.end_time.format("%Y-%m-%d %H:%M:%S"));

        self.fetch_zones_from_main_app().await?;
        info!("✅ Loaded {} zones for analysis", self.zones.len());

        self.fetch_price_data().await?;
        info!("✅ Loaded price data for {} symbols", self.price_data.len());

        let trade_results = self.analyze_trades().await?;
        info!("✅ Analysis complete: {} trades identified", trade_results.len());

        self.write_results_to_csv(&trade_results).await?;
        self.print_summary(&trade_results);

        info!("🎉 Trade analysis completed successfully!");
        Ok(())
    }
}

fn parse_time_or_default(time_str: Option<String>, default_fn: fn() -> DateTime<Utc>) -> DateTime<Utc> {
    match time_str {
        Some(s) => match DateTime::parse_from_rfc3339(&s) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(e) => {
                eprintln!("Warning: Invalid time format '{}': {}. Using default.", s, e);
                default_fn()
            }
        },
        None => default_fn(),
    }
}

fn setup_logging(debug: bool) {
    use env_logger::{Builder, Target};
    use log::LevelFilter;
    
    let mut builder = Builder::from_default_env();
    builder.target(Target::Stdout);
    
    if debug {
        builder.filter_level(LevelFilter::Debug);
    } else {
        builder.filter_level(LevelFilter::Info);
    }
    
    builder.init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();
    
    let args = Args::parse();
    setup_logging(args.debug);
    
    let start_time = parse_time_or_default(args.start_time, || {
        let now = Utc::now();
        now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
    });
    
    let end_time = parse_time_or_default(args.end_time, || Utc::now());
    
    let config = AnalysisConfig {
        start_time,
        end_time,
        app_url: args.app_url,
        output_file: args.output,
        min_timeframe: args.min_timeframe,
        debug: args.debug,
    };
    
    let mut analyzer = TradeAnalyzer::new(config);
    analyzer.run().await?;
    
    Ok(())
}