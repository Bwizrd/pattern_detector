// src/bin/ratatui_dashboard.rs - Dashboard with API-based trade notifications
use std::collections::HashMap;
use std::env;
use std::io;
use tokio::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap,
    },
    Frame, Terminal,
};

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

#[derive(Debug, Clone, PartialEq)]
pub enum ZoneStatus {
    Approaching,
    AtProximal,
    InsideZone,
    AtDistal,
    Breached,
}

impl ZoneStatus {
    fn color(&self) -> Color {
        match self {
            ZoneStatus::AtProximal => Color::Magenta,
            ZoneStatus::InsideZone => Color::Red,
            ZoneStatus::AtDistal => Color::Blue,
            ZoneStatus::Breached => Color::DarkGray,
            ZoneStatus::Approaching => Color::Green,
        }
    }

    fn symbol(&self) -> &str {
        match self {
            ZoneStatus::AtProximal => "🚨",
            ZoneStatus::InsideZone => "📍",
            ZoneStatus::AtDistal => "🔵",
            ZoneStatus::Breached => "❌",
            ZoneStatus::Approaching => "👀",
        }
    }

    fn text(&self) -> &str {
        match self {
            ZoneStatus::AtProximal => "TRIGGER",
            ZoneStatus::InsideZone => "INSIDE",
            ZoneStatus::AtDistal => "AT_DISTAL",
            ZoneStatus::Breached => "BREACHED",
            ZoneStatus::Approaching => "APPROACHING",
        }
    }
}

pub struct App {
    client: Client,
    api_base_url: String,
    pip_values: HashMap<String, f64>,
    zones: Vec<ZoneDistanceInfo>,
    trade_notifications: Vec<TradeNotificationDisplay>,
    last_update: Instant,
    error_message: Option<String>,
    update_count: u64,
    timeframe_filters: HashMap<String, bool>,
    show_breached: bool,
    previous_triggers: std::collections::HashSet<String>,
}

