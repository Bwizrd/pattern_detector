// src/bin/dashboard.rs - Back to simple working version
use std::collections::HashMap;
use std::env;
use tokio::time::{interval, Duration};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;
use crossterm::{
    execute,
    terminal::{Clear, ClearType, SetTitle},
    cursor::{Hide, Show},
    style::{Color, Print, ResetColor, SetForegroundColor},
};
use std::io::{self, Write};

#[derive(Debug, Clone)]
pub struct ZoneDistanceInfo {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub zone_type: String,
    pub current_price: f64,
    pub proximal_line: f64,
    pub distal_line: f64,
    pub distance_pips: f64,
    pub last_update: DateTime<Utc>,
    pub touch_count: i32,
    pub strength_score: f64,
}

pub struct StandaloneDashboard {
    client: Client,
    api_base_url: String,
    pip_values: HashMap<String, f64>,
    update_interval: Duration,
}

impl StandaloneDashboard {
    pub fn new() -> Self {
        let mut pip_values = HashMap::new();
        
        // Define pip values for major currency pairs
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
        
        // Indices (larger pip values)
        pip_values.insert("NAS100".to_string(), 1.0);
        pip_values.insert("US500".to_string(), 0.1);

        let api_base_url = env::var("API_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());

        let update_interval_secs = env::var("DASHBOARD_UPDATE_INTERVAL")
            .unwrap_or_else(|_| "1".to_string())
            .parse::<u64>()
            .unwrap_or(1);

        Self {
            client: Client::new(),
            api_base_url,
            pip_values,
            update_interval: Duration::from_secs(update_interval_secs),
        }
    }

    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Setup terminal with crossterm
        let mut stdout_handle = io::stdout();
        execute!(stdout_handle, Hide, Clear(ClearType::All), SetTitle("Zone Dashboard"))?;
        
        // Show startup message
        println!("🎯 Real-time Zone Dashboard - Standalone Mode");
        println!("Connected to: {}", self.api_base_url);
        println!("Press Ctrl+C to exit\n");
        println!("Starting in 3 seconds...");
        
        tokio::time::sleep(Duration::from_millis(3000)).await;

        let mut ticker = interval(self.update_interval);
        
        // Setup Ctrl+C handler
        let _ = ctrlc::set_handler(move || {
            let mut stdout_handle = io::stdout();
            let _ = execute!(stdout_handle, Show, Clear(ClearType::All));
            println!("👋 Dashboard stopped.");
            std::process::exit(0);
        });
        
        loop {
            ticker.tick().await;
            
            match self.update_dashboard().await {
                Ok(_) => {},
                Err(e) => {
                    execute!(stdout_handle, Clear(ClearType::All))?;
                    println!("❌ Dashboard Error: {}", e);
                    println!("Retrying in {} seconds...", self.update_interval.as_secs());
                    println!("\nIs the main application running on {}?", self.api_base_url);
                }
            }
        }
    }

