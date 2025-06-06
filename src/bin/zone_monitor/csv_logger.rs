// src/bin/zone_monitor/csv_logger.rs
// CSV logging for proximity alerts and trading signals

use crate::types::ZoneAlert;
use std::path::Path;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::{error, info};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct CsvLogger {
    logs_dir: String,
    proximity_file_initialized: Arc<Mutex<bool>>,
    trading_file_initialized: Arc<Mutex<bool>>,
}

impl CsvLogger {
    pub fn new() -> Self {
        Self {
            logs_dir: "logs".to_string(),
            proximity_file_initialized: Arc::new(Mutex::new(false)),
            trading_file_initialized: Arc::new(Mutex::new(false)),
        }
    }

    /// Ensure logs directory exists
    async fn ensure_logs_dir(&self) -> Result<(), Box<dyn std::error::Error>> {
        if !Path::new(&self.logs_dir).exists() {
            tokio::fs::create_dir_all(&self.logs_dir).await?;
            info!("📁 Created logs directory: {}", self.logs_dir);
        }
        Ok(())
    }

    /// Get today's date string for filename
    fn get_date_string() -> String {
        chrono::Utc::now().format("%Y-%m-%d").to_string()
    }

    /// Get proximity log filename for today
    fn get_proximity_filename(&self) -> String {
        format!("{}/proximity_{}.csv", self.logs_dir, Self::get_date_string())
    }

    /// Get trading signal log filename for today
    fn get_trading_filename(&self) -> String {
        format!("{}/trading_signal_{}.csv", self.logs_dir, Self::get_date_string())
    }

    /// Check if file needs headers (thread-safe)
    async fn ensure_headers_written(&self, filename: &str, is_trading: bool) -> Result<(), Box<dyn std::error::Error>> {
        let initialized_flag = if is_trading {
            &self.trading_file_initialized
        } else {
            &self.proximity_file_initialized
        };

        let mut initialized = initialized_flag.lock().await;
        
        if !*initialized {
            // Check if file actually exists and has content
            let file_exists = Path::new(filename).exists();
            let needs_headers = if file_exists {
                // Check if file is empty or very small (< 50 bytes means likely no headers)
                match tokio::fs::metadata(filename).await {
                    Ok(metadata) => metadata.len() < 50,
                    Err(_) => true,
                }
            } else {
                true
            };

            if needs_headers {
                if is_trading {
                    self.write_trading_headers(filename).await?;
                } else {
                    self.write_proximity_headers(filename).await?;
                }
            }
            
            *initialized = true;
        }
        
        Ok(())
    }

    /// Write proximity alert headers
    async fn write_proximity_headers(&self, filename: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(filename)
            .await?;

        let headers = "timestamp,symbol,zone_type,zone_id,timeframe,touch_count,distance_pips,strength,current_price,zone_high,zone_low\n";
        file.write_all(headers.as_bytes()).await?;
        Ok(())
    }

    /// Write trading signal headers
    async fn write_trading_headers(&self, filename: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(filename)
            .await?;

        let headers = "timestamp,symbol,zone_type,zone_id,timeframe,touch_count,distance_pips,strength,current_price,zone_high,zone_low,signal_type\n";
        file.write_all(headers.as_bytes()).await?;
        Ok(())
    }

    /// Log proximity alert to CSV
    pub async fn log_proximity_alert(&self, alert: &ZoneAlert) {
        if let Err(e) = self.ensure_logs_dir().await {
            error!("Failed to create logs directory: {}", e);
            return;
        }

        let filename = self.get_proximity_filename();
        
        // Ensure headers are written (thread-safe)
        if let Err(e) = self.ensure_headers_written(&filename, false).await {
            error!("Failed to ensure proximity headers: {}", e);
            return;
        }

        // Prepare CSV row
        let row = format!(
            "{},{},{},{},{},{},{:.1},{:.1},{:.5},{:.5},{:.5}\n",
            alert.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            alert.symbol,
            alert.zone_type,
            alert.zone_id,
            alert.timeframe,
            alert.touch_count,
            alert.distance_pips,
            alert.strength,
            alert.current_price,
            alert.zone_high,
            alert.zone_low
        );

        // Append to file
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filename)
            .await
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(row.as_bytes()).await {
                    error!("Failed to write proximity log: {}", e);
                } else {
                    info!("📝 Logged proximity: {}", filename);
                }
            }
            Err(e) => {
                error!("Failed to open proximity log file: {}", e);
            }
        }
    }

    /// Log trading signal to CSV
    pub async fn log_trading_signal(&self, alert: &ZoneAlert, signal_type: &str) {
        if let Err(e) = self.ensure_logs_dir().await {
            error!("Failed to create logs directory: {}", e);
            return;
        }

        let filename = self.get_trading_filename();
        
        // Ensure headers are written (thread-safe)
        if let Err(e) = self.ensure_headers_written(&filename, true).await {
            error!("Failed to ensure trading headers: {}", e);
            return;
        }

        // Prepare CSV row
        let row = format!(
            "{},{},{},{},{},{},{:.1},{:.1},{:.5},{:.5},{:.5},{}\n",
            alert.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            alert.symbol,
            alert.zone_type,
            alert.zone_id,
            alert.timeframe,
            alert.touch_count,
            alert.distance_pips,
            alert.strength,
            alert.current_price,
            alert.zone_high,
            alert.zone_low,
            signal_type
        );

        // Append to file
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filename)
            .await
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(row.as_bytes()).await {
                    error!("Failed to write trading log: {}", e);
                } else {
                    info!("📝 Logged trading signal: {}", filename);
                }
            }
            Err(e) => {
                error!("Failed to open trading log file: {}", e);
            }
        }
    }
}