impl App {
    pub fn new() -> Self {
        let mut pip_values = HashMap::new();
        
        pip_values.insert("EURUSD".to_string(), 0.0001);
        pip_values.insert("GBPUSD".to_string(), 0.0001);
        pip_values.insert("AUDUSD".to_string(), 0.0001);
        pip_values.insert("NZDUSD".to_string(), 0.0001);
        pip_values.insert("USDCAD".to_string(), 0.0001);
        pip_values.insert("USDCHF".to_string(), 0.0001);
        pip_values.insert("EURGBP".to_string(), 0.0001);
        pip_values.insert("EURAUD".to_string(), 0.0001);
        pip_values.insert("EURNZD".to_string(), 0.0001);
        pip_values.insert("EURJPY".to_string(), 0.01);
        pip_values.insert("GBPJPY".to_string(), 0.01);
        pip_values.insert("AUDJPY".to_string(), 0.01);
        pip_values.insert("NZDJPY".to_string(), 0.01);
        pip_values.insert("USDJPY".to_string(), 0.01);
        pip_values.insert("CADJPY".to_string(), 0.01);
        pip_values.insert("CHFJPY".to_string(), 0.01);
        pip_values.insert("AUDCAD".to_string(), 0.0001);
        pip_values.insert("AUDCHF".to_string(), 0.0001);
        pip_values.insert("AUDNZD".to_string(), 0.0001);
        pip_values.insert("CADCHF".to_string(), 0.0001);
        pip_values.insert("EURCHF".to_string(), 0.0001);
        pip_values.insert("EURCAD".to_string(), 0.0001);
        pip_values.insert("GBPAUD".to_string(), 0.0001);
        pip_values.insert("GBPCAD".to_string(), 0.0001);
        pip_values.insert("GBPCHF".to_string(), 0.0001);
        pip_values.insert("GBPNZD".to_string(), 0.0001);
        pip_values.insert("NZDCAD".to_string(), 0.0001);
        pip_values.insert("NZDCHF".to_string(), 0.0001);
        pip_values.insert("NAS100".to_string(), 1.0);
        pip_values.insert("US500".to_string(), 0.1);

        let api_base_url = env::var("API_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());

        let mut timeframe_filters = HashMap::new();
        timeframe_filters.insert("5m".to_string(), false);
        timeframe_filters.insert("15m".to_string(), false);
        timeframe_filters.insert("30m".to_string(), true);
        timeframe_filters.insert("1h".to_string(), true);
        timeframe_filters.insert("4h".to_string(), true);
        timeframe_filters.insert("1d".to_string(), true);

        Self {
            client: Client::new(),
            api_base_url,
            pip_values,
            zones: Vec::new(),
            trade_notifications: Vec::new(),
            last_update: Instant::now(),
            error_message: None,
            update_count: 0,
            timeframe_filters,
            show_breached: true,
            previous_triggers: std::collections::HashSet::new(),
        }
    }

    fn toggle_timeframe(&mut self, timeframe: &str) {
        if let Some(enabled) = self.timeframe_filters.get_mut(timeframe) {
            *enabled = !*enabled;
        }
    }

    fn toggle_breached(&mut self) {
        self.show_breached = !self.show_breached;
    }

    fn is_timeframe_enabled(&self, timeframe: &str) -> bool {
        self.timeframe_filters.get(timeframe).copied().unwrap_or(true)
    }

    async fn update_data(&mut self) {
        self.error_message = None;
        
        match self.fetch_dashboard_data().await {
            Ok(()) => {
                self.last_update = Instant::now();
                self.update_count += 1;
            }
            Err(e) => {
                self.error_message = Some(format!("Error: {}", e));
            }
        }
    }

    async fn fetch_dashboard_data(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Fetch zones with current prices (existing logic)
        let zones_response = self.get_zones_from_api().await?;
        let current_prices = self.get_current_prices().await.unwrap_or_default();
        
        // Fetch trade notifications from new API
        let notifications_response = self.fetch_trade_notifications_from_api().await?;
        
        // Process zones (existing logic)
        self.zones = self.process_zones_response(zones_response, current_prices)?;
        
        // Process notifications from API
        self.trade_notifications = self.process_api_notifications_response(notifications_response)?;
        
        Ok(())
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

        for notif_json in notifications_array.iter().take(25) {
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

    async fn clear_notifications_via_api(&self) {
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

    fn get_stats(&self) -> (usize, usize, usize, usize, usize) {
        let total = self.zones.len();
        let triggers = self.zones.iter().filter(|z| z.zone_status == ZoneStatus::AtProximal).count();
        let inside = self.zones.iter().filter(|z| z.zone_status == ZoneStatus::InsideZone).count();
        let close = self.zones.iter().filter(|z| z.distance_pips < 10.0).count();
        let watch = self.zones.iter().filter(|z| z.distance_pips < 25.0).count();
        
        (total, triggers, inside, close, watch)
    }

    fn get_timeframe_status(&self) -> String {
        let enabled_timeframes: Vec<&str> = self.timeframe_filters
            .iter()
            .filter_map(|(tf, &enabled)| if enabled { Some(tf.as_str()) } else { None })
            .collect();
        
        if enabled_timeframes.is_empty() {
            "None".to_string()
        } else {
            enabled_timeframes.join(", ")
        }
    }

    fn get_breached_status(&self) -> &str {
        if self.show_breached { "ON" } else { "OFF" }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let size = f.size();
    
    // Split into main area (80%) and right panel (20%)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(80),
            Constraint::Percentage(20),
        ])
        .split(size);

    // Left side layout
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(main_chunks[0]);

    // Right side layout
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(main_chunks[1]);

    // Header
    let header_block = Block::default()
        .borders(Borders::ALL)
        .title("🎯 Zone Trading Dashboard")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Cyan));

    let now = Utc::now();
    let elapsed = app.last_update.elapsed().as_secs();
    let header_text = format!(
        "Connected: {} | Updates: {} | Last: {}s ago | Time: {}",
        app.api_base_url,
        app.update_count,
        elapsed,
        now.format("%H:%M:%S")
    );

    let header = Paragraph::new(header_text)
        .block(header_block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White));

    f.render_widget(header, left_chunks[0]);

    // Stats and timeframe controls
    let (total, triggers, inside, close, watch) = app.get_stats();
    
    let stats_block = Block::default()
        .borders(Borders::ALL)
        .title("📊 Stats & Timeframes")
        .border_style(Style::default().fg(Color::Green));

