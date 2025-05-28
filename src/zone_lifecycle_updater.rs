// src/zone_lifecycle_updater.rs (formerly stale_zone_checker.rs)
use std::sync::Arc;
use std::env;
use reqwest::Client as HttpClient;
use tokio::time::{interval, Duration};
use tokio::sync::Semaphore;
use log::{info, warn, error, debug};
use serde_json::json;
use futures::future::join_all;

// Assuming StoredZone, ZoneCsvRecord, map_csv_to_stored_zone are in types.rs and pub
use crate::types::{StoredZone, ZoneCsvRecord, map_csv_to_stored_zone};
// Assuming revalidate_one_zone_activity_by_id is in zone_revalidator_util.rs and is pub
use crate::zone_revalidator_util::{revalidate_one_zone_activity_by_id};

// --- BEGIN: Text Logger for Zone Lifecycle Updater ---
use chrono::Local as ChronoLocal; // Alias to avoid conflict
use lazy_static::lazy_static;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::sync::Mutex as StdMutex; // Alias for std::sync::Mutex

lazy_static! {
    static ref LIFECYCLE_UPDATER_LOG: StdMutex<BufWriter<File>> = {
        let file_path = "zone_lifecycle_updater_log.txt";
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)
            .unwrap_or_else(|e| panic!("Failed to open or create {}: {}", file_path, e));
        StdMutex::new(BufWriter::new(file))
    };
}

fn log_event_to_text_file(event_type: &str, zone_id_opt: Option<&str>, details: &str) {
    let log_timestamp = ChronoLocal::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let zone_id_str = zone_id_opt.unwrap_or("N/A");
    let log_line = format!("[{}] Event: {}, ZoneID: {}, Details: {}\n",
        log_timestamp, event_type, zone_id_str, details
    );

    match LIFECYCLE_UPDATER_LOG.lock() {
        Ok(mut writer) => {
            if let Err(e) = writer.write_all(log_line.as_bytes()) {
                error!("[LIFECYCLE_UPDATER_TEXT_LOG_WRITE_ERROR] {}", e);
            }
            let _ = writer.flush();
        }
        Err(e) => {
            error!("[LIFECYCLE_UPDATER_TEXT_LOG_LOCK_ERROR] {}", e);
        }
    }
}

// Enhanced logger for lifecycle update details
fn log_lifecycle_update_details_to_text(
    original_zone_id: &str,
    updated_zone: &StoredZone,
    action_taken: &str, // e.g., "Updated", "Deactivated", "NoChange"
    old_bars_active: Option<u64>,
    old_touch_count: Option<i64>,
    old_is_active: bool,
) {
    let log_timestamp = ChronoLocal::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let symbol = updated_zone.symbol.as_deref().unwrap_or("N/A");
    let timeframe = updated_zone.timeframe.as_deref().unwrap_or("N/A");
    let zone_type = updated_zone.zone_type.as_deref().unwrap_or("N/A");
    let formation_time_str = updated_zone.start_time.as_deref().unwrap_or("N/A");
    
    let new_is_active = updated_zone.is_active;
    let new_touch_count = updated_zone.touch_count.unwrap_or(0);
    let new_bars_active = updated_zone.bars_active.unwrap_or(0);
    
    let old_bars_str = old_bars_active.map_or("N/A".to_string(), |b| b.to_string());
    let old_touches_str = old_touch_count.map_or("N/A".to_string(), |t| t.to_string());
    
    let days_formed_ago_str = if !formation_time_str.eq("N/A") {
        if let Ok(form_time) = chrono::DateTime::parse_from_rfc3339(formation_time_str) {
            let duration_since_formation = chrono::Utc::now().signed_duration_since(form_time.with_timezone(&chrono::Utc));
            format!("{}d ago", duration_since_formation.num_days())
        } else { "InvalidDate".to_string() }
    } else { "N/A".to_string() };

    let log_line = format!(
        "[{}] Action: {}, ZoneID: {} | Symbol: {}, TF: {}, Type: {}, Formed: {} ({}) | Active: {}->{}, Touches: {}->{}, BarsActive: {}->{}\n",
        log_timestamp, action_taken, original_zone_id,
        symbol, timeframe, zone_type, formation_time_str, days_formed_ago_str,
        old_is_active, new_is_active, old_touches_str, new_touch_count, old_bars_str, new_bars_active
    );

    match LIFECYCLE_UPDATER_LOG.lock() {
        Ok(mut writer) => {
            if let Err(e) = writer.write_all(log_line.as_bytes()) {
                error!("[LIFECYCLE_UPDATER_DETAIL_LOG_WRITE_ERROR] {}", e);
            }
            let _ = writer.flush();
        }
        Err(e) => { 
            error!("[LIFECYCLE_UPDATER_DETAIL_LOG_LOCK_ERROR] {}", e); 
        }
    }
}
// --- END: Text Logger ---

