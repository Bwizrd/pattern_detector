// src/bin/dashboard/app.rs - Complete app state and business logic with independent timeframe filters
use chrono::{DateTime, Utc};
use log;
use reqwest::Client as HttpClient;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::process::Command;
use tokio::time::{Duration, Instant}; // Add this line with your other imports

use redis::{Client as RedisClient, Commands};
use std::sync::{Arc, Mutex};

use crate::types::{
    AppPage, LiveTrade, TradeNotificationDisplay, TradeStatus, ValidatedTradeDisplay,
    ZoneDistanceInfo, ZoneStatus,
};
use pattern_detector::zone_interactions::ZoneInteractionContainer;
use serde::{Deserialize, Serialize};
// REMOVE: mod trading_plan;
use crate::trading_plan::TradingPlan;

// Import PendingOrder from zone monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOrder {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub order_type: String, // "BUY_LIMIT" or "SELL_LIMIT"
    pub entry_price: f64,   // The limit price
    pub lot_size: i32,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub ctrader_order_id: Option<String>, // Order ID returned by cTrader
    pub placed_at: DateTime<Utc>,
    pub status: String, // "PENDING", "FILLED", "CANCELLED", "FAILED"
    pub zone_high: f64,
    pub zone_low: f64,
    pub zone_strength: f64,
    pub touch_count: i32,
    pub distance_when_placed: f64, // Distance in pips when order was placed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOrdersContainer {
    pub last_updated: DateTime<Utc>,
    pub orders: HashMap<String, PendingOrder>, // zone_id -> PendingOrder
}

use crate::notifications::{NotificationRuleValidator, NotificationValidation};

// NEW: Add this struct to track streaming price data
#[derive(Debug, Clone)]
pub struct PriceStreamData {
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub spread: f64,
    pub last_update: DateTime<Utc>,
    pub update_count: u64,
}

pub struct App {
    pub client: HttpClient,
    pub api_base_url: String,
    pub pip_values: HashMap<String, f64>,
    pub current_page: AppPage,
    pub current_prices: HashMap<String, PriceStreamData>,
    pub price_update_count: u64,
    pub last_price_update: Option<Instant>,

    // Dashboard data
    pub zones: Vec<ZoneDistanceInfo>,
    pub placed_trades: Vec<ValidatedTradeDisplay>, // For dashboard right panel

    // Notification Monitor data
    pub all_notifications: Vec<TradeNotificationDisplay>,
    pub validated_trades: Vec<ValidatedTradeDisplay>,

    pub last_update: Instant,
    pub error_message: Option<String>,
    pub update_count: u64,

    // CHANGED: Separate timeframe filters for each page
    pub dashboard_timeframe_filters: HashMap<String, bool>,
    pub notification_timeframe_filters: HashMap<String, bool>,

    pub show_breached: bool,
    pub previous_triggers: std::collections::HashSet<String>,

    // New fields for zone ID navigation and copying
    pub selected_notification_index: Option<usize>,
    pub selected_zone_index: Option<usize>,
    pub last_copied_zone_id: Option<String>,

    pub min_strength_filter: f64,
    pub strength_input_mode: bool,
    pub strength_input_buffer: String,

    pub selected_notification_validation: Option<NotificationValidation>,
    pub notification_validator: NotificationRuleValidator,

    // Zone interaction data
    pub zone_interactions: Option<ZoneInteractionContainer>,

    // Display options
    pub show_indices: bool,     // Toggle for showing NAS100, US500 etc.
    pub show_start_dates: bool, // Toggle for showing zone start dates

    // Live trading
    pub live_trades: Vec<LiveTrade>,
    pub trading_sl_pips: f64,
    pub trading_tp_pips: f64,
    pub trading_threshold_pips: f64, // Zone extension threshold for trigger
    pub websocket_connected: bool,
    pub last_processed_notification_time: Option<DateTime<Utc>>,
    pub trading_enabled: bool, // Manual toggle for enabling/disabling trade creation
    pub show_trading_reminder: bool, // Show reminder popup to enable trading

    // Symbol filtering
    pub symbol_filter: String,    // Current symbol filter text
    pub symbol_filter_mode: bool, // Whether we're in symbol filter input mode

    // Pending orders
    pub pending_orders: HashMap<String, PendingOrder>, // zone_id -> PendingOrder

    pub redis_client: Option<RedisClient>,
    pub redis_connection: Option<Arc<Mutex<redis::Connection>>>,

    pub trading_plan_enabled: bool,
    pub trading_plan: Option<TradingPlan>,
}

