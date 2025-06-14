// src/bin/dashboard/types.rs - Type definitions for the dashboard
use chrono::{DateTime, Utc};
use ratatui::style::Color;

#[derive(Debug, Clone, PartialEq)]
pub enum AppPage {
    Dashboard,
    NotificationMonitor,
    Prices,
}

// Add this struct for WebSocket price updates
#[derive(Debug, Clone)]
pub struct PriceUpdate {
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub timestamp: DateTime<Utc>,
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
    
    // Zone interaction metrics
    pub has_ever_entered: bool,
    pub total_time_inside_seconds: u64,
    pub zone_entries: u32, // Number of times price has entered the zone
    pub last_crossing_time: Option<DateTime<Utc>>,
    pub is_zone_active: bool,
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
    pub distance_pips: Option<f64>,
    pub strength: Option<f64>,
    pub touch_count: Option<i32>,
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
    // Enhanced status for interaction history
    FreshZone,      // Never touched before
    RecentlyTested, // Touched within last hour
    WeakZone,       // Multiple touches, potentially weakening
}

impl ZoneStatus {
    pub fn color(&self) -> Color {
        match self {
            ZoneStatus::AtProximal => Color::Magenta,
            ZoneStatus::InsideZone => Color::Red,
            ZoneStatus::AtDistal => Color::Blue,
            ZoneStatus::Breached => Color::DarkGray,
            ZoneStatus::Approaching => Color::Green,
            // Enhanced interaction-based colors
            ZoneStatus::FreshZone => Color::Cyan,      // Bright for untested zones
            ZoneStatus::RecentlyTested => Color::Yellow,  // Warning for recent activity
            ZoneStatus::WeakZone => Color::LightRed,   // Faded for weak zones
        }
    }

    pub fn symbol(&self) -> &str {
        match self {
            ZoneStatus::AtProximal => "🚨",
            ZoneStatus::InsideZone => "📍",
            ZoneStatus::AtDistal => "🔵",
            ZoneStatus::Breached => "❌",
            ZoneStatus::Approaching => "👀",
            // Enhanced interaction-based symbols
            ZoneStatus::FreshZone => "✨",      // Fresh/untested
            ZoneStatus::RecentlyTested => "🔄", // Recently tested
            ZoneStatus::WeakZone => "💔",       // Weak zone
        }
    }

    pub fn text(&self) -> &str {
        match self {
            ZoneStatus::AtProximal => "TRIGGER",
            ZoneStatus::InsideZone => "INSIDE",
            ZoneStatus::AtDistal => "AT_DISTAL",
            ZoneStatus::Breached => "BREACHED",
            ZoneStatus::Approaching => "APPROACHING",
            // Enhanced interaction-based text
            ZoneStatus::FreshZone => "FRESH",
            ZoneStatus::RecentlyTested => "RECENT_TEST",
            ZoneStatus::WeakZone => "WEAK",
        }
    }

    /// Priority for sorting zones - higher values appear first
    pub fn priority(&self) -> u8 {
        match self {
            ZoneStatus::InsideZone => 10,      // Highest priority - price is inside
            ZoneStatus::AtProximal => 9,       // Ready to trigger
            ZoneStatus::RecentlyTested => 8,   // Recently active zones
            ZoneStatus::AtDistal => 7,         // At exit side
            ZoneStatus::FreshZone => 6,        // Fresh zones are interesting
            ZoneStatus::Approaching => 5,      // Approaching zones
            ZoneStatus::WeakZone => 4,         // Weak zones lower priority
            ZoneStatus::Breached => 1,         // Lowest priority
        }
    }
}