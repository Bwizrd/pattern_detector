// src/telegram_notifier.rs
use log::{error, info, warn};
use reqwest::Client;
use serde_json::json;
use std::env;
use crate::trade_decision_engine::ValidatedTradeSignal;

pub struct TelegramNotifier {
    client: Client,
    bot_token: Option<String>,
    chat_id: Option<String>,
    enabled: bool,
}

impl TelegramNotifier {
    pub fn new() -> Self {
        let bot_token = env::var("TELEGRAM_BOT_TOKEN").ok();
        let chat_id = env::var("TELEGRAM_CHAT_ID").ok();
        
        let enabled = bot_token.is_some() && chat_id.is_some();
        
        if enabled {
            info!("📱 Telegram notifier initialized");
        } else {
            warn!("📱 Telegram notifier disabled - missing TELEGRAM_BOT_TOKEN or TELEGRAM_CHAT_ID");
        }

        Self {
            client: Client::new(),
            bot_token,
            chat_id,
            enabled,
        }
    }

    pub async fn send_trade_signal(&self, signal: &ValidatedTradeSignal) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        let bot_token = self.bot_token.as_ref().unwrap();
        let chat_id = self.chat_id.as_ref().unwrap();

        // Format the message with trading details
        let emoji = if signal.action == "BUY" { "🟢" } else { "🔴" };
        let strength_stars = "★".repeat(signal.zone_strength.min(5.0) as usize);
        
        let message = format!(
            "{} *TRADE SIGNAL VALIDATED* {}\n\
            \n\
            📊 *Symbol:* `{}`\n\
            📈 *Action:* `{}`\n\
            💰 *Price:* `{:.5}`\n\
            ⏰ *Timeframe:* `{}`\n\
            🎯 *Zone ID:* `{}`\n\
            💪 *Strength:* `{}` {}\n\
            📅 *Time:* `{}`\n\
            \n\
            ⚡ *Ready to place trade!*",
            emoji,
            emoji,
            signal.symbol,
            signal.action,
            signal.price,
            signal.timeframe,
            signal.zone_id,
            signal.zone_strength,
            strength_stars,
            signal.validation_timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );

        let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
        
        let payload = json!({
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "Markdown",
            "disable_web_page_preview": true
        });

        let response = self.client
            .post(&url)
            .json(&payload)
            .send()
            .await?;

        if response.status().is_success() {
            info!("📱 Telegram notification sent for {} {} @ {:.5}", 
                  signal.action, signal.symbol, signal.price);
        } else {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            error!("📱 Failed to send Telegram notification: {}", error_text);
        }

        Ok(())
    }

    pub async fn send_test_message(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Err("Telegram notifier not enabled".into());
        }

        let bot_token = self.bot_token.as_ref().unwrap();
        let chat_id = self.chat_id.as_ref().unwrap();

        let message = "🤖 *Trading Bot Test*\n\nTelegram notifications are working correctly!\n\n✅ Ready to receive trade signals.";

        let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
        
        let payload = json!({
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "Markdown"
        });

        let response = self.client
            .post(&url)
            .json(&payload)
            .send()
            .await?;

        if response.status().is_success() {
            info!("📱 Telegram test message sent successfully");
            Ok(())
        } else {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            Err(format!("Failed to send test message: {}", error_text).into())
        }
    }

    pub async fn send_daily_summary(&self, signals_today: u32, max_signals: u32) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        let bot_token = self.bot_token.as_ref().unwrap();
        let chat_id = self.chat_id.as_ref().unwrap();

        let progress_bar = {
            let filled = (signals_today * 10 / max_signals.max(1)) as usize;
            let empty = 10 - filled;
            format!("{}{}", "█".repeat(filled), "░".repeat(empty))
        };

        let message = format!(
            "📊 *Daily Trading Summary*\n\
            \n\
            🎯 *Signals Today:* `{}/{}` \n\
            📈 *Progress:* `{}`\n\
            📅 *Date:* `{}`\n\
            \n\
            {} *Status:* {}",
            signals_today,
            max_signals,
            progress_bar,
            chrono::Utc::now().format("%Y-%m-%d"),
            if signals_today >= max_signals { "🔴" } else { "🟢" },
            if signals_today >= max_signals { "Limit Reached" } else { "Active" }
        );

        let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
        
        let payload = json!({
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "Markdown"
        });

        let response = self.client
            .post(&url)
            .json(&payload)
            .send()
            .await?;

        if response.status().is_success() {
            info!("📱 Daily summary sent to Telegram");
        } else {
            error!("📱 Failed to send daily summary");
        }

        Ok(())
    }
}