// src/bin/zone_monitor/csv_logger.rs
// CSV logging for proximity alerts and trading signals

use crate::types::ZoneAlert;
use std::path::Path;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::{error, info};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashSet;
use tokio::time::{interval, Duration};

#[derive(Debug)]
pub struct CsvLogger {
    logs_dir: String,
    proximity_file_initialized: Arc<Mutex<bool>>,
    trading_file_initialized: Arc<Mutex<bool>>,
    booking_file_initialized: Arc<Mutex<bool>>,
    // Buffer: Accumulate logs and write them periodically
    log_buffer: Arc<Mutex<HashSet<String>>>,
}

impl CsvLogger {
    pub fn new() -> Self {
        Self {
            logs_dir: "logs".to_string(),
            proximity_file_initialized: Arc::new(Mutex::new(false)),
            trading_file_initialized: Arc::new(Mutex::new(false)),
            booking_file_initialized: Arc::new(Mutex::new(false)),
            log_buffer: Arc::new(Mutex::new(HashSet::new())),
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

    /// Get booking log filename for today
    fn get_booking_filename(&self) -> String {
        format!("{}/booking_{}.csv", self.logs_dir, Self::get_date_string())
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

    /// Write booking headers
    async fn write_booking_headers(&self, filename: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(filename)
            .await?;

        let headers = "timestamp,symbol,zone_id,timeframe,zone_type,current_price,proximal_price,distance_pips,zone_strength,touch_count,booking_attempt,booking_result,ctrader_order_id,failure_reason\n";
        file.write_all(headers.as_bytes()).await?;
        Ok(())
    }

    /// Buffer booking attempt logs and write them periodically
    pub async fn log_booking_attempt(
        &self,
        alert: &ZoneAlert,
        booking_result: &str,
        ctrader_order_id: Option<&str>,
        failure_reason: Option<&str>,
    ) {
        // Generate a unique entry key for the buffer
        let entry = format!(
            "{},{},{},{},{},{:.5},{:.5},{:.1},{:.1},{},{},{},{},{}",
            alert.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            alert.symbol,
            alert.zone_id,
            alert.timeframe,
            alert.zone_type,
            alert.current_price,
            alert.zone_high,
            alert.distance_pips,
            alert.strength,
            alert.touch_count,
            true, // booking attempt
            booking_result,
            ctrader_order_id.unwrap_or(""),
            failure_reason.unwrap_or("")
        );

        // Insert the entry into the buffer
        let mut buffer = self.log_buffer.lock().await;
        buffer.insert(entry);
    }

    /// Start an interval to flush buffer to the CSV file every second
    pub async fn start_logging_interval(&self) {
        let mut interval = interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let mut buffer = self.log_buffer.lock().await;
            if !buffer.is_empty() {
                if let Err(e) = self.ensure_logs_dir().await {
                    error!("Failed to create logs directory: {}", e);
                    return;
                }
                let filename = self.get_booking_filename();
                // Ensure headers are written (thread-safe)
                {
                    let mut initialized = self.booking_file_initialized.lock().await;
                    if !*initialized {
                        let file_exists = Path::new(&filename).exists();
                        let needs_headers = if file_exists {
                            match tokio::fs::metadata(&filename).await {
                                Ok(metadata) => metadata.len() < 50,
                                Err(_) => true,
                            }
                        } else {
                            true
                        };

                        if needs_headers {
                            if let Err(e) = self.write_booking_headers(&filename).await {
                                error!("Failed to write booking headers: {}", e);
                                return;
                            }
                        }
                        *initialized = true;
                    }
                }
                
                match OpenOptions::new() .create(true) .append(true) .open(&filename).await {
                    Ok(mut file) => {
                        for entry in buffer.drain() {
                            if let Err(e) = file.write_all(format!("{}\n", entry).as_bytes()).await {
                                error!("Failed to write booking log: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to open booking log file: {}", e);
                    }
                }
            }
        }
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

    /// Clear the log buffer (for testing/debugging)
    pub async fn clear_buffer(&self) {
        let mut buffer = self.log_buffer.lock().await;
        let count = buffer.len();
        buffer.clear();
        if count > 0 {
            info!("📝 Cleared {} buffered log entries", count);
        }
    }
}
