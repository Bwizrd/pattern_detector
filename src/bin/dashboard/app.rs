// src/bin/dashboard/app.rs - Complete app state and business logic with independent timeframe filters
use chrono::{DateTime, Utc};
use log;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::process::Command;
use tokio::fs;
use tokio::time::{Duration, Instant}; // Add this line with your other imports

use crate::types::{
    AppPage, TradeNotificationDisplay, ValidatedTradeDisplay, ZoneDistanceInfo, ZoneStatus,
};

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
    pub client: Client,
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
}

impl App {
    pub fn new() -> Self {
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

        Self {
            client: Client::new(),
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

    // CHANGED: Update is_timeframe_enabled to use the current page's filters
    pub fn is_timeframe_enabled(&self, timeframe: &str) -> bool {
        let filters = match self.current_page {
            AppPage::Dashboard => &self.dashboard_timeframe_filters,
            AppPage::NotificationMonitor => &self.notification_timeframe_filters,
            AppPage::Prices => return true, // All timeframes enabled for prices page
        };

        filters.get(timeframe).copied().unwrap_or(true)
    }

    pub fn select_next_zone(&mut self) {
        if !self.zones.is_empty() {
            match self.selected_zone_index {
                Some(current_index) => {
                    let next_index = (current_index + 1) % self.zones.len();
                    self.selected_zone_index = Some(next_index);
                }
                None => {
                    self.selected_zone_index = Some(0);
                }
            }
        }
    }

    pub fn select_previous_zone(&mut self) {
        if !self.zones.is_empty() {
            match self.selected_zone_index {
                Some(current_index) => {
                    let prev_index = if current_index == 0 {
                        self.zones.len() - 1
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

    // NEW: Copy selected zone ID from dashboard
    pub fn copy_selected_dashboard_zone_id(&mut self) {
        if let Some(index) = self.selected_zone_index {
            if let Some(zone) = self.zones.get(index) {
                // Try to copy to system clipboard
                match self.copy_to_clipboard(&zone.zone_id) {
                    Ok(()) => {
                        self.last_copied_zone_id = Some(format!("✅ Copied: {}", zone.zone_id));
                    }
                    Err(_) => {
                        // Fallback: just show the zone ID
                        self.last_copied_zone_id = Some(format!("Zone ID: {}", zone.zone_id));
                    }
                }
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

    pub async fn update_data(&mut self) {
        self.error_message = None;

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

        // CHANGED: Use WebSocket prices instead of API prices
        let current_prices = self.convert_websocket_to_simple_prices();

        // Rest of your existing code...
        let notifications_response = self.fetch_trade_notifications_from_api().await?;
        let validated_signals_response = self.fetch_validated_signals_from_api().await?;

        // Process all data with WebSocket prices
        self.zones = self.process_zones_response(zones_response, current_prices)?;
        self.all_notifications = self.process_api_notifications_response(notifications_response)?;
        self.validated_trades =
            self.process_validated_signals_response(validated_signals_response)?;
        self.placed_trades = Vec::new();

        Ok(())
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

                    zone_distances.push(zone_info);
                }
            }
        }

        // Keep only valid zones
        zone_distances.retain(|z| {
            z.current_price > 0.0 && z.distance_pips >= 0.0 && z.distance_pips < 10000.0
        });

        // FIXED: Better sorting - prioritize by status, then by distance
        zone_distances.sort_by(|a, b| {
            use std::cmp::Ordering;

            // First, sort by status priority
            let status_priority = |status: &ZoneStatus| -> u8 {
                match status {
                    ZoneStatus::AtProximal => 1,  // Highest priority
                    ZoneStatus::InsideZone => 2,  // Second priority
                    ZoneStatus::AtDistal => 3,    // Third priority
                    ZoneStatus::Approaching => 4, // Fourth priority
                    ZoneStatus::Breached => 5,    // Lowest priority
                }
            };

            let a_priority = status_priority(&a.zone_status);
            let b_priority = status_priority(&b.zone_status);

            match a_priority.cmp(&b_priority) {
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
                                        "created_at": zone.get("created_at").unwrap_or(&Value::Null)
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

        if is_supply {
            // Supply zone: distal_line (top) > proximal_line (bottom)
            if current_price > distal_line + proximity_threshold {
                ZoneStatus::Breached
            } else if current_price > distal_line - proximity_threshold
                && current_price <= distal_line + proximity_threshold
            {
                ZoneStatus::AtDistal
            } else if current_price > proximal_line && current_price <= distal_line {
                ZoneStatus::InsideZone
            } else if current_price > proximal_line - proximity_threshold
                && current_price <= proximal_line + proximity_threshold
            {
                ZoneStatus::AtProximal
            } else {
                ZoneStatus::Approaching
            }
        } else {
            // Demand zone: proximal_line (top) > distal_line (bottom)
            if current_price < distal_line - proximity_threshold {
                ZoneStatus::Breached
            } else if current_price < distal_line + proximity_threshold
                && current_price >= distal_line - proximity_threshold
            {
                ZoneStatus::AtDistal
            } else if current_price < proximal_line && current_price >= distal_line {
                ZoneStatus::InsideZone
            } else if current_price < proximal_line + proximity_threshold
                && current_price >= proximal_line - proximity_threshold
            {
                ZoneStatus::AtProximal
            } else {
                ZoneStatus::Approaching
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

        // FIXED: Determine zone type and lines correctly
        let is_supply = zone_type.contains("supply") || zone_type.contains("resistance");
        let (proximal_line, distal_line) = if is_supply {
            // Supply zone: price approaches from below, so proximal is the bottom (zone_low)
            (zone_low, zone_high)
        } else {
            // Demand zone: price approaches from above, so proximal is the top (zone_high)
            (zone_high, zone_low)
        };

        let pip_value = self.pip_values.get(&symbol).cloned().unwrap_or(0.0001);

        // FIXED: Calculate distance to closest edge of zone
        let distance_to_proximal = (current_price - proximal_line).abs() / pip_value;
        let distance_to_distal = (current_price - distal_line).abs() / pip_value;

        // Use the smaller distance (closest edge)
        let distance_pips = distance_to_proximal.min(distance_to_distal);

        // FIXED: Calculate signed distance properly
        let signed_distance_pips = if is_supply {
            // For supply zones, negative means price is below zone (approaching)
            if current_price < zone_low {
                -(zone_low - current_price) / pip_value // Negative = approaching
            } else if current_price > zone_high {
                (current_price - zone_high) / pip_value // Positive = breached
            } else {
                0.0 // Inside zone
            }
        } else {
            // For demand zones, negative means price is above zone (approaching)
            if current_price > zone_high {
                -(current_price - zone_high) / pip_value // Negative = approaching
            } else if current_price < zone_low {
                (zone_low - current_price) / pip_value // Positive = breached
            } else {
                0.0 // Inside zone
            }
        };

        let zone_status = self.calculate_zone_status(
            current_price,
            proximal_line,
            distal_line,
            is_supply,
            pip_value,
        );

        if distance_pips.is_nan() || distance_pips.is_infinite() {
            return Err("Invalid distance calculation".into());
        }

        Ok(ZoneDistanceInfo {
            zone_id,
            symbol,
            timeframe,
            zone_type,
            current_price,
            proximal_line,
            distal_line,
            signed_distance_pips,
            distance_pips,
            zone_status,
            last_update: Utc::now(),
            touch_count,
            strength_score,
        })
    }
}
