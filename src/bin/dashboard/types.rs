// src/bin/dashboard/types.rs - Type definitions for the dashboard
use chrono::{DateTime, Utc};
use ratatui::style::Color;

#[derive(Debug, Clone, PartialEq)]
pub enum AppPage {
    Dashboard,
    NotificationMonitor,
}

#[derive(Debug, Clone)]
pub struct ZoneDistanceInfo {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub current_price: f64,
    pub proximal_line: f64,
    pub distal_line: f64,
    pub signed_distance_pips: f64,
    pub distance_pips: f64,
    pub zone_status: ZoneStatus,
    pub last_update: DateTime<Utc>,
    pub touch_count: i32,
    pub strength_score: f64,
}

#[derive(Debug, Clone)]
pub struct TradeNotificationDisplay {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub timeframe: String,
    pub action: String,
    pub price: f64,
    pub notification_type: String,
    pub signal_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidatedTradeDisplay {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub timeframe: String,
    pub direction: String, // BUY/SELL
    pub entry_price: f64,
    pub stop_loss: f64,    // Hardcoded for now
    pub take_profit: f64,  // Hardcoded for now
    pub signal_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ZoneStatus {
    Approaching,
    AtProximal,
    InsideZone,
    AtDistal,
    Breached,
}

impl ZoneStatus {
    pub fn color(&self) -> Color {
        match self {
            ZoneStatus::AtProximal => Color::Magenta,
            ZoneStatus::InsideZone => Color::Red,
            ZoneStatus::AtDistal => Color::Blue,
            ZoneStatus::Breached => Color::DarkGray,
            ZoneStatus::Approaching => Color::Green,
        }
    }

    pub fn symbol(&self) -> &str {
        match self {
            ZoneStatus::AtProximal => "🚨",
            ZoneStatus::InsideZone => "📍",
            ZoneStatus::AtDistal => "🔵",
            ZoneStatus::Breached => "❌",
            ZoneStatus::Approaching => "👀",
        }
    }

    pub fn text(&self) -> &str {
        match self {
            ZoneStatus::AtProximal => "TRIGGER",
            ZoneStatus::InsideZone => "INSIDE",
            ZoneStatus::AtDistal => "AT_DISTAL",
            ZoneStatus::Breached => "BREACHED",
            ZoneStatus::Approaching => "APPROACHING",
        }
    }
}