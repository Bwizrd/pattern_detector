// src/zone_detection.rs
// Extracted zone detection logic that can be used by both HTTP endpoint and real-time monitor

use crate::api::detect::{CandleData, CoreError};
use crate::types::{EnrichedZone, TouchPoint};
use crate::zones::patterns::{FiftyPercentBeforeBigBarRecognizer, PatternRecognizer};
use log::{debug, info, warn};
use serde_json::Value;
use std::collections::HashMap;

// ==================== ZONE DETECTION REQUEST ====================

#[derive(Debug, Clone)]
pub struct ZoneDetectionRequest {
    pub symbol: String,
    pub timeframe: String,
    pub pattern: String,
    pub candles: Vec<CandleData>,
    pub max_touch_count: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ZoneDetectionResult {
    pub symbol: String,
    pub timeframe: String,
    pub supply_zones: Vec<EnrichedZone>,
    pub demand_zones: Vec<EnrichedZone>,
    pub total_zones_detected: usize,
    pub candles_analyzed: usize,
}

// ==================== CORE ZONE DETECTION ENGINE ====================
#[derive(Clone)]
pub struct ZoneDetectionEngine {
    // Could add configuration here if needed
}

impl ZoneDetectionEngine {
    pub fn new() -> Self {
        Self {}
    }

    /// Main zone detection function - works with any candlestick data
    pub fn detect_zones(
        &self,
        request: ZoneDetectionRequest,
    ) -> Result<ZoneDetectionResult, CoreError> {
        debug!(
            "[ZoneEngine] Detecting zones for {}/{} with {} candles",
            request.symbol,
            request.timeframe,
            request.candles.len()
        );

        if request.candles.len() < 2 {
            return Err(CoreError::Processing(
                "Not enough candles for zone detection".to_string(),
            ));
        }

        // Create the pattern recognizer
        let recognizer: Box<dyn PatternRecognizer> = match request.pattern.as_str() {
            "fifty_percent_before_big_bar" => {
                Box::new(FiftyPercentBeforeBigBarRecognizer::default())
            }
            other => {
                return Err(CoreError::Config(format!(
                    "Pattern '{}' is not supported by the zone detection engine.",
                    other
                )));
            }
        };

        // Run pattern detection
        let detection_results = recognizer.detect(&request.candles);

        // Extract and enrich zones
        let (supply_zones, demand_zones) = self.extract_and_enrich_zones(
            &detection_results,
            &request.candles,
            &request.symbol,
            &request.timeframe,
            request.max_touch_count,
        )?;

        let total_detected = supply_zones.len() + demand_zones.len();

        debug!(
            "[ZoneEngine] Completed detection for {}/{}: {} supply, {} demand zones",
            request.symbol,
            request.timeframe,
            supply_zones.len(),
            demand_zones.len()
        );

        Ok(ZoneDetectionResult {
            symbol: request.symbol,
            timeframe: request.timeframe,
            supply_zones,
            demand_zones,
            total_zones_detected: total_detected,
            candles_analyzed: request.candles.len(),
        })
    }

    /// Extract zones from recognizer output and enrich with activity data
    fn extract_and_enrich_zones(
        &self,
        detection_results: &Value,
        candles: &[CandleData],
        symbol: &str,
        timeframe: &str,
        max_touch_count: Option<i64>,
    ) -> Result<(Vec<EnrichedZone>, Vec<EnrichedZone>), CoreError> {
        let mut final_supply_zones = Vec::new();
        let mut final_demand_zones = Vec::new();
        let candle_count = candles.len();
        let last_candle_timestamp = candles.last().map(|c| c.time.clone());

        // Extract zones from recognizer results
        if let Some(data) = detection_results.get("data").and_then(|d| d.get("price")) {
            // Process supply zones
            if let Some(supply_zones_data) = data
                .get("supply_zones")
                .and_then(|sz| sz.get("zones"))
                .and_then(|z| z.as_array())
            {
                final_supply_zones = self.enrich_zones_with_activity(
                    supply_zones_data,
                    true, // is_supply
                    candles,
                    candle_count,
                    last_candle_timestamp.clone(),
                    symbol,
                    timeframe,
                )?;
            }

            // Process demand zones
            if let Some(demand_zones_data) = data
                .get("demand_zones")
                .and_then(|dz| dz.get("zones"))
                .and_then(|z| z.as_array())
            {
                final_demand_zones = self.enrich_zones_with_activity(
                    demand_zones_data,
                    false, // is_supply
                    candles,
                    candle_count,
                    last_candle_timestamp.clone(),
                    symbol,
                    timeframe,
                )?;
            }
        } else {
            warn!("[ZoneEngine] No 'data.price' structure found in detection results");
        }

        // Apply touch count filtering if specified
        if let Some(max_touches) = max_touch_count {
            let initial_supply = final_supply_zones.len();
            let initial_demand = final_demand_zones.len();

            final_supply_zones.retain(|zone| zone.touch_count.map_or(true, |tc| tc <= max_touches));

            final_demand_zones.retain(|zone| zone.touch_count.map_or(true, |tc| tc <= max_touches));

            debug!(
                "[ZoneEngine] Touch count filter (max: {}): Supply {} -> {}, Demand {} -> {}",
                max_touches,
                initial_supply,
                final_supply_zones.len(),
                initial_demand,
                final_demand_zones.len()
            );
        }

        Ok((final_supply_zones, final_demand_zones))
    }

