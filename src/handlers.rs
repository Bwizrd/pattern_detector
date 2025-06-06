// src/handlers.rs - All route handlers

use actix_web::{web, HttpResponse, Responder};
use log::{info, warn, error};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use crate::realtime::realtime_zone_monitor::NewRealTimeZoneMonitor;
use crate::notifications::notification_manager::get_global_notification_manager;
use crate::{get_global_trade_engine, get_global_trade_logger};

#[derive(serde::Deserialize)]
pub struct ProximityCheckQuery {
    pub symbol: String,
    pub price: f64,
}

pub async fn get_current_prices_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let prices = monitor.get_current_prices_by_symbol().await;
        HttpResponse::Ok().json(serde_json::json!({
            "prices": prices,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "count": prices.len()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "error": "Zone monitor not available",
            "prices": {}
        }))
    }
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
        "note": "Test prices for WebSocket testing"
    }))
}

pub async fn get_trade_notifications_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let notifications = monitor.get_trade_notifications_from_cache().await;
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "count": notifications.len(),
            "notifications": notifications,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available",
            "notifications": []
        }))
    }
}

pub async fn clear_trade_notifications_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        monitor.clear_cache_notifications().await;
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Trade notifications cleared",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available"
        }))
    }
}

pub async fn get_validated_signals_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let signals = monitor.get_validated_signals().await;
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "count": signals.len(),
            "signals": signals,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available",
            "signals": []
        }))
    }
}

pub async fn clear_validated_signals_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        monitor.clear_validated_signals().await;
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Validated signals cleared",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available"
        }))
    }
}

pub async fn get_trading_status() -> impl Responder {
    if let Some(engine_arc) = get_global_trade_engine() {
        let engine_guard = engine_arc.lock().await;
        let rules = engine_guard.get_rules();
        let daily_signals = engine_guard.get_daily_signal_count();

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "trading_enabled": rules.trading_enabled,
            "daily_signal_count": daily_signals,
            "max_daily_signals": rules.max_daily_signals,
            "allowed_symbols": rules.allowed_symbols,
            "allowed_timeframes": rules.allowed_timeframes,
            "min_zone_strength": rules.min_zone_strength,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine not available"
        }))
    }
}

pub async fn get_validated_signals_from_engine() -> impl Responder {
    if let Some(engine_arc) = get_global_trade_engine() {
        let engine_guard = engine_arc.lock().await;
        let signals = engine_guard.get_validated_signals();

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "count": signals.len(),
            "signals": signals,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine not available"
        }))
    }
}

pub async fn clear_validated_signals_from_engine() -> impl Responder {
    if let Some(engine_arc) = get_global_trade_engine() {
        let mut engine_guard = engine_arc.lock().await;
        engine_guard.clear_signals();

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Validated signals cleared from engine",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine not available"
        }))
    }
}

pub async fn reload_trading_rules() -> impl Responder {
    if let Some(engine_arc) = get_global_trade_engine() {
        let mut engine_guard = engine_arc.lock().await;
        engine_guard.reload_rules();

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Trading rules reloaded from environment",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine not available"
        }))
    }
}