impl App {
    pub fn new() -> Self {
        let (redis_client, redis_connection) = match RedisClient::open("redis://127.0.0.1:6379/") {
            Ok(client) => match client.get_connection() {
                Ok(conn) => {
                    log::info!("📊 Dashboard Redis connected for pending orders");
                    (Some(client), Some(Arc::new(Mutex::new(conn))))
                }
                Err(e) => {
                    log::warn!("📊 Dashboard Redis connection failed: {}", e);
                    (None, None)
                }
            },
            Err(e) => {
                log::warn!("📊 Dashboard Redis client creation failed: {}", e);
                (None, None)
            }
        };

        let mut pip_values = HashMap::new();

        // Major pairs
        pip_values.insert("EURUSD".to_string(), 0.0001);
        pip_values.insert("GBPUSD".to_string(), 0.0001);
        pip_values.insert("AUDUSD".to_string(), 0.0001);
        pip_values.insert("NZDUSD".to_string(), 0.0001);
        pip_values.insert("USDCAD".to_string(), 0.0001);
        pip_values.insert("USDCHF".to_string(), 0.0001);

        // Cross pairs
        pip_values.insert("EURGBP".to_string(), 0.0001);
        pip_values.insert("EURAUD".to_string(), 0.0001);
        pip_values.insert("EURNZD".to_string(), 0.0001);
        pip_values.insert("EURCHF".to_string(), 0.0001);
        pip_values.insert("EURCAD".to_string(), 0.0001);
        pip_values.insert("GBPAUD".to_string(), 0.0001);
        pip_values.insert("GBPCAD".to_string(), 0.0001);
        pip_values.insert("GBPCHF".to_string(), 0.0001);
        pip_values.insert("GBPNZD".to_string(), 0.0001);
        pip_values.insert("AUDCAD".to_string(), 0.0001);
        pip_values.insert("AUDCHF".to_string(), 0.0001);
        pip_values.insert("AUDNZD".to_string(), 0.0001);
        pip_values.insert("CADCHF".to_string(), 0.0001);
        pip_values.insert("NZDCAD".to_string(), 0.0001);
        pip_values.insert("NZDCHF".to_string(), 0.0001);

        // JPY pairs
        pip_values.insert("EURJPY".to_string(), 0.01);
        pip_values.insert("GBPJPY".to_string(), 0.01);
        pip_values.insert("AUDJPY".to_string(), 0.01);
        pip_values.insert("NZDJPY".to_string(), 0.01);
        pip_values.insert("USDJPY".to_string(), 0.01);
        pip_values.insert("CADJPY".to_string(), 0.01);
        pip_values.insert("CHFJPY".to_string(), 0.01);

        // Indices
        pip_values.insert("NAS100".to_string(), 1.0);
        pip_values.insert("US500".to_string(), 0.1);

        let api_base_url =
            env::var("API_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());

        // CHANGED: Create separate timeframe filters for each page
        let mut dashboard_timeframe_filters = HashMap::new();
        dashboard_timeframe_filters.insert("5m".to_string(), false);
        dashboard_timeframe_filters.insert("15m".to_string(), false);
        dashboard_timeframe_filters.insert("30m".to_string(), true);
        dashboard_timeframe_filters.insert("1h".to_string(), true);
        dashboard_timeframe_filters.insert("4h".to_string(), true);
        dashboard_timeframe_filters.insert("1d".to_string(), true);

        let mut notification_timeframe_filters = HashMap::new();
        notification_timeframe_filters.insert("5m".to_string(), true); // Different defaults for notifications
        notification_timeframe_filters.insert("15m".to_string(), true);
        notification_timeframe_filters.insert("30m".to_string(), true);
        notification_timeframe_filters.insert("1h".to_string(), true);
        notification_timeframe_filters.insert("4h".to_string(), true);
        notification_timeframe_filters.insert("1d".to_string(), true);

        let trading_plan_enabled = std::env::var("TRADING_PLAN_ENABLED").unwrap_or_else(|_| "false".to_string()) == "true";
        let trading_plan = if trading_plan_enabled {
            TradingPlan::load_from_file("trading_plan.json")
        } else {
            None
        };

        Self {
            client: HttpClient::new(),
            api_base_url,
            pip_values,
            current_page: AppPage::Dashboard,
            zones: Vec::new(),
            placed_trades: Vec::new(), // Empty until order placement is implemented
            all_notifications: Vec::new(),
            validated_trades: Vec::new(),
            last_update: Instant::now(),
            error_message: None,
            update_count: 0,
            dashboard_timeframe_filters,
            notification_timeframe_filters,
            show_breached: true,
            previous_triggers: std::collections::HashSet::new(),
            selected_notification_index: None,
            selected_zone_index: None,
            last_copied_zone_id: None,
            min_strength_filter: 100.0,
            strength_input_mode: false,
            strength_input_buffer: String::new(),
            current_prices: HashMap::new(),
            price_update_count: 0,
            last_price_update: None,
            selected_notification_validation: None,
            notification_validator: NotificationRuleValidator::new(),
            zone_interactions: None,

            // Display options - indices off by default
            show_indices: false,
            show_start_dates: false, // Start dates off by default

            // Live trading - get from environment variables
            live_trades: Vec::new(),
            trading_sl_pips: std::env::var("TRADING_STOP_LOSS_PIPS")
                .unwrap_or_else(|_| "40.0".to_string())
                .parse()
                .unwrap_or(40.0),
            trading_tp_pips: std::env::var("TRADING_TAKE_PROFIT_PIPS")
                .unwrap_or_else(|_| "10.0".to_string())
                .parse()
                .unwrap_or(10.0),
            trading_threshold_pips: {
                let threshold = std::env::var("TRADING_THRESHOLD_PIPS")
                    .unwrap_or_else(|_| "0.0".to_string())
                    .parse()
                    .unwrap_or(0.0);
                log::info!("📐 Dashboard trading threshold: {:.1} pips", threshold);
                threshold
            },
            websocket_connected: false,
            last_processed_notification_time: None,
            trading_enabled: false,       // Trading disabled by default
            show_trading_reminder: false, // Will show after websocket connects

            // Symbol filtering - empty by default (shows all symbols)
            symbol_filter: String::new(),
            symbol_filter_mode: false,
            pending_orders: HashMap::new(),
            redis_client,
            redis_connection,

            trading_plan_enabled,
            trading_plan,
        }
    }

    async fn ensure_redis_available(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match &self.redis_connection {
            Some(conn_arc) => {
                let mut conn = conn_arc
                    .lock()
                    .map_err(|e| format!("Redis lock error: {}", e))?;

                match conn.set::<&str, &str, ()>("dashboard_health_check", "ok") {
                    Ok(_) => {
                        let _: Result<(), _> = conn.del("dashboard_health_check");
                        Ok(())
                    }
                    Err(e) => {
                        log::warn!("🚨 Dashboard Redis health check failed: {}", e);
                        Err(format!("Redis unavailable: {}", e).into())
                    }
                }
            }
            None => Err("Redis connection not available".into()),
        }
    }

    // NEW: Method to switch to Prices page
    pub fn switch_to_prices_page(&mut self) {
        self.current_page = AppPage::Prices;
        // Reset selections when switching pages
        self.selected_notification_index = None;
        self.selected_zone_index = None;
    }

    pub fn switch_page(&mut self, page: AppPage) {
        // Reset selection when switching pages
        match page {
            AppPage::Dashboard => {
                self.selected_notification_index = None;
            }
            AppPage::NotificationMonitor => {
                self.selected_zone_index = None;
            }
            AppPage::Prices => {
                // Reset both selections when going to prices page
                self.selected_notification_index = None;
                self.selected_zone_index = None;
            }
        }
        self.current_page = page;
    }

    // CHANGED: Update toggle_timeframe to work with the current page's filters
    pub fn toggle_timeframe(&mut self, timeframe: &str) {
        let filters = match self.current_page {
            AppPage::Dashboard => &mut self.dashboard_timeframe_filters,
            AppPage::NotificationMonitor => &mut self.notification_timeframe_filters,
            AppPage::Prices => return, // Don't do timeframe filtering on prices page
        };

        if let Some(enabled) = filters.get_mut(timeframe) {
            *enabled = !*enabled;
        }

        // Reset selection if currently selected notification is now filtered out
        // (only relevant for notification page)
        if matches!(self.current_page, AppPage::NotificationMonitor) {
            if let Some(selected_index) = self.selected_notification_index {
                if let Some(notification) = self.all_notifications.get(selected_index) {
                    if !self.is_timeframe_enabled(&notification.timeframe) {
                        self.selected_notification_index = None;
                    }
                }
            }
        }
    }
    pub fn toggle_breached(&mut self) {
        self.show_breached = !self.show_breached;
    }

    pub fn toggle_indices(&mut self) {
        self.show_indices = !self.show_indices;
    }

    pub fn toggle_start_dates(&mut self) {
        self.show_start_dates = !self.show_start_dates;
    }

    pub fn toggle_trading(&mut self) {
        self.trading_enabled = !self.trading_enabled;

        // When turning trading ON, set last processed time to NOW to prevent
        // processing old notifications
        if self.trading_enabled {
            let now = Utc::now();
            self.last_processed_notification_time = Some(now);
            log::info!("🟢 Trading ENABLED at {}", now.format("%H:%M:%S"));
            // Hide reminder popup when trading is enabled
            self.show_trading_reminder = false;
        } else {
            log::info!("🔴 Trading DISABLED");
        }
    }

    pub fn dismiss_trading_reminder(&mut self) {
        self.show_trading_reminder = false;
    }

    // Live trading methods
    pub fn create_trade_from_zone_trigger(&mut self, zone: &ZoneDistanceInfo) {
        let direction = if zone.zone_type == "supply_zone" {
            "SELL"
        } else {
            "BUY"
        };
        let trade_id = format!(
            "{}_{}_{}",
            zone.symbol,
            zone.zone_id,
            Utc::now().timestamp_millis()
        );

        // Get current price from WebSocket
        if let Some(price_data) = self.current_prices.get(&zone.symbol) {
            let current_price = (price_data.bid + price_data.ask) / 2.0;
            let pip_value = self.get_pip_value(&zone.symbol);

            let (stop_loss, take_profit) = if direction == "BUY" {
                (
                    current_price - (self.trading_sl_pips * pip_value),
                    current_price + (self.trading_tp_pips * pip_value),
                )
            } else {
                (
                    current_price + (self.trading_sl_pips * pip_value),
                    current_price - (self.trading_tp_pips * pip_value),
                )
            };

            let trade = LiveTrade {
                id: trade_id,
                timestamp: Utc::now(),
                symbol: zone.symbol.clone(),
                timeframe: zone.timeframe.clone(),
                direction: direction.to_string(),
                entry_price: current_price,
                stop_loss,
                take_profit,
                zone_id: zone.zone_id.clone(),
                current_price,
                unrealized_pips: 0.0,
                status: crate::types::TradeStatus::Open,
            };

            self.live_trades.push(trade);
            log::info!(
                "✅ Trade created: {} {} @ {:.4} Zone:{}",
                direction,
                zone.symbol,
                current_price,
                &zone.zone_id[..8]
            );
        }
    }

    pub fn create_trade_from_trigger(&mut self, notification: &TradeNotificationDisplay) {
        // Only create trade if it's a trading signal (not proximity alert)
        if notification.notification_type == "trading_signal" {
            let trade_id = format!(
                "{}_{}",
                notification.symbol,
                notification.timestamp.timestamp_millis()
            );

            let direction = notification.action.clone();
            let entry_price = notification.price;

            // Calculate SL and TP based on direction and environment variables
            let pip_value = self.get_pip_value(&notification.symbol);
            let (stop_loss, take_profit) = if direction == "BUY" {
                (
                    entry_price - (self.trading_sl_pips * pip_value),
                    entry_price + (self.trading_tp_pips * pip_value),
                )
            } else {
                (
                    entry_price + (self.trading_sl_pips * pip_value),
                    entry_price - (self.trading_tp_pips * pip_value),
                )
            };

            let live_trade = LiveTrade {
                id: trade_id,
                timestamp: notification.timestamp,
                symbol: notification.symbol.clone(),
                timeframe: notification.timeframe.clone(),
                direction,
                entry_price,
                stop_loss,
                take_profit,
                zone_id: notification.signal_id.clone().unwrap_or_default(),
                current_price: entry_price, // Initialize with entry price
                unrealized_pips: 0.0,
                status: TradeStatus::Open,
            };

            self.live_trades.push(live_trade);
        }
    }

    pub fn update_live_trades_with_price(&mut self, symbol: &str, price: f64) {
        let pip_value = self.get_pip_value(symbol);

        for trade in &mut self.live_trades {
            if trade.symbol == symbol && trade.status == TradeStatus::Open {
                trade.current_price = price;

                // Calculate unrealized P&L in pips
                if trade.direction == "BUY" {
                    trade.unrealized_pips = (price - trade.entry_price) / pip_value;

                    // Check for SL/TP hits
                    if price <= trade.stop_loss {
                        trade.status = TradeStatus::StoppedOut;
                        trade.unrealized_pips = -self.trading_sl_pips;
                    } else if price >= trade.take_profit {
                        trade.status = TradeStatus::TakenProfit;
                        trade.unrealized_pips = self.trading_tp_pips;
                    }
                } else {
                    // SELL
                    trade.unrealized_pips = (trade.entry_price - price) / pip_value;

                    // Check for SL/TP hits
                    if price >= trade.stop_loss {
                        trade.status = TradeStatus::StoppedOut;
                        trade.unrealized_pips = -self.trading_sl_pips;
                    } else if price <= trade.take_profit {
                        trade.status = TradeStatus::TakenProfit;
                        trade.unrealized_pips = self.trading_tp_pips;
                    }
                }
            }
        }
    }

    pub fn get_total_unrealized_pips(&self) -> f64 {
        self.live_trades
            .iter()
            .filter(|t| t.status == TradeStatus::Open)
            .map(|t| t.unrealized_pips)
            .sum()
    }

    pub fn get_closed_trades_total_pips(&self) -> f64 {
        self.live_trades
            .iter()
            .filter(|t| t.status == TradeStatus::StoppedOut || t.status == TradeStatus::TakenProfit)
            .map(|t| t.unrealized_pips)
            .sum()
    }

    fn get_pip_value(&self, symbol: &str) -> f64 {
        match symbol {
            s if s.contains("JPY") => 0.01,
            "NAS100" => 1.0,
            "US500" => 0.1,
            _ => 0.0001,
        }
    }

    fn check_for_zone_triggers(&mut self) {
        // Only create trades if trading is enabled and WebSocket is connected
        if !self.trading_enabled || !self.websocket_connected {
            return;
        }

        let now = Utc::now();

        // SAFETY: Don't process any signals if trading was just enabled (within last 10 seconds)
        // This prevents processing triggers when user first turns on trading
        if let Some(last_processed) = self.last_processed_notification_time {
            let time_since_baseline = now.signed_duration_since(last_processed);
            if time_since_baseline < chrono::Duration::seconds(10) {
                return; // Too soon after enabling trading
            }
        } else {
            return; // No baseline set yet
        }

        let triggers: Vec<ZoneDistanceInfo> = self
            .zones
            .iter()
            .filter(|zone| zone.zone_status == ZoneStatus::AtProximal)
            .cloned()
            .collect();

        if !triggers.is_empty() {
            log::info!("🎯 Found {} zone triggers on dashboard", triggers.len());
        }

        for zone in triggers {
            // Check if we already have a trade for this zone
            let exists = self
                .live_trades
                .iter()
                .any(|t| t.id.contains(&zone.zone_id));

            if !exists {
                // Check if we have current price for this symbol
                if self.current_prices.contains_key(&zone.symbol) {
                    log::info!(
                        "🚨 Creating trade from zone trigger: {} {} Zone:{}",
                        zone.symbol,
                        zone.zone_type,
                        &zone.zone_id[..8]
                    );
                    self.create_trade_from_zone_trigger(&zone);
                } else {
                    log::info!("⚠️ No current price for symbol: {}", zone.symbol);
                }
            }
        }
    }

    // CHANGED: Update is_timeframe_enabled to use the current page's filters
    pub fn is_timeframe_enabled(&self, timeframe: &str) -> bool {
        let filters = match self.current_page {
            AppPage::Dashboard => &self.dashboard_timeframe_filters,
            AppPage::NotificationMonitor => &self.notification_timeframe_filters,
            AppPage::Prices => return true, // All timeframes enabled for prices page
        };

        filters.get(timeframe).copied().unwrap_or(true)
    }

    /// Returns the filtered list of zones as used by the dashboard UI (trading plan, day trading mode, symbol, timeframe, etc.)
    pub fn get_filtered_zones(&self) -> Vec<&ZoneDistanceInfo> {
        let day_trading_mode = std::env::var("DAY_TRADING_MODE_ACTIVE").unwrap_or_else(|_| "false".to_string()) == "true";
        let mut filtered: Vec<&ZoneDistanceInfo> = if self.trading_plan_enabled || day_trading_mode {
            let mut allowed = std::collections::HashSet::new();
            if self.trading_plan_enabled {
                if let Some(plan) = &self.trading_plan {
                    for s in &plan.top_setups {
                        allowed.insert((s.symbol.as_str(), s.timeframe.as_str()));
                    }
                }
            }
            if day_trading_mode {
                // Allow all 5m/15m zones (for all symbols)
                for zone in &self.zones {
                    if zone.timeframe == "5m" || zone.timeframe == "15m" {
                        allowed.insert((zone.symbol.as_str(), zone.timeframe.as_str()));
                    }
                }
            }
            self.zones.iter().filter(|zone|
                allowed.contains(&(zone.symbol.as_str(), zone.timeframe.as_str()))
            ).collect()
        } else {
            self.zones.iter().collect()
        };
        // Apply timeframe filter
        filtered.retain(|zone| self.is_timeframe_enabled(&zone.timeframe));
        // Apply breached filter
        if !self.show_breached {
            filtered.retain(|zone| zone.zone_status != ZoneStatus::Breached);
        }
        // Apply strength filter (dashboard only)
        if matches!(self.current_page, AppPage::Dashboard) {
            filtered.retain(|zone| zone.strength_score >= self.min_strength_filter);
        }
        // Apply indices filter
        if !self.show_indices {
            filtered.retain(|zone| zone.symbol != "NAS100" && zone.symbol != "US500");
        }
        // Apply symbol filter (dashboard only)
        if matches!(self.current_page, AppPage::Dashboard) {
            filtered.retain(|zone| self.symbol_matches_filter(&zone.symbol));
        }
        // Sorting (same as process_zones_response)
        filtered.sort_by(|a, b| {
            use std::cmp::Ordering;
            let a_priority = a.zone_status.priority();
            let b_priority = b.zone_status.priority();
            match b_priority.cmp(&a_priority) {
                Ordering::Equal => a.distance_pips.partial_cmp(&b.distance_pips).unwrap_or(Ordering::Equal),
                other => other,
            }
        });
        filtered
    }

    pub fn select_next_zone(&mut self) {
        let filtered = self.get_filtered_zones();
        if !filtered.is_empty() {
            match self.selected_zone_index {
                Some(current_index) => {
                    let next_index = (current_index + 1) % filtered.len();
                    self.selected_zone_index = Some(next_index);
                }
                None => {
                    self.selected_zone_index = Some(0);
                }
            }
        }
    }

    pub fn select_previous_zone(&mut self) {
        let filtered = self.get_filtered_zones();
        if !filtered.is_empty() {
            match self.selected_zone_index {
                Some(current_index) => {
                    let prev_index = if current_index == 0 {
                        filtered.len() - 1
                    } else {
                        current_index - 1
                    };
                    self.selected_zone_index = Some(prev_index);
                }
                None => {
                    self.selected_zone_index = Some(0);
                }
            }
        }
    }

    pub fn copy_selected_dashboard_zone_id(&mut self) {
        let filtered = self.get_filtered_zones();
        if let Some(index) = self.selected_zone_index {
            if let Some(zone) = filtered.get(index) {
                match self.copy_to_clipboard(&zone.zone_id) {
                    Ok(()) => {
                        self.last_copied_zone_id = Some(format!("✅ Copied: {}", zone.zone_id));
                    }
                    Err(_) => {
                        self.last_copied_zone_id = Some(format!("Zone ID: {}", zone.zone_id));
                    }
                }
            } else {
                self.last_copied_zone_id = Some("❌ No zone selected".to_string());
            }
        } else {
            self.last_copied_zone_id = Some("❌ No zone selected".to_string());
        }
    }

    pub fn copy_selected_zone_details(&mut self) {
        let filtered = self.get_filtered_zones();
        if let Some(index) = self.selected_zone_index {
            if let Some(zone) = filtered.get(index) {
                let zone_type_display = if zone.zone_type.contains("supply") {
                    "Supply"
                } else {
                    "Demand"
                };
                let (high, low) = if zone.zone_type.contains("supply") {
                    (zone.distal_line, zone.proximal_line)
                } else {
                    (zone.proximal_line, zone.distal_line)
                };
                let date_time_str = if let Some(created_at) = zone.created_at {
                    created_at.format("%Y-%m-%d, %H:%M").to_string()
                } else {
                    "N/A".to_string()
                };
                let zone_details = format!(
                    "{},{:.4},{:.4},{}",
                    zone_type_display, high, low, date_time_str
                );
                match self.copy_to_clipboard(&zone_details) {
                    Ok(()) => {
                        self.last_copied_zone_id = Some(format!("✅ Zone Details: {}", zone_details));
                    }
                    Err(_) => {
                        self.last_copied_zone_id = Some(format!("Zone Details: {}", zone_details));
                    }
                }
            } else {
                self.last_copied_zone_id = Some("❌ No zone selected".to_string());
            }
        } else {
            self.last_copied_zone_id = Some("❌ No zone selected".to_string());
        }
    }

    pub fn validate_selected_notification(&mut self) {
        if let Some(index) = self.selected_notification_index {
            if let Some(notification) = self.all_notifications.get(index) {
                self.selected_notification_validation = Some(
                    self.notification_validator
                        .validate_notification(notification, index),
                );
            }
        } else {
            self.selected_notification_validation = None;
        }
    }

    // ADD this new method for direct number selection
    pub fn select_notification_by_number(&mut self, number: usize) {
        if number > 0 && number <= self.all_notifications.len() {
            let filtered_notifications: Vec<_> = self
                .all_notifications
                .iter()
                .enumerate()
                .filter(|(_, notif)| self.is_timeframe_enabled(&notif.timeframe))
                .collect();

            if number <= filtered_notifications.len() {
                let actual_index = filtered_notifications[number - 1].0;
                self.selected_notification_index = Some(actual_index);
                self.validate_selected_notification();
            }
        }
    }

    pub fn get_price_stats(&self) -> (usize, u64, Option<Duration>) {
        let symbol_count = self.current_prices.len();
        let total_updates = self.price_update_count;
        let last_update_elapsed = self.last_price_update.map(|t| t.elapsed());

        (symbol_count, total_updates, last_update_elapsed)
    }

    // Navigation methods for notifications
    pub fn select_next_notification(&mut self) {
        let filtered_notifications: Vec<_> = self
            .all_notifications
            .iter()
            .enumerate()
            .filter(|(_, notif)| self.is_timeframe_enabled(&notif.timeframe))
            .collect();

        if !filtered_notifications.is_empty() {
            match self.selected_notification_index {
                Some(current_index) => {
                    // Find current position in filtered list
                    if let Some(current_pos) = filtered_notifications
                        .iter()
                        .position(|(idx, _)| *idx == current_index)
                    {
                        // Move to next in filtered list
                        let next_pos = (current_pos + 1) % filtered_notifications.len();
                        self.selected_notification_index = Some(filtered_notifications[next_pos].0);
                    } else {
                        // Current selection not in filtered list, select first
                        self.selected_notification_index = Some(filtered_notifications[0].0);
                    }
                }
                None => {
                    // No selection, select first filtered notification
                    self.selected_notification_index = Some(filtered_notifications[0].0);
                }
            }
        }
        self.validate_selected_notification();
    }

    pub fn select_previous_notification(&mut self) {
        let filtered_notifications: Vec<_> = self
            .all_notifications
            .iter()
            .enumerate()
            .filter(|(_, notif)| self.is_timeframe_enabled(&notif.timeframe))
            .collect();

        if !filtered_notifications.is_empty() {
            match self.selected_notification_index {
                Some(current_index) => {
                    // Find current position in filtered list
                    if let Some(current_pos) = filtered_notifications
                        .iter()
                        .position(|(idx, _)| *idx == current_index)
                    {
                        // Move to previous in filtered list
                        let prev_pos = if current_pos == 0 {
                            filtered_notifications.len() - 1
                        } else {
                            current_pos - 1
                        };
                        self.selected_notification_index = Some(filtered_notifications[prev_pos].0);
                    } else {
                        // Current selection not in filtered list, select first
                        self.selected_notification_index = Some(filtered_notifications[0].0);
                    }
                }
                None => {
                    // No selection, select first filtered notification
                    self.selected_notification_index = Some(filtered_notifications[0].0);
                }
            }
        }
        self.validate_selected_notification();
    }

    pub fn copy_selected_zone_id(&mut self) {
        if let Some(index) = self.selected_notification_index {
            if let Some(notification) = self.all_notifications.get(index) {
                if let Some(zone_id) = &notification.signal_id {
                    // Try to copy to system clipboard
                    match self.copy_to_clipboard(zone_id) {
                        Ok(()) => {
                            self.last_copied_zone_id = Some(format!("✅ Copied: {}", zone_id));
                        }
                        Err(_) => {
                            // Fallback: just show the zone ID
                            self.last_copied_zone_id = Some(format!("Zone ID: {}", zone_id));
                        }
                    }
                } else {
                    self.last_copied_zone_id = Some("❌ No zone ID available".to_string());
                }
            }
        } else {
            self.last_copied_zone_id = Some("❌ No notification selected".to_string());
        }
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Try different clipboard commands based on OS
        let commands = [
            // macOS
            ("pbcopy", vec![]),
            // Linux with xclip
            ("xclip", vec!["-selection", "clipboard"]),
            // Linux with xsel
            ("xsel", vec!["--clipboard", "--input"]),
            // Windows (if you're using WSL)
            ("clip.exe", vec![]),
        ];

        for (cmd, args) in commands.iter() {
            if let Ok(mut process) = Command::new(cmd)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(stdin) = process.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = process.wait();
                return Ok(());
            }
        }

        Err("No clipboard command available".into())
    }