    /// Enrich zones with activity analysis - this is the core enrichment logic
    fn enrich_zones_with_activity(
        &self,
        raw_zones: &[Value],
        is_supply: bool,
        all_candles: &[CandleData],
        total_candle_count: usize,
        last_candle_timestamp: Option<String>,
        symbol: &str,
        timeframe: &str,
    ) -> Result<Vec<EnrichedZone>, CoreError> {
        let mut enriched_zones = Vec::new();
        let zone_type_str = if is_supply { "supply" } else { "demand" };

        debug!(
            "[ZoneEngine] Enriching {} {} zones for {}/{}",
            raw_zones.len(),
            zone_type_str,
            symbol,
            timeframe
        );

        for (zone_counter, zone_json) in raw_zones.iter().enumerate() {
            // Deserialize the base zone
            let mut enriched_zone = match crate::types::deserialize_raw_zone_value(zone_json) {
                Ok(mut z) => {
                    // Generate zone_id if missing
                    if z.zone_id.is_none() {
                        // ⭐ CRITICAL: Use clean symbol (remove _SB) to match cache behavior
                        let clean_symbol_for_id = symbol.replace("_SB", "");

                        let id = crate::api::detect::generate_deterministic_zone_id(
                            &clean_symbol_for_id, // Use clean symbol without _SB
                            timeframe,
                            if is_supply { "supply" } else { "demand" },
                            z.start_time.as_deref(),
                            z.zone_high,
                            z.zone_low,
                        );
                        z.zone_id = Some(id);
                    }

                    // Set zone_type and symbol/timeframe if missing
                    if z.zone_type.is_none() {
                        z.zone_type = Some(format!("{}_zone", zone_type_str));
                    }
                    if z.symbol.is_none() {
                        z.symbol = Some(symbol.replace("_SB", "")); // Store clean symbol
                    }
                    if z.timeframe.is_none() {
                        z.timeframe = Some(timeframe.to_string());
                    }

                    z
                }
                Err(e) => {
                    warn!(
                        "[ZoneEngine] Failed to deserialize zone #{}: {}",
                        zone_counter + 1,
                        e
                    );
                    continue;
                }
            };

            // Clear any existing end time data (fresh analysis)
            enriched_zone.end_time = None;
            enriched_zone.end_idx = None;
            enriched_zone.zone_end_time = None;
            enriched_zone.zone_end_idx = None;
            enriched_zone.invalidation_time = None;
            enriched_zone.last_data_candle = last_candle_timestamp.clone();

            let zone_id_log = enriched_zone.zone_id.as_deref().unwrap_or("UNKNOWN");

            // Get formation end index
            let formation_end_idx = match enriched_zone.start_idx {
                Some(start_idx) => start_idx as usize + 1, // Formation ends after the big bar
                None => {
                    warn!("[ZoneEngine] Zone {} missing start_idx", zone_id_log);
                    continue;
                }
            };

            // Validate zone prices
            let zone_high = match enriched_zone.zone_high {
                Some(val) if val.is_finite() => val,
                _ => {
                    warn!("[ZoneEngine] Zone {} invalid zone_high", zone_id_log);
                    continue;
                }
            };

            let zone_low = match enriched_zone.zone_low {
                Some(val) if val.is_finite() => val,
                _ => {
                    warn!("[ZoneEngine] Zone {} invalid zone_low", zone_id_log);
                    continue;
                }
            };

            // Set proximal/distal lines
            let (proximal_line, distal_line) = if is_supply {
                (zone_low, zone_high)
            } else {
                (zone_high, zone_low)
            };

            // Activity check starts AFTER formation
            let check_start_idx = formation_end_idx + 1;

            // Initialize tracking variables
            let mut is_currently_active = true;
            let mut touch_count: i64 = 0;
            let mut first_invalidation_idx: Option<usize> = None;
            let mut current_touch_points: Vec<TouchPoint> = Vec::new();

            // Determine initial state based on first candle after formation
            let mut is_outside_zone = true;
            if check_start_idx < total_candle_count {
                let first_candle_after_formation = &all_candles[check_start_idx];
                if is_supply {
                    is_outside_zone = first_candle_after_formation.high < proximal_line;
                } else {
                    is_outside_zone = first_candle_after_formation.low > proximal_line;
                }
            }

            // Process candles after formation
            if check_start_idx < total_candle_count {
                for i in check_start_idx..total_candle_count {
                    if !is_currently_active {
                        break;
                    }

                    let candle = &all_candles[i];
                    if !candle.high.is_finite()
                        || !candle.low.is_finite()
                        || !candle.close.is_finite()
                    {
                        continue;
                    }

                    let mut interaction_occurred = false;
                    let mut interaction_price = 0.0;

                    // Check for invalidation FIRST
                    if is_supply {
                        if candle.high > distal_line {
                            is_currently_active = false;
                            first_invalidation_idx = Some(i);
                            enriched_zone.invalidation_time = Some(candle.time.clone());
                            enriched_zone.end_time = Some(candle.time.clone());
                            enriched_zone.end_idx = Some(i as u64);
                            break;
                        }
                    } else {
                        if candle.low < distal_line {
                            is_currently_active = false;
                            first_invalidation_idx = Some(i);
                            enriched_zone.invalidation_time = Some(candle.time.clone());
                            enriched_zone.end_time = Some(candle.time.clone());
                            enriched_zone.end_idx = Some(i as u64);
                            break;
                        }
                    }

                    // Check for interaction with proximal line
                    if is_supply {
                        if candle.high >= proximal_line {
                            interaction_occurred = true;
                            interaction_price = candle.high;
                        }
                    } else {
                        if candle.low <= proximal_line {
                            interaction_occurred = true;
                            interaction_price = candle.low;
                        }
                    }

                    // Count touches only when transitioning from outside to inside
                    if interaction_occurred && is_outside_zone {
                        touch_count += 1;
                        current_touch_points.push(TouchPoint {
                            time: candle.time.clone(),
                            price: interaction_price,
                        });
                    }

                    // Update state for next iteration
                    is_outside_zone = !interaction_occurred;
                }
            }

            // Calculate bars_active correctly
            if !is_currently_active {
                // Zone was invalidated - count bars from activity start to invalidation
                if let Some(invalidation_idx) = first_invalidation_idx {
                    if invalidation_idx > check_start_idx {
                        enriched_zone.bars_active =
                            Some((invalidation_idx - check_start_idx) as u64);
                    } else {
                        enriched_zone.bars_active = Some(0);
                    }
                } else {
                    enriched_zone.bars_active = Some(0);
                }
            } else {
                // Zone is still active - count bars from activity start to end of data
                if total_candle_count > check_start_idx {
                    enriched_zone.bars_active = Some((total_candle_count - check_start_idx) as u64);
                } else {
                    enriched_zone.bars_active = Some(0);
                }
            }

            // Finalize enrichment
            enriched_zone.is_active = is_currently_active;
            enriched_zone.touch_count = Some(touch_count);

            if !current_touch_points.is_empty() {
                enriched_zone.touch_points = Some(current_touch_points);
            }

            // Calculate strength score
            let score_raw = 100.0 - (touch_count as f64 * 10.0);
            enriched_zone.strength_score = Some(score_raw.max(0.0));

            enriched_zones.push(enriched_zone);
        }

        Ok(enriched_zones)
    }

