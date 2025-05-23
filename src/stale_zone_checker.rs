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

// --- BEGIN: Text Logger for Stale Zone Checker ---
use chrono::Local as ChronoLocal; // Alias to avoid conflict
use lazy_static::lazy_static;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::sync::Mutex as StdMutex; // Alias for std::sync::Mutex

lazy_static! {
    static ref STALE_CHECKER_LOG: StdMutex<BufWriter<File>> = {
        let file_path = "stale_zone_checker_log.txt";
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

    match STALE_CHECKER_LOG.lock() {
        Ok(mut writer) => {
            if let Err(e) = writer.write_all(log_line.as_bytes()) {
                error!("[STALE_CHECKER_TEXT_LOG_WRITE_ERROR] {}", e);
            }
            let _ = writer.flush();
        }
        Err(e) => {
            error!("[STALE_CHECKER_TEXT_LOG_LOCK_ERROR] {}", e);
        }
    }
}

// Modified logger for revalidated zone details
fn log_revalidated_zone_details_to_text(
    original_zone_id: &str,
    revalidated_zone: &StoredZone,
    action_taken: &str, // e.g., "Revalidated", "Deactivated"
) {
    let log_timestamp = ChronoLocal::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let symbol = revalidated_zone.symbol.as_deref().unwrap_or("N/A");
    let timeframe = revalidated_zone.timeframe.as_deref().unwrap_or("N/A");
    let zone_type = revalidated_zone.zone_type.as_deref().unwrap_or("N/A");
    let formation_time_str = revalidated_zone.start_time.as_deref().unwrap_or("N/A");
    let is_active_now = revalidated_zone.is_active;
    let touches_now = revalidated_zone.touch_count.map_or("N/A".to_string(), |t| t.to_string());
    let bars_active_now = revalidated_zone.bars_active.map_or("N/A".to_string(), |b| b.to_string());
    
    let days_formed_ago_str = if !formation_time_str.eq("N/A") {
        if let Ok(form_time) = chrono::DateTime::parse_from_rfc3339(formation_time_str) {
            let duration_since_formation = chrono::Utc::now().signed_duration_since(form_time.with_timezone(&chrono::Utc));
            format!("{}d ago", duration_since_formation.num_days())
        } else { "InvalidDate".to_string() }
    } else { "N/A".to_string() };

    let log_line = format!(
        "[{}] Action: {}, ZoneID: {} | Symbol: {}, TF: {}, Type: {}, Formed: {} ({}), NewActive: {}, NewTouches: {}, NewBarsActive: {}\n",
        log_timestamp, action_taken, original_zone_id,
        symbol, timeframe, zone_type, formation_time_str, days_formed_ago_str,
        is_active_now, touches_now, bars_active_now
    );
    // This function now only formats, actual writing is done by log_event_to_text_file or similar
    // For direct writing here:
     match STALE_CHECKER_LOG.lock() {
        Ok(mut writer) => {
            if let Err(e) = writer.write_all(log_line.as_bytes()) {
                error!("[STALE_CHECKER_DETAIL_LOG_WRITE_ERROR] {}", e);
            }
            let _ = writer.flush();
        }
        Err(e) => { error!("[STALE_CHECKER_DETAIL_LOG_LOCK_ERROR] {}", e); }
    }
}
// --- END: Text Logger ---


async fn fetch_candidate_stale_zones(
    http_client: &HttpClient,
    influx_host: &str,
    influx_org: &str,
    influx_token: &str,
    zone_bucket: &str,
    zone_measurement: &str,
    min_age_to_be_stale_candidate_days: i64,
    limit_of_zones_to_check: usize,
) -> Result<Vec<StoredZone>, String> {
    info!("[STALE_CHECKER] Fetching general stale zone candidates (older than {}d, limit {})...",
        min_age_to_be_stale_candidate_days, limit_of_zones_to_check);

    let newest_formation_to_still_be_stale_candidate = format!("-{}d", min_age_to_be_stale_candidate_days);

    let flux_query = format!(
        r#"
        from(bucket: "{zone_bucket}")
          |> range(start: 0, stop: {newest_formation_to_still_be_stale_candidate})
          |> filter(fn: (r) => r._measurement == "{zone_measurement}")
          |> pivot( 
              rowKey:["_time", "symbol", "timeframe", "pattern", "zone_type"],
              columnKey: ["_field"],
              valueColumn: "_value"
             )
          |> filter(fn: (r) => exists r.is_active and r.is_active == 1) 
          |> sort(columns: ["_time"], desc: false) 
          |> limit(n: {limit_of_zones_to_check})
          |> yield(name: "candidate_stale_zones")
        "#,
        zone_bucket = zone_bucket,
        zone_measurement = zone_measurement,
        newest_formation_to_still_be_stale_candidate = newest_formation_to_still_be_stale_candidate,
        limit_of_zones_to_check = limit_of_zones_to_check
    );

    debug!("[STALE_CHECKER] Query for general stale candidates: {}", flux_query);
    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);
    
    let response = match http_client.post(&query_url)
        .bearer_auth(influx_token).header("Accept", "application/csv")
        .header("Content-Type", "application/json").json(&json!({ "query": flux_query, "type": "flux" })).send().await {
        Ok(resp) => resp,
        Err(e) => return Err(format!("HTTP request for stale zones failed: {}", e)),
    };

    let status = response.status();
    if !status.is_success() {
        let err_body = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        error!("[STALE_CHECKER] InfluxDB query for stale zones failed (status {}): {}. Query: {}", status, err_body, flux_query);
        return Err(format!("InfluxDB query for stale zones failed (status {}): {}", status, err_body));
    }

    let csv_text = response.text().await.map_err(|e| format!("Failed to read CSV response for stale zones: {}", e))?;
    let mut zones: Vec<StoredZone> = Vec::new();
    if csv_text.lines().skip_while(|l| l.starts_with('#') || l.is_empty()).count() > 1 {
        let mut rdr = csv::ReaderBuilder::new().has_headers(true).flexible(true).comment(Some(b'#')).from_reader(csv_text.as_bytes());
        for result in rdr.deserialize::<ZoneCsvRecord>() {
            match result {
                Ok(csv_rec) => zones.push(map_csv_to_stored_zone(csv_rec)),
                Err(e) => warn!("[STALE_CHECKER] CSV deserialize error for zone row: {}", e),
            }
        }
    }
    info!("[STALE_CHECKER] Found {} general candidate stale zones to revalidate.", zones.len());
    Ok(zones)
}

