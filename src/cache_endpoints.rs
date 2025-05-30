// src/cache_endpoints.rs - Cache testing and debugging endpoints
use actix_web::{HttpResponse, Responder};
use log;
use std::collections::HashMap;

use crate::minimal_zone_cache::{CacheSymbolConfig, MinimalZoneCache};
use crate::types::{BulkActiveZonesResponse, BulkResultData, BulkResultItem, ChartQuery, EnrichedZone};

pub async fn test_cache_endpoint() -> impl Responder {
    log::info!("Test cache endpoint called");

    let symbols_for_cache_config = vec![CacheSymbolConfig {
        symbol: "EURUSD".to_string(),
        timeframes: vec!["1h".to_string(), "4h".to_string()],
    }];

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

    log::info!(
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

pub async fn get_minimal_cache_zones_debug() -> impl Responder {
    log::info!("Debug minimal cache zones endpoint called");

    let symbols_for_cache_config = vec![
        CacheSymbolConfig {
            symbol: "EURUSD".to_string(),
            timeframes: vec!["1h".to_string(), "4h".to_string()],
        },
        CacheSymbolConfig {
            symbol: "GBPUSD".to_string(),
            timeframes: vec!["1h".to_string(), "4h".to_string()],
        },
        CacheSymbolConfig {
            symbol: "USDJPY".to_string(),
            timeframes: vec!["1h".to_string(), "4h".to_string()],
        },
    ];

    let mut cache = match MinimalZoneCache::new(symbols_for_cache_config.clone()) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to create MinimalZoneCache: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to create MinimalZoneCache: {}", e)
            }));
        }
    };

    if let Err(e) = cache.refresh_zones().await {
        log::error!("Failed to refresh zones in MinimalZoneCache: {}", e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to refresh zones: {}", e)
        }));
    }

    let (total_in_cache, supply_in_cache, demand_in_cache) = cache.get_stats();
    log::info!(
        "Cache refresh complete. Stats: Total {}, Supply {}, Demand {}",
        total_in_cache,
        supply_in_cache,
        demand_in_cache
    );

    let all_enriched_zones: Vec<EnrichedZone> = cache.get_all_zones();

    HttpResponse::Ok().json(serde_json::json!({
        "source": "MinimalZoneCache Debug Endpoint",
        "cache_config_used": symbols_for_cache_config,
        "total_zones_in_cache": total_in_cache,
        "supply_zones_in_cache": supply_in_cache,
        "demand_zones_in_cache": demand_in_cache,
        "retrieved_zones": all_enriched_zones,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}