    pub fn toggle_strength_input_mode(&mut self) {
        if self.strength_input_mode {
            // Exiting input mode - try to parse the buffer
            if let Ok(value) = self.strength_input_buffer.parse::<f64>() {
                if value >= 0.0 {
                    self.min_strength_filter = value;
                }
            }
            self.strength_input_buffer.clear();
            self.strength_input_mode = false;
        } else {
            // Entering input mode - populate buffer with current value
            self.strength_input_buffer = self.min_strength_filter.to_string();
            self.strength_input_mode = true;
        }
    }

    pub fn handle_strength_input(&mut self, c: char) {
        if self.strength_input_mode {
            match c {
                '0'..='9' | '.' => {
                    // Only allow one decimal point
                    if c == '.' && self.strength_input_buffer.contains('.') {
                        return;
                    }
                    self.strength_input_buffer.push(c);
                }
                _ => {} // Ignore other characters
            }
        }
    }

    pub fn handle_strength_backspace(&mut self) {
        if self.strength_input_mode {
            self.strength_input_buffer.pop();
        }
    }

    pub fn cancel_strength_input(&mut self) {
        if self.strength_input_mode {
            self.strength_input_buffer.clear();
            self.strength_input_mode = false;
        }
    }

    pub fn get_strength_filter_status(&self) -> String {
        if self.strength_input_mode {
            format!("Min Strength: {} (editing)", self.strength_input_buffer)
        } else {
            format!("Min Strength: {}", self.min_strength_filter)
        }
    }

