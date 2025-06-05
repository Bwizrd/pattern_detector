// use crate::types::{StoredZone, CandleData}; // Using StoredZone and CandleData
// use crate::detect::fetch_candles_direct;    // Assuming this helper exists and is pub in detect.rs
// use std::sync::Arc;
// use reqwest::Client as HttpClient;
// use serde_json::json;
// use log::{info, warn, error, debug};

// // Assuming ZoneCsvRecord and map_csv_to_stored_zone have been moved to types.rs and are pub
// use crate::types::{ZoneCsvRecord, map_csv_to_stored_zone};

// // This is the enrichment logic. It needs to be robust and match your primary enrichment.
// // This version includes a more complete structure based on common needs.
// fn perform_enrichment_for_revalidation(
//     original_zone: &StoredZone, // The zone whose boundaries and initial state we're using
//     subsequent_candles: &[CandleData], // Candles strictly AFTER the zone's formation pattern ended
// ) -> (bool, i64, Option<u64>, Option<String>, Option<f64>) {
//     // Returns: (new_is_active, new_total_touch_count, new_total_bars_active, invalidation_time_if_any, new_strength_score)

//     // If no new candles, assume no change to activity status from this revalidation pass.
//     // Historical touch count and bars_active are preserved from the original zone.
//     if subsequent_candles.is_empty() {
//         debug!("[REVALIDATE_ENRICH] No subsequent candles for zone {:?}, activity status remains as per DB: {}", original_zone.zone_id, original_zone.is_active);
//         return (
//             original_zone.is_active,
//             original_zone.touch_count.unwrap_or(0),
//             original_zone.bars_active, // Preserve original bars_active
//             None, // No new invalidation event
//             original_zone.strength_score // Preserve original score
//         );
//     }

//     // Essential zone properties for processing
//     let zone_high = match original_zone.zone_high {
//         Some(zh) => zh,
//         None => {
//             warn!("[REVALIDATE_ENRICH] Zone ID {:?} is missing zone_high. Cannot revalidate.", original_zone.zone_id);
//             return (false, original_zone.touch_count.unwrap_or(0), original_zone.bars_active, None, Some(0.0));
//         }
//     };
//     let zone_low = match original_zone.zone_low {
//         Some(zl) => zl,
//         None => {
//             warn!("[REVALIDATE_ENRICH] Zone ID {:?} is missing zone_low. Cannot revalidate.", original_zone.zone_id);
//             return (false, original_zone.touch_count.unwrap_or(0), original_zone.bars_active, None, Some(0.0));
//         }
//     };
//     let zone_type_str = original_zone.zone_type.as_deref().unwrap_or("");
//     let is_supply = zone_type_str.eq_ignore_ascii_case("supply") || zone_type_str.eq_ignore_ascii_case("supply_zone");

//     let (proximal_line, distal_line) = if is_supply {
//         (zone_low, zone_high)
//     } else { // Assume demand
//         (zone_high, zone_low)
//     };

//     let mut current_is_active = true; // Assume active based on new candles, until invalidated
//     let mut touches_in_this_reval_period: i64 = 0;
//     let mut is_outside_zone_for_touch_counting = true;
//     let mut invalidation_time_found: Option<String> = None;
//     let mut bars_processed_in_reval: u64 = 0;

//     for candle in subsequent_candles {
//         bars_processed_in_reval += 1;

//         // 1. Invalidation Check (Distal Line Breach)
//         if (is_supply && candle.high > distal_line) || (!is_supply && candle.low < distal_line) {
//             current_is_active = false;
//             invalidation_time_found = Some(candle.time.clone());
//             debug!("[REVALIDATE_ENRICH] Zone {:?} invalidated by candle at {} (Price: H{}, L{} vs Distal: {})",
//                 original_zone.zone_id.as_deref().unwrap_or("N/A"), candle.time, candle.high, candle.low, distal_line);
//             break; // Stop processing further candles for this zone
//         }

//         // 2. Touch Check (Proximal Line Breach) - only if still active
//         let mut interaction_occurred = false;
//         if (is_supply && candle.high >= proximal_line) || (!is_supply && candle.low <= proximal_line) {
//             interaction_occurred = true;
//         }

//         if interaction_occurred && is_outside_zone_for_touch_counting {
//             touches_in_this_reval_period += 1;
//             debug!("[REVALIDATE_ENRICH] Zone {:?} new touch recorded by candle at {}. Total new touches in reval: {}",
//                 original_zone.zone_id.as_deref().unwrap_or("N/A"), candle.time, touches_in_this_reval_period);
//         }
//         is_outside_zone_for_touch_counting = !interaction_occurred;
//     }

//     // Combine historical touches with new touches found in this revalidation window
//     let new_total_touch_count = original_zone.touch_count.unwrap_or(0) + touches_in_this_reval_period;