async fn deactivate_old_short_tf_zones(
    http_client: &HttpClient,
    influx_host: &str, influx_org: &str, influx_token: &str,
    zone_bucket: &str, zone_measurement: &str,
    deactivation_age_days: i64,
) -> Result<usize, String> {
    let timeframes_to_deactivate = ["5m", "15m"];
    info!("[STALE_DEACTIVATE] Checking active {:?} zones older than {} days.",
        timeframes_to_deactivate, deactivation_age_days);

    let cutoff_time_for_old_zones = format!("-{}d", deactivation_age_days);
    let mut deactivated_count = 0;

    for tf_to_check in timeframes_to_deactivate.iter() {
        let flux_query = format!(r#" /* ... query as in previous response ... */ "#); // Same query as before
         let query_for_specific_tf = format!(
            r#"
            from(bucket: "{zone_bucket}")
              |> range(start: 0, stop: {cutoff_time_for_old_zones}) 
              |> filter(fn: (r) => r._measurement == "{zone_measurement}")
              |> filter(fn: (r) => r.timeframe == "{tf_to_check}") 
              |> pivot(rowKey:["_time", "symbol", "timeframe", "pattern", "zone_type"], columnKey: ["_field"], valueColumn: "_value")
              |> filter(fn: (r) => exists r.is_active and r.is_active == 1)
              |> yield(name: "old_short_tf_active_zones_{tf_to_check}")
            "#,
            zone_bucket = zone_bucket,
            zone_measurement = zone_measurement,
            cutoff_time_for_old_zones = cutoff_time_for_old_zones,
            tf_to_check = tf_to_check
        );

        debug!("[STALE_DEACTIVATE] Query for {}: {}", tf_to_check, query_for_specific_tf);
        let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);

        let response = match http_client.post(&query_url).bearer_auth(influx_token)
            .header("Accept", "application/csv").header("Content-Type", "application/json")
            .json(&json!({ "query": query_for_specific_tf, "type": "flux" })).send().await {
            Ok(resp) => resp,
            Err(e) => { error!("[STALE_DEACTIVATE] HTTP request for old {} zones failed: {}", tf_to_check, e); continue; }
        };
        // ... (rest of fetch and parse logic from previous complete version of this function)
        let status = response.status();
        if !status.is_success() { /* ... log and continue ... */ continue; }
        let csv_text = match response.text().await { Ok(t) => t, Err(_) => continue, };
        if csv_text.lines().skip_while(|l| l.starts_with('#') || l.is_empty()).count() > 1 {
            let mut rdr = csv::ReaderBuilder::new().has_headers(true).flexible(true).comment(Some(b'#')).from_reader(csv_text.as_bytes());
            for result in rdr.deserialize::<ZoneCsvRecord>() {
                if let Ok(csv_rec) = result {
                    let mut zone = map_csv_to_stored_zone(csv_rec);
                    if zone.is_active {
                        zone.is_active = false; // Mark for deactivation
                        info!("[STALE_DEACTIVATE] Marking zone ID {:?} ({}/{}) for deactivation (older than {}d).",
                            zone.zone_id, zone.symbol.as_deref().unwrap_or("?"), zone.timeframe.as_deref().unwrap_or("?"), deactivation_age_days);
                        
                        // Call the update function from zone_revalidator_util
                        match crate::zone_revalidator_util::update_zone_in_influxdb(
                            &zone, http_client,
                            influx_host, influx_org, influx_token,
                            zone_bucket, zone_measurement
                        ).await {
                            Ok(_) => {
                                log_revalidated_zone_details_to_text(zone.zone_id.as_deref().unwrap_or("?"), &zone, "Deactivated (Age/ShortTF)");
                                deactivated_count += 1;
                            }
                            Err(e) => {
                                error!("[STALE_DEACTIVATE] Failed to DB update zone ID {:?} as inactive: {}", zone.zone_id, e);
                                log_event_to_text_file("DEACTIVATE_DB_FAIL", zone.zone_id.as_deref(), &e);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
    info!("[STALE_DEACTIVATE] Deactivated {} old 5m/15m zones in this cycle.", deactivated_count);
    Ok(deactivated_count)
}


pub async fn run_stale_zone_check_service(http_client: Arc<HttpClient>) {
    let check_interval_mins_str = env::var("STALE_ZONE_CHECK_INTERVAL_MINUTES").unwrap_or_else(|_| "15".to_string());
    let check_interval_mins = check_interval_mins_str.parse::<u64>().unwrap_or(15);
    if check_interval_mins == 0 { info!("[STALE_CHECKER] Interval 0, service won't run periodically."); return; }

    let min_age_to_be_stale_candidate_days_str = env::var("STALE_ZONE_MIN_AGE_DAYS").unwrap_or_else(|_| "2".to_string());
    let min_age_to_be_stale_candidate_days = min_age_to_be_stale_candidate_days_str.parse::<i64>().unwrap_or(2);
    let limit_per_run_str = env::var("STALE_ZONE_LIMIT_PER_RUN").unwrap_or_else(|_| "20".to_string());
    let limit_per_run = limit_per_run_str.parse::<usize>().unwrap_or(20);
    let concurrency_limit_str = env::var("STALE_ZONE_CONCURRENCY_LIMIT").unwrap_or_else(|_| "5".to_string());
    let concurrency_limit = concurrency_limit_str.parse::<usize>().unwrap_or(5).max(1);
    let short_tf_deactivation_age_days_str = env::var("SHORT_TF_DEACTIVATION_AGE_DAYS").unwrap_or_else(|_| "30".to_string());
    let short_tf_deactivation_age_days = short_tf_deactivation_age_days_str.parse::<i64>().unwrap_or(30);

    let influx_host = env::var("INFLUXDB_HOST").expect("STALE_CHECKER: INFLUXDB_HOST not set");
    let influx_org = env::var("INFLUXDB_ORG").expect("STALE_CHECKER: INFLUXDB_ORG not set");
    let influx_token = env::var("INFLUXDB_TOKEN").expect("STALE_CHECKER: INFLUXDB_TOKEN not set");
    let zone_bucket = env::var("GENERATOR_WRITE_BUCKET").expect("STALE_CHECKER: GENERATOR_WRITE_BUCKET not set");
    let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT").expect("STALE_CHECKER: GENERATOR_ZONE_MEASUREMENT not set");
    let candle_bucket = env::var("INFLUXDB_BUCKET").expect("STALE_CHECKER: INFLUXDB_BUCKET (for candles) not set");
    let candle_measurement_for_candles = env::var("GENERATOR_TRENDBAR_MEASUREMENT").expect("STALE_CHECKER: GENERATOR_TRENDBAR_MEASUREMENT not set");

    info!("[STALE_CHECKER] Service started. Main Interval: {}m. Stale Recheck MinAge: {}d. Short TF Deactivation Age: {}d. BatchLimit: {}, Concurrency: {}.",
        check_interval_mins, min_age_to_be_stale_candidate_days, short_tf_deactivation_age_days, limit_per_run, concurrency_limit);

    let mut interval = interval(Duration::from_secs(check_interval_mins * 60));
    let mut first_run = true;

    loop {
        if !first_run { interval.tick().await; }
        first_run = false;
        info!("[STALE_CHECKER] Starting full check cycle...");

        if short_tf_deactivation_age_days > 0 {
            match deactivate_old_short_tf_zones(&http_client, &influx_host, &influx_org, &influx_token, &zone_bucket, &zone_measurement, short_tf_deactivation_age_days).await {
                Ok(count) => info!("[STALE_CHECKER] Short TF deactivation run complete. Deactivated {} zones.", count),
                Err(e) => error!("[STALE_CHECKER] Error during deactivation of old short TF zones: {}", e),
            }
        }

        match fetch_candidate_stale_zones(&http_client, &influx_host, &influx_org, &influx_token, &zone_bucket, &zone_measurement, min_age_to_be_stale_candidate_days, limit_per_run).await {
            Ok(zones_to_revalidate_data) => {
                if zones_to_revalidate_data.is_empty() {
                    info!("[STALE_CHECKER] No general stale zones found for revalidation (older than {} day(s) and still active).", min_age_to_be_stale_candidate_days);
                    log_event_to_text_file("GENERAL_STALE_CYCLE_END", None, "No general stale candidates found.");
                } else {
                    info!("[STALE_CHECKER] Revalidating {} general stale zone(s) concurrently (max {} parallel)...", zones_to_revalidate_data.len(), concurrency_limit);
                    let semaphore = Arc::new(Semaphore::new(concurrency_limit));
                    let mut join_handles = Vec::new();
                    for zone_data_for_task in zones_to_revalidate_data {
                        if let Some(zone_id) = &zone_data_for_task.zone_id {
                            let permit_clone = Arc::clone(&semaphore);
                            let client_clone = Arc::clone(&http_client);
                            let ih_clone = influx_host.clone(); let io_clone = influx_org.clone(); let it_clone = influx_token.clone();
                            let zb_clone = zone_bucket.clone(); let zm_clone = zone_measurement.clone();
                            let cb_clone = candle_bucket.clone(); let cmc_clone = candle_measurement_for_candles.clone();
                            let zid_clone = zone_id.clone();
                            join_handles.push(tokio::spawn(async move {
                                let _permit = permit_clone.acquire_owned().await.expect("Semaphore permit failed");
                                let reval_result = revalidate_one_zone_activity_by_id( zid_clone.clone(), &client_clone, &ih_clone, &io_clone, &it_clone, &zb_clone, &zm_clone, &cb_clone, &cmc_clone).await;
                                match &reval_result {
                                    Ok(Some(zone)) => log_revalidated_zone_details_to_text(&zid_clone, zone, "Revalidated"),
                                    Ok(None) => log_event_to_text_file("REVAL_NO_ZONE_OR_NO_CHANGE", Some(&zid_clone), "Not found by revalidator or no update needed."),
                                    Err(e_str) => log_event_to_text_file("REVAL_ERROR", Some(&zid_clone), e_str),
                                }
                                reval_result
                            }));
                        }
                    }
                    let results = join_all(join_handles).await;
                    let (successful_revals, failed_revals) = results.into_iter().fold((0,0), |(s,f), res| {
                        match res { Ok(Ok(Some(_))) => (s+1,f), Ok(Ok(None)) => (s,f), _ => (s,f+1) }
                    });
                    info!("[STALE_CHECKER] General stale batch processed. Success/NoChange: {}, Failures/Panics: {}", successful_revals, failed_revals);
                    log_event_to_text_file("GENERAL_STALE_CYCLE_END", None, &format!("Batch complete. Success/NoChange: {}, Fail: {}", successful_revals, failed_revals));
                }
            }
            Err(e) => { error!("[STALE_CHECKER] Failed to fetch general stale zone candidates: {}", e); log_event_to_text_file("GENERAL_STALE_FETCH_ERROR", None, &e); }
        }
        info!("[STALE_CHECKER] Full check cycle complete. Next check in approx {} minutes.", check_interval_mins);
    }
}