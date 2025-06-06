// src/cache_endpoints.rs - Updated to use shared real-time cache
use actix_web::{web, HttpResponse, Responder};
use log;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cache::minimal_zone_cache::{get_minimal_cache, CacheSymbolConfig, MinimalZoneCache};
use crate::types::{BulkActiveZonesResponse, BulkResultData, BulkResultItem, ChartQuery, EnrichedZone};

// Updated to accept the shared cache as a parameter
pub async fn get_minimal_cache_zones_debug_with_shared_cache(
    shared_cache: web::Data<Arc<Mutex<MinimalZoneCache>>>
) -> impl Responder {
    log::debug!("📡 [CACHE_ENDPOINT] Debug minimal cache zones endpoint called (using shared real-time cache)");

    // Try non-blocking access first (fast path)
    if let Ok(cache_guard) = shared_cache.try_lock() {
        let (total_in_cache, supply_in_cache, demand_in_cache) = cache_guard.get_stats();
        let all_enriched_zones: Vec<EnrichedZone> = cache_guard.get_all_zones();
        let (current_start, current_end) = MinimalZoneCache::get_current_date_range();

        log::debug!("📡 [CACHE_ENDPOINT] Fast path: Returning {} zones from shared real-time cache", all_enriched_zones.len());

        return HttpResponse::Ok().json(serde_json::json!({
            "source": "Shared Real-time MinimalZoneCache (Fast Path)",
            "total_zones_in_cache": total_in_cache,
            "supply_zones_in_cache": supply_in_cache,
            "demand_zones_in_cache": demand_in_cache,
            "retrieved_zones": all_enriched_zones,
            "cache_date_range": {
                "start": current_start,
                "end": current_end
            },
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "note": "Data retrieved instantly (cache not busy)"
        }));
    }

    // Fast path failed, try with longer timeout (slow path)
    log::debug!("📡 [CACHE_ENDPOINT] Cache busy, trying slow path with extended timeout...");
    
    match tokio::time::timeout(std::time::Duration::from_secs(10), shared_cache.lock()).await {
        Ok(cache_guard) => {
            let (total_in_cache, supply_in_cache, demand_in_cache) = cache_guard.get_stats();
            let all_enriched_zones: Vec<EnrichedZone> = cache_guard.get_all_zones();
            let (current_start, current_end) = MinimalZoneCache::get_current_date_range();

            log::debug!("📡 [CACHE_ENDPOINT] Slow path: Returning {} zones from shared real-time cache", all_enriched_zones.len());

            HttpResponse::Ok().json(serde_json::json!({
                "source": "Shared Real-time MinimalZoneCache (Slow Path)",
                "total_zones_in_cache": total_in_cache,
                "supply_zones_in_cache": supply_in_cache,
                "demand_zones_in_cache": demand_in_cache,
                "retrieved_zones": all_enriched_zones,
                "cache_date_range": {
                    "start": current_start,
                    "end": current_end
                },
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "note": "Data retrieved after waiting (cache was refreshing)"
            }))
        }
        Err(_) => {
            log::warn!("⏰ [CACHE_ENDPOINT] Cache still busy after 10s, returning service unavailable");
            
            // Return a user-friendly response instead of an error
            HttpResponse::ServiceUnavailable()
                .insert_header(("Retry-After", "5")) // Tell client to retry in 5 seconds
                .json(serde_json::json!({
                    "status": "cache_refreshing",
                    "message": "Cache is currently refreshing with new data",
                    "retry_after_seconds": 5,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "note": "This is temporary - cache refresh should complete soon"
                }))
        }
    }
}

// Fallback endpoint that creates a new cache (for comparison/debugging)
pub async fn get_minimal_cache_zones_debug() -> impl Responder {
    log::debug!("📡 [CACHE_ENDPOINT] Debug minimal cache zones endpoint called (creating new cache instance)");

    match get_minimal_cache().await {
        Ok(cache) => {
            let (total_in_cache, supply_in_cache, demand_in_cache) = cache.get_stats();
            let all_enriched_zones: Vec<EnrichedZone> = cache.get_all_zones();

            log::debug!("📡 [CACHE_ENDPOINT] Returning {} zones from new cache instance", all_enriched_zones.len());

            HttpResponse::Ok().json(serde_json::json!({
                "source": "New MinimalZoneCache Instance (NOT real-time)",
                "total_zones_in_cache": total_in_cache,
                "supply_zones_in_cache": supply_in_cache,
                "demand_zones_in_cache": demand_in_cache,
                "retrieved_zones": all_enriched_zones,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "note": "This creates a fresh cache and does NOT use the real-time updater"
            }))
        }
        Err(e) => {
            log::error!("❌ [CACHE_ENDPOINT] Failed to get cache: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to get cache: {}", e)
            }))
        }
    }
}

