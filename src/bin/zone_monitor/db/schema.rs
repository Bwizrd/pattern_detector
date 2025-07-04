// src/bin/zone_monitor/db/schema.rs
// Data structures for order lifecycle tracking in InfluxDB

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OrderStatus {
    Pending,
    Filled,
    Closed,
    Cancelled,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderStatus::Pending => write!(f, "PENDING"),
            OrderStatus::Filled => write!(f, "FILLED"),
            OrderStatus::Closed => write!(f, "CLOSED"),
            OrderStatus::Cancelled => write!(f, "CANCELLED"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderEvent {
    // Time of this event
    pub timestamp: DateTime<Utc>,
    
    // Tags (indexed in InfluxDB)
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub status: OrderStatus,
    pub zone_type: String, // "demand_zone" | "supply_zone"
    pub order_type: String, // "BUY_LIMIT" | "SELL_LIMIT"
    
    // Fields (data in InfluxDB)
    pub entry_price: f64,
    pub lot_size: i32,
    pub stop_loss: f64,
    pub take_profit: f64,
    
    // Zone metadata
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub zone_strength: Option<f64>,
    pub touch_count: Option<i32>,
    pub distance_when_placed: Option<f64>,
    
    // cTrader IDs for correlation
    pub ctrader_order_id: Option<String>,
    pub ctrader_position_id: Option<String>,
    pub ctrader_deal_id: Option<String>,
    
    // Fill/close data (only present for FILLED/CLOSED events)
    pub fill_price: Option<f64>,
    pub close_price: Option<f64>,
    pub actual_pnl_pips: Option<f64>,
    pub close_reason: Option<String>, // "Take Profit" | "Stop Loss" | "Manual"
}

impl OrderEvent {
    /// Create a new PENDING order event
    pub fn new_pending(
        zone_id: String,
        symbol: String,
        timeframe: String,
        zone_type: String,
        order_type: String,
        entry_price: f64,
        lot_size: i32,
        stop_loss: f64,
        take_profit: f64,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            zone_id,
            symbol,
            timeframe,
            status: OrderStatus::Pending,
            zone_type,
            order_type,
            entry_price,
            lot_size,
            stop_loss,
            take_profit,
            zone_high: None,
            zone_low: None,
            zone_strength: None,
            touch_count: None,
            distance_when_placed: None,
            ctrader_order_id: None,
            ctrader_position_id: None,
            ctrader_deal_id: None,
            fill_price: None,
            close_price: None,
            actual_pnl_pips: None,
            close_reason: None,
        }
    }
    
    /// Create a FILLED event from a PENDING event
    pub fn to_filled(&self, ctrader_position_id: String, fill_price: f64) -> Self {
        let mut filled = self.clone();
        filled.timestamp = Utc::now();
        filled.status = OrderStatus::Filled;
        filled.ctrader_position_id = Some(ctrader_position_id);
        filled.fill_price = Some(fill_price);
        filled
    }
    
    /// Create a CLOSED event from a FILLED event
    pub fn to_closed(
        &self,
        ctrader_deal_id: String,
        close_price: f64,
        actual_pnl_pips: f64,
        close_reason: String,
    ) -> Self {
        let mut closed = self.clone();
        closed.timestamp = Utc::now();
        closed.status = OrderStatus::Closed;
        closed.ctrader_deal_id = Some(ctrader_deal_id);
        closed.close_price = Some(close_price);
        closed.actual_pnl_pips = Some(actual_pnl_pips);
        closed.close_reason = Some(close_reason);
        closed
    }
    
    /// Create a CANCELLED event from a PENDING event
    pub fn to_cancelled(&self) -> Self {
        let mut cancelled = self.clone();
        cancelled.timestamp = Utc::now();
        cancelled.status = OrderStatus::Cancelled;
        cancelled
    }
}

/// Builder pattern for creating OrderEvent with zone metadata
pub struct OrderEventBuilder {
    event: OrderEvent,
}

impl OrderEventBuilder {
    pub fn new(
        zone_id: String,
        symbol: String,
        timeframe: String,
        zone_type: String,
        order_type: String,
        entry_price: f64,
        lot_size: i32,
        stop_loss: f64,
        take_profit: f64,
    ) -> Self {
        Self {
            event: OrderEvent::new_pending(
                zone_id, symbol, timeframe, zone_type, order_type,
                entry_price, lot_size, stop_loss, take_profit,
            ),
        }
    }
    
    pub fn with_zone_metadata(
        mut self,
        zone_high: f64,
        zone_low: f64,
        zone_strength: f64,
        touch_count: i32,
        distance_when_placed: f64,
    ) -> Self {
        self.event.zone_high = Some(zone_high);
        self.event.zone_low = Some(zone_low);
        self.event.zone_strength = Some(zone_strength);
        self.event.touch_count = Some(touch_count);
        self.event.distance_when_placed = Some(distance_when_placed);
        self
    }
    
    pub fn with_ctrader_order_id(mut self, ctrader_order_id: String) -> Self {
        self.event.ctrader_order_id = Some(ctrader_order_id);
        self
    }
    
    pub fn build(self) -> OrderEvent {
        self.event
    }
}