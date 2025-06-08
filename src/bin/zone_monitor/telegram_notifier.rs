// src/bin/zone_monitor/telegram_notifier.rs
// Simple fix - just add rate limiting to your existing code

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
    // Simple rate limiting - just track last send time
    last_send: Arc<Mutex<Option<Instant>>>,
}

impl TelegramNotifier {
    pub fn new() -> Self {
        let bot_token = env::var("TELEGRAM_BOT_TOKEN").ok();
        let chat_id = env::var("TELEGRAM_CHAT_ID").ok();
        
        let enabled = bot_token.is_some() && chat_id.is_some();
        
        if enabled {
            info!("📱 Telegram notifier initialized for zone monitor with rate limiting");
        } else {
            warn!("📱 Telegram notifier disabled - missing TELEGRAM_BOT_TOKEN or TELEGRAM_CHAT_ID");
        }

        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            bot_token,
            chat_id,
            enabled,
            last_send: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Simple rate limiting - wait at least 1 second between sends
    async fn wait_for_rate_limit(&self) {
        let mut last_send = self.last_send.lock().await;
        
        if let Some(last_time) = *last_send {
            let elapsed = last_time.elapsed();
            let min_interval = Duration::from_millis(1000); // 1 second between messages
            
            if elapsed < min_interval {
                let wait_time = min_interval - elapsed;
                drop(last_send); // Release lock before sleeping
                sleep(wait_time).await;
                
                // Update last send time
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

        // Wait for rate limit
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

        // Try sending with retry on rate limit
        self.send_message_with_retry(&message).await?;

        info!("📱 Proximity alert sent to Telegram for {} {} @ {:.5} ({}pips from zone)", 
              alert.symbol, alert.zone_type, alert.current_price, alert.distance_pips);

        Ok(())
    }

    /// Send trading signal to Telegram
    pub async fn send_trading_signal(&self, alert: &ZoneAlert, signal_type: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        // Wait for rate limit
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
            "TRADE_FAILED" => {
                let action_emoji = if alert.zone_type.to_lowercase().contains("supply") { "🔴" } else { "🟢" };
                let action = if alert.zone_type.to_lowercase().contains("supply") { "SELL" } else { "BUY" };

                format!(
                    "❌ *TRADE FAILED* ❌\n\
                    \n\
                    {} *{}* `{}`\n\
                    💰 *Price:* `{:.5}`\n\
                    ⏰ *Time:* `{}`\n\
                    🔧 *Timeframe:* `{}`\n\
                    🆔 *Zone:* `{}`\n\
                    \n\
                    🔧 *Technical failure during execution*\n\
                    💡 *Check logs for error details*",
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

        self.send_message_with_retry(&message).await?;

        info!("📱 Trading signal sent to Telegram: {} {} {} @ {:.5}", 
              signal_type, alert.symbol, alert.zone_type, alert.current_price);

        Ok(())
    }

    /// Send test message
    pub async fn send_test_message(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Err("Telegram notifier not enabled".into());
        }

        let message = "🤖 *Zone Monitor Test*\n\nTelegram notifications are working correctly!\n\n✅ Ready to receive zone alerts and trading signals.";

        self.send_message_with_retry(&message).await?;
        info!("📱 Telegram test message sent successfully");
        Ok(())
    }

    /// Send message with simple retry logic
    async fn send_message_with_retry(&self, message: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let bot_token = self.bot_token.as_ref().unwrap();
        let chat_id = self.chat_id.as_ref().unwrap();

        let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
        
        let payload = json!({
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "Markdown",
            "disable_web_page_preview": true
        });

        // Try up to 3 times with increasing delays
        for attempt in 1..=3 {
            match self.client.post(&url).json(&payload).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        return Ok(());
                    } else {
                        let status = response.status().as_u16();
                        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                        
                        // Check for rate limit
                        if error_text.contains("Too Many Requests") {
                            // Extract retry_after if present
                            let wait_time = if let Some(start) = error_text.find("retry after ") {
                                let after_text = &error_text[start + 12..];
                                if let Some(end) = after_text.find(' ') {
                                    after_text[..end].parse().unwrap_or(5)
                                } else {
                                    after_text.parse().unwrap_or(5)
                                }
                            } else {
                                5 // Default wait time
                            };
                            
                            warn!("📱 Telegram rate limited, waiting {} seconds (attempt {})", wait_time, attempt);
                            sleep(Duration::from_secs(wait_time)).await;
                            continue;
                        } else {
                            return Err(format!("HTTP {}: {}", status, error_text).into());
                        }
                    }
                }
                Err(e) => {
                    if attempt < 3 {
                        let wait_time = attempt * 2; // 2, 4 seconds
                        warn!("📱 Telegram send failed (attempt {}), retrying in {}s: {}", attempt, wait_time, e);
                        sleep(Duration::from_secs(wait_time)).await;
                        continue;
                    } else {
                        return Err(format!("Failed after 3 attempts: {}", e).into());
                    }
                }
            }
        }

        Err("Failed to send message after retries".into())
    }
}