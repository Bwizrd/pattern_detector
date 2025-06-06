// tests/optimization/data_loader.rs
use pattern_detector::api::detect::CandleData;
use csv::ReaderBuilder;
use reqwest;
use serde_json::json;
use std::io::Cursor;

pub async fn load_candles(
    host: &str,
    org: &str,
    token: &str,
    bucket: &str,
    symbol: &str,
    timeframe: &str,
    start_time: &str,
    end_time: &str,
) -> Result<Vec<CandleData>, Box<dyn std::error::Error>> {
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