    /// Batch zone detection for multiple symbol/timeframe combinations
    pub fn detect_zones_batch(
        &self,
        requests: Vec<ZoneDetectionRequest>,
    ) -> Vec<Result<ZoneDetectionResult, CoreError>> {
        requests
            .into_iter()
            .map(|req| self.detect_zones(req))
            .collect()
    }
}

// ==================== DATA FETCHING UTILITIES ====================
#[derive(Clone)]
pub struct InfluxDataFetcher {
    client: reqwest::Client,
    host: String,
    org: String,
    token: String,
    bucket: String,
}

impl InfluxDataFetcher {
    pub fn new() -> Result<Self, CoreError> {
        dotenv::dotenv().ok();

        Ok(Self {
            client: reqwest::Client::new(),
            host: std::env::var("INFLUXDB_HOST").unwrap_or("http://localhost:8086".to_string()),
            org: std::env::var("INFLUXDB_ORG")?,
            token: std::env::var("INFLUXDB_TOKEN")?,
            bucket: std::env::var("INFLUXDB_BUCKET")?,
        })
    }

    pub async fn fetch_candles(
        &self,
        symbol: &str,
        timeframe: &str,
        start_time: &str,
        end_time: &str,
    ) -> Result<Vec<CandleData>, CoreError> {
        let symbol_with_suffix = if symbol.ends_with("_SB") {
            symbol.to_string()
        } else {
            format!("{}_SB", symbol)
        };

        let timeframe_lower = timeframe.to_lowercase();

        let flux_query = format!(
            r#"from(bucket: "{}") 
               |> range(start: {}, stop: {}) 
               |> filter(fn: (r) => r["_measurement"] == "trendbar") 
               |> filter(fn: (r) => r["symbol"] == "{}") 
               |> filter(fn: (r) => r["timeframe"] == "{}") 
               |> toFloat() 
               |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value") 
               |> sort(columns: ["_time"])"#,
            self.bucket, start_time, end_time, symbol_with_suffix, timeframe_lower
        );

        let url = format!("{}/api/v2/query?org={}", self.host, self.org);

        let response_text = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .json(&serde_json::json!({"query": flux_query, "type": "flux"}))
            .send()
            .await?
            .text()
            .await?;

        // Parse CSV response into CandleData (reuse existing logic from detect.rs)
        self.parse_csv_to_candles(&response_text)
    }

