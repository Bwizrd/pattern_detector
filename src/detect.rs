// src/detect.rs
use actix_web::{web, HttpResponse, Responder};
use csv::ReaderBuilder;
use dotenv::dotenv;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Cursor;

// Import from crate root
use crate::patterns::{
    BigBarRecognizer, BullishEngulfingRecognizer, DemandZoneRecognizer, PatternRecognizer,
    PinBarRecognizer, SupplyZoneRecognizer, RallyRecognizer, SupplyDemandZoneRecognizer
};

// Data structures
#[derive(Deserialize)]
pub struct ChartQuery {
    pub start_time: String,
    pub end_time: String,
    pub symbol: String,
    pub timeframe: String,
    pub pattern: String,
    pub min_imbalance_pips: Option<f64>,
    pub zone_lookback: Option<usize>,
    pub pip_value: Option<f64>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct CandleData {
    pub time: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Serialize)]
pub struct BuyZone {
    pub start_time: String,
    pub upper_line: f64,
    pub lower_line: f64,
}

pub async fn detect_patterns(query: web::Query<ChartQuery>) -> impl Responder {
    dotenv().ok();
    let host = std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string());
    let org = std::env::var("INFLUXDB_ORG").expect("INFLUXDB_ORG must be set");
    let token = std::env::var("INFLUXDB_TOKEN").expect("INFLUXDB_TOKEN must be set");
    let bucket = std::env::var("INFLUXDB_BUCKET").expect("INFLUXDB_BUCKET must be set");

    // Append _SB to the symbol if it's not already present
    let symbol = if query.symbol.ends_with("_SB") {
        query.symbol.clone()
    } else {
        format!("{}_SB", query.symbol)
    };

    let flux_query = format!(
        r#"from(bucket: "{}")
        |> range(start: {}, stop: {})
        |> filter(fn: (r) => r._measurement == "trendbar")
        |> filter(fn: (r) => r.symbol == "{}")
        |> toFloat()
        |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value")
        |> sort(columns: ["_time"])"#,
        bucket, query.start_time, query.end_time, query.symbol
    );

    let url = format!("{}/api/v2/query?org={}", host, org);
    let client = reqwest::Client::new();
    let response = match client
        .post(&url)
        .bearer_auth(&token)
        .json(&serde_json::json!({"query": flux_query, "type": "flux"}))
        .send()
        .await
    {
        Ok(resp) => resp.text().await.unwrap_or_default(),
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let mut candles = Vec::new();
    let cursor = Cursor::new(&response);
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(cursor);

    let headers = match rdr.headers() {
        Ok(h) => h.clone(),
        Err(e) => {
            return HttpResponse::InternalServerError().body(format!("CSV header error: {}", e))
        }
    };

      // Find column indices
      let mut time_idx = None;
      let mut open_idx = None;
      let mut high_idx = None;
      let mut low_idx = None;
      let mut close_idx = None;
      
      for (i, name) in headers.iter().enumerate() {
          match name {
              "_time" => time_idx = Some(i),
              "open" => open_idx = Some(i),
              "high" => high_idx = Some(i),
              "low" => low_idx = Some(i),
              "close" => close_idx = Some(i),
              _ => {}
          }
      }

    for result in rdr.records() {
        if let Ok(record) = result {
              if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx)) = 
               (time_idx, open_idx, high_idx, low_idx, close_idx) {
                
                if let Some(time_val) = record.get(t_idx) {
                    let open = record.get(o_idx)
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let high = record.get(h_idx)
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let low = record.get(l_idx)
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let close = record.get(c_idx)
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    
                    candles.push(CandleData {
                        time: time_val.to_string(),
                        open,
                        high,
                        low,
                        close,
                    });
                }
               }
        }
    }

    if candles.is_empty() {
        return HttpResponse::NotFound().json(json!({
            "error": "No data found",
            "message": format!("No candles found for symbol {} in the specified time range.", symbol)
        }));
    }

    let candles = if query.timeframe == "5m" {
        aggregate_to_5m(candles)
    } else {
        candles
    };

    // Select recognizer based on pattern query parameter
    let result = match query.pattern.as_str() {
        // For the new combined supply/demand zone recognizer
        "supply_demand_zone" => {
            // Create a configured recognizer with optional parameters from query
            let recognizer = SupplyDemandZoneRecognizer::new(
                query.min_imbalance_pips.unwrap_or(0.0010), // Default 10 pips
                query.zone_lookback.unwrap_or(20),          // Default 20 candles
                query.pip_value.unwrap_or(0.0001),          // Default pip value
            );
            recognizer.detect(&candles)
        },
        // Existing recognizers
        "demand_zone" => DemandZoneRecognizer.detect(&candles),
        "bullish_engulfing" => BullishEngulfingRecognizer.detect(&candles),
        "supply_zone" => SupplyZoneRecognizer.detect(&candles),
        "big_bar" => BigBarRecognizer.detect(&candles),
        "pin_bar" => PinBarRecognizer.detect(&candles),
        "rally" => RallyRecognizer.detect(&candles),
        _ => return HttpResponse::BadRequest().body(format!("Unknown pattern: {}", query.pattern)),
    };

    {
        // Future Extension: Combining Multiple Patterns
        // When you’re ready to handle multiple patterns, you can combine the responses into an array in detect.rs. Here’s how:

        // Collect Responses from Multiple Recognizers:

        // let mut results = Vec::new();

        // // Detect big_bar patterns
        // let big_bar_recognizer = BigBarRecognizer;
        // let big_bar_response = big_bar_recognizer.detect(&candles);
        // results.push(big_bar_response);

        // // Detect pin_bar patterns
        // let pin_bar_recognizer = PinBarRecognizer;
        // let pin_bar_response = pin_bar_recognizer.detect(&candles);
        // results.push(pin_bar_response);
    }

    // Detect patterns and return as JSON (changed from Vec<BuyZone> to Vec<Value>)

    HttpResponse::Ok().json(result)
}

fn aggregate_to_5m(candles: Vec<CandleData>) -> Vec<CandleData> {
    let mut result = Vec::new();
    for chunk in candles.chunks(5) {
        if !chunk.is_empty() {
            let high = chunk
                .iter()
                .map(|c| c.high)
                .fold(f64::NEG_INFINITY, f64::max);
            let low = chunk.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
            result.push(CandleData {
                time: chunk[0].time.clone(),
                open: chunk[0].open,
                high,
                low,
                close: chunk.last().unwrap().close,
            });
        }
    }
    result
}
