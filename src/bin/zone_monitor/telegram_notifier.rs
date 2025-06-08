// src/bin/zone_monitor/telegram_notifier.rs
// Your ORIGINAL code with just minimal rate limiting added

use crate::types::ZoneAlert;
use reqwest::Client;
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{error, info, warn};

#[derive(Debug)]
pub struct TelegramNotifier {
    client: Client,
    bot_token: Option<String>,
    chat_id: Option<String>,
    enabled: bool,
    // ONLY ADDITION: simple rate limiting
    last_send: Arc<Mutex<Option<Instant>>>,
}

impl TelegramNotifier {
    pub fn new() -> Self {
        let bot_token = env::var("TELEGRAM_BOT_TOKEN").ok();
        let chat_id = env::var("TELEGRAM_CHAT_ID").ok();
        
        let enabled = bot_token.is_some() && chat_id.is_some();
        
        if enabled {
            info!("📱 Telegram notifier initialized for zone monitor");
        } else {
            warn!("📱 Telegram notifier disabled - missing TELEGRAM_BOT_TOKEN or TELEGRAM_CHAT_ID");
        }

        Self {
            client: Client::new(),
            bot_token,
            chat_id,
            enabled,
            last_send: Arc::new(Mutex::new(None)), // ONLY ADDITION
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// ONLY ADDITION: Simple rate limiting
    async fn wait_for_rate_limit(&self) {
        let mut last_send = self.last_send.lock().await;
        
        if let Some(last_time) = *last_send {
            let elapsed = last_time.elapsed();
            let min_interval = Duration::from_millis(3000); // 3 seconds between messages
            
            if elapsed < min_interval {
                let wait_time = min_interval - elapsed;
                drop(last_send);
                sleep(wait_time).await;
                let mut last_send = self.last_send.lock().await;
                *last_send = Some(Instant::now());
            } else {
                *last_send = Some(Instant::now());
            }
        } else {
            *last_send = Some(Instant::now());
        }
    }

    /// Send proximity alert to Telegram
    pub async fn send_proximity_alert(&self, alert: &ZoneAlert) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        // ONLY ADDITION: Wait for rate limit
        self.wait_for_rate_limit().await;

        let bot_token = self.bot_token.as_ref().unwrap();
        let chat_id = self.chat_id.as_ref().unwrap();

        // Choose emoji based on zone type
        let emoji = if alert.zone_type.to_lowercase().contains("supply") { "🟡" } else { "🟠" };
        let direction = if alert.zone_type.to_lowercase().contains("supply") { "↗️" } else { "↙️" };
        
        let target_level = if alert.zone_type.to_lowercase().contains("supply") {
            alert.zone_low // Proximal line for supply
        } else {
            alert.zone_high // Proximal line for demand
        };

        let message = format!(
            "{} *ZONE PROXIMITY ALERT* {}\n\
            \n\
            📊 *Symbol:* `{}`\n\
            📍 *Current Price:* `{:.5}`\n\
            🎯 *Target Level:* `{:.5}`\n\
            📏 *Distance:* `{:.1} pips`\n\
            ⏰ *Timeframe:* `{}`\n\
            👆 *Touch Count:* `{}`\n\
            💪 *Strength:* `{:.1}%`\n\
            🏷️ *Zone ID:* `{}`\n\
            📈 *Zone Type:* `{}`\n\
            📊 *Zone Range:* `{:.5} - {:.5}`\n\
            \n\
            {} *Price approaching zone!*",
            emoji,
            emoji,
            alert.symbol,
            alert.current_price,
            target_level,
            alert.distance_pips,
            alert.timeframe,
            alert.touch_count,
            alert.strength,
            &alert.zone_id, // Full zone ID
            alert.zone_type,
            alert.zone_low,
            alert.zone_high,
            direction
        );

        self.send_message(&message).await?;

        info!("📱 Proximity alert sent to Telegram for {} {} @ {:.5} ({}pips from zone) [Zone: {}]", 
              alert.symbol, alert.zone_type, alert.current_price, alert.distance_pips, alert.zone_id);

        Ok(())
    }

    /// Send trading signal to Telegram
    pub async fn send_trading_signal(&self, alert: &ZoneAlert, signal_type: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        // ONLY ADDITION: Wait for rate limit
        self.wait_for_rate_limit().await;

        let message = match signal_type {
            "SIGNAL_GENERATED" => {
                let action_emoji = if alert.zone_type.to_lowercase().contains("supply") { "🔴" } else { "🟢" };
                let action = if alert.zone_type.to_lowercase().contains("supply") { "SELL" } else { "BUY" };
                
                let strength_emoji = if alert.strength >= 90.0 {
                    "🔥"
                } else if alert.strength >= 80.0 {
                    "💪"
                } else if alert.strength >= 70.0 {
                    "✅"
                } else {
                    "⚠️"
                };

                format!(
                    "🚨 *TRADING SIGNAL* 🚨\n\
                    \n\
                    {} *{}* `{}`\n\
                    💰 *Entry:* `{:.5}`\n\
                    ⏰ *Time:* `{}`\n\
                    🔧 *Timeframe:* `{}`\n\
                    👆 *Touch Count:* `{}`\n\
                    📏 *Distance:* `{:.1} pips`\n\
                    🆔 *Zone:* `{}`\n\
                    \n\
                    {} *Strength:* `{:.1}%`\n\
                    📈 *Zone Type:* `{}`\n\
                    📊 *Zone Range:* `{:.5} - {:.5}`\n\
                    \n\
                    ⚡ *READY TO TRADE!*",
                    action_emoji,
                    action,
                    alert.symbol,
                    alert.current_price,
                    alert.timestamp.format("%H:%M:%S UTC"),
                    alert.timeframe,
                    alert.touch_count,
                    alert.distance_pips,
                    &alert.zone_id, // Full zone ID
                    strength_emoji,
                    alert.strength,
                    alert.zone_type,
                    alert.zone_low,
                    alert.zone_high
                )
            },
            "TRADE_EXECUTED" => {
                let action_emoji = if alert.zone_type.to_lowercase().contains("supply") { "🔴" } else { "🟢" };
                let action = if alert.zone_type.to_lowercase().contains("supply") { "SELL" } else { "BUY" };

                format!(
                    "✅ *TRADE EXECUTED* ✅\n\
                    \n\
                    {} *{}* `{}`\n\
                    💰 *Executed at:* `{:.5}`\n\
                    ⏰ *Time:* `{}`\n\
                    🔧 *Timeframe:* `{}`\n\
                    🆔 *Zone:* `{}`\n\
                    \n\
                    🎉 *Trade successfully opened!*\n\
                    📈 *Monitor for SL/TP levels*",
                    action_emoji,
                    action,
                    alert.symbol,
                    alert.current_price,
                    alert.timestamp.format("%H:%M:%S UTC"),
                    alert.timeframe,
                    &alert.zone_id // Full zone ID
                )
            },
            "TRADE_REJECTED" => {
                let action_emoji = if alert.zone_type.to_lowercase().contains("supply") { "🔴" } else { "🟢" };
                let action = if alert.zone_type.to_lowercase().contains("supply") { "SELL" } else { "BUY" };

                format!(
                    "🚫 *TRADE REJECTED* 🚫\n\
                    \n\
                    {} *{}* `{}`\n\
                    💰 *Price:* `{:.5}`\n\
                    ⏰ *Time:* `{}`\n\
                    🔧 *Timeframe:* `{}`\n\
                    🆔 *Zone:* `{}`\n\
                    \n\
                    ❌ *Trade blocked by trading rules*\n\
                    💡 *Check logs for rejection reason*",
                    action_emoji,
                    action,
                    alert.symbol,
                    alert.current_price,
                    alert.timestamp.format("%H:%M:%S UTC"),
                    alert.timeframe,
                    &alert.zone_id // Full zone ID
                )
            },
            _ => {
                format!(
                    "📊 *TRADING UPDATE*\n\
                    \n\
                    📈 *{}* `{}`\n\
                    💰 *Price:* `{:.5}`\n\
                    📏 *Signal:* `{}`",
                    alert.symbol,
                    alert.zone_type,
                    alert.current_price,
                    signal_type
                )
            }
        };

        self.send_message(&message).await?;

        info!("📱 Trading signal sent to Telegram: {} {} {} @ {:.5} [Zone: {}]", 
              signal_type, alert.symbol, alert.zone_type, alert.current_price, alert.zone_id);

        Ok(())
    }

    /// Send test message
    pub async fn send_test_message(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Err("Telegram notifier not enabled".into());
        }

        let message = "🤖 *Zone Monitor Test*\n\nTelegram notifications are working correctly!\n\n✅ Ready to receive zone alerts and trading signals.";

        self.send_message(&message).await?;
        info!("📱 Telegram test message sent successfully");
        Ok(())
    }