    let stats_line = if triggers > 0 {
        Line::from(vec![
            Span::styled("Total: ", Style::default().fg(Color::White)),
            Span::styled(format!("{} ", total), Style::default().fg(Color::White)),
            Span::styled("🚨 TRIGGERS: ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} ", triggers), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled("📍 Inside: ", Style::default().fg(Color::Red)),
            Span::styled(format!("{} ", inside), Style::default().fg(Color::Red)),
            Span::styled("🔴 <10 pips: ", Style::default().fg(Color::Yellow)),
            Span::styled(format!("{} ", close), Style::default().fg(Color::Yellow)),
            Span::styled("🟢 <25 pips: ", Style::default().fg(Color::Green)),
            Span::styled(format!("{}", watch), Style::default().fg(Color::Green)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Total: ", Style::default().fg(Color::White)),
            Span::styled(format!("{} ", total), Style::default().fg(Color::White)),
            Span::styled("📍 Inside: ", Style::default().fg(Color::Red)),
            Span::styled(format!("{} ", inside), Style::default().fg(Color::Red)),
            Span::styled("🔴 <10 pips: ", Style::default().fg(Color::Yellow)),
            Span::styled(format!("{} ", close), Style::default().fg(Color::Yellow)),
            Span::styled("🟢 <25 pips: ", Style::default().fg(Color::Green)),
            Span::styled(format!("{}", watch), Style::default().fg(Color::Green)),
        ])
    };

    let timeframes_line = Line::from(vec![
        Span::styled("Timeframes: ", Style::default().fg(Color::Cyan)),
        Span::styled(format!("{} ", app.get_timeframe_status()), Style::default().fg(Color::White)),
        Span::styled("| Breached: ", Style::default().fg(Color::Cyan)),
        Span::styled(app.get_breached_status(), if app.show_breached { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
    ]);

    let controls_line = Line::from(vec![
        Span::styled("Toggle: ", Style::default().fg(Color::Gray)),
        Span::styled("[1]5m ", if app.is_timeframe_enabled("5m") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[2]15m ", if app.is_timeframe_enabled("15m") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[3]30m ", if app.is_timeframe_enabled("30m") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[4]1h ", if app.is_timeframe_enabled("1h") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[5]4h ", if app.is_timeframe_enabled("4h") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[6]1d ", if app.is_timeframe_enabled("1d") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[b]breached", if app.show_breached { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
    ]);

    let stats_text = Text::from(vec![stats_line, timeframes_line, controls_line]);

    let stats = Paragraph::new(stats_text)
        .block(stats_block)
        .alignment(Alignment::Left);

    f.render_widget(stats, left_chunks[1]);

    // Main table
    if let Some(error) = &app.error_message {
        let error_block = Block::default()
            .borders(Borders::ALL)
            .title("❌ Error")
            .border_style(Style::default().fg(Color::Red));

        let error_widget = Paragraph::new(error.as_str())
            .block(error_block)
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });

        f.render_widget(error_widget, left_chunks[2]);
    } else {
        let table_block = Block::default()
            .borders(Borders::ALL)
            .title("🎯 Active Zones")
            .border_style(Style::default().fg(Color::Blue));

        let header_cells = ["Symbol/TF", "Type", "S.Dist", "Status", "Price", "Proximal", "Distal", "Str"]
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));

        let header = Row::new(header_cells).height(1).bottom_margin(1);

        let rows: Vec<Row> = app.zones.iter().take(30).map(|zone| {
            let symbol_tf = format!("{}/{}", zone.symbol, zone.timeframe);
            let zone_type_short = if zone.zone_type.contains("supply") { "SELL" } else { "BUY" };
            let status_text = format!("{} {}", zone.zone_status.symbol(), zone.zone_status.text());
            
            let row_style = Style::default().fg(zone.zone_status.color());
            
            Row::new(vec![
                Cell::from(symbol_tf),
                Cell::from(zone_type_short),
                Cell::from(format!("{:+.1}", zone.signed_distance_pips)),
                Cell::from(status_text),
                Cell::from(format!("{:.5}", zone.current_price)),
                Cell::from(format!("{:.5}", zone.proximal_line)),
                Cell::from(format!("{:.5}", zone.distal_line)),
                Cell::from(format!("{:.0}", zone.strength_score)),
            ]).style(row_style)
        }).collect();

        let table = Table::new(rows)
            .header(header)
            .block(table_block)
            .widths(&[
                Constraint::Length(10),
                Constraint::Length(5),
                Constraint::Length(7),
                Constraint::Length(11),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(4),
            ]);

        f.render_widget(table, left_chunks[2]);
    }

    // Help
    let help_block = Block::default()
        .borders(Borders::ALL)
        .title("🔧 Controls")
        .border_style(Style::default().fg(Color::Gray));

    let help_text = "Press 'q' to quit | 'r' to refresh | '1-6' toggle timeframes | 'b' toggle breached | 'c' clear notifications";
    let help = Paragraph::new(help_text)
        .block(help_block)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);

