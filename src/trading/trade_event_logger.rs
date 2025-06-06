// src/trade_event_logger.rs - Simplified for today only with daily rollover
use chrono::{DateTime, Utc, Duration};
use log::{error, info, warn, debug};
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use csv;

use crate::cache::minimal_zone_cache::{TradeNotification, MinimalZoneCache};
use crate::trading::trade_decision_engine::{ValidatedTradeSignal, TradeDecisionEngine};
use crate::types::EnrichedZone;

#[derive(Serialize, Debug, Clone)]
pub enum TradeEventType {
    NotificationReceived,
    SignalValidated,
    SignalRejected,
    HistoricalNotification,
    HistoricalValidated,
    HistoricalRejected,
}

#[derive(Serialize, Debug)]
pub struct TradeLogEntry<T> {
    pub event_timestamp: DateTime<Utc>,
    pub log_timestamp: DateTime<Utc>,
    pub event_type: TradeEventType,
    pub details: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct HistoricalTradeAnalysis {
    pub analysis_period_start: DateTime<Utc>,
    pub analysis_period_end: DateTime<Utc>,
    pub total_zones_analyzed: usize,
    pub total_potential_notifications: usize,
    pub total_validated_signals: usize,
    pub total_rejected_signals: usize,
    pub symbols_analyzed: Vec<String>,
    pub timeframes_analyzed: Vec<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct SimulatedPricePoint {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub timeframe: String,
    pub price: f64,
    pub zone_id: String,
    pub trigger_type: String,
}

pub struct TradeEventLogger {
    writer: Option<Arc<Mutex<BufWriter<std::fs::File>>>>,
    csv_writer: Option<Arc<Mutex<csv::Writer<std::fs::File>>>>,
    log_file_path: PathBuf,
    csv_file_path: PathBuf,
    current_date: String,
    historical_analysis_complete: bool,
}

impl TradeEventLogger {
    pub fn new() -> Result<Self, std::io::Error> {
        let log_dir = PathBuf::from("trades");
        if !log_dir.exists() {
            create_dir_all(&log_dir)?;
        }

        let current_date = Utc::now().format("%Y-%m-%d").to_string();
        
        // JSON log file
        let log_file_name = format!("{}_trades.log", current_date);
        let log_file_path = log_dir.join(log_file_name);

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)?;
        let writer = Some(Arc::new(Mutex::new(BufWriter::new(file))));

        // CSV file for validated trades only
        let csv_file_name = format!("{}_validated_trades.csv", current_date);
        let csv_file_path = log_dir.join(csv_file_name);

        let csv_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&csv_file_path)?;

        let mut csv_writer = csv::Writer::from_writer(csv_file);
        
        // Write CSV header
        csv_writer.write_record(&[
            "timestamp",
            "symbol", 
            "timeframe",
            "action",
            "price",
            "zone_type",
            "strength",
            "zone_id",
            "touch_count",
            "validation_reason",
            "event_type"
        ])?;
        csv_writer.flush()?;

        let csv_writer_arc = Some(Arc::new(Mutex::new(csv_writer)));

        info!("📝 Trade event logger initialized:");
        info!("📝   JSON Log: {:?}", log_file_path);
        info!("📝   CSV File: {:?}", csv_file_path);

        Ok(Self {
            writer,
            csv_writer: csv_writer_arc,
            log_file_path,
            csv_file_path,
            current_date,
            historical_analysis_complete: false,
        })
    }

    pub fn get_log_file_path(&self) -> &PathBuf {
        &self.log_file_path
    }

    /// Check if we need to roll over to a new file (new day)
    async fn check_and_rollover(&mut self) -> Result<(), std::io::Error> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        
        if today != self.current_date {
            info!("📅 [TRADE_LOGGER] Rolling over to new day: {} -> {}", self.current_date, today);
            
            // Close current writers
            if let Some(writer_arc) = &self.writer {
                let mut writer_guard = writer_arc.lock().await;
                writer_guard.flush()?;
            }
            if let Some(csv_writer_arc) = &self.csv_writer {
                let mut csv_writer_guard = csv_writer_arc.lock().await;
                csv_writer_guard.flush()?;
            }
            
            // Create new files for today
            let log_dir = PathBuf::from("trades");
            
            // New JSON log file
            let log_file_name = format!("{}_trades.log", today);
            let new_log_file_path = log_dir.join(log_file_name);

            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&new_log_file_path)?;
            let new_writer = Some(Arc::new(Mutex::new(BufWriter::new(file))));

