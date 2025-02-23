// src/detect.rs
use actix_web::{web, HttpResponse, Responder};
use csv::ReaderBuilder;
use dotenv::dotenv;
use reqwest;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use serde_json::json;


// Import from crate root
use crate::patterns::{
    PatternRecognizer, 
    DemandZoneRecognizer, 
    BullishEngulfingRecognizer, 
    SupplyZoneRecognizer, 
    BigBarRecognizer,
    PinBarRecognizer
};

// Data structures
#[derive(Deserialize)]
pub struct ChartQuery {
    pub start_time: String,
    pub end_time: String,
    pub symbol: String,
    pub timeframe: String,
    pub pattern: String,
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
    for result in rdr.records() {
        if let Ok(record) = result {
            if record.len() >= 13 && !record[0].starts_with(',') {
                candles.push(CandleData {
                    time: record[5].to_string(),
                    open: record[11].parse().unwrap_or(0.0),
                    high: record[10].parse().unwrap_or(0.0),
                    low: record[12].parse().unwrap_or(0.0),
                    close: record[9].parse().unwrap_or(0.0),
                });
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
    let recognizer: &dyn PatternRecognizer = match query.pattern.as_str() {
        "demand_zone" => &DemandZoneRecognizer,
        "bullish_engulfing" => &BullishEngulfingRecognizer,
        "supply_zone" => &SupplyZoneRecognizer,
        "big_bar" => &BigBarRecognizer, // Added
        "pin_bar" => &PinBarRecognizer, // Added
        _ => return HttpResponse::BadRequest().body(format!("Unknown pattern: {}", query.pattern)),
    };

    // Detect patterns and return as JSON (changed from Vec<BuyZone> to Vec<Value>)
    let result = recognizer.detect(&candles);

    HttpResponse::Ok().json(result)
}

fn aggregate_to_5m(candles: Vec<CandleData>) -> Vec<CandleData> {
    let mut result = Vec::new();
    for chunk in candles.chunks(5) {
        if !chunk.is_empty() {
            let high = chunk.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
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