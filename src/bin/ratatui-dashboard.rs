// src/bin/ratatui_dashboard.rs - Advanced TUI dashboard with ratatui
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
    backend::{Backend, CrosstermBackend},
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
    last_update: Instant,
    error_message: Option<String>,
    update_count: u64,
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

        Self {
            client: Client::new(),
            api_base_url,
            pip_values,
            zones: Vec::new(),
            last_update: Instant::now(),
            error_message: None,
            update_count: 0,
        }
    }

    async fn update_data(&mut self) {
        self.error_message = None;
        
        match self.fetch_zones().await {
            Ok(zones) => {
                self.zones = zones;
                self.last_update = Instant::now();
                self.update_count += 1;
            }
            Err(e) => {
                self.error_message = Some(format!("Error: {}", e));
            }
        }
    }

    async fn fetch_zones(&self) -> Result<Vec<ZoneDistanceInfo>, Box<dyn std::error::Error>> {
        let zones_json = self.get_zones_from_api().await?;
        let current_prices = self.get_current_prices().await.unwrap_or_default();
        
        let mut zone_distances = Vec::new();
        
        for zone_json in &zones_json {
            if let Ok(zone_info) = self.extract_zone_distance_info(zone_json, &current_prices).await {
                zone_distances.push(zone_info);
            }
        }

        zone_distances.retain(|z| z.current_price > 0.0 && z.distance_pips >= 0.0 && z.distance_pips < 10000.0);
        
        zone_distances.sort_by(|a, b| {
            match (&a.zone_status, &b.zone_status) {
                (ZoneStatus::AtProximal, ZoneStatus::AtProximal) => a.distance_pips.partial_cmp(&b.distance_pips).unwrap_or(std::cmp::Ordering::Equal),
                (ZoneStatus::AtProximal, _) => std::cmp::Ordering::Less,
                (_, ZoneStatus::AtProximal) => std::cmp::Ordering::Greater,
                _ => a.distance_pips.partial_cmp(&b.distance_pips).unwrap_or(std::cmp::Ordering::Equal),
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
            if current_price >= distal_line {
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
            if current_price <= distal_line {
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

    async fn extract_zone_distance_info(
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
            return Err("Invalid zone boundaries".into());
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
}

fn ui(f: &mut Frame, app: &App) {
    let size = f.size();
    
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(size);

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

    f.render_widget(header, chunks[0]);

    let (total, triggers, inside, close, watch) = app.get_stats();
    
    let stats_block = Block::default()
        .borders(Borders::ALL)
        .title("📊 Live Stats")
        .border_style(Style::default().fg(Color::Green));

    let stats_text = if triggers > 0 {
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

    let stats = Paragraph::new(stats_text)
        .block(stats_block)
        .alignment(Alignment::Center);

    f.render_widget(stats, chunks[1]);

    if let Some(error) = &app.error_message {
        let error_block = Block::default()
            .borders(Borders::ALL)
            .title("❌ Error")
            .border_style(Style::default().fg(Color::Red));

        let error_widget = Paragraph::new(error.as_str())
            .block(error_block)
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });

        f.render_widget(error_widget, chunks[2]);
    } else {
        let table_block = Block::default()
            .borders(Borders::ALL)
            .title("🎯 Active Zones")
            .border_style(Style::default().fg(Color::Blue));

        let header_cells = ["Symbol/TF", "Type", "Signed Dist", "Status", "Price", "Proximal", "Distal", "Strength"]
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
                Constraint::Length(12),
                Constraint::Length(6),
                Constraint::Length(10),
                Constraint::Length(12),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(8),
            ]);

        f.render_widget(table, chunks[2]);
    }

    let help_block = Block::default()
        .borders(Borders::ALL)
        .title("🔧 Controls")
        .border_style(Style::default().fg(Color::Gray));

    let help_text = "Press 'q' to quit | 'r' to refresh | Arrow keys to navigate | Updates every 1 second";
    let help = Paragraph::new(help_text)
        .block(help_block)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);

    f.render_widget(help, chunks[3]);

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