    /// Helper method to send message (YOUR ORIGINAL METHOD with rate limit handling)
    async fn send_message(&self, message: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let bot_token = self.bot_token.as_ref().unwrap();
        let chat_id = self.chat_id.as_ref().unwrap();

        let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
        
        let payload = json!({
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "Markdown",
            "disable_web_page_preview": true
        });

        // Try with rate limit handling
        for attempt in 1..=2 {
            match self.client.post(&url).json(&payload).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        return Ok(());
                    } else {
                        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                        
                        // Handle rate limiting specifically
                        if error_text.contains("Too Many Requests") && error_text.contains("retry after") {
                            // Extract retry_after value
                            let wait_time = if let Some(start) = error_text.find("retry after ") {
                                let after_text = &error_text[start + 12..];
                                if let Some(end) = after_text.find('"') {
                                    after_text[..end].parse().unwrap_or(5)
                                } else {
                                    5
                                }
                            } else {
                                5
                            };
                            
                            if attempt == 1 {
                                warn!("📱 Telegram rate limited, waiting {} seconds", wait_time);
                                sleep(Duration::from_secs(wait_time as u64)).await;
                                continue;
                            }
                        }
                        
                        return Err(format!("Failed to send Telegram message: {}", error_text).into());
                    }
                }
                Err(e) => {
                    if attempt == 1 && e.to_string().contains("timeout") {
                        warn!("📱 Telegram timeout, retrying once: {}", e);
                        sleep(Duration::from_secs(2)).await;
                        continue;
                    } else {
                        return Err(e.into());
                    }
                }
            }
        }

        Err("Failed to send after retry".into())
    }
}