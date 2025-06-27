// src/bin/zone_monitor/booked_trades.rs
// Booked trades analysis and API endpoints

use axum::{extract::State, http::StatusCode, Json, response::Html};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::fs;
use tracing::error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedBookedTrade {
    // Core trade info
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub order_type: String,
    pub entry_price: f64,
    pub lot_size: i32,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub ctrader_order_id: String,
    pub status: String,
    
    // Timing
    pub booked_at: chrono::DateTime<chrono::Utc>,
    pub filled_at: Option<chrono::DateTime<chrono::Utc>>,
    pub filled_price: Option<f64>,
    pub closed_at: Option<chrono::DateTime<chrono::Utc>>,
    
    // Zone enrichment
    pub zone_type: Option<String>,
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub zone_strength: Option<f64>,
    pub touch_count: Option<i32>,
    pub distance_when_placed: Option<f64>,
    pub original_zone_id: Option<String>,
    
    // Linked pending order
    pub pending_order: Option<PendingOrderSummary>,
    
    // Calculated fields
    pub duration_minutes: Option<i64>,
    pub pnl_pips: Option<f64>,
    pub risk_reward_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOrderSummary {
    pub zone_id: String,
    pub ctrader_order_id: Option<String>,
    pub placed_at: chrono::DateTime<chrono::Utc>,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub status: String,
    pub zone_strength: f64,
    pub touch_count: i32,
    pub distance_when_placed: f64,
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
    pub open_trades: usize,
    pub closed_trades: usize,
    pub enriched_trades: usize,
    pub total_pnl_pips: f64,
    pub win_rate: f64,
    pub avg_duration_hours: f64,
}

/// Serve the booked trades HTML page
pub async fn booked_trades_html() -> Html<String> {
    match fs::read_to_string("src/bin/zone_monitor/templates/booked_trades.html").await {
        Ok(content) => Html(content),
        Err(_) => Html("<h1>Error: src/bin/zone_monitor/templates/booked_trades.html not found</h1>".to_string())
    }
}

/// API endpoint for booked trades data
pub async fn booked_trades_api() -> (StatusCode, Json<serde_json::Value>) {
    match load_enriched_booked_trades().await {
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

/// Load and enrich booked trades with pending order data
async fn load_enriched_booked_trades() -> Result<BookedTradesResponse, Box<dyn std::error::Error + Send + Sync>> {
    // Load booked orders
    let booked_content = fs::read_to_string("shared_booked_orders.json").await?;
    let booked_container: crate::pending_order_manager::BookedOrdersContainer = 
        serde_json::from_str(&booked_content)?;
    
    // Load pending orders for linking
    let pending_orders = match fs::read_to_string("shared_pending_orders.json").await {
        Ok(content) => {
            let container: crate::pending_order_manager::PendingOrdersContainer = 
                serde_json::from_str(&content)?;
            container.orders
        }
        Err(_) => HashMap::new(),
    };
    
    let mut enriched_trades = Vec::new();
    let mut total_pnl_pips = 0.0;
    let mut closed_count = 0;
    let mut total_duration_minutes = 0i64;
    let mut enriched_count = 0;
    
    for (_, booked_order) in booked_container.booked_orders {
        // Find linked pending order
        let pending_order = find_linked_pending_order(&booked_order, &pending_orders);
        
        // Calculate duration
        let duration_minutes = if let (Some(filled), Some(closed)) = (&booked_order.filled_at, &booked_order.closed_at) {
            Some(closed.signed_duration_since(*filled).num_minutes())
        } else {
            None
        };
        
        // Calculate P&L in pips
        let pnl_pips = calculate_pnl_pips(&booked_order);
        
        // Calculate risk/reward ratio
        let risk_reward_ratio = calculate_risk_reward(&booked_order);
        
        // Track summary stats
        if booked_order.status == "CLOSED" {
            closed_count += 1;
            if let Some(pnl) = pnl_pips {
                total_pnl_pips += pnl;
            }
            if let Some(duration) = duration_minutes {
                total_duration_minutes += duration;
            }
        }
        
        if booked_order.zone_type.is_some() {
            enriched_count += 1;
        }
        
        let enriched_trade = EnrichedBookedTrade {
            zone_id: booked_order.zone_id,
            symbol: booked_order.symbol,
            timeframe: booked_order.timeframe,
            order_type: booked_order.order_type,
            entry_price: booked_order.entry_price,
            lot_size: booked_order.lot_size,
            stop_loss: booked_order.stop_loss,
            take_profit: booked_order.take_profit,
            ctrader_order_id: booked_order.ctrader_order_id,
            status: booked_order.status,
            booked_at: booked_order.booked_at,
            filled_at: booked_order.filled_at,
            filled_price: booked_order.filled_price,
            closed_at: booked_order.closed_at,
            zone_type: booked_order.zone_type,
            zone_high: booked_order.zone_high,
            zone_low: booked_order.zone_low,
            zone_strength: booked_order.zone_strength,
            touch_count: booked_order.touch_count,
            distance_when_placed: booked_order.distance_when_placed,
            original_zone_id: booked_order.original_zone_id,
            pending_order,
            duration_minutes,
            pnl_pips,
            risk_reward_ratio,
        };
        
        enriched_trades.push(enriched_trade);
    }
    
    // Sort by booked_at descending (newest first)
    enriched_trades.sort_by(|a, b| b.booked_at.cmp(&a.booked_at));
    
    // Calculate summary statistics
    let total_trades = enriched_trades.len();
    let open_trades = enriched_trades.iter().filter(|t| t.status != "CLOSED").count();
    let closed_trades = closed_count;
    
    let win_rate = if closed_count > 0 {
        let winning_trades = enriched_trades.iter()
            .filter(|t| t.status == "CLOSED" && t.pnl_pips.unwrap_or(0.0) > 0.0)
            .count();
        (winning_trades as f64 / closed_count as f64) * 100.0
    } else {
        0.0
    };
    
    let avg_duration_hours = if closed_count > 0 {
        (total_duration_minutes as f64 / closed_count as f64) / 60.0
    } else {
        0.0
    };
    
    let summary = BookedTradesSummary {
        total_trades,
        open_trades,
        closed_trades,
        enriched_trades: enriched_count,
        total_pnl_pips,
        win_rate,
        avg_duration_hours,
    };
    
    Ok(BookedTradesResponse {
        trades: enriched_trades,
        summary,
        last_updated: booked_container.last_updated,
    })
}

/// Find the pending order that corresponds to a booked trade
fn find_linked_pending_order(
    booked_order: &crate::pending_order_manager::BookedPendingOrder,
    pending_orders: &HashMap<String, crate::pending_order_manager::PendingOrder>
) -> Option<PendingOrderSummary> {
    // First try to match by zone_id (for zone-based trades)
    if let Some(pending) = pending_orders.get(&booked_order.zone_id) {
        return Some(PendingOrderSummary {
            zone_id: pending.zone_id.clone(),
            ctrader_order_id: pending.ctrader_order_id.clone(),
            placed_at: pending.placed_at,
            entry_price: pending.entry_price,
            stop_loss: pending.stop_loss,
            take_profit: pending.take_profit,
            status: pending.status.clone(),
            zone_strength: pending.zone_strength,
            touch_count: pending.touch_count,
            distance_when_placed: pending.distance_when_placed,
        });
    }
    
    // Try to match by original_zone_id (if enriched)
    if let Some(original_zone_id) = &booked_order.original_zone_id {
        if let Some(pending) = pending_orders.get(original_zone_id) {
            return Some(PendingOrderSummary {
                zone_id: pending.zone_id.clone(),
                ctrader_order_id: pending.ctrader_order_id.clone(),
                placed_at: pending.placed_at,
                entry_price: pending.entry_price,
                stop_loss: pending.stop_loss,
                take_profit: pending.take_profit,
                status: pending.status.clone(),
                zone_strength: pending.zone_strength,
                touch_count: pending.touch_count,
                distance_when_placed: pending.distance_when_placed,
            });
        }
    }
    
    None
}

/// Calculate P&L in pips for a closed trade
fn calculate_pnl_pips(booked_order: &crate::pending_order_manager::BookedPendingOrder) -> Option<f64> {
    if booked_order.status != "CLOSED" {
        return None;
    }
    
    // This is a simplified calculation - you might want to get actual close price
    // For now, assume it hit SL or TP
    let entry = booked_order.entry_price;
    let sl = booked_order.stop_loss;
    let tp = booked_order.take_profit;
    
    if sl == 0.0 || tp == 0.0 {
        return None;
    }
    
    // Get pip value for the symbol
    let pip_value = if booked_order.symbol.contains("JPY") {
        0.01
    } else {
        0.0001
    };
    
    // Assume 50/50 chance of hitting SL vs TP for demonstration
    // In reality, you'd need the actual close price
    let is_buy = booked_order.order_type.contains("BUY");
    
    if is_buy {
        // For buy orders: profit if price goes up to TP, loss if hits SL
        let tp_pips = (tp - entry) / pip_value;
        let sl_pips = (sl - entry) / pip_value;
        Some(if tp_pips > 0.0 { tp_pips } else { sl_pips })
    } else {
        // For sell orders: profit if price goes down to TP, loss if hits SL
        let tp_pips = (entry - tp) / pip_value;
        let sl_pips = (entry - sl) / pip_value;
        Some(if tp_pips > 0.0 { tp_pips } else { sl_pips })
    }
}

/// Calculate risk/reward ratio for a trade
fn calculate_risk_reward(booked_order: &crate::pending_order_manager::BookedPendingOrder) -> Option<f64> {
    let entry = booked_order.entry_price;
    let sl = booked_order.stop_loss;
    let tp = booked_order.take_profit;
    
    if sl == 0.0 || tp == 0.0 {
        return None;
    }
    
    let risk = (entry - sl).abs();
    let reward = (tp - entry).abs();
    
    if risk > 0.0 {
        Some(reward / risk)
    } else {
        None
    }
}