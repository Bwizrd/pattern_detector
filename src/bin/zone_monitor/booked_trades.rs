// src/bin/zone_monitor/booked_trades.rs
// Booked trades analysis and API endpoints - Updated for Redis architecture

use axum::{extract::State, http::StatusCode, Json, response::Html};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::fs;
use tracing::error;
use crate::state::MonitorState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedBookedTrade {
    // Core trade info
    pub position_id: u64,
    pub symbol: String,
    pub timeframe: Option<String>,
    pub trade_side: String, // "BUY" or "SELL"
    pub volume_lots: f64,
    pub entry_price: f64,
    pub current_price: Option<f64>,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub status: String, // "ACTIVE" or "CLOSED"
    
    // Timing
    pub open_time: chrono::DateTime<chrono::Utc>,
    pub close_time: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_minutes: Option<i64>,
    
    // Zone enrichment data
    pub zone_id: Option<String>,
    pub zone_type: Option<String>, // "supply_zone" or "demand_zone"
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub zone_strength: Option<f64>,
    pub touch_count: Option<i32>,
    pub distance_when_placed: Option<f64>,
    pub original_entry_price: Option<f64>, // From pending order
    pub slippage_pips: Option<f64>,
    
    // Performance metrics
    pub unrealized_pnl: Option<f64>,
    pub pips_profit: Option<f64>,
    pub max_profit_pips: Option<f64>,
    pub max_drawdown_pips: Option<f64>,
    pub risk_reward_ratio: Option<f64>,
    pub is_profitable: Option<bool>,
    
    // Trade source
    pub source: String, // "active" or "historical"
}

