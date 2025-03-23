// src/detect.rs
use actix_web::{web, HttpResponse, Responder};
use csv::ReaderBuilder;
use dotenv::dotenv;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Cursor;

// Import from crate root
use crate::patterns::{
    BigBarRecognizer, BullishEngulfingRecognizer, CombinedDemandRecognizer, ConsolidationRecognizer, DemandMoveAwayRecognizer, DropBaseRallyRecognizer, EnhancedSupplyDemandZoneRecognizer, FiftyPercentBeforeBigBarRecognizer, FiftyPercentBodyCandleRecognizer, FlexibleDemandZoneRecognizer, PatternRecognizer, PinBarRecognizer, RallyRecognizer, SimpleSupplyDemandZoneRecognizer, SupplyZoneRecognizer
};

// Import trading modules
use crate::trades::TradeConfig;

// Data structures
#[derive(Deserialize)]
pub struct ChartQuery {
    pub start_time: String,
    pub end_time: String,
    pub symbol: String,
    pub timeframe: String,
    pub pattern: String,
    pub enable_trading: Option<bool>,
    pub lot_size: Option<f64>,
    pub stop_loss_pips: Option<f64>,
    pub take_profit_pips: Option<f64>,
    pub enable_trailing_stop: Option<bool>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct CandleData {
    pub time: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u32,
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

    // Get the requested timeframe from the query parameters
    let timeframe = &query.timeframe;

    // Construct the flux query with timeframe filtering
    let flux_query = format!(
        r#"from(bucket: "{}")
        |> range(start: {}, stop: {})
        |> filter(fn: (r) => r["_measurement"] == "trendbar")
        |> filter(fn: (r) => r["symbol"] == "{}")
        |> filter(fn: (r) => r["timeframe"] == "{}")
        |> toFloat()
        |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value")
        |> sort(columns: ["_time"])"#,
        bucket, query.start_time, query.end_time, symbol, timeframe
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

    for result in rdr.records() {
        if let Ok(record) = result {
            if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(c_vdx)) =
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
                        .get(c_vdx)
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
    }

    if candles.is_empty() {
        return HttpResponse::NotFound().json(json!({
            "error": "No data found",
            "message": format!("No candles found for symbol {} with timeframe {} in the specified time range.", symbol, timeframe)
        }));
    }
    ///// Set up trading config
    let enable_trading = query.enable_trading.unwrap_or(false);
    let trade_config = if enable_trading {
        TradeConfig {
            enabled: true,
            lot_size: query.lot_size.unwrap_or(0.01),
            default_stop_loss_pips: query.stop_loss_pips.unwrap_or(20.0),
            default_take_profit_pips: query.take_profit_pips.unwrap_or(40.0),
            enable_trailing_stop: query.enable_trailing_stop.unwrap_or(false),
            ..TradeConfig::default()
        }
    } else {
        TradeConfig::default()
    };

    // Select recognizer based on pattern query parameter
    let result = match query.pattern.as_str() {
        // For DBR pattern
        "drop_base_rally" => {
            let recognizer = DropBaseRallyRecognizer::default();
            recognizer.detect(&candles)
        }
        "demand_zone_flexible" => {
            let recognizer = FlexibleDemandZoneRecognizer::default();
            recognizer.detect(&candles)
        }
        "demand_move_away" => {
            let recognizer = DemandMoveAwayRecognizer::default();
            recognizer.detect(&candles)
        }
        // For the new combined supply/demand zone recognizer
        "supply_demand_zone" => {
            let recognizer = EnhancedSupplyDemandZoneRecognizer::default();
            let pattern_result = recognizer.detect(&candles);

            if enable_trading {
                let (trades, summary) = recognizer.trade(&candles, trade_config);

                let mut response_obj = pattern_result.as_object().unwrap().clone();
                response_obj.insert("trades".to_string(), json!(trades));
                response_obj.insert("trade_summary".to_string(), json!(summary));

                Value::Object(response_obj)
            } else {
                pattern_result
            }
        }
        "simple_supply_demand_zone" => {
            let recognizer = SimpleSupplyDemandZoneRecognizer;
            recognizer.detect(&candles)
        }
        "fifty_percent_body_candle" => {
            let recognizer = FiftyPercentBodyCandleRecognizer::default();
            recognizer.detect(&candles)
        }
        "fifty_percent_before_big_bar" => {
            let recognizer = FiftyPercentBeforeBigBarRecognizer::default();
            recognizer.detect(&candles)
        }
        "consolidation_zone" => {
            let recognizer = ConsolidationRecognizer::default();
            recognizer.detect(&candles)
        }
        "combined_demand" => {
            let recognizer = CombinedDemandRecognizer::default();
            recognizer.detect(&candles)
        }
        // Existing recognizers
        "bullish_engulfing" => BullishEngulfingRecognizer.detect(&candles),
        "supply_zone" => SupplyZoneRecognizer.detect(&candles),
        "big_bar" => BigBarRecognizer.detect(&candles),
        "pin_bar" => PinBarRecognizer.detect(&candles),
        "rally" => RallyRecognizer.detect(&candles),
        _ => return HttpResponse::BadRequest().body(format!("Unknown pattern: {}", query.pattern)),
    };

    HttpResponse::Ok().json(result)
}