    async fn update_dashboard(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout_handle = io::stdout();
        
        // Clear screen and reset cursor
        execute!(stdout_handle, Clear(ClearType::All))?;
        
        let now = Utc::now();
        
        // Get zones from API
        let zones_json = self.get_zones_from_api().await?;
        
        if zones_json.is_empty() {
            execute!(stdout_handle, 
                SetForegroundColor(Color::Yellow),
                Print("📊 Real-time Zone Dashboard - "),
                Print(now.format("%H:%M:%S").to_string()),
                Print("\n══════════════════════════════════════════════════════════════\n"),
                Print("⚠️  No zones available from API\n"),
                SetForegroundColor(Color::White),
                Print(format!("   Check if main application is running on {}\n", self.api_base_url)),
                ResetColor
            )?;
            return Ok(());
        }

        // Get current prices
        let current_prices = self.get_current_prices().await.unwrap_or_default();

        // Extract zone distance info
        let mut zone_distances: Vec<ZoneDistanceInfo> = Vec::new();
        let zones_count = zones_json.len();
        
        for zone_json in &zones_json {
            if let Ok(zone_info) = self.extract_zone_distance_info(zone_json, &current_prices).await {
                zone_distances.push(zone_info);
            }
        }

        // Filter and sort
        zone_distances.retain(|z| z.current_price > 0.0 && z.distance_pips >= 0.0 && z.distance_pips < 10000.0);
        zone_distances.sort_by(|a, b| a.distance_pips.partial_cmp(&b.distance_pips).unwrap_or(std::cmp::Ordering::Equal));

        // Display header
        execute!(stdout_handle,
            SetForegroundColor(Color::Cyan),
            Print("💡 Legend: DIST=Distance in pips, TCHES=Touch count, STRTH=Strength score\n"),
            Print("🔴 <5 pips (URGENT) | 🟡 <10 pips (CLOSE) | 🟢 <25 pips (WATCH)\n"),
            SetForegroundColor(Color::White),
            Print(format!("🎯 Real-time Zone Dashboard - {} | 🔗 Connected to {}\n", 
                         now.format("%H:%M:%S"), self.api_base_url)),
            Print("══════════════════════════════════════════════════════════════\n"),
            Print(format!("{:<15} {:<8} {:<6} {:<11} {:<11} {:<11} {:<6} {:<6}\n", 
                         "SYMBOL/TF", "TYPE", "DIST", "PRICE", "PROXIMAL", "DISTAL", "TCHES", "STRTH")),
            Print("──────────────────────────────────────────────────────────────\n"),
            ResetColor
        )?;

        let display_count = zone_distances.len().min(20);
        
        if display_count == 0 {
            execute!(stdout_handle,
                SetForegroundColor(Color::Yellow),
                Print("⚠️  No zones with valid data available\n\n"),
                Print("🔍 Debug Info:\n"),
                SetForegroundColor(Color::White),
                Print(format!("   Total zones from API: {}\n", zones_count)),
                Print(format!("   Zones after filtering: {}\n", zone_distances.len())),
                ResetColor
            )?;
        } else {
            for zone in zone_distances.iter().take(display_count) {
                let color = if zone.distance_pips < 5.0 {
                    Color::Red
                } else if zone.distance_pips < 10.0 {
                    Color::Yellow
                } else if zone.distance_pips < 25.0 {
                    Color::Green
                } else {
                    Color::White
                };
                
                let symbol_tf = format!("{}/{}", zone.symbol, zone.timeframe);
                let zone_type_short = if zone.zone_type.contains("supply") { "SELL" } else { "BUY " };
                
                execute!(stdout_handle,
                    SetForegroundColor(color),
                    Print(format!("{:<15} {:<8} {:<6.1} {:<11.5} {:<11.5} {:<11.5} {:<6} {:<6.0}\n",
                                 symbol_tf, zone_type_short, zone.distance_pips,
                                 zone.current_price, zone.proximal_line, zone.distal_line,
                                 zone.touch_count, zone.strength_score)),
                    ResetColor
                )?;
            }
        }

        // Summary stats
        let very_close = zone_distances.iter().filter(|z| z.distance_pips < 5.0).count();
        let close = zone_distances.iter().filter(|z| z.distance_pips < 10.0).count();
        let medium = zone_distances.iter().filter(|z| z.distance_pips < 25.0).count();
        
        execute!(stdout_handle,
            Print("──────────────────────────────────────────────────────────────\n"),
            SetForegroundColor(Color::Cyan),
            Print(format!("📊 Total: {} | 🔴 <5 pips: {} | 🟡 <10 pips: {} | 🟢 <25 pips: {} | ⏱️  Updated: {}\n", 
                         zone_distances.len(), very_close, close, medium, now.format("%H:%M:%S"))),
            ResetColor
        )?;

        Ok(())
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
                if let Some(zones_array) = obj.get("retrieved_zones") {
                    if let Value::Array(zones) = zones_array {
                        Ok(zones.clone())
                    } else {
                        Err("'retrieved_zones' field is not an array".into())
                    }
                } else if let Some(zones_array) = obj.get("zones") {
                    if let Value::Array(zones) = zones_array {
                        Ok(zones.clone())
                    } else {
                        Err("'zones' field is not an array".into())
                    }
                } else if let Some(data_array) = obj.get("data") {
                    if let Value::Array(zones) = data_array {
                        Ok(zones.clone())
                    } else {
                        Err("'data' field is not an array".into())
                    }
                } else {
                    let keys: Vec<_> = obj.keys().collect();
                    Err(format!("Unexpected object structure. Available keys: {:?}", keys).into())
                }
            }
            _ => {
                Err(format!("Unexpected response type").into())
            }
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
            return Err(format!("Invalid zone boundaries").into());
        }
        
        let is_supply = zone_type.contains("supply");
        let (proximal_line, distal_line) = if is_supply {
            (zone_low, zone_high)
        } else {
            (zone_high, zone_low)
        };

        let price_distance = (current_price - proximal_line).abs();
        let pip_value = self.pip_values.get(&symbol).cloned().unwrap_or(0.0001);
        let distance_pips = price_distance / pip_value;

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
            distance_pips,
            last_update: Utc::now(),
            touch_count,
            strength_score,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let dashboard = StandaloneDashboard::new();
    dashboard.start().await?;

    Ok(())
}