/// Simplified: Fetches ALL active zones at once (no pagination) with configurable timeframe exclusions
/// More efficient for most use cases since active zones are typically manageable in size
async fn fetch_all_active_zones_for_lifecycle_update_simple(
    http_client: &HttpClient,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    zone_bucket: &str,
    zone_measurement: &str,
    max_zones: usize, // Safety limit to prevent memory issues
    excluded_timeframes: &[String], // NEW: Timeframes to exclude from processing
) -> Result<Vec<StoredZone>, String> {
    info!("[LIFECYCLE_UPDATER] Fetching active zones for lifecycle update (max: {}, excluding TFs: {:?})...", 
          max_zones, excluded_timeframes);

    // Build timeframe exclusion filter
    let timeframe_filter = if excluded_timeframes.is_empty() {
        "".to_string()
    } else {
        let exclusions = excluded_timeframes.iter()
            .map(|tf| format!("r.timeframe != \"{}\"", tf))
            .collect::<Vec<_>>()
            .join(" and ");
        format!("|> filter(fn: (r) => {})", exclusions)
    };

    // Simple query for ALL is_active=1 zones with timeframe exclusions
    let flux_query = format!(
        r#"
        from(bucket: "{zone_bucket}")
          |> range(start: 0) // No time restriction - get all active zones regardless of age
          |> filter(fn: (r) => r._measurement == "{zone_measurement}")
          |> pivot( 
              rowKey:["_time", "symbol", "timeframe", "pattern", "zone_type"],
              columnKey: ["_field"],
              valueColumn: "_value"
             )
          |> filter(fn: (r) => exists r.is_active and r.is_active == 1) 
          {timeframe_filter}
          |> sort(columns: ["_time"], desc: false) // Process oldest formations first
          |> limit(n: {max_zones}) // Safety limit
          |> yield(name: "all_active_zones_for_lifecycle_update")
        "#,
        zone_bucket = zone_bucket,
        zone_measurement = zone_measurement,
        timeframe_filter = timeframe_filter,
        max_zones = max_zones
    );

    debug!("[LIFECYCLE_UPDATER] Query for active zones: {}", flux_query);
    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);
    
    let response = match http_client.post(&query_url)
        .bearer_auth(influx_token)
        .header("Accept", "application/csv")
        .header("Content-Type", "application/json")
        .json(&json!({ "query": flux_query, "type": "flux" }))
        .send()
        .await {
        Ok(resp) => resp,
        Err(e) => return Err(format!("HTTP request for active zones failed: {}", e)),
    };

    let status = response.status();
    if !status.is_success() {
        let err_body = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        error!("[LIFECYCLE_UPDATER] InfluxDB query for active zones failed (status {}): {}. Query: {}", 
               status, err_body, flux_query);
        return Err(format!("InfluxDB query for active zones failed (status {}): {}", status, err_body));
    }

    let csv_text = response.text().await.map_err(|e| format!("Failed to read CSV response for active zones: {}", e))?;
    let mut zones: Vec<StoredZone> = Vec::new();
    
    if csv_text.lines().skip_while(|l| l.starts_with('#') || l.is_empty()).count() > 1 {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .comment(Some(b'#'))
            .from_reader(csv_text.as_bytes());
            
        for result in rdr.deserialize::<ZoneCsvRecord>() {
            match result {
                Ok(csv_rec) => {
                    let zone = map_csv_to_stored_zone(csv_rec);
                    // Double-check that zone is actually active (defensive programming)
                    if zone.is_active {
                        zones.push(zone);
                    }
                },
                Err(e) => warn!("[LIFECYCLE_UPDATER] CSV deserialize error for zone row: {}", e),
            }
        }
    }
    
    info!("[LIFECYCLE_UPDATER] Found {} active zones to update in this batch.", zones.len());
    Ok(zones)
}