pub async fn manual_trade_test(
    request_body: web::Json<crate::cache::minimal_zone_cache::TradeNotification>,
) -> impl Responder {
    let notification = request_body.into_inner();

    if let (Some(engine_arc), Some(logger_arc)) =
        (get_global_trade_engine(), get_global_trade_logger())
    {
        let mut engine_guard = engine_arc.lock().await;
        let logger_guard = logger_arc.lock().await;

        // Log the incoming notification
        logger_guard.log_notification_received(&notification).await;

        // Process through trade decision engine
        match engine_guard
            .process_notification_with_reason(notification.clone())
            .await
        {
            Ok(validated_signal) => {
                logger_guard.log_signal_validated(&validated_signal).await;
                info!(
                    "📈 Manual test - Trade validated: {} {} @ {:.5}",
                    validated_signal.action, validated_signal.symbol, validated_signal.price
                );

                if let Some(notification_manager) = get_global_notification_manager() {
                    info!("📢 Sending notifications for validated manual test signal");
                    notification_manager
                        .notify_trade_signal(&validated_signal)
                        .await;
                } else {
                    warn!("📢 Notification manager not available for manual test signal");
                }

                HttpResponse::Ok().json(serde_json::json!({
                    "status": "success",
                    "result": "validated",
                    "signal": validated_signal,
                    "message": "Trade notification successfully validated",
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
            Err(reason) => {
                logger_guard
                    .log_signal_rejected(&notification, reason.clone())
                    .await;
                warn!(
                    "📈 Manual test - Trade rejected: {} {} @ {:.5} - {}",
                    notification.action, notification.symbol, notification.price, reason
                );

                HttpResponse::Ok().json(serde_json::json!({
                    "status": "success",
                    "result": "rejected",
                    "reason": reason,
                    "notification": notification,
                    "message": "Trade notification was rejected",
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
        }
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Trade engine or logger not available"
        }))
    }
}

pub async fn test_notifications_handler() -> impl Responder {
    if let Some(manager) = get_global_notification_manager() {
        let (telegram_ok, sound_ok) = manager.test_notifications().await;

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "telegram": telegram_ok,
            "sound": sound_ok,
            "message": format!("Telegram: {}, Sound: {}",
                if telegram_ok { "✅" } else { "❌" },
                if sound_ok { "✅" } else { "❌" }
            ),
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Notification manager not available"
        }))
    }
}

pub async fn send_daily_summary_handler() -> impl Responder {
    if let (Some(manager), Some(engine_arc)) =
        (get_global_notification_manager(), get_global_trade_engine())
    {
        let engine_guard = engine_arc.lock().await;
        let signals_today = engine_guard.get_daily_signal_count();
        let max_signals = engine_guard.get_rules().max_daily_signals;
        drop(engine_guard);

        manager.send_daily_summary(signals_today, max_signals).await;

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Daily summary sent",
            "signals_today": signals_today,
            "max_signals": max_signals,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Notification manager or trade engine not available"
        }))
    }
}

pub async fn test_proximity_alert_handler(
    _zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(notification_manager) = get_global_notification_manager() {
        notification_manager
            .notify_proximity_alert(
                "EURUSD",
                1.12340,
                "supply",
                1.12400,
                1.12300,
                "test_zone_123",
                "4h",
                12.5,
            )
            .await;

        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Test proximity alert sent",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Notification manager not available"
        }))
    }
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
        "proximity_alerts": {
            "enabled": enabled,
            "threshold_pips": threshold_pips,
            "cooldown_minutes": cooldown_minutes
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

pub async fn get_proximity_distances_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let proximity_distances = monitor.get_proximity_distances_debug().await;
        
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "total_zones": proximity_distances.len(),
            "zones": proximity_distances,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available"
        }))
    }
}

pub async fn get_zones_within_proximity_handler(
    zone_monitor_data: web::Data<Option<Arc<NewRealTimeZoneMonitor>>>,
) -> impl Responder {
    if let Some(monitor) = zone_monitor_data.get_ref().as_ref() {
        let close_zones = monitor.get_zones_within_proximity().await;
        
        HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "zones_within_proximity": close_zones.len(),
            "zones": close_zones,
            "message": if close_zones.is_empty() { 
                "No zones within proximity threshold" 
            } else { 
                "Found zones within proximity threshold" 
            },
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "error",
            "error": "Zone monitor not available"
        }))
    }
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
        "status": "success",
        "symbol": symbol,
        "current_price": current_price,
        "proximity_threshold_pips": proximity_threshold_pips,
        "message": "For full zone distance checking, zones need to be loaded from cache",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "note": "This endpoint needs zone cache access to calculate actual distances"
    }))
}

pub async fn trigger_proximity_check_handler() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "message": "Manual proximity check would require integration with your zone monitor",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "suggestion": "Check your zone monitor logs for proximity detection activity"
    }))
}