//     // Calculate new bars_active:
//     // If invalidated in this window, it's original bars_active + bars_processed_in_reval up to invalidation.
//     // If still active, it's original bars_active + all subsequent_candles.len().
//     // This assumes `original_zone.bars_active` was correctly the count until `original_zone.end_time`.
//     let new_total_bars_active = original_zone.bars_active.unwrap_or(0) + bars_processed_in_reval;

//     // Recalculate strength score
//     let new_strength_score = Some(100.0 - (new_total_touch_count as f64 * 10.0).max(0.0).min(100.0));


//     (current_is_active, new_total_touch_count, Some(new_total_bars_active), invalidation_time_found, new_strength_score)
// }

// pub async fn update_zone_in_influxdb(
//     updated_zone: &StoredZone,
//     http_client: &HttpClient,
//     influx_host: &str, influx_org: &str, influx_token: &str,
//     zone_bucket: &str, zone_measurement: &str,
// ) -> Result<(), String> {
//     if updated_zone.zone_id.is_none() || updated_zone.start_time.is_none() {
//         return Err("Zone ID or start_time missing for DB update".to_string());
//     }

//     let mut fields_vec: Vec<String> = Vec::new();
//     // Always include zone_id as a field as per your schema from previous queries
//     fields_vec.push(format!("zone_id=\"{}\"", updated_zone.zone_id.as_ref().unwrap().escape_default()));

//     if let Some(v) = updated_zone.start_idx { fields_vec.push(format!("start_idx={}i", v)); }
//     if let Some(v) = updated_zone.end_idx { fields_vec.push(format!("end_idx={}i", v)); }
//     // start_time is the point's timestamp, end_time is a field
//     if let Some(v) = &updated_zone.end_time { fields_vec.push(format!("end_time_rfc3339=\"{}\"", v.escape_default())); } else { /* Optionally write empty string or omit */ }
//     if let Some(v) = updated_zone.zone_high { fields_vec.push(format!("zone_high={}", v)); }
//     if let Some(v) = updated_zone.zone_low { fields_vec.push(format!("zone_low={}", v)); }
//     if let Some(v) = updated_zone.fifty_percent_line { fields_vec.push(format!("fifty_percent_line={}", v)); }
//     if let Some(v) = &updated_zone.detection_method { fields_vec.push(format!("detection_method=\"{}\"", v.escape_default())); } else { /* ... */ }
//     if let Some(v) = updated_zone.extended { fields_vec.push(format!("extended={}", v)); } // Booleans are fine
//     if let Some(v) = updated_zone.extension_percent { fields_vec.push(format!("extension_percent={}", v)); }

//     fields_vec.push(format!("is_active={}i", if updated_zone.is_active { 1 } else { 0 })); // Crucial update

//     if let Some(v) = updated_zone.bars_active { fields_vec.push(format!("bars_active={}i", v)); }
//     if let Some(v) = updated_zone.touch_count { fields_vec.push(format!("touch_count={}i", v)); }
//     if let Some(v) = updated_zone.strength_score { fields_vec.push(format!("strength_score={}", v)); }
    
//     // formation_candles: serialize if you store it, ensure escaping
//     let candles_json_str = match serde_json::to_string(&updated_zone.formation_candles) {
//         Ok(s) => s.replace("\"", "\\\""), // Escape double quotes for line protocol
//         Err(_) => "[]".to_string(),
//     };
//     fields_vec.push(format!("formation_candles_json=\"{}\"", candles_json_str));


//     if fields_vec.is_empty() { return Err("No fields to update for zone".to_string()); }

//     let timestamp_nanos = chrono::DateTime::parse_from_rfc3339(updated_zone.start_time.as_ref().unwrap())
//         .map_err(|e| format!("Failed to parse zone start_time: {}", e))?
//         .timestamp_nanos_opt().ok_or_else(|| "Failed to get nanos from start_time".to_string())?;

//     // Tags: These MUST match the original point's tags to overwrite it.
//     // If any of these are None in StoredZone, use a default or skip the tag (which might create a new series).
//     let symbol_tag = format!("symbol={}", updated_zone.symbol.as_deref().unwrap_or("unknown_symbol").replace(" ", "\\ ").replace(",", "\\,").replace("=", "\\="));
//     let timeframe_tag = format!("timeframe={}", updated_zone.timeframe.as_deref().unwrap_or("unknown_tf").replace(" ", "\\ ").replace(",", "\\,").replace("=", "\\="));
//     let pattern_tag = format!("pattern={}", updated_zone.pattern.as_deref().unwrap_or("unknown_pattern").replace(" ", "\\ ").replace(",", "\\,").replace("=", "\\="));
//     let zone_type_tag = format!("zone_type={}", updated_zone.zone_type.as_deref().unwrap_or("unknown_type").replace(" ", "\\ ").replace(",", "\\,").replace("=", "\\="));
    