/// Deactivates very old zones from short timeframes to prevent database bloat
/// This is similar to the old function but with better naming and logging
async fn deactivate_ancient_short_timeframe_zones(
    http_client: &HttpClient,
    influx_host: &str, 
    influx_org: &str, 
    influx_token: &str,
    zone_bucket: &str, 
    zone_measurement: &str,
    deactivation_age_days: i64,
) -> Result<usize, String> {
    if deactivation_age_days <= 0 {
        info!("[LIFECYCLE_UPDATER] Short timeframe deactivation disabled (age <= 0).");
        return Ok(0);
    }

    let timeframes_to_deactivate = ["1m", "5m", "15m"]; // Expanded to include 1m
    info!("[LIFECYCLE_UPDATER] Deactivating ancient {:?} zones older than {} days to prevent database bloat.",
        timeframes_to_deactivate, deactivation_age_days);

    let cutoff_time_for_old_zones = format!("-{}d", deactivation_age_days);
    let mut deactivated_count = 0;

    for tf_to_check in timeframes_to_deactivate.iter() {
        let query_for_specific_tf = format!(
            r#"
            from(bucket: "{zone_bucket}")
              |> range(start: 0, stop: {cutoff_time_for_old_zones}) 
              |> filter(fn: (r) => r._measurement == "{zone_measurement}")
              |> filter(fn: (r) => r.timeframe == "{tf_to_check}") 
              |> pivot(rowKey:["_time", "symbol", "timeframe", "pattern", "zone_type"], columnKey: ["_field"], valueColumn: "_value")
              |> filter(fn: (r) => exists r.is_active and r.is_active == 1)
              |> yield(name: "ancient_short_tf_active_zones_{tf_to_check}")
            "#,
            zone_bucket = zone_bucket,
            zone_measurement = zone_measurement,
            cutoff_time_for_old_zones = cutoff_time_for_old_zones,
            tf_to_check = tf_to_check
        );

        debug!("[LIFECYCLE_UPDATER] Deactivation query for {}: {}", tf_to_check, query_for_specific_tf);
        let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);

        let response = match http_client.post(&query_url)
            .bearer_auth(influx_token)
            .header("Accept", "application/csv")
            .header("Content-Type", "application/json")
            .json(&json!({ "query": query_for_specific_tf, "type": "flux" }))
            .send()
            .await {
            Ok(resp) => resp,
            Err(e) => { 
                error!("[LIFECYCLE_UPDATER] HTTP request for ancient {} zones failed: {}", tf_to_check, e); 
                continue; 
            }
        };
        
        let status = response.status();
        if !status.is_success() { 
            let err_body = response.text().await.unwrap_or_default();
            error!("[LIFECYCLE_UPDATER] Query failed for ancient {} zones (status {}): {}", tf_to_check, status, err_body);
            continue; 
        }
        
        let csv_text = match response.text().await { 
            Ok(t) => t, 
            Err(e) => {
                error!("[LIFECYCLE_UPDATER] Failed to read CSV for ancient {} zones: {}", tf_to_check, e);
                continue;
            }
        };
        
        if csv_text.lines().skip_while(|l| l.starts_with('#') || l.is_empty()).count() > 1 {
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(true)
                .flexible(true)
                .comment(Some(b'#'))
                .from_reader(csv_text.as_bytes());
                
            for result in rdr.deserialize::<ZoneCsvRecord>() {
                if let Ok(csv_rec) = result {
                    let mut zone = map_csv_to_stored_zone(csv_rec);
                    if zone.is_active {
                        let old_is_active = zone.is_active;
                        let old_bars_active = zone.bars_active;
                        let old_touch_count = zone.touch_count;
                        
                        zone.is_active = false; // Mark for deactivation
                        info!("[LIFECYCLE_UPDATER] Marking ancient zone ID {:?} ({}/{}) for deactivation (older than {}d).",
                            zone.zone_id, 
                            zone.symbol.as_deref().unwrap_or("?"), 
                            zone.timeframe.as_deref().unwrap_or("?"), 
                            deactivation_age_days);
                        
                        // Update the zone in InfluxDB
                        match crate::zone_revalidator_util::update_zone_in_influxdb(
                            &zone, http_client,
                            influx_host, influx_org, influx_token,
                            zone_bucket, zone_measurement
                        ).await {
                            Ok(_) => {
                                log_lifecycle_update_details_to_text(
                                    zone.zone_id.as_deref().unwrap_or("?"), 
                                    &zone, 
                                    "Deactivated (Ancient/ShortTF)",
                                    old_bars_active,
                                    old_touch_count,
                                    old_is_active
                                );
                                deactivated_count += 1;
                            }
                            Err(e) => {
                                error!("[LIFECYCLE_UPDATER] Failed to deactivate ancient zone ID {:?}: {}", zone.zone_id, e);
                                log_event_to_text_file("DEACTIVATE_ANCIENT_FAIL", zone.zone_id.as_deref(), &e);
                            }
                        }
                        
                        // Small delay to avoid overwhelming InfluxDB
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }
    
    info!("[LIFECYCLE_UPDATER] Deactivated {} ancient short timeframe zones in this cycle.", deactivated_count);
    Ok(deactivated_count)
}

/// Main service function for the Zone Lifecycle Updater
/// This replaces run_stale_zone_check_service with improved functionality
pub async fn run_zone_lifecycle_updater_service(http_client: Arc<HttpClient>) {
    let update_interval_mins_str = env::var("ZONE_LIFECYCLE_UPDATE_INTERVAL_MINUTES").unwrap_or_else(|_| "10".to_string());
    let update_interval_mins = update_interval_mins_str.parse::<u64>().unwrap_or(10);
    
    if update_interval_mins == 0 { 
        info!("[LIFECYCLE_UPDATER] Update interval is 0, service won't run periodically."); 
        return; 
    }

    // No minimum age restriction - we update ALL active zones (except excluded timeframes)
    let max_zones_per_cycle_str = env::var("ZONE_LIFECYCLE_MAX_ZONES").unwrap_or_else(|_| "1000".to_string());
    let max_zones_per_cycle = max_zones_per_cycle_str.parse::<usize>().unwrap_or(1000);
    
    let concurrency_limit_str = env::var("ZONE_LIFECYCLE_CONCURRENCY_LIMIT").unwrap_or_else(|_| "8".to_string());
    let concurrency_limit = concurrency_limit_str.parse::<usize>().unwrap_or(8).max(1);
    
    // NEW: Read excluded timeframes from same env var as generator
    let excluded_timeframes_str = env::var("GENERATOR_EXCLUDE_TIMEFRAMES").unwrap_or_default();
    let excluded_timeframes: Vec<String> = excluded_timeframes_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    
    // Deactivation age for very old short timeframe zones (to prevent DB bloat)
    let ancient_deactivation_age_days_str = env::var("ANCIENT_ZONE_DEACTIVATION_AGE_DAYS").unwrap_or_else(|_| "60".to_string());
    let ancient_deactivation_age_days = ancient_deactivation_age_days_str.parse::<i64>().unwrap_or(60);

    let influx_host = env::var("INFLUXDB_HOST").expect("LIFECYCLE_UPDATER: INFLUXDB_HOST not set");
    let influx_org = env::var("INFLUXDB_ORG").expect("LIFECYCLE_UPDATER: INFLUXDB_ORG not set");
    let influx_token = env::var("INFLUXDB_TOKEN").expect("LIFECYCLE_UPDATER: INFLUXDB_TOKEN not set");
    let zone_bucket = env::var("GENERATOR_WRITE_BUCKET").expect("LIFECYCLE_UPDATER: GENERATOR_WRITE_BUCKET not set");
    let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT").expect("LIFECYCLE_UPDATER: GENERATOR_ZONE_MEASUREMENT not set");
    let candle_bucket = env::var("INFLUXDB_BUCKET").expect("LIFECYCLE_UPDATER: INFLUXDB_BUCKET (for candles) not set");
    let candle_measurement = env::var("GENERATOR_TRENDBAR_MEASUREMENT").expect("LIFECYCLE_UPDATER: GENERATOR_TRENDBAR_MEASUREMENT not set");

    info!("[LIFECYCLE_UPDATER] Zone Lifecycle Updater service started.");
    info!("[LIFECYCLE_UPDATER] Update Interval: {}m, Max Zones/Cycle: {}, Concurrency: {}", 
          update_interval_mins, max_zones_per_cycle, concurrency_limit);
    info!("[LIFECYCLE_UPDATER] Excluded Timeframes: {:?}", excluded_timeframes);
    info!("[LIFECYCLE_UPDATER] Ancient Zone Deactivation Age: {}d", ancient_deactivation_age_days);

    let mut interval_timer = interval(Duration::from_secs(update_interval_mins * 60));
    let mut first_run = true;

    loop {
        if !first_run { 
            interval_timer.tick().await; 
        }
        first_run = false;
        
        info!("[LIFECYCLE_UPDATER] Starting zone lifecycle update cycle...");
        log_event_to_text_file("CYCLE_START", None, "Beginning new lifecycle update cycle");

        // Step 1: Deactivate ancient short timeframe zones to prevent database bloat
        if ancient_deactivation_age_days > 0 {
            match deactivate_ancient_short_timeframe_zones(
                &http_client, 
                &influx_host, 
                &influx_org, 
                &influx_token, 
                &zone_bucket, 
                &zone_measurement, 
                ancient_deactivation_age_days
            ).await {
                Ok(count) => {
                    info!("[LIFECYCLE_UPDATER] Ancient zone cleanup complete. Deactivated {} zones.", count);
                    log_event_to_text_file("ANCIENT_CLEANUP_COMPLETE", None, &format!("Deactivated {} ancient zones", count));
                }
                Err(e) => {
                    error!("[LIFECYCLE_UPDATER] Error during ancient zone cleanup: {}", e);
                    log_event_to_text_file("ANCIENT_CLEANUP_ERROR", None, &e);
                }
            }
        }

        // Step 2: Process active zones using simplified approach with timeframe exclusions
        let mut total_updated_in_cycle = 0;
        let mut total_errors_in_cycle = 0;

        // Simplified approach - fetch all active zones at once (excluding specified timeframes)
        match fetch_all_active_zones_for_lifecycle_update_simple(
            &http_client, 
            &influx_host, 
            &influx_org, 
            &influx_token, 
            &zone_bucket, 
            &zone_measurement, 
            max_zones_per_cycle, // Use configured max zones
            &excluded_timeframes // Pass excluded timeframes
        ).await {
            Ok(zones_to_update) => {
                if zones_to_update.is_empty() {
                    info!("[LIFECYCLE_UPDATER] No active zones found for lifecycle update.");
                    log_event_to_text_file("NO_ACTIVE_ZONES", None, "No active zones found in database");
                } else {
                    info!("[LIFECYCLE_UPDATER] Processing {} active zones for lifecycle update...", zones_to_update.len());

                    let semaphore = Arc::new(Semaphore::new(concurrency_limit));
                    let mut join_handles = Vec::new();

                    for zone_data in zones_to_update {
                        if let Some(zone_id) = &zone_data.zone_id {
                            let permit_clone = Arc::clone(&semaphore);
                            let client_clone = Arc::clone(&http_client);
                            let ih_clone = influx_host.clone(); 
                            let io_clone = influx_org.clone(); 
                            let it_clone = influx_token.clone();
                            let zb_clone = zone_bucket.clone(); 
                            let zm_clone = zone_measurement.clone();
                            let cb_clone = candle_bucket.clone(); 
                            let cm_clone = candle_measurement.clone();
                            let zid_clone = zone_id.clone();

                            // Store old values for logging
                            let old_bars_active = zone_data.bars_active;
                            let old_touch_count = zone_data.touch_count;
                            let old_is_active = zone_data.is_active;

                            join_handles.push(tokio::spawn(async move {
                                let _permit = permit_clone.acquire_owned().await.expect("Semaphore permit failed");
                                
                                let reval_result = revalidate_one_zone_activity_by_id(
                                    zid_clone.clone(), 
                                    &client_clone, 
                                    &ih_clone, 
                                    &io_clone, 
                                    &it_clone, 
                                    &zb_clone, 
                                    &zm_clone, 
                                    &cb_clone, 
                                    &cm_clone
                                ).await;

                                match &reval_result {
                                    Ok(Some(updated_zone)) => {
                                        // Check if there were actual changes
                                        let bars_changed = updated_zone.bars_active != old_bars_active;
                                        let touches_changed = updated_zone.touch_count != old_touch_count;
                                        let activity_changed = updated_zone.is_active != old_is_active;
                                        
                                        if bars_changed || touches_changed || activity_changed {
                                            log_lifecycle_update_details_to_text(
                                                &zid_clone, 
                                                updated_zone, 
                                                "Updated",
                                                old_bars_active,
                                                old_touch_count,
                                                old_is_active
                                            );
                                        } else {
                                            // No changes - log only at debug level to avoid spam
                                            debug!("[LIFECYCLE_UPDATER] Zone {} revalidated but no lifecycle changes detected", zid_clone);
                                        }
                                    }
                                    Ok(None) => {
                                        log_event_to_text_file("NOT_FOUND_OR_NO_UPDATE", Some(&zid_clone), "Zone not found by revalidator or no update needed");
                                    }
                                    Err(e_str) => {
                                        log_event_to_text_file("REVALIDATION_ERROR", Some(&zid_clone), e_str);
                                    }
                                }
                                reval_result
                            }));
                        }
                    }

                    let results = join_all(join_handles).await;
                    let (successful_updates, failed_updates) = results.into_iter().fold((0, 0), |(s, f), res| {
                        match res { 
                            Ok(Ok(Some(_))) => (s + 1, f),     // Successful update
                            Ok(Ok(None)) => (s, f),            // No update needed (count as success)
                            _ => (s, f + 1)                     // Error or panic
                        }
                    });

                    total_updated_in_cycle += successful_updates;
                    total_errors_in_cycle += failed_updates;

                    info!("[LIFECYCLE_UPDATER] All zones processed. Success: {}, Failures: {}", 
                          successful_updates, failed_updates);
                }
            }
            Err(e) => {
                error!("[LIFECYCLE_UPDATER] Failed to fetch active zones: {}", e);
                log_event_to_text_file("FETCH_ERROR", None, &e);
                total_errors_in_cycle += 1;
            }
        }

        info!("[LIFECYCLE_UPDATER] Lifecycle update cycle complete. Total updated: {}, Total errors: {}", 
              total_updated_in_cycle, total_errors_in_cycle);
        
        log_event_to_text_file("CYCLE_COMPLETE", None, 
                               &format!("Updated: {}, Errors: {}", 
                                       total_updated_in_cycle, total_errors_in_cycle));

        info!("[LIFECYCLE_UPDATER] Next update cycle in {} minutes.", update_interval_mins);
    }
}