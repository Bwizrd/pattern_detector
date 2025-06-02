// src/trade_event_logger.rs
use chrono::{DateTime, Utc};
use log::{error, info};
use serde::Serialize;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::minimal_zone_cache::TradeNotification;
use crate::trade_decision_engine::ValidatedTradeSignal;

#[derive(Serialize, Debug)]
pub enum TradeEventType {
    NotificationReceived,
    SignalValidated,
    SignalRejected,
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

pub struct TradeEventLogger {
    writer: Option<Arc<Mutex<BufWriter<std::fs::File>>>>,
    log_file_path: PathBuf,
}

impl TradeEventLogger {
    pub fn new() -> Result<Self, std::io::Error> {
        let log_dir = PathBuf::from("trades");
        if !log_dir.exists() {
            create_dir_all(&log_dir)?;
        }

        let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let log_file_name = format!("{}_trades.log", timestamp);
        let log_file_path = log_dir.join(log_file_name);

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)?;
        let writer = Some(Arc::new(Mutex::new(BufWriter::new(file))));

        info!("📝 Trade event logger initialized. Logging to: {:?}", log_file_path);

        Ok(Self {
            writer,
            log_file_path,
        })
    }

    pub fn get_log_file_path(&self) -> &PathBuf {
        &self.log_file_path
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
        self.log(
            signal.validation_timestamp,
            TradeEventType::SignalValidated,
            signal,
            None, // Validation reason is part of the signal struct
        )
        .await;
    }

    pub async fn log_signal_rejected(
        &self,
        notification: &TradeNotification,
        reason: String,
    ) {
        self.log(
            notification.timestamp,
            TradeEventType::SignalRejected,
            notification, // Log the original notification that was rejected
            Some(reason),
        )
        .await;
    }
}