pub async fn test_cache_endpoint() -> impl Responder {
    log::debug!("Test cache endpoint called");

    let symbols_for_cache_config = vec![
        CacheSymbolConfig {
            symbol: "EURUSD".to_string(),
            timeframes: vec!["1h".to_string(), "4h".to_string()],
        },
    ];

    let mut cache = match MinimalZoneCache::new(symbols_for_cache_config.clone()) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to create MinimalZoneCache: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to create cache: {}", e)
            }));
        }
    };

    if let Err(e) = cache.refresh_zones().await {
        log::error!("Failed to refresh zones in MinimalZoneCache: {}", e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to refresh zones: {}", e)
        }));
    }

    log::debug!(
        "Cache refresh complete. Total zones in cache: {}",
        cache.get_all_zones().len()
    );

    let all_enriched_zones_from_cache: Vec<EnrichedZone> = cache.get_all_zones();

    // Group zones by symbol and timeframe
    let mut grouped_by_symbol_tf: HashMap<
        String,
        HashMap<String, (Vec<EnrichedZone>, Vec<EnrichedZone>)>,
    > = HashMap::new();

    for zone in all_enriched_zones_from_cache {
        let symbol_key = zone
            .symbol
            .clone()
            .unwrap_or_else(|| "UNKNOWN_SYMBOL".to_string());
        let timeframe_key = zone
            .timeframe
            .clone()
            .unwrap_or_else(|| "UNKNOWN_TIMEFRAME".to_string());

        let symbol_entry = grouped_by_symbol_tf.entry(symbol_key).or_default();
        let timeframe_entry = symbol_entry
            .entry(timeframe_key)
            .or_insert((Vec::new(), Vec::new()));

        if zone.zone_type.as_deref().unwrap_or("").contains("supply") {
            timeframe_entry.0.push(zone);
        } else if zone.zone_type.as_deref().unwrap_or("").contains("demand") {
            timeframe_entry.1.push(zone);
        }
    }

    // Construct BulkResultItems
    let mut bulk_results: Vec<BulkResultItem> = Vec::new();

    for config_item in &symbols_for_cache_config {
        let symbol_for_item = &config_item.symbol;

        if let Some(timeframe_map_for_symbol) = grouped_by_symbol_tf.get(symbol_for_item) {
            for timeframe_for_item in &config_item.timeframes {
                if let Some((supply_zones, demand_zones)) =
                    timeframe_map_for_symbol.get(timeframe_for_item)
                {
                    bulk_results.push(BulkResultItem {
                        symbol: symbol_for_item.clone(),
                        timeframe: timeframe_for_item.clone(),
                        status: "Success".to_string(),
                        data: Some(BulkResultData {
                            supply_zones: supply_zones.clone(),
                            demand_zones: demand_zones.clone(),
                        }),
                        error_message: None,
                    });
                } else {
                    bulk_results.push(BulkResultItem {
                        symbol: symbol_for_item.clone(),
                        timeframe: timeframe_for_item.clone(),
                        status: "Success".to_string(),
                        data: Some(BulkResultData {
                            supply_zones: Vec::new(),
                            demand_zones: Vec::new(),
                        }),
                        error_message: None,
                    });
                }
            }
        }
    }

    let query_params_for_response = ChartQuery {
        start_time: "-90d".to_string(),
        end_time: "now()".to_string(),
        symbol: "DUMMY".to_string(),
        timeframe: "DUMMY".to_string(),
        pattern: "fifty_percent_before_big_bar".to_string(),
        enable_trading: None,
        lot_size: None,
        stop_loss_pips: None,
        take_profit_pips: None,
        enable_trailing_stop: None,
        max_touch_count: None,
    };

    let response_payload = BulkActiveZonesResponse {
        results: bulk_results,
        query_params: Some(query_params_for_response),
        symbols: HashMap::new(),
    };

    HttpResponse::Ok().json(response_payload)
}