//     let tags_vec = vec![symbol_tag, timeframe_tag, pattern_tag, zone_type_tag];

//     let line = format!("{measurement},{tags} {fields} {timestamp}",
//         measurement = zone_measurement, tags = tags_vec.join(","),
//         fields = fields_vec.join(","), timestamp = timestamp_nanos
//     );

//     debug!("[DB_UPDATE] Line protocol for update: {}", line);
//     let write_url = format!("{}/api/v2/write?org={}&bucket={}&precision=ns", influx_host, influx_org, zone_bucket);

//     match http_client.post(&write_url).bearer_auth(influx_token).header("Content-Type", "text/plain; charset=utf-8").body(line).send().await {
//         Ok(response) => {
//             let status = response.status();
//             if status.is_success() {
//                 info!("[DB_UPDATE] Successfully updated zone ID: {}", updated_zone.zone_id.as_deref().unwrap_or("N/A"));
//                 Ok(())
//             } else {
//                 let err_text = response.text().await.unwrap_or_else(|_| format!("Unknown error updating zone {}", updated_zone.zone_id.as_deref().unwrap_or("N/A")));
//                 error!("[DB_UPDATE] Failed to update zone in InfluxDB (status {}): {}", status, err_text);
//                 Err(format!("Failed to update zone (status {}): {}", status, err_text))
//             }
//         }
//         Err(e) => {
//             error!("[DB_UPDATE] HTTP error updating zone in InfluxDB: {}", e);
//             Err(format!("HTTP error updating zone: {}", e))
//         }
//     }
// }

// pub async fn revalidate_one_zone_activity_by_id(
//     zone_id_to_check: String,
//     http_client: &Arc<HttpClient>,
//     influx_host: &str, influx_org: &str, influx_token: &str,
//     zone_bucket: &str, zone_measurement: &str,
//     candle_bucket: &str, candle_measurement_for_candles: &str,
// ) -> Result<Option<StoredZone>, String> {
//     info!("[REVALIDATE] Revalidating zone ID: {}", zone_id_to_check);

//     let query_for_zone = format!(
//         r#"
//         from(bucket: "{zone_bucket}")
//           |> range(start: 0) 
//           |> filter(fn: (r) => r._measurement == "{zone_measurement}")
//           |> pivot(rowKey:["_time"], columnKey: ["_field"], valueColumn: "_value")
//           |> filter(fn: (r) => r.zone_id == "{esc_zone_id}") // Use escaped zone_id
//           |> limit(n: 1)
//         "#,
//         zone_bucket = zone_bucket,
//         zone_measurement = zone_measurement,
//         esc_zone_id = zone_id_to_check.replace("\"", "\\\"") // Escape quotes for the query
//     );

//     debug!("[REVALIDATE] Querying for zone details: {}", query_for_zone);
//     let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);
    
//     let response = match http_client // Renamed to avoid conflict if you modify it
//         .post(&query_url)
//         .bearer_auth(influx_token)
//         .header("Accept", "application/csv")
//         .header("Content-Type", "application/json")
//         .json(&json!({ "query": query_for_zone, "type": "flux" }))
//         .send().await
//     {
//         Ok(resp) => resp,
//         Err(e) => return Err(format!("HTTP error fetching zone {}: {}", zone_id_to_check, e)),
//     };

//     let status = response.status(); 
//     if !status.is_success() {
//         let err_body = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
//         error!("[REVALIDATE] Failed to fetch zone {} details (status {}): {}. Query: {}", zone_id_to_check, status, err_body, query_for_zone);
//         return Err(format!("Failed to fetch zone {} details (status {}): {}", zone_id_to_check, status, err_body));
//     }

//     // <<< --- CRITICAL DEBUG LOG --- >>>
//     let text = match response.text().await {
//         Ok(t) => t,
//         Err(e) => return Err(format!("Error reading InfluxDB response text for zone {}: {}", zone_id_to_check, e)),
//     };
//     log::debug!("[REVALIDATE_RAW_CSV] For zone_id '{}', InfluxDB returned CSV:\n{}", zone_id_to_check, text);
//     // <<< --- END OF CRITICAL DEBUG LOG --- >>>
            