    f.render_widget(help, left_chunks[3]);

    // Right panel - Notification header
    let notif_header_block = Block::default()
        .borders(Borders::ALL)
        .title("📢 Trade Notifications")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Magenta));

    let notif_count = app.trade_notifications.len();
    let notif_header_text = format!("Total: {} notifications", notif_count);

    let notif_header = Paragraph::new(notif_header_text)
        .block(notif_header_block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White));

    f.render_widget(notif_header, right_chunks[0]);

    // Right panel - Notifications list
    let notif_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    if app.trade_notifications.is_empty() {
        let empty_text = Paragraph::new("No trade notifications yet.\n\nTriggers will appear here when zones are touched.")
            .block(notif_block)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(empty_text, right_chunks[1]);
    } else {
        let notif_lines: Vec<Line> = app.trade_notifications.iter().take(25).map(|notif| {
            let time_str = notif.timestamp.format("%H:%M:%S").to_string();
            let action_color = if notif.action == "BUY" { Color::Green } else { Color::Red };
            
            Line::from(vec![
                Span::styled(format!("{} ", time_str), Style::default().fg(Color::Gray)),
                Span::styled("🎯 ", Style::default().fg(Color::White)),
                Span::styled(format!("{} ", notif.action), Style::default().fg(action_color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}/", notif.symbol), Style::default().fg(Color::Cyan)),
                Span::styled(format!("{}", notif.timeframe), Style::default().fg(Color::Yellow)),
            ])
        }).collect();

        let notif_text = Text::from(notif_lines);

        let notifications = Paragraph::new(notif_text)
            .block(notif_block)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });

        f.render_widget(notifications, right_chunks[1]);
    }

    // Trade alert popup (overlay)
    if triggers > 0 {
        let alert_area = Rect {
            x: size.width / 4,
            y: size.height / 4,
            width: size.width / 2,
            height: 7,
        };

        f.render_widget(Clear, alert_area);

        let alert_block = Block::default()
            .borders(Borders::ALL)
            .title("🚨 TRADE ALERT 🚨")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD));

        let trigger_zones: Vec<&ZoneDistanceInfo> = app.zones.iter()
            .filter(|z| z.zone_status == ZoneStatus::AtProximal)
            .take(3)
            .collect();

        let alert_lines: Vec<Line> = trigger_zones.iter().map(|zone| {
            let action = if zone.zone_type.contains("supply") { "SELL" } else { "BUY" };
            Line::from(vec![
                Span::styled(format!("{} ", action), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}/{} ", zone.symbol, zone.timeframe), Style::default().fg(Color::Cyan)),
                Span::styled(format!("@ {:.5}", zone.current_price), Style::default().fg(Color::Yellow)),
            ])
        }).collect();

        let alert_text = Text::from(alert_lines);

        let alert = Paragraph::new(alert_text)
            .block(alert_block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Magenta));

        f.render_widget(alert, alert_area);
    }
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> io::Result<()> {
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(1000);

    loop {
        terminal.draw(|f| ui(f, &app))?;

        let timeout = tick_rate.checked_sub(last_tick.elapsed()).unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('r') => {
                        app.update_data().await;
                    }
                    KeyCode::Char('c') => {
                        app.clear_notifications_via_api().await;
                        app.update_data().await;
                    }
                    KeyCode::Char('1') => {
                        app.toggle_timeframe("5m");
                    }
                    KeyCode::Char('2') => {
                        app.toggle_timeframe("15m");
                    }
                    KeyCode::Char('3') => {
                        app.toggle_timeframe("30m");
                    }
                    KeyCode::Char('4') => {
                        app.toggle_timeframe("1h");
                    }
                    KeyCode::Char('5') => {
                        app.toggle_timeframe("4h");
                    }
                    KeyCode::Char('6') => {
                        app.toggle_timeframe("1d");
                    }
                    KeyCode::Char('b') => {
                        app.toggle_breached();
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.update_data().await;
            last_tick = Instant::now();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.update_data().await;

    let res = run_app(&mut terminal, app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}