#[derive(Debug, Serialize)]
pub struct BookedTradesResponse {
    pub trades: Vec<EnrichedBookedTrade>,
    pub summary: BookedTradesSummary,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct BookedTradesSummary {
    pub total_trades: usize,
    pub active_trades: usize,
    pub historical_trades: usize,
    pub enriched_trades: usize,
    pub total_unrealized_pnl: f64,
    pub total_pips: f64,
    pub win_rate: f64,
    pub avg_duration_hours: f64,
    pub by_symbol: HashMap<String, usize>,
    pub by_zone_type: HashMap<String, usize>,
}

/// Serve the booked trades HTML page
pub async fn booked_trades_html() -> Html<String> {
    match fs::read_to_string("src/bin/zone_monitor/templates/booked_trades.html").await {
        Ok(content) => Html(content),
        Err(_) => Html("<h1>Error: src/bin/zone_monitor/templates/booked_trades.html not found</h1>".to_string())
    }
}

/// API endpoint for booked trades data - now uses Redis data
pub async fn booked_trades_api(State(state): State<MonitorState>) -> (StatusCode, Json<serde_json::Value>) {
    match load_enriched_booked_trades_from_redis(&state).await {
        Ok(response) => (StatusCode::OK, Json(serde_json::json!(response))),
        Err(e) => {
            error!("Error loading booked trades: {}", e);
            let error = serde_json::json!({
                "error": "Failed to load booked trades",
                "message": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error))
        }
    }
}

/// Load enriched booked trades from Redis (active trades)
async fn load_enriched_booked_trades_from_redis(
    state: &MonitorState
) -> Result<BookedTradesResponse, Box<dyn std::error::Error + Send + Sync>> {
    
    // Get active trades from Redis
    let active_trades = state.get_active_trades_enriched().await;
    
    // TODO: Add historical trades from InfluxDB if needed
    // For now, we'll just show active trades from Redis
    
    let mut enriched_trades = Vec::new();
    let mut total_unrealized_pnl = 0.0;
    let mut total_pips = 0.0;
    let mut enriched_count = 0;
    let mut total_duration_minutes = 0i64;
    let mut by_symbol: HashMap<String, usize> = HashMap::new();
    let mut by_zone_type: HashMap<String, usize> = HashMap::new();
    
    // Process active trades
    for active_trade in active_trades {
        // Convert to EnrichedBookedTrade format
        let enriched_trade = EnrichedBookedTrade {
            position_id: active_trade.position_id,
            symbol: active_trade.symbol.clone(),
            timeframe: active_trade.timeframe.clone(),
            trade_side: active_trade.trade_side.clone(),
            volume_lots: active_trade.volume_lots,
            entry_price: active_trade.entry_price,
            current_price: Some(active_trade.current_price),
            stop_loss: active_trade.stop_loss,
            take_profit: active_trade.take_profit,
            status: "ACTIVE".to_string(),
            
            open_time: active_trade.open_time,
            close_time: None, // Active trades don't have close time
            duration_minutes: Some(active_trade.duration_minutes),
            
            zone_id: active_trade.zone_id.clone(),
            zone_type: active_trade.zone_type.clone(),
            zone_high: active_trade.zone_high,
            zone_low: active_trade.zone_low,
            zone_strength: active_trade.zone_strength,
            touch_count: active_trade.touch_count,
            distance_when_placed: active_trade.distance_when_placed,
            original_entry_price: active_trade.original_entry_price,
            slippage_pips: active_trade.slippage_pips,
            
            unrealized_pnl: Some(active_trade.unrealized_pnl),
            pips_profit: Some(active_trade.pips_profit),
            max_profit_pips: active_trade.max_profit_pips,
            max_drawdown_pips: active_trade.max_drawdown_pips,
            risk_reward_ratio: active_trade.risk_reward_ratio,
            is_profitable: Some(active_trade.is_profitable),
            
            source: "active".to_string(),
        };
        
        // Update statistics
        total_unrealized_pnl += active_trade.unrealized_pnl;
        total_pips += active_trade.pips_profit;
        total_duration_minutes += active_trade.duration_minutes;
        
        if active_trade.zone_id.is_some() {
            enriched_count += 1;
        }
        
        // Group by symbol
        *by_symbol.entry(active_trade.symbol.clone()).or_insert(0) += 1;
        
        // Group by zone type
        if let Some(zone_type) = &active_trade.zone_type {
            *by_zone_type.entry(zone_type.clone()).or_insert(0) += 1;
        }
        
        enriched_trades.push(enriched_trade);
    }
    
    // Sort by open time descending (newest first)
    enriched_trades.sort_by(|a, b| b.open_time.cmp(&a.open_time));
    
    // Calculate summary statistics
    let total_trades = enriched_trades.len();
    let active_trades_count = enriched_trades.iter().filter(|t| t.status == "ACTIVE").count();
    let historical_trades_count = enriched_trades.iter().filter(|t| t.status == "CLOSED").count();
    
    // Win rate calculation (for active trades, based on current P&L)
    let profitable_trades = enriched_trades.iter()
        .filter(|t| t.is_profitable.unwrap_or(false))
        .count();
    
    let win_rate = if total_trades > 0 {
        (profitable_trades as f64 / total_trades as f64) * 100.0
    } else {
        0.0
    };
    
    let avg_duration_hours = if total_trades > 0 {
        (total_duration_minutes as f64 / total_trades as f64) / 60.0
    } else {
        0.0
    };
    
    let summary = BookedTradesSummary {
        total_trades,
        active_trades: active_trades_count,
        historical_trades: historical_trades_count,
        enriched_trades: enriched_count,
        total_unrealized_pnl,
        total_pips,
        win_rate,
        avg_duration_hours,
        by_symbol,
        by_zone_type,
    };
    
    Ok(BookedTradesResponse {
        trades: enriched_trades,
        summary,
        last_updated: chrono::Utc::now(),
    })
}

// Helper function to get pip value for a symbol
fn get_pip_value(symbol: &str) -> f64 {
    if symbol.contains("JPY") {
        0.01
    } else {
        match symbol {
            "NAS100" => 1.0,
            "US500" => 0.1,
            _ => 0.0001,
        }
    }
}

// Future enhancement: Load historical trades from InfluxDB
// This would complement the active trades from Redis
async fn _load_historical_trades_from_influx(
    _state: &MonitorState
) -> Result<Vec<EnrichedBookedTrade>, Box<dyn std::error::Error + Send + Sync>> {
    // TODO: Implement InfluxDB query for historical closed trades
    // This would query the order_database for CLOSED events with enriched data
    Ok(Vec::new())
}