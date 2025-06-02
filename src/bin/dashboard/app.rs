// src/bin/dashboard/app.rs - Complete app state and business logic with independent timeframe filters
use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::io::Write;
use tokio::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;
use log;

use crate::types::{
    AppPage, ZoneDistanceInfo, TradeNotificationDisplay, ValidatedTradeDisplay, ZoneStatus
};

pub struct App {
    pub client: Client,
    pub api_base_url: String,
    pub pip_values: HashMap<String, f64>,
    pub current_page: AppPage,
    
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

        let api_base_url = env::var("API_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());

        // CHANGED: Create separate timeframe filters for each page
        let mut dashboard_timeframe_filters = HashMap::new();
        dashboard_timeframe_filters.insert("5m".to_string(), false);
        dashboard_timeframe_filters.insert("15m".to_string(), false);
        dashboard_timeframe_filters.insert("30m".to_string(), true);
        dashboard_timeframe_filters.insert("1h".to_string(), true);
        dashboard_timeframe_filters.insert("4h".to_string(), true);
        dashboard_timeframe_filters.insert("1d".to_string(), true);

        let mut notification_timeframe_filters = HashMap::new();
        notification_timeframe_filters.insert("5m".to_string(), true);  // Different defaults for notifications
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
        }
    }

    pub fn switch_page(&mut self, page: AppPage) {
        // Reset selection when switching to dashboard
        match page {
            AppPage::Dashboard => {
                self.selected_notification_index = None;
            }
            AppPage::NotificationMonitor => {
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

    // Navigation methods for notifications
    pub fn select_next_notification(&mut self) {
        let filtered_notifications: Vec<_> = self.all_notifications.iter()
            .enumerate()
            .filter(|(_, notif)| self.is_timeframe_enabled(&notif.timeframe))
            .collect();

        if !filtered_notifications.is_empty() {
            match self.selected_notification_index {
                Some(current_index) => {
                    // Find current position in filtered list
                    if let Some(current_pos) = filtered_notifications.iter().position(|(idx, _)| *idx == current_index) {
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
    }

    pub fn select_previous_notification(&mut self) {
        let filtered_notifications: Vec<_> = self.all_notifications.iter()
            .enumerate()
            .filter(|(_, notif)| self.is_timeframe_enabled(&notif.timeframe))
            .collect();

        if !filtered_notifications.is_empty() {
            match self.selected_notification_index {
                Some(current_index) => {
                    // Find current position in filtered list
                    if let Some(current_pos) = filtered_notifications.iter().position(|(idx, _)| *idx == current_index) {
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
        // Fetch zones with current prices
        let zones_response = self.get_zones_from_api().await?;
        let current_prices = self.get_current_prices().await.unwrap_or_default();
        
        // Fetch all notifications for notification monitor
        let notifications_response = self.fetch_trade_notifications_from_api().await?;
        
        // Fetch validated signals for both dashboard and notification monitor
        let validated_signals_response = self.fetch_validated_signals_from_api().await?;
        
        // Process all data
        self.zones = self.process_zones_response(zones_response, current_prices)?;
        self.all_notifications = self.process_api_notifications_response(notifications_response)?;
        self.validated_trades = self.process_validated_signals_response(validated_signals_response)?;
        
        // For dashboard: placed_trades is empty until we implement order placement
        self.placed_trades = Vec::new(); // TODO: Replace with actual placed trades
        
        Ok(())
    }

    async fn fetch_validated_signals_from_api(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let url = format!("{}/validated-signals", self.api_base_url);
        
        let response = self.client
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

    fn process_validated_signals_response(&self, response: Value) -> Result<Vec<ValidatedTradeDisplay>, Box<dyn std::error::Error>> {
        let empty_vec = vec![];
        let signals_array = response.get("signals")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);

        let mut trades = Vec::new();

        for signal_json in signals_array.iter().take(50) {
            let timestamp_str = signal_json.get("validation_timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            
            let timestamp = DateTime::parse_from_rfc3339(timestamp_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let entry_price = signal_json.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let direction = signal_json.get("action").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let symbol = signal_json.get("symbol").and_then(|v| v.as_str()).unwrap_or("").to_string();
            
            // Get pip value for this symbol
            let pip_value = self.pip_values.get(&symbol).cloned().unwrap_or(0.0001);
            
            // Hardcoded SL and TP for now (50 pip SL, 100 pip TP)
            let (stop_loss, take_profit) = if direction == "BUY" {
                (entry_price - (50.0 * pip_value), entry_price + (100.0 * pip_value))
            } else {
                (entry_price + (50.0 * pip_value), entry_price - (100.0 * pip_value))
            };

            let trade = ValidatedTradeDisplay {
                timestamp,
                symbol,
                timeframe: signal_json.get("timeframe").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                direction,
                entry_price,
                stop_loss,
                take_profit,
                signal_id: signal_json.get("signal_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            };

            trades.push(trade);
        }

        Ok(trades)
    }

    async fn fetch_trade_notifications_from_api(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let url = format!("{}/trade-notifications", self.api_base_url);
        
        let response = self.client
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

    fn process_api_notifications_response(&self, response: Value) -> Result<Vec<TradeNotificationDisplay>, Box<dyn std::error::Error>> {
        let empty_vec = vec![];
        let notifications_array = response.get("notifications")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);

        let mut notifications = Vec::new();

        for notif_json in notifications_array.iter().take(50) {
            let timestamp_str = notif_json.get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            
            let timestamp = DateTime::parse_from_rfc3339(timestamp_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let notification = TradeNotificationDisplay {
                timestamp,
                symbol: notif_json.get("symbol").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                timeframe: notif_json.get("timeframe").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                action: notif_json.get("action").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                price: notif_json.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0),
                notification_type: "zone_trigger".to_string(),
                signal_id: notif_json.get("zone_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
            };

            notifications.push(notification);
        }

        Ok(notifications)
    }

    pub async fn clear_notifications_via_api(&self) {
        let url = format!("{}/trade-notifications/clear", self.api_base_url);
        
        match self.client
            .post(&url)
            .send()
            .await
        {
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
        let triggers = self.zones.iter().filter(|z| z.zone_status == ZoneStatus::AtProximal).count();
        let inside = self.zones.iter().filter(|z| z.zone_status == ZoneStatus::InsideZone).count();
        let close = self.zones.iter().filter(|z| z.distance_pips < 10.0).count();
        let watch = self.zones.iter().filter(|z| z.distance_pips < 25.0).count();
        
        (total, triggers, inside, close, watch)
    }

    // CHANGED: Update get_timeframe_status to use the current page's filters
    pub fn get_timeframe_status(&self) -> String {
        let filters = match self.current_page {
            AppPage::Dashboard => &self.dashboard_timeframe_filters,
            AppPage::NotificationMonitor => &self.notification_timeframe_filters,
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
        if self.show_breached { "ON" } else { "OFF" }
    }

    // Zone processing methods (keep all existing methods)...
    fn process_zones_response(&self, zones_json: Vec<Value>, current_prices: HashMap<String, f64>) -> Result<Vec<ZoneDistanceInfo>, Box<dyn std::error::Error>> {
        let mut zone_distances = Vec::new();
        
        for zone_json in &zones_json {
            if let Ok(zone_info) = self.extract_zone_distance_info(zone_json, &current_prices) {
                // Filter by timeframe
                if self.is_timeframe_enabled(&zone_info.timeframe) {
                    // Filter out breached zones if disabled
                    if zone_info.zone_status == ZoneStatus::Breached && !self.show_breached {
                        continue;
                    }
                    
                    // NEW: Filter by strength (only on dashboard)
                    if matches!(self.current_page, AppPage::Dashboard) && zone_info.strength_score < self.min_strength_filter {
                        continue;
                    }
                    
                    zone_distances.push(zone_info);
                }
            }
        }

        zone_distances.retain(|z| z.current_price > 0.0 && z.distance_pips >= 0.0 && z.distance_pips < 10000.0);
        
        zone_distances.sort_by(|a, b| {
            match (&a.zone_status, &b.zone_status) {
                (ZoneStatus::AtProximal, ZoneStatus::AtProximal) => a.distance_pips.partial_cmp(&b.distance_pips).unwrap_or(std::cmp::Ordering::Equal),
                (ZoneStatus::AtProximal, _) => std::cmp::Ordering::Less,
                (_, ZoneStatus::AtProximal) => std::cmp::Ordering::Greater,
                (ZoneStatus::InsideZone, ZoneStatus::InsideZone) => a.signed_distance_pips.partial_cmp(&b.signed_distance_pips).unwrap_or(std::cmp::Ordering::Equal),
                (ZoneStatus::InsideZone, _) => std::cmp::Ordering::Less,
                (_, ZoneStatus::InsideZone) => std::cmp::Ordering::Greater,
                (ZoneStatus::Breached, ZoneStatus::Breached) => a.distance_pips.partial_cmp(&b.distance_pips).unwrap_or(std::cmp::Ordering::Equal),
                (ZoneStatus::Breached, _) => std::cmp::Ordering::Greater,
                (_, ZoneStatus::Breached) => std::cmp::Ordering::Less,
                _ => a.signed_distance_pips.partial_cmp(&b.signed_distance_pips).unwrap_or(std::cmp::Ordering::Equal),
            }
        });

        Ok(zone_distances)
    }

    async fn get_zones_from_api(&self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let url = format!("{}/debug/minimal-cache-zones", self.api_base_url);
        
        let response = self.client
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

    async fn get_current_prices(&self) -> Result<HashMap<String, f64>, Box<dyn std::error::Error>> {
        let url = format!("{}/current-prices", self.api_base_url);
        
        let response = self.client
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
            if current_price >= distal_line + proximity_threshold {
                ZoneStatus::Breached
            } else if (current_price - distal_line).abs() <= proximity_threshold {
                ZoneStatus::AtDistal
            } else if current_price > proximal_line && current_price < distal_line {
                ZoneStatus::InsideZone
            } else if (current_price - proximal_line).abs() <= proximity_threshold {
                ZoneStatus::AtProximal
            } else {
                ZoneStatus::Approaching
            }
        } else {
            if current_price <= distal_line - proximity_threshold {
                ZoneStatus::Breached
            } else if (current_price - distal_line).abs() <= proximity_threshold {
                ZoneStatus::AtDistal
            } else if current_price < proximal_line && current_price > distal_line {
                ZoneStatus::InsideZone
            } else if (current_price - proximal_line).abs() <= proximity_threshold {
                ZoneStatus::AtProximal
            } else {
                ZoneStatus::Approaching
            }
        }
    }

    fn extract_zone_distance_info(
        &self, 
        zone_json: &Value,
        current_prices: &HashMap<String, f64>
    ) -> Result<ZoneDistanceInfo, Box<dyn std::error::Error>> {
        let zone_id = zone_json.get("zone_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
            
        let symbol = zone_json.get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();
            
        let timeframe = zone_json.get("timeframe")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();
            
        let zone_type = zone_json.get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
            
        let zone_high = zone_json.get("zone_high")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
            
        let zone_low = zone_json.get("zone_low")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
            
        let touch_count = zone_json.get("touch_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
            
        let strength_score = zone_json.get("strength_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let current_price = current_prices.get(&symbol)
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
        
        let is_supply = zone_type.contains("supply");
        let (proximal_line, distal_line) = if is_supply {
            (zone_low, zone_high)
        } else {
            (zone_high, zone_low)
        };

        let pip_value = self.pip_values.get(&symbol).cloned().unwrap_or(0.0001);
        
        let signed_distance_pips = if is_supply {
            (proximal_line - current_price) / pip_value
        } else {
            (current_price - proximal_line) / pip_value
        };
        
        let distance_pips = signed_distance_pips.abs();
        
        let zone_status = self.calculate_zone_status(
            current_price, 
            proximal_line, 
            distal_line, 
            is_supply, 
            pip_value
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