//     let mut original_stored_zones: Vec<StoredZone> = Vec::new();
//     // Check if there's more than just a header or empty lines
//     if text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).count() > 1 {
//         let mut rdr = csv::ReaderBuilder::new().has_headers(true).flexible(true).comment(Some(b'#')).from_reader(text.as_bytes());
//         for result in rdr.deserialize::<ZoneCsvRecord>() { 
//             match result {
//                 Ok(csv_rec) => {
//                     // Log the successfully deserialized CSV record before mapping
//                     if csv_rec.zone_id.as_deref() == Some(&zone_id_to_check) {
//                         log::debug!("[REVALIDATE_DESERIALIZED_CSV_REC] For target zone_id '{}', deserialized ZoneCsvRecord: {:?}", zone_id_to_check, csv_rec);
//                     }
//                     original_stored_zones.push(map_csv_to_stored_zone(csv_rec));
//                 }
//                 Err(e) => {
//                     // This is where your error is currently originating
//                     log::error!("[REVALIDATE_CSV_DESERIALIZE_ERROR] For zone_id '{}', CSV deserialize error: {}", zone_id_to_check, e);
//                     return Err(format!("CSV deserialize error for zone {}: {}", zone_id_to_check, e));
//                 }
//             }
//         }
//     } else {
//         warn!("[REVALIDATE] Zone ID {} query success but no data rows returned by InfluxDB or in CSV text.", zone_id_to_check);
//         return Ok(None);
//     }
    
//     // ... (rest of your revalidate_one_zone_activity_by_id function: processing original_stored_zones, fetching subsequent candles, etc.) ...
//     // Ensure the rest of the function (after original_stored_zones is populated) is present.
//     if original_stored_zones.is_empty() {
//         warn!("[REVALIDATE] Zone ID {} not found in database after attempting to parse CSV response.", zone_id_to_check);
//         return Ok(None);
//     }
//     let mut zone_to_revalidate = original_stored_zones.remove(0);

//     info!("[REVALIDATE] Found zone for revalidation: ID: {:?}, Formed: {:?}, Current DB is_active: {}, Current DB touches: {:?}",
//         zone_to_revalidate.zone_id, zone_to_revalidate.start_time, zone_to_revalidate.is_active, zone_to_revalidate.touch_count);

//     // ... (fetch_candles_direct logic, perform_enrichment_for_revalidation, update_zone_in_influxdb)
//     // For brevity, I'm omitting the rest but it needs to be there.
//     // The following is a placeholder for the rest of your logic
//     let activity_check_start_time = zone_to_revalidate.end_time.clone().unwrap_or_else(|| zone_to_revalidate.start_time.clone().unwrap_or_default());
//     if activity_check_start_time.is_empty() {
//         return Err(format!("Zone {} has no valid start/end time for revalidation.", zone_id_to_check));
//     }

//     let subsequent_candles: Vec<CandleData> = match fetch_candles_direct(
//         http_client.as_ref(),
//         influx_host, influx_org, influx_token,
//         candle_bucket, candle_measurement_for_candles,
//         zone_to_revalidate.symbol.as_deref().unwrap_or_default(),
//         zone_to_revalidate.timeframe.as_deref().unwrap_or_default(),
//         &activity_check_start_time,
//         "now()",
//     ).await {
//         Ok(candles) => candles,
//         Err(e) => return Err(format!("Candle fetch error for revalidation of zone {}: {}", zone_id_to_check, e)),
//     };
//     info!("[REVALIDATE] Fetched {} subsequent candles for zone {} to check activity (from {} to now()).", 
//         subsequent_candles.len(), zone_id_to_check, activity_check_start_time);

//     let (new_is_active, new_total_touch_count, new_total_bars_active_opt, _new_invalidation_time_opt, new_strength_score_opt) =
//         perform_enrichment_for_revalidation(&zone_to_revalidate, &subsequent_candles);

//     let mut updated_in_db = false;
//     // ... (comparisons and setting updated_in_db = true) ...
//     if zone_to_revalidate.is_active != new_is_active { zone_to_revalidate.is_active = new_is_active; updated_in_db = true; }
//     if zone_to_revalidate.touch_count != Some(new_total_touch_count) { zone_to_revalidate.touch_count = Some(new_total_touch_count); updated_in_db = true; }
//     if new_total_bars_active_opt.is_some() && zone_to_revalidate.bars_active != new_total_bars_active_opt { zone_to_revalidate.bars_active = new_total_bars_active_opt; updated_in_db = true; }
//     if new_strength_score_opt.is_some() && zone_to_revalidate.strength_score != new_strength_score_opt { zone_to_revalidate.strength_score = new_strength_score_opt; updated_in_db = true; }


//     if updated_in_db {
//         info!("[REVALIDATE] Changes detected for zone {}. Attempting to update InfluxDB.", zone_id_to_check);
//         if let Err(e) = crate::zone_revalidator_util::update_zone_in_influxdb(&zone_to_revalidate, http_client.as_ref(), influx_host, influx_org, influx_token, zone_bucket, zone_measurement).await {
//             return Err(format!("DB update error after revalidation for zone {}: {}", zone_id_to_check, e));
//         }
//     } else {
//         info!("[REVALIDATE] Zone {} status and relevant fields unchanged after revalidation.", zone_id_to_check);
//     }

//     Ok(Some(zone_to_revalidate))

// }