    fn parse_csv_to_candles(&self, csv_text: &str) -> Result<Vec<CandleData>, CoreError> {
        let mut candles = Vec::new();

        if !csv_text.trim().is_empty() {
            let cursor = std::io::Cursor::new(csv_text.as_bytes());
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(true)
                .flexible(true)
                .from_reader(cursor);

            let headers = rdr.headers()?.clone();
            let get_idx = |name: &str| headers.iter().position(|h| h == name);

            if let (Some(t_idx), Some(o_idx), Some(h_idx), Some(l_idx), Some(c_idx), Some(v_idx)) = (
                get_idx("_time"),
                get_idx("open"),
                get_idx("high"),
                get_idx("low"),
                get_idx("close"),
                get_idx("volume"),
            ) {
                for result in rdr.records() {
                    match result {
                        Ok(record) => {
                            let parse_f64 =
                                |idx: usize| record.get(idx).and_then(|v| v.parse::<f64>().ok());
                            let parse_u32 =
                                |idx: usize| record.get(idx).and_then(|v| v.parse::<u32>().ok());

                            if record.len()
                                > std::cmp::max(
                                    t_idx,
                                    std::cmp::max(
                                        o_idx,
                                        std::cmp::max(
                                            h_idx,
                                            std::cmp::max(l_idx, std::cmp::max(c_idx, v_idx)),
                                        ),
                                    ),
                                )
                            {
                                let time = record.get(t_idx).unwrap_or("").to_string();
                                if !time.is_empty() {
                                    candles.push(CandleData {
                                        time,
                                        open: parse_f64(o_idx).unwrap_or(0.0),
                                        high: parse_f64(h_idx).unwrap_or(0.0),
                                        low: parse_f64(l_idx).unwrap_or(0.0),
                                        close: parse_f64(c_idx).unwrap_or(0.0),
                                        volume: parse_u32(v_idx).unwrap_or(0),
                                    });
                                }
                            }
                        }
                        Err(e) => warn!("[InfluxFetcher] CSV record error: {}", e),
                    }
                }
            } else {
                return Err(CoreError::Config("CSV header mismatch".to_string()));
            }
        }

        Ok(candles)
    }
}
