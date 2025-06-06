// src/handlers.rs - Simplified handlers for API-only main.rs

use actix_web::{web, HttpResponse, Responder};
use log::{info, warn};
use std::collections::HashMap;

#[derive(serde::Deserialize)]
pub struct ProximityCheckQuery {
    pub symbol: String,
    pub price: f64,
}

// Simple price handlers that return test data
pub async fn get_current_prices_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "error": "Real-time prices available in zone monitor binary",
        "message": "Start the zone_monitor binary for live price feeds",
        "test_endpoint": "/test-prices"
    }))
}

pub async fn test_prices_handler() -> impl Responder {
    let mut test_prices = HashMap::new();
    test_prices.insert("EURUSD".to_string(), 1.1234);
    test_prices.insert("GBPUSD".to_string(), 1.2567);
    test_prices.insert("USDJPY".to_string(), 143.45);

    HttpResponse::Ok().json(serde_json::json!({
        "prices": test_prices,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "count": test_prices.len(),
        "note": "Test prices - real prices available in zone monitor binary"
    }))
}

// Simplified trade notification handlers
pub async fn get_trade_notifications_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Trade notifications available in zone monitor binary",
        "message": "Start the zone_monitor binary for real-time trade notifications",
        "notifications": []
    }))
}

pub async fn clear_trade_notifications_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Trade notifications managed by zone monitor binary",
        "message": "Use the zone monitor endpoints for clearing notifications"
    }))
}

pub async fn get_validated_signals_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Validated signals available in zone monitor binary",
        "message": "Start the zone_monitor binary for trade signal validation",
        "signals": []
    }))
}

pub async fn clear_validated_signals_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Validated signals managed by zone monitor binary"
    }))
}

// Trading status handlers
pub async fn get_trading_status() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Trading engine not active in API server",
        "message": "Trading functionality moved to zone monitor binary"
    }))
}

pub async fn get_validated_signals_from_engine() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Trading engine not active in API server",
        "message": "Use zone monitor binary for trade signals"
    }))
}

pub async fn clear_validated_signals_from_engine() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Trading engine not active in API server"
    }))
}

pub async fn reload_trading_rules() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Trading engine not active in API server"
    }))
}

pub async fn manual_trade_test(
    request_body: web::Json<crate::cache::minimal_zone_cache::TradeNotification>,
) -> impl Responder {
    let notification = request_body.into_inner();
    
    info!("📈 Manual trade test received: {} {} @ {:.5}", 
          notification.action, notification.symbol, notification.price);
    
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Trading engine not active in API server",
        "message": "Use zone monitor binary for trade testing",
        "received_notification": notification
    }))
}

// Notification handlers
pub async fn test_notifications_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Notification manager not active in API server",
        "message": "Notifications handled by zone monitor binary"
    }))
}

pub async fn send_daily_summary_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Notification manager not active in API server"
    }))
}

// Proximity handlers
pub async fn test_proximity_alert_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Proximity alerts handled by zone monitor binary",
        "message": "Start the zone_monitor binary for proximity monitoring"
    }))
}

pub async fn get_proximity_status_handler() -> impl Responder {
    let enabled = std::env::var("ENABLE_PROXIMITY_ALERTS")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        == "true";

    let threshold_pips = std::env::var("PROXIMITY_THRESHOLD_PIPS")
        .unwrap_or_else(|_| "15.0".to_string())
        .parse::<f64>()
        .unwrap_or(15.0);

    let cooldown_minutes = std::env::var("PROXIMITY_ALERT_COOLDOWN_MINUTES")
        .unwrap_or_else(|_| "30".to_string())
        .parse::<i64>()
        .unwrap_or(30);

    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "message": "Proximity monitoring handled by zone monitor binary",
        "config": {
            "enabled": enabled,
            "threshold_pips": threshold_pips,
            "cooldown_minutes": cooldown_minutes
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

pub async fn get_proximity_distances_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Proximity distances available in zone monitor binary",
        "message": "Use zone monitor for real-time proximity calculations"
    }))
}

pub async fn get_zones_within_proximity_handler() -> impl Responder {
    HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "status": "error",
        "error": "Proximity zones available in zone monitor binary"
    }))
}

pub async fn check_proximity_distance_handler(
    query: web::Query<ProximityCheckQuery>,
) -> impl Responder {
    let symbol = &query.symbol;
    let current_price = query.price;
    
    let proximity_threshold_pips = std::env::var("PROXIMITY_THRESHOLD_PIPS")
        .unwrap_or_else(|_| "15.0".to_string())
        .parse::<f64>()
        .unwrap_or(15.0);
    
    HttpResponse::Ok().json(serde_json::json!({
        "status": "info",
        "symbol": symbol,
        "current_price": current_price,
        "proximity_threshold_pips": proximity_threshold_pips,
        "message": "For real-time proximity checking, use the zone monitor binary",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "note": "This API server focuses on zone detection and analysis"
    }))
}

pub async fn trigger_proximity_check_handler() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "info",
        "message": "Proximity checking handled by zone monitor binary",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "suggestion": "Start the zone_monitor binary for real-time monitoring"
    }))
}