            // New CSV file
            let csv_file_name = format!("{}_validated_trades.csv", today);
            let new_csv_file_path = log_dir.join(csv_file_name);

            let csv_file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&new_csv_file_path)?;

            let mut csv_writer = csv::Writer::from_writer(csv_file);
            
            // Write CSV header
            csv_writer.write_record(&[
                "timestamp",
                "symbol", 
                "timeframe",
                "action",
                "price",
                "zone_type",
                "strength",
                "zone_id",
                "touch_count",
                "validation_reason",
                "event_type"
            ])?;
            csv_writer.flush()?;

            // Update state
            self.writer = new_writer;
            self.csv_writer = Some(Arc::new(Mutex::new(csv_writer)));
            self.log_file_path = new_log_file_path;
            self.csv_file_path = new_csv_file_path;
            self.current_date = today;
            self.historical_analysis_complete = false; // Allow new analysis for new day

            info!("📝 [TRADE_LOGGER] New files created:");
            info!("📝   JSON Log: {:?}", self.log_file_path);
            info!("📝   CSV File: {:?}", self.csv_file_path);
        }
        
        Ok(())
    }

    /// Analyze trades for TODAY ONLY
    pub async fn analyze_historical_trades(
        &mut self,
        cache: Arc<Mutex<MinimalZoneCache>>,
    ) -> Result<HistoricalTradeAnalysis, Box<dyn std::error::Error + Send + Sync>> {
        if self.historical_analysis_complete {
            warn!("📊 Historical analysis already completed for today");
            return Err("Historical analysis already completed for today".into());
        }

        info!("📊 Starting TODAY ONLY trade analysis...");
        let analysis_start_time = std::time::Instant::now();

        // TODAY ONLY - from start of today to now
        let now = Utc::now();
        let today_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
        let today_end = now;

        let period_start_str = today_start.to_rfc3339();
        let period_end_str = today_end.to_rfc3339();

        let zones = {
            let cache_guard = cache.lock().await;
            cache_guard.get_all_zones()
        };

        info!("📊 Analyzing {} zones for TODAY ONLY: {} to {}", zones.len(), period_start_str, period_end_str);

        // Group zones by symbol/timeframe
        let mut zones_by_symbol_tf: HashMap<String, Vec<EnrichedZone>> = HashMap::new();
        let mut symbols_set = std::collections::HashSet::new();
        let mut timeframes_set = std::collections::HashSet::new();

        for zone in &zones {
            if let (Some(symbol), Some(timeframe)) = (&zone.symbol, &zone.timeframe) {
                let key = format!("{}_{}", symbol, timeframe);
                zones_by_symbol_tf.entry(key).or_insert_with(Vec::new).push(zone.clone());
                symbols_set.insert(symbol.clone());
                timeframes_set.insert(timeframe.clone());
            }
        }

        let symbols_analyzed: Vec<String> = symbols_set.into_iter().collect();
        let timeframes_analyzed: Vec<String> = timeframes_set.into_iter().collect();

        // Create a trade decision engine for validation
        let mut trade_engine = TradeDecisionEngine::new_with_cache(cache.clone());

        let mut total_potential_notifications = 0;
        let mut total_validated_signals = 0;
        let mut total_rejected_signals = 0;

        // Process each symbol/timeframe combination
        for (symbol_tf_key, symbol_zones) in zones_by_symbol_tf {
            let parts: Vec<&str> = symbol_tf_key.split('_').collect();
            if parts.len() != 2 {
                continue;
            }
            let symbol = parts[0];
            let timeframe = parts[1];

            // SKIP 5m and 15m timeframes - don't log them at all
            if timeframe == "5m" || timeframe == "15m" {
                continue;
            }

            // Generate triggers for TODAY ONLY - evenly spaced throughout the day
            let triggers = self.generate_today_triggers(&symbol_zones, today_start, today_end);

            for trigger in triggers {
                total_potential_notifications += 1;

                // Create a simulated notification
                let notification = TradeNotification {
                    timestamp: trigger.timestamp,
                    symbol: trigger.symbol.clone(),
                    timeframe: trigger.timeframe.clone(),
                    action: if trigger.trigger_type == "supply_touch" { "SELL".to_string() } else { "BUY".to_string() },
                    price: trigger.price,
                    zone_type: trigger.trigger_type.clone(),
                    strength: self.get_zone_strength_by_id(&zones, &trigger.zone_id).unwrap_or(50.0),
                    zone_id: trigger.zone_id.clone(),
                };

                // Log the historical notification
                self.log_historical_notification(&notification).await;

                // Validate through trade engine and get specific rejection reason
                match trade_engine.process_notification_with_reason(notification.clone()).await {
                    Ok(validated_signal) => {
                        total_validated_signals += 1;
                        self.log_historical_validated(&validated_signal).await;
                        info!("✅ TODAY VALIDATED: {} {} @ {:.5} ({})", 
                              validated_signal.action, validated_signal.symbol, 
                              validated_signal.price, validated_signal.zone_id);
                    }
                    Err(specific_reason) => {
                        total_rejected_signals += 1;
                        self.log_historical_rejected(&notification, specific_reason.clone()).await;
                        debug!("❌ TODAY REJECTED: {} {} @ {:.5} ({}) - {}", 
                               notification.action, notification.symbol, 
                               notification.price, notification.zone_id, specific_reason);
                    }
                }
            }
        }

        let analysis_duration = analysis_start_time.elapsed();

        let analysis_summary = HistoricalTradeAnalysis {
            analysis_period_start: today_start,
            analysis_period_end: today_end,
            total_zones_analyzed: zones.len(),
            total_potential_notifications,
            total_validated_signals,
            total_rejected_signals,
            symbols_analyzed: symbols_analyzed.clone(),
            timeframes_analyzed: timeframes_analyzed.clone(),
        };

        // Log the analysis summary
        self.log_analysis_summary(&analysis_summary).await;

        info!("📊 === TODAY'S ANALYSIS COMPLETE ===");
        info!("📊 Analysis Duration: {:.2}s", analysis_duration.as_secs_f64());
        info!("📊 Period: TODAY ONLY {} to {}", period_start_str, period_end_str);
        info!("📊 Zones Analyzed: {}", zones.len());
        info!("📊 Potential Notifications: {}", total_potential_notifications);
        info!("📊 Validated Signals: {}", total_validated_signals);
        info!("📊 Rejected Signals: {}", total_rejected_signals);
        info!("📊 Success Rate: {:.1}%", 
              if total_potential_notifications > 0 { 
                  (total_validated_signals as f64 / total_potential_notifications as f64) * 100.0 
              } else { 0.0 });
        info!("📊 Symbols: {:?}", &symbols_analyzed);
        info!("📊 Timeframes: {:?}", &timeframes_analyzed);

        self.historical_analysis_complete = true;
        Ok(analysis_summary)
    }

    /// Generate triggers for TODAY only - evenly spaced
    fn generate_today_triggers(
        &self,
        zones: &[EnrichedZone],
        today_start: DateTime<Utc>,
        today_end: DateTime<Utc>,
    ) -> Vec<SimulatedPricePoint> {
        let mut triggers = Vec::new();
        let total_seconds = (today_end - today_start).num_seconds() as f64;

        for (index, zone) in zones.iter().enumerate() {
            if let (Some(zone_id), Some(symbol), Some(timeframe)) = 
                (&zone.zone_id, &zone.symbol, &zone.timeframe) {
                
                let zone_high = zone.zone_high.unwrap_or(0.0);
                let zone_low = zone.zone_low.unwrap_or(0.0);
                let is_supply = zone.zone_type.as_deref().unwrap_or("").contains("supply");

                // Evenly space triggers throughout the day
                let progress = if zones.len() > 1 { index as f64 / (zones.len() - 1) as f64 } else { 0.5 };
                let seconds_offset = (progress * total_seconds) as i64;
                let trigger_time = today_start + Duration::seconds(seconds_offset);
                
                let (trigger_price, trigger_type) = if is_supply {
                    (zone_low, "supply_touch")
                } else {
                    (zone_high, "demand_touch")
                };

                triggers.push(SimulatedPricePoint {
                    timestamp: trigger_time,
                    symbol: symbol.clone(),
                    timeframe: timeframe.clone(),
                    price: trigger_price,
                    zone_id: zone_id.clone(),
                    trigger_type: trigger_type.to_string(),
                });
            }
        }

        triggers.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        triggers
    }

    fn get_zone_strength_by_id(&self, zones: &[EnrichedZone], zone_id: &str) -> Option<f64> {
        zones.iter()
            .find(|zone| zone.zone_id.as_deref() == Some(zone_id))
            .and_then(|zone| zone.strength_score)
    }

    async fn log<T: Serialize + std::fmt::Debug>(
        &self,
        event_timestamp: DateTime<Utc>,
        event_type: TradeEventType,
        details: T,
        rejection_reason: Option<String>,
    ) {
        if let Some(writer_arc) = &self.writer {
            let entry = TradeLogEntry {
                event_timestamp,
                log_timestamp: Utc::now(),
                event_type,
                details,
                rejection_reason,
            };

            match serde_json::to_string(&entry) {
                Ok(json_string) => {
                    let mut writer_guard = writer_arc.lock().await;
                    if let Err(e) = writeln!(writer_guard, "{}", json_string) {
                        error!("Failed to write to trade log: {}", e);
                    }
                    if let Err(e) = writer_guard.flush() {
                         error!("Failed to flush trade log: {}", e);
                    }
                }
                Err(e) => {
                    error!("Failed to serialize trade log entry: {:?}", e);
                }
            }
        }
    }

    // Real-time logging methods - KEEP IMMUTABLE, handle rollover externally
    pub async fn log_notification_received(&self, notification: &TradeNotification) {
        self.log(
            notification.timestamp,
            TradeEventType::NotificationReceived,
            notification,
            None,
        )
        .await;
    }

    pub async fn log_signal_validated(&self, signal: &ValidatedTradeSignal) {
        // Log to JSON
        self.log(
            signal.validation_timestamp,
            TradeEventType::SignalValidated,
            signal,
            None,
        )
        .await;

        // Also log to CSV
        self.write_to_csv(signal, "SignalValidated").await;
    }

    pub async fn log_signal_rejected(
        &self,
        notification: &TradeNotification,
        reason: String,
    ) {
        self.log(
            notification.timestamp,
            TradeEventType::SignalRejected,
            notification,
            Some(reason),
        )
        .await;
    }

    // Separate method for manual rollover check (call from main.rs periodically)
    pub async fn force_rollover_check(&mut self) -> Result<(), std::io::Error> {
        self.check_and_rollover().await
    }

    // Historical logging methods
    async fn log_historical_notification(&self, notification: &TradeNotification) {
        self.log(
            notification.timestamp,
            TradeEventType::HistoricalNotification,
            notification,
            None,
        )
        .await;
    }

    async fn log_historical_validated(&self, signal: &ValidatedTradeSignal) {
        // Log to JSON
        self.log(
            signal.validation_timestamp,
            TradeEventType::HistoricalValidated,
            signal,
            None,
        )
        .await;

        // Also log to CSV
        self.write_to_csv(signal, "HistoricalValidated").await;
    }

    async fn log_historical_rejected(
        &self,
        notification: &TradeNotification,
        reason: String,
    ) {
        self.log(
            notification.timestamp,
            TradeEventType::HistoricalRejected,
            notification,
            Some(reason),
        )
        .await;
    }

    // Helper method to write validated signals to CSV
    async fn write_to_csv(&self, signal: &ValidatedTradeSignal, event_type: &str) {
        if let Some(csv_writer_arc) = &self.csv_writer {
            let mut csv_writer_guard = csv_writer_arc.lock().await;
            
            let record = vec![
                signal.validation_timestamp.to_rfc3339(),
                signal.symbol.clone(),
                signal.timeframe.clone(),
                signal.action.clone(),
                signal.price.to_string(),
                "validated".to_string(), // zone_type equivalent
                signal.zone_strength.to_string(),
                signal.zone_id.clone(),
                signal.touch_count.to_string(),
                signal.validation_reason.clone(),
                event_type.to_string(),
            ];

            if let Err(e) = csv_writer_guard.write_record(&record) {
                error!("Failed to write to CSV: {}", e);
            }
            if let Err(e) = csv_writer_guard.flush() {
                error!("Failed to flush CSV: {}", e);
            }
        }
    }

    async fn log_analysis_summary(&self, analysis: &HistoricalTradeAnalysis) {
        self.log(
            Utc::now(),
            TradeEventType::HistoricalNotification,
            analysis,
            None,
        )
        .await;
    }

    pub fn has_completed_historical_analysis(&self) -> bool {
        self.historical_analysis_complete
    }
}