    // Symbol filtering methods
    pub fn toggle_symbol_filter_mode(&mut self) {
        self.symbol_filter_mode = !self.symbol_filter_mode;
        if !self.symbol_filter_mode {
            // Reset zone selection when exiting filter mode
            self.selected_zone_index = None;
        }
    }

    pub fn handle_symbol_filter_input(&mut self, c: char) {
        if self.symbol_filter_mode {
            match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' => {
                    self.symbol_filter.push(c.to_ascii_uppercase());
                }
                _ => {} // Ignore other characters
            }
        }
    }

    pub fn handle_symbol_filter_backspace(&mut self) {
        if self.symbol_filter_mode {
            self.symbol_filter.pop();
        }
    }

    pub fn cancel_symbol_filter(&mut self) {
        if self.symbol_filter_mode {
            self.symbol_filter.clear();
            self.symbol_filter_mode = false;
            self.selected_zone_index = None;
        }
    }

    pub fn clear_symbol_filter(&mut self) {
        self.symbol_filter.clear();
        self.selected_zone_index = None;
    }

    pub fn symbol_matches_filter(&self, symbol: &str) -> bool {
        if self.symbol_filter.is_empty() {
            return true; // No filter means show all
        }

        // Case-insensitive partial matching
        let filter_upper = self.symbol_filter.to_uppercase();
        let symbol_upper = symbol.to_uppercase();

        // Support intelligent matching - if user types "JPY", show all JPY pairs
        symbol_upper.contains(&filter_upper)
    }

    pub fn get_symbol_filter_status(&self) -> String {
        if self.symbol_filter_mode {
            if self.symbol_filter.is_empty() {
                "Symbol Filter: _ (editing)".to_string()
            } else {
                format!("Symbol Filter: {} (editing)", self.symbol_filter)
            }
        } else if self.symbol_filter.is_empty() {
            "Symbol Filter: ALL".to_string()
        } else {
            format!("Symbol Filter: {}", self.symbol_filter)
        }
    }

    pub async fn update_data(&mut self) {
        self.error_message = None;

        // Check WebSocket connection status based on recent price updates
        if let Some(last_update) = self.last_price_update {
            let now = tokio::time::Instant::now();
            let elapsed = now.duration_since(last_update);

            // Mark as disconnected if no price updates for more than 30 seconds
            if elapsed.as_secs() > 30 {
                self.websocket_connected = false;
            }
        } else {
            self.websocket_connected = false;
        }

        match self.fetch_all_data().await {
            Ok(()) => {
                self.last_update = Instant::now();
                self.update_count += 1;

                // Reset zone selection if we have fewer zones now
                if let Some(index) = self.selected_zone_index {
                    if index >= self.zones.len() {
                        self.selected_zone_index = if self.zones.is_empty() {
                            None
                        } else {
                            Some(self.zones.len() - 1)
                        };
                    }
                }

                // Reset notification selection if we have fewer notifications now
                if let Some(index) = self.selected_notification_index {
                    if index >= self.all_notifications.len() {
                        self.selected_notification_index = if self.all_notifications.is_empty() {
                            None
                        } else {
                            Some(self.all_notifications.len() - 1)
                        };
                    }
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Error: {}", e));
            }
        }
    }

    async fn fetch_all_data(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Fetch zones
        let zones_response = self.get_zones_from_api().await?;

        // Load zone interaction data
        self.load_zone_interactions().await;

        // Load pending orders
        self.load_pending_orders().await;

        // CHANGED: Use WebSocket prices instead of API prices
        let current_prices = self.convert_websocket_to_simple_prices();

        // Rest of your existing code...
        let notifications_response = self.fetch_trade_notifications_from_api().await?;
        let validated_signals_response = self.fetch_validated_signals_from_api().await?;

        // Process all data with WebSocket prices
        self.zones = self.process_zones_response(zones_response, current_prices)?;
        let new_notifications = self.process_api_notifications_response(notifications_response)?;

        // Check for zone triggers and create trades (independent from notifications)
        self.check_for_zone_triggers();

        self.all_notifications = new_notifications;
        self.validated_trades =
            self.process_validated_signals_response(validated_signals_response)?;
        self.placed_trades = Vec::new();

        Ok(())
    }

    async fn load_zone_interactions(&mut self) {
        match ZoneInteractionContainer::load_from_file("zone_interactions.json") {
            Ok(interactions) => {
                self.zone_interactions = Some(interactions);
            }
            Err(_) => {
                // If file doesn't exist or can't be loaded, start with empty container
                self.zone_interactions = Some(ZoneInteractionContainer::new());
            }
        }
    }

    fn convert_websocket_to_simple_prices(&self) -> HashMap<String, f64> {
        self.current_prices
            .iter()
            .map(|(symbol, price_data)| {
                let mid_price = (price_data.bid + price_data.ask) / 2.0;
                (symbol.clone(), mid_price)
            })
            .collect()
    }

    async fn fetch_validated_signals_from_api(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let url = format!("{}/validated-signals", self.api_base_url);

        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.json().await?)
        } else {
            Ok(serde_json::json!({"signals": []}))
        }
    }

    fn process_validated_signals_response(
        &self,
        response: Value,
    ) -> Result<Vec<ValidatedTradeDisplay>, Box<dyn std::error::Error>> {
        let empty_vec = vec![];
        let signals_array = response
            .get("signals")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);

        let mut trades = Vec::new();

        for signal_json in signals_array.iter().take(50) {
            let timestamp_str = signal_json
                .get("validation_timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let timestamp = DateTime::parse_from_rfc3339(timestamp_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let entry_price = signal_json
                .get("price")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let direction = signal_json
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let symbol = signal_json
                .get("symbol")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Get pip value for this symbol
            let pip_value = self.pip_values.get(&symbol).cloned().unwrap_or(0.0001);

            // Hardcoded SL and TP for now (50 pip SL, 100 pip TP)
            let (stop_loss, take_profit) = if direction == "BUY" {
                (
                    entry_price - (50.0 * pip_value),
                    entry_price + (100.0 * pip_value),
                )
            } else {
                (
                    entry_price + (50.0 * pip_value),
                    entry_price - (100.0 * pip_value),
                )
            };

            let trade = ValidatedTradeDisplay {
                timestamp,
                symbol,
                timeframe: signal_json
                    .get("timeframe")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                direction,
                entry_price,
                stop_loss,
                take_profit,
                signal_id: signal_json
                    .get("signal_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            };

            trades.push(trade);
        }

        Ok(trades)
    }

    async fn fetch_trade_notifications_from_api(
        &self,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        // Try to read from shared notifications file first
        let notifications_file = "shared_notifications.json";

        match tokio::fs::read_to_string(notifications_file).await {
            Ok(content) => {
                let shared_file: Value = serde_json::from_str(&content)?;

                if let Some(notifications) = shared_file.get("notifications") {
                    // Convert shared format to the format expected by dashboard
                    Ok(json!({
                        "notifications": notifications
                    }))
                } else {
                    Ok(json!({"notifications": []}))
                }
            }
            Err(_) => {
                // Fallback to API if file doesn't exist
                self.fetch_trade_notifications_from_api_fallback().await
            }
        }
    }
    async fn fetch_trade_notifications_from_api_fallback(
        &self,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let url = format!("{}/trade-notifications", self.api_base_url);

        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.json().await?)
        } else {
            Ok(serde_json::json!({"notifications": []}))
        }
    }

    // Update the processing method to handle the shared format
    fn process_api_notifications_response(
        &self,
        response: Value,
    ) -> Result<Vec<TradeNotificationDisplay>, Box<dyn std::error::Error>> {
        let empty_vec = vec![];
        let notifications_array = response
            .get("notifications")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);

        let mut notifications = Vec::new();

        for notif_json in notifications_array.iter().take(50) {
            let timestamp_str = notif_json
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let timestamp = DateTime::parse_from_rfc3339(timestamp_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let notification = TradeNotificationDisplay {
                timestamp,
                symbol: notif_json
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                timeframe: notif_json
                    .get("timeframe")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                action: notif_json
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                price: notif_json
                    .get("price")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                notification_type: notif_json
                    .get("zone_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("zone_trigger")
                    .to_string(),
                signal_id: notif_json
                    .get("zone_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                // Add the new fields:
                distance_pips: notif_json.get("distance_pips").and_then(|v| v.as_f64()),
                strength: notif_json.get("strength").and_then(|v| v.as_f64()),
                touch_count: notif_json
                    .get("touch_count")
                    .and_then(|v| v.as_i64())
                    .map(|i| i as i32),
            };

            notifications.push(notification);
        }

        Ok(notifications)
    }

    pub async fn clear_notifications_via_api(&self) {
        let url = format!("{}/trade-notifications/clear", self.api_base_url);

        match self.client.post(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    log::info!("✅ Notifications cleared via API");
                }
            }
            Err(_) => {
                // Ignore errors
            }
        }
    }

    pub fn get_stats(&self) -> (usize, usize, usize, usize, usize) {
        let total = self.zones.len();
        let triggers = self
            .zones
            .iter()
            .filter(|z| z.zone_status == ZoneStatus::AtProximal)
            .count();
        let inside = self
            .zones
            .iter()
            .filter(|z| z.zone_status == ZoneStatus::InsideZone)
            .count();
        let close = self.zones.iter().filter(|z| z.distance_pips < 10.0).count();
        let watch = self.zones.iter().filter(|z| z.distance_pips < 25.0).count();

        (total, triggers, inside, close, watch)
    }

    // CHANGED: Update get_timeframe_status to use the current page's filters
    pub fn get_timeframe_status(&self) -> String {
        let filters = match self.current_page {
            AppPage::Dashboard => &self.dashboard_timeframe_filters,
            AppPage::NotificationMonitor => &self.notification_timeframe_filters,
            AppPage::Prices => return "All timeframes".to_string(), // Prices page shows all
        };

        let enabled_timeframes: Vec<&str> = filters
            .iter()
            .filter_map(|(tf, &enabled)| if enabled { Some(tf.as_str()) } else { None })
            .collect();

        if enabled_timeframes.is_empty() {
            "None".to_string()
        } else {
            enabled_timeframes.join(", ")
        }
    }

    pub fn get_breached_status(&self) -> &str {
        if self.show_breached {
            "ON"
        } else {
            "OFF"
        }
    }

    pub fn update_price_from_websocket(&mut self, price_update: crate::types::PriceUpdate) {
        let price_stream_data = PriceStreamData {
            symbol: price_update.symbol.clone(),
            bid: price_update.bid,
            ask: price_update.ask,
            spread: price_update.ask - price_update.bid,
            last_update: price_update.timestamp,
            update_count: self
                .current_prices
                .get(&price_update.symbol)
                .map(|p| p.update_count + 1)
                .unwrap_or(1),
        };

        self.current_prices
            .insert(price_update.symbol.clone(), price_stream_data);
        self.price_update_count += 1;
        self.last_price_update = Some(tokio::time::Instant::now());

        // Mark WebSocket as connected when we receive prices
        let was_connected = self.websocket_connected;
        self.websocket_connected = true;

        // Show trading reminder popup when WebSocket first connects (and trading is disabled)
        if !was_connected && !self.trading_enabled {
            self.show_trading_reminder = true;
        }

        // Update live trades with new price
        let mid_price = (price_update.bid + price_update.ask) / 2.0;
        self.update_live_trades_with_price(&price_update.symbol, mid_price);
    }

    // Zone processing methods (keep all existing methods)...
    fn process_zones_response(
        &self,
        zones_json: Vec<Value>,
        current_prices: HashMap<String, f64>,
    ) -> Result<Vec<ZoneDistanceInfo>, Box<dyn std::error::Error>> {
        let mut zone_distances = Vec::new();

        for zone_json in &zones_json {
            if let Ok(zone_info) = self.extract_zone_distance_info(zone_json, &current_prices) {
                // Filter by timeframe
                if self.is_timeframe_enabled(&zone_info.timeframe) {
                    // Filter out breached zones if disabled
                    if zone_info.zone_status == ZoneStatus::Breached && !self.show_breached {
                        continue;
                    }

                    // Filter by strength (only on dashboard)
                    if matches!(self.current_page, AppPage::Dashboard)
                        && zone_info.strength_score < self.min_strength_filter
                    {
                        continue;
                    }

                    // Filter out indices if disabled
                    if !self.show_indices
                        && (zone_info.symbol == "NAS100" || zone_info.symbol == "US500")
                    {
                        continue;
                    }

                    // Filter by symbol (only on dashboard)
                    if matches!(self.current_page, AppPage::Dashboard)
                        && !self.symbol_matches_filter(&zone_info.symbol)
                    {
                        continue;
                    }

                    zone_distances.push(zone_info);
                }
            }
        }

        // Keep only valid zones
        zone_distances.retain(|z| {
            z.current_price > 0.0 && z.distance_pips >= 0.0 && z.distance_pips < 10000.0
        });

        // Enhanced sorting - prioritize by status (including interaction history), then by distance
        zone_distances.sort_by(|a, b| {
            use std::cmp::Ordering;

            let a_priority = a.zone_status.priority();
            let b_priority = b.zone_status.priority();

            match b_priority.cmp(&a_priority) {
                // Note: reversed for descending order (higher priority first)
                Ordering::Equal => {
                    // Same status, sort by distance (closest first)
                    a.distance_pips
                        .partial_cmp(&b.distance_pips)
                        .unwrap_or(Ordering::Equal)
                }
                other => other,
            }
        });

        Ok(zone_distances)
    }

    async fn get_zones_from_api(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let cache_path = "shared_zones.json";

        // Try to read the file
        match tokio::fs::read_to_string(cache_path).await {
            Ok(content) => {
                let json_value: Value = serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse shared_zones.json: {}", e))?;

                // Extract zones from the cache format
                if let Some(zones_obj) = json_value.get("zones") {
                    if let Some(zones_map) = zones_obj.as_object() {
                        // Flatten all zones from all symbols into a single array
                        let mut all_zones = Vec::new();

                        for (_symbol, symbol_zones) in zones_map {
                            if let Some(zones_array) = symbol_zones.as_array() {
                                for zone in zones_array {
                                    // Convert to the format expected by process_zones_response
                                    let converted_zone = json!({
                                        "zone_id": zone.get("id").unwrap_or(&Value::Null),
                                        "symbol": zone.get("symbol").unwrap_or(&Value::Null),
                                        "timeframe": zone.get("timeframe").unwrap_or(&Value::Null),
                                        "type": zone.get("zone_type").unwrap_or(&Value::Null),
                                        "zone_high": zone.get("high").unwrap_or(&Value::Null),
                                        "zone_low": zone.get("low").unwrap_or(&Value::Null),
                                        "touch_count": zone.get("touch_count").unwrap_or(&json!(0)),
                                        "strength_score": zone.get("strength").unwrap_or(&json!(0.0)),
                                        "created_at": zone.get("created_at").unwrap_or(&Value::Null),
                                        "start_time": zone.get("start_time").unwrap_or(&Value::Null)
                                    });
                                    all_zones.push(converted_zone);
                                }
                            }
                        }

                        // println!(
                        //     "📄 [CACHE] Loaded {} zones from shared_zones.json",
                        //     all_zones.len()
                        // );
                        Ok(all_zones)
                    } else {
                        Err("Invalid zones structure in cache file".into())
                    }
                } else {
                    Err("No zones found in cache file".into())
                }
            }
            Err(e) => {
                // Fallback to API if cache file doesn't exist or can't be read
                println!(
                    "⚠️ [CACHE] Failed to read shared_zones.json: {}, falling back to API",
                    e
                );
                self.get_zones_from_debug_api().await
            }
        }
    }

    async fn get_current_prices(&self) -> Result<HashMap<String, f64>, Box<dyn std::error::Error>> {
        let url = format!("{}/current-prices", self.api_base_url);

        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(HashMap::new());
        }

        let json: serde_json::Value = response.json().await?;

        if let Some(prices_obj) = json.get("prices") {
            if let Some(prices_map) = prices_obj.as_object() {
                let mut result = HashMap::new();
                for (symbol, price_value) in prices_map {
                    if let Some(price) = price_value.as_f64() {
                        result.insert(symbol.clone(), price);
                    }
                }
                return Ok(result);
            }
        }

        Ok(HashMap::new())
    }

    async fn fetch_detailed_prices(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/current-prices-detailed", self.api_base_url);

        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let json: serde_json::Value = resp.json().await?;

                if let Some(prices_obj) = json.get("prices") {
                    if let Some(prices_map) = prices_obj.as_object() {
                        for (symbol, price_data) in prices_map {
                            if let Some(price_obj) = price_data.as_object() {
                                let bid =
                                    price_obj.get("bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                let ask =
                                    price_obj.get("ask").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                let spread = ask - bid;

                                let price_stream_data = PriceStreamData {
                                    symbol: symbol.clone(),
                                    bid,
                                    ask,
                                    spread,
                                    last_update: Utc::now(),
                                    update_count: self
                                        .current_prices
                                        .get(symbol)
                                        .map(|p| p.update_count + 1)
                                        .unwrap_or(1),
                                };

                                self.current_prices
                                    .insert(symbol.clone(), price_stream_data);
                            }
                        }
                        self.price_update_count += 1;
                        self.last_price_update = Some(Instant::now());
                    }
                }
            }
            _ => {
                // Fallback to basic price fetch if detailed endpoint doesn't exist
                let basic_prices = self.get_current_prices().await.unwrap_or_default();
                for (symbol, price) in basic_prices {
                    // Create basic price data from mid price
                    let spread_estimate = price * 0.00002; // Rough 2 pip spread estimate
                    let bid = price - (spread_estimate / 2.0);
                    let ask = price + (spread_estimate / 2.0);

                    let price_stream_data = PriceStreamData {
                        symbol: symbol.clone(),
                        bid,
                        ask,
                        spread: spread_estimate,
                        last_update: Utc::now(),
                        update_count: self
                            .current_prices
                            .get(&symbol)
                            .map(|p| p.update_count + 1)
                            .unwrap_or(1),
                    };

                    self.current_prices.insert(symbol, price_stream_data);
                }
                self.price_update_count += 1;
                self.last_price_update = Some(Instant::now());
            }
        }

        Ok(())
    }

    async fn get_zones_from_debug_api(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let url = format!("{}/debug/minimal-cache-zones", self.api_base_url);

        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("API returned status: {}", response.status()).into());
        }

        let response_text = response.text().await?;
        let json_value: Value = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;

        match json_value {
            Value::Array(zones) => Ok(zones),
            Value::Object(obj) => {
                if let Some(Value::Array(zones)) = obj.get("retrieved_zones") {
                    Ok(zones.clone())
                } else if let Some(Value::Array(zones)) = obj.get("zones") {
                    Ok(zones.clone())
                } else if let Some(Value::Array(zones)) = obj.get("data") {
                    Ok(zones.clone())
                } else {
                    let keys: Vec<_> = obj.keys().collect();
                    Err(format!("Unexpected object structure. Available keys: {:?}", keys).into())
                }
            }
            _ => Err("Unexpected response type".into()),
        }
    }

    fn calculate_zone_status(
        &self,
        current_price: f64,
        proximal_line: f64,
        distal_line: f64,
        is_supply: bool,
        pip_value: f64,
    ) -> ZoneStatus {
        let proximity_threshold = 2.0 * pip_value;

        // Extend the zone boundary by TRADING_THRESHOLD_PIPS
        let extended_proximal_line = if is_supply {
            // Supply zone: extend lower (reduce proximal line)
            proximal_line - (self.trading_threshold_pips * pip_value)
        } else {
            // Demand zone: extend higher (increase proximal line)
            proximal_line + (self.trading_threshold_pips * pip_value)
        };

        if is_supply {
            // Supply zone: distal_line (top) > proximal_line (bottom)
            if current_price > distal_line + proximity_threshold {
                ZoneStatus::Breached
            } else if current_price > distal_line - proximity_threshold
                && current_price <= distal_line + proximity_threshold
            {
                ZoneStatus::AtDistal
            } else if current_price > extended_proximal_line && current_price <= distal_line {
                ZoneStatus::InsideZone
            } else if current_price >= extended_proximal_line {
                ZoneStatus::AtProximal // Price has reached/crossed the extended proximal line (zone entry)
            } else {
                // Check distance to determine if truly approaching
                let distance_to_proximal = (current_price - proximal_line).abs() / pip_value;
                if distance_to_proximal < 10.0 {
                    ZoneStatus::Approaching
                } else {
                    ZoneStatus::FreshZone // Too far to be approaching
                }
            }
        } else {
            // Demand zone: proximal_line (top) > distal_line (bottom)
            if current_price < distal_line - proximity_threshold {
                ZoneStatus::Breached
            } else if current_price < distal_line + proximity_threshold
                && current_price >= distal_line - proximity_threshold
            {
                ZoneStatus::AtDistal
            } else if current_price < extended_proximal_line && current_price >= distal_line {
                ZoneStatus::InsideZone
            } else if current_price <= extended_proximal_line {
                ZoneStatus::AtProximal // Price has reached/crossed the extended proximal line (zone entry)
            } else {
                // Check distance to determine if truly approaching
                let distance_to_proximal = (proximal_line - current_price).abs() / pip_value;
                if distance_to_proximal < 10.0 {
                    ZoneStatus::Approaching
                } else {
                    ZoneStatus::FreshZone // Too far to be approaching
                }
            }
        }
    }

    fn extract_zone_distance_info(
        &self,
        zone_json: &Value,
        current_prices: &HashMap<String, f64>,
    ) -> Result<ZoneDistanceInfo, Box<dyn std::error::Error>> {
        let zone_id = zone_json
            .get("zone_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let symbol = zone_json
            .get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();

        let timeframe = zone_json
            .get("timeframe")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();

        let zone_type = zone_json
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let zone_high = zone_json
            .get("zone_high")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let zone_low = zone_json
            .get("zone_low")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let touch_count = zone_json
            .get("touch_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        let strength_score = zone_json
            .get("strength_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        // Try to get start_time first (preferred), fallback to created_at for backward compatibility
        let created_at = zone_json
            .get("start_time")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| {
                // Fallback to created_at for backward compatibility
                zone_json
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
            });

        let current_price = current_prices
            .get(&symbol)
            .copied()
            .or_else(|| {
                if zone_high > 0.0 && zone_low > 0.0 {
                    Some((zone_high + zone_low) / 2.0)
                } else {
                    None
                }
            })
            .ok_or_else(|| format!("No price data for symbol: {}", symbol))?;

        if zone_high <= 0.0 || zone_low <= 0.0 || zone_high <= zone_low {
            return Err(format!("Invalid zone boundaries").into());
        }

        // FIXED: Determine zone type and lines correctly (matching zone_interactions.rs)
        let is_supply = zone_type.contains("supply") || zone_type.contains("resistance");
        let (proximal_line, distal_line) = if is_supply {
            // Supply zone: proximal = low (entry from below), distal = high (break above)
            (zone_low, zone_high)
        } else {
            // Demand zone: proximal = high (entry from above), distal = low (break below)
            (zone_high, zone_low)
        };

        let pip_value = self.pip_values.get(&symbol).cloned().unwrap_or(0.0001);

        // Calculate signed distance to proximal line
        // Positive = inside zone (past proximal), Negative = outside zone (before proximal)
        let signed_distance_pips = if is_supply {
            // Supply zone: proximal = low, price enters from below
            (current_price - proximal_line) / pip_value // + when above proximal (inside), - when below (outside)
        } else {
            // Demand zone: proximal = high, price enters from above
            (proximal_line - current_price) / pip_value // + when below proximal (inside), - when above (outside)
        };

        let distance_to_proximal = signed_distance_pips.abs();

        let zone_status = self.calculate_zone_status(
            current_price,
            proximal_line,
            distal_line,
            is_supply,
            pip_value,
        );

        if distance_to_proximal.is_nan() || distance_to_proximal.is_infinite() {
            return Err("Invalid distance calculation".into());
        }

        // Get zone interaction data if available
        let interaction_metrics = self
            .zone_interactions
            .as_ref()
            .and_then(|interactions| interactions.metrics.get(&zone_id));

        let (
            has_ever_entered,
            total_time_inside_seconds,
            zone_entries,
            last_crossing_time,
            is_zone_active,
        ) = if let Some(metrics) = interaction_metrics {
            (
                metrics.has_ever_entered,
                metrics.total_time_inside_seconds,
                metrics.zone_entries,
                metrics
                    .last_crossing_time
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                metrics.is_zone_active,
            )
        } else {
            (false, 0, 0, None, true)
        };

        // Enhance zone status based on interaction history
        let enhanced_zone_status = self.determine_enhanced_zone_status(
            &zone_status,
            has_ever_entered,
            last_crossing_time,
            zone_entries,
            signed_distance_pips,
        );

        Ok(ZoneDistanceInfo {
            zone_id,
            symbol,
            timeframe,
            zone_type,
            current_price,
            proximal_line,
            distal_line,
            signed_distance_pips,
            distance_pips: distance_to_proximal,
            zone_status: enhanced_zone_status,
            last_update: Utc::now(),
            touch_count,
            strength_score,
            created_at,

            // Zone interaction metrics
            has_ever_entered,
            total_time_inside_seconds,
            zone_entries,
            last_crossing_time,
            is_zone_active,
        })
    }

    fn determine_enhanced_zone_status(
        &self,
        base_status: &ZoneStatus,
        has_ever_entered: bool,
        last_crossing_time: Option<DateTime<Utc>>,
        zone_entries: u32,
        signed_distance_pips: f64,
    ) -> ZoneStatus {
        // CRITICAL: Match zone_monitor logic exactly - check distance threshold AND first-time entry
        let is_first_time_entry = !has_ever_entered && zone_entries == 0;
        let distance_pips = signed_distance_pips.abs();

        // Zone can only trigger if BOTH conditions are met (matching zone_monitor):
        // 1. distance_pips <= trading_threshold_pips
        // 2. is_first_time_entry = true
        let can_trigger = distance_pips <= self.trading_threshold_pips && is_first_time_entry;

        // For zones that are at proximal or inside, apply the exact trigger logic
        if matches!(base_status, ZoneStatus::AtProximal | ZoneStatus::InsideZone) {
            if can_trigger {
                // Both conditions met - show as TRIGGER
                return ZoneStatus::AtProximal;
            } else if !is_first_time_entry {
                // Zone has been entered before - show as recently tested
                return ZoneStatus::RecentlyTested;
            } else {
                // First time but distance too far - show as inside/approaching
                return if matches!(base_status, ZoneStatus::InsideZone) {
                    ZoneStatus::InsideZone
                } else {
                    ZoneStatus::Approaching
                };
            }
        }

        // For other statuses, enhance based on interaction history
        match base_status {
            ZoneStatus::Approaching | ZoneStatus::AtDistal => {
                // PRIORITY 1: Check for recent activity (within last hour) AND reasonable distance
                if let Some(last_crossing) = last_crossing_time {
                    let now = Utc::now();
                    let duration = now.signed_duration_since(last_crossing);
                    // Only mark as recently tested if within last hour AND within reasonable distance (< 10 pips)
                    if duration.num_hours() < 1 && signed_distance_pips.abs() < 10.0 {
                        return ZoneStatus::RecentlyTested;
                    }
                }

                // PRIORITY 2: Current proximity takes precedence - if close to zone, show base status
                if signed_distance_pips.abs() < 10.0 {
                    return base_status.clone(); // Show APPROACHING/AT_DISTAL regardless of history
                }

                // PRIORITY 3: Check if zone has never been entered (only for distant zones)
                if !has_ever_entered && zone_entries == 0 {
                    return ZoneStatus::FreshZone;
                }

                // PRIORITY 4: Mark as weak only if distant AND heavily tested
                if zone_entries > 10 {
                    return ZoneStatus::WeakZone;
                }

                // Otherwise use base status
                base_status.clone()
            }
            ZoneStatus::Breached => ZoneStatus::Breached,
            _ => base_status.clone(),
        }
    }

    pub async fn load_pending_orders(&mut self) {
        self.pending_orders = HashMap::new();

        if self.ensure_redis_available().await.is_err() {
            log::debug!("📋 Dashboard Redis unavailable, no pending orders loaded");
            return;
        }

        if let Some(conn_arc) = &self.redis_connection {
            if let Ok(mut conn) = conn_arc.lock() {
                match conn.keys::<&str, Vec<String>>("pending_order:*") {
                    Ok(keys) => {
                        let mut loaded_count = 0;

                        for key in keys {
                            if let Some(zone_id) = key.strip_prefix("pending_order:") {
                                match conn.get::<&str, String>(&key) {
                                    Ok(order_json) => {
                                        match serde_json::from_str::<PendingOrder>(&order_json) {
                                            Ok(order) => {
                                                self.pending_orders
                                                    .insert(zone_id.to_string(), order);
                                                loaded_count += 1;
                                            }
                                            Err(e) => {
                                                log::warn!("📋 Dashboard failed to parse pending order {}: {}", zone_id, e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "📋 Dashboard failed to get pending order {}: {}",
                                            zone_id,
                                            e
                                        );
                                    }
                                }
                            }
                        }

                        if loaded_count > 0 {
                            log::info!(
                                "📋 Dashboard loaded {} pending orders from Redis",
                                loaded_count
                            );
                        } else {
                            log::debug!("📋 Dashboard found no pending orders in Redis");
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "📋 Dashboard failed to get pending order keys from Redis: {}",
                            e
                        );
                    }
                }
            } else {
                log::warn!("📋 Dashboard failed to acquire Redis lock for pending orders");
            }
        }
    }

    pub async fn get_redis_stats(&self) -> (usize, usize) {
        if self.ensure_redis_available().await.is_err() {
            return (0, 0);
        }

        if let Some(conn_arc) = &self.redis_connection {
            if let Ok(mut conn) = conn_arc.lock() {
                let pending_orders: Result<Vec<String>, _> = conn.keys("pending_order:*");
                let lookups: Result<Vec<String>, _> = conn.keys("order_lookup:*");

                return (
                    pending_orders.map(|v| v.len()).unwrap_or(0),
                    lookups.map(|v| v.len()).unwrap_or(0),
                );
            }
        }
        (0, 0)
    }

    pub fn is_redis_connected(&self) -> bool {
        self.redis_connection.is_some()
    }

    /// Check if a zone has a pending order
    pub fn has_pending_order(&self, zone_id: &str) -> bool {
        self.pending_orders.contains_key(zone_id)
    }

    /// Get pending order info for a zone
    pub fn get_pending_order(&self, zone_id: &str) -> Option<&PendingOrder> {
        self.pending_orders.get(zone_id)
    }
}
