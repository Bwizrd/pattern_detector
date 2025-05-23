use std::sync::Arc;
use std::env;
use reqwest::Client as HttpClient;
use tokio::time::{interval, Duration};
use tokio::sync::Semaphore; // <<< Added for concurrency limiting
use log::{info, warn, error, debug};
use serde_json::json;
use futures::future::join_all;

// Assuming StoredZone, ZoneCsvRecord, map_csv_to_stored_zone are in types.rs and pub
use crate::types::{StoredZone, ZoneCsvRecord, map_csv_to_stored_zone};
// Assuming revalidate_one_zone_activity_by_id is in zone_revalidator_util.rs and is pub
use crate::zone_revalidator_util::{revalidate_one_zone_activity_by_id};

// Helper function to fetch candidate StoredZone objects directly
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
    info!("[STALE_CHECKER] Fetching candidate stale zones (older than {}d, limit {})...",
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

    debug!("[STALE_CHECKER] Query for candidate zones: {}", flux_query);
    let query_url = format!("{}/api/v2/query?org={}", influx_host, influx_org);
    
    let response = match http_client.post(&query_url)
        .bearer_auth(influx_token)
        .header("Accept", "application/csv")
        .header("Content-Type", "application/json")
        .json(&json!({ "query": flux_query, "type": "flux" })).send().await {
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
    info!("[STALE_CHECKER] Found {} candidate stale zones to revalidate.", zones.len());
    Ok(zones)
}


pub async fn run_stale_zone_check_service(http_client: Arc<HttpClient>) {
    let check_interval_mins_str = env::var("STALE_ZONE_CHECK_INTERVAL_MINUTES").unwrap_or_else(|_| "15".to_string());
    let check_interval_mins = check_interval_mins_str.parse::<u64>().unwrap_or(15);
    if check_interval_mins == 0 {
        info!("[STALE_CHECKER] Stale zone check interval is 0, service will not run periodically.");
        return;
    }

    let min_age_to_be_stale_candidate_days_str = env::var("STALE_ZONE_MIN_AGE_DAYS").unwrap_or_else(|_| "2".to_string());
    let min_age_to_be_stale_candidate_days = min_age_to_be_stale_candidate_days_str.parse::<i64>().unwrap_or(2);
    
    let limit_per_run_str = env::var("STALE_ZONE_LIMIT_PER_RUN").unwrap_or_else(|_| "20".to_string());
    let limit_per_run = limit_per_run_str.parse::<usize>().unwrap_or(20);

    let concurrency_limit_str = env::var("STALE_ZONE_CONCURRENCY_LIMIT").unwrap_or_else(|_| "5".to_string());
    let concurrency_limit = concurrency_limit_str.parse::<usize>().unwrap_or(5).max(1);


    let influx_host = env::var("INFLUXDB_HOST").expect("STALE_CHECKER: INFLUXDB_HOST not set");
    let influx_org = env::var("INFLUXDB_ORG").expect("STALE_CHECKER: INFLUXDB_ORG not set");
    let influx_token = env::var("INFLUXDB_TOKEN").expect("STALE_CHECKER: INFLUXDB_TOKEN not set");
    let zone_bucket = env::var("GENERATOR_WRITE_BUCKET").expect("STALE_CHECKER: GENERATOR_WRITE_BUCKET not set");
    let zone_measurement = env::var("GENERATOR_ZONE_MEASUREMENT").expect("STALE_CHECKER: GENERATOR_ZONE_MEASUREMENT not set");
    let candle_bucket = env::var("INFLUXDB_BUCKET").expect("STALE_CHECKER: INFLUXDB_BUCKET (for candles) not set");
    let candle_measurement_for_candles = env::var("GENERATOR_TRENDBAR_MEASUREMENT").expect("STALE_CHECKER: GENERATOR_TRENDBAR_MEASUREMENT not set");

    info!("[STALE_CHECKER] Service started. Interval: {}m, MinAgeToRecheck: {}d, BatchLimit: {}, Concurrency: {}.",
        check_interval_mins, min_age_to_be_stale_candidate_days, limit_per_run, concurrency_limit);

    let mut interval = interval(Duration::from_secs(check_interval_mins * 60));
    let mut first_run = true;

    loop {
        if !first_run {
            interval.tick().await;
        }
        first_run = false;

        info!("[STALE_CHECKER] Starting revalidation cycle...");
        match fetch_candidate_stale_zones(
            &http_client, &influx_host, &influx_org, &influx_token,
            &zone_bucket, &zone_measurement,
            min_age_to_be_stale_candidate_days, limit_per_run
        ).await {
            Ok(zones_to_revalidate_data) => {
                if zones_to_revalidate_data.is_empty() {
                    info!("[STALE_CHECKER] No candidate stale zones found in this cycle (older than {} day(s) and still active).", min_age_to_be_stale_candidate_days);
                } else {
                    info!("[STALE_CHECKER] Revalidating {} zone(s) concurrently (max {} parallel)...", zones_to_revalidate_data.len(), concurrency_limit);
                    
                    let semaphore = Arc::new(Semaphore::new(concurrency_limit));
                    let mut join_handles = Vec::new();

                    for zone_data in zones_to_revalidate_data { // Iterate over Vec<StoredZone>
                        if let Some(zone_id) = &zone_data.zone_id { // Use the zone_id from the already fetched StoredZone
                            let permit_clone = Arc::clone(&semaphore);
                            let client_clone = Arc::clone(&http_client);
                            let ih_clone = influx_host.clone();
                            let io_clone = influx_org.clone();
                            let it_clone = influx_token.clone();
                            let zb_clone = zone_bucket.clone();
                            let zm_clone = zone_measurement.clone();
                            let cb_clone = candle_bucket.clone();
                            let cmc_clone = candle_measurement_for_candles.clone();
                            let zid_clone = zone_id.clone(); // Clone zone_id for the task

                            join_handles.push(tokio::spawn(async move {
                                // Acquire a permit. This will block if the semaphore is full.
                                let _permit = permit_clone.acquire_owned().await.expect("Semaphore permit acquisition failed");
                                // The permit is held for the duration of this task and released when _permit is dropped.
                                revalidate_one_zone_activity_by_id(
                                    zid_clone, &client_clone, // Pass cloned zone_id
                                    &ih_clone, &io_clone, &it_clone,
                                    &zb_clone, &zm_clone,
                                    &cb_clone, &cmc_clone,
                                ).await
                            }));
                        }
                    }

                    let results = join_all(join_handles).await; // Await all spawned tasks

                    for result in results { // result here is Result<InnerResultFromRevalidate, JoinError>
                        match result {
                            Ok(Ok(Some(updated_zone))) => { // Task completed, revalidation function succeeded and returned Some
                                info!("[STALE_CHECKER] Concurrent revalidation successful for zone ID: {}. New active: {}",
                                    updated_zone.zone_id.as_deref().unwrap_or("?"), updated_zone.is_active);
                            }
                            Ok(Ok(None)) => { // Task completed, revalidation function succeeded but returned None (zone not found by revalidator or no update)
                                debug!("[STALE_CHECKER] Concurrent revalidation: Zone not found by revalidator or no DB update made.");
                            }
                            Ok(Err(e)) => { // Task completed, revalidation function returned an Err(String)
                                error!("[STALE_CHECKER] Concurrent revalidation function error: {}", e);
                            }
                            Err(e) => { // Task panicked (JoinError)
                                error!("[STALE_CHECKER] Concurrent revalidation task panicked: {}", e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("[STALE_CHECKER] Failed to fetch candidate stale zones: {}", e);
            }
        }
        info!("[STALE_CHECKER] Revalidation cycle complete. Next check in approx {} minutes.", check_interval_mins);
    }
}