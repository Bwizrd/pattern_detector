// price_fetcher.rs
use crate::types::{PriceCandle, ZoneData};
use crate::config::AnalysisConfig;
use chrono::{DateTime, Utc};
use log::{info, warn};
use reqwest::Client;
use std::collections::HashMap;

pub struct PriceFetcher {
    client: Client,
}

impl PriceFetcher {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn fetch_price_data(
        &self, 
        config: &AnalysisConfig,
        zones: &[ZoneData]
    ) -> Result<HashMap<String, Vec<PriceCandle>>, Box<dyn std::error::Error>> {
        info!("📈 Fetching 1-minute price data...");
        
        let symbols: std::collections::HashSet<String> = zones.iter().map(|zone| zone.symbol.clone()).collect();
        
        let host = std::env::var("INFLUXDB_HOST").map_err(|_| "INFLUXDB_HOST not set")?;
        let org = std::env::var("INFLUXDB_ORG").map_err(|_| "INFLUXDB_ORG not set")?;
        let token = std::env::var("INFLUXDB_TOKEN").map_err(|_| "INFLUXDB_TOKEN not set")?;
        let bucket = std::env::var("INFLUXDB_BUCKET").map_err(|_| "INFLUXDB_BUCKET not set")?;
            
        let start_time_str = config.start_time.to_rfc3339();
        let end_time_str = config.end_time.to_rfc3339();
        
        let mut price_data = HashMap::new();
        
        for symbol in symbols {
            let db_symbol = if symbol.ends_with("_SB") { symbol.clone() } else { format!("{}_SB", symbol) };
            
            let flux_query = format!(
                r#"from(bucket: "{}") |> range(start: {}, stop: {}) |> filter(fn: (r) => r._measurement == "trendbar") |> filter(fn: (r) => r.symbol == "{}") |> filter(fn: (r) => r.timeframe == "1m") |> toFloat() |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") |> sort(columns: ["_time"])"#,
                bucket, start_time_str, end_time_str, db_symbol
            );
            
            let response = self.client
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
            price_data.insert(symbol, candles);
        }
        
        info!("✅ Price data loading complete for {} symbols", price_data.len());
        Ok(price_data)
    }
    
    fn parse_influx_csv(&self, response_text: &str) -> Result<Vec<PriceCandle>, Box<dyn std::error::Error>> {
        use csv::ReaderBuilder;
        use std::io::Cursor;
        
        let mut candles = Vec::new();
        let cursor = Cursor::new(response_text);
        let mut rdr = ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .comment(Some(b'#'))
            .trim(csv::Trim::All)
            .from_reader(cursor);

        let headers = rdr.headers()?.clone();
        if headers.is_empty() { 
            return Ok(vec![]); 
        }

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
                candles.push(PriceCandle { 
                    timestamp, 
                    symbol: "TEMP".to_string(), 
                    open, 
                    high, 
                    low, 
                    close 
                });
            }
        }
        
        Ok(candles)
    }
}