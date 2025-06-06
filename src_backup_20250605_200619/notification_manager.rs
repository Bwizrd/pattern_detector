// src/notification_manager.rs
use log::{error, info};
use std::sync::Arc;
use crate::trade_decision_engine::ValidatedTradeSignal;
use crate::minimal_zone_cache::TradeNotification;
use crate::telegram_notifier::TelegramNotifier;
use crate::sound_notifier::SoundNotifier;

pub struct NotificationManager {
    telegram: TelegramNotifier,
    sound: SoundNotifier,
}

impl NotificationManager {
    pub fn new() -> Self {
        Self {
            telegram: TelegramNotifier::new(),
            sound: SoundNotifier::new(),
        }
    }

    /// Send proximity alert notifications (price approaching zone)
    pub async fn notify_proximity_alert(&self, symbol: &str, current_price: f64, zone_type: &str, zone_high: f64, zone_low: f64, zone_id: &str, timeframe: &str, distance_pips: f64) {
        info!(
            "📢 Sending proximity notifications for {} {} zone @ {:.5} ({:.1} pips away)", 
            symbol, zone_type, current_price, distance_pips
        );

        // Send notifications concurrently
        let telegram_task = self.telegram.send_proximity_alert(symbol, current_price, zone_type, zone_high, zone_low, zone_id, timeframe, distance_pips);
        let sound_task = self.sound.play_proximity_alert(symbol, zone_type, distance_pips);

        // Wait for both to complete
        let (telegram_result, sound_result) = tokio::join!(telegram_task, sound_task);

        // Log any errors but don't fail
        if let Err(e) = telegram_result {
            error!("📱 Proximity Telegram notification failed: {}", e);
        }

        if let Err(e) = sound_result {
            error!("🔊 Proximity sound notification failed: {}", e);
        }
    }

    /// Send inside zone alert notifications (price is inside the zone)
    pub async fn notify_inside_zone_alert(
        &self,
        symbol: &str,
        current_price: f64,
        zone_type: &str,
        zone_high: f64,
        zone_low: f64,
        zone_id: &str,
        timeframe: &str,
        distance_from_proximal: f64,
        distance_from_distal: f64,
    ) {
        info!(
            "📢 Sending INSIDE ZONE notifications for {} {} zone @ {:.5} ({:.1} pips from proximal, {:.1} pips from distal)", 
            symbol, zone_type, current_price, distance_from_proximal, distance_from_distal
        );

        // Send notifications concurrently
        let telegram_task = self.telegram.send_inside_zone_alert(
            symbol, 
            current_price, 
            zone_type, 
            zone_high, 
            zone_low, 
            zone_id, 
            timeframe, 
            distance_from_proximal, 
            distance_from_distal
        );
        let sound_task = self.sound.play_inside_zone_alert(symbol, zone_type, distance_from_proximal);

        // Wait for both to complete
        let (telegram_result, sound_result) = tokio::join!(telegram_task, sound_task);

        // Log any errors but don't fail
        if let Err(e) = telegram_result {
            error!("📱 Inside Zone Telegram notification failed: {}", e);
        }

        if let Err(e) = sound_result {
            error!("🔊 Inside Zone sound notification failed: {}", e);
        }
    }

    /// Send all notifications for a validated trade signal
    pub async fn notify_trade_signal(&self, signal: &ValidatedTradeSignal) {
        info!(
            "📢 Sending notifications for {} {} @ {:.5}", 
            signal.action, signal.symbol, signal.price
        );

        // Use the enhanced telegram method but keep this function name
        let telegram_task = self.telegram.send_trade_signal(signal);
        let sound_task = self.sound.play_trade_signal(signal);

        // Wait for both to complete
        let (telegram_result, sound_result) = tokio::join!(telegram_task, sound_task);

        // Log any errors but don't fail
        if let Err(e) = telegram_result {
            error!("📱 Telegram notification failed: {}", e);
        }

        if let Err(e) = sound_result {
            error!("🔊 Sound notification failed: {}", e);
        }
    }

    /// Send notifications for blocked trade signals
    pub async fn notify_blocked_trade_signal(
        &self,
        notification: &TradeNotification,
        rejection_reason: &str,
    ) {
        info!(
            "📢 Sending BLOCKED TRADE notifications for {} {} @ {:.5} - Reason: {}", 
            notification.action, notification.symbol, notification.price, rejection_reason
        );

        // Send telegram notification for blocked trade
        let telegram_task = self.telegram.send_blocked_trade_signal(notification, rejection_reason);
        
        // Play a different sound for blocked trades
        let sound_task = self.sound.play_blocked_trade_alert(&notification.action, &notification.symbol);

        // Wait for both to complete
        let (telegram_result, sound_result) = tokio::join!(telegram_task, sound_task);

        // Log any errors but don't fail
        if let Err(e) = telegram_result {
            error!("📱 Blocked trade Telegram notification failed: {}", e);
        }

        if let Err(e) = sound_result {
            error!("🔊 Blocked trade sound notification failed: {}", e);
        }
    }

    /// Test all notification systems
    pub async fn test_notifications(&self) -> (bool, bool) {
        info!("🧪 Testing notification systems...");

        let telegram_ok = match self.telegram.send_test_message().await {
            Ok(()) => {
                info!("✅ Telegram test passed");
                true
            }
            Err(e) => {
                error!("❌ Telegram test failed: {}", e);
                false
            }
        };

        let sound_ok = match self.sound.play_test_sound().await {
            Ok(()) => {
                info!("✅ Sound test passed");
                true
            }
            Err(e) => {
                error!("❌ Sound test failed: {}", e);
                false
            }
        };

        (telegram_ok, sound_ok)
    }

    /// Send daily trading summary
    pub async fn send_daily_summary(&self, signals_today: u32, max_signals: u32) {
        if let Err(e) = self.telegram.send_daily_summary(signals_today, max_signals).await {
            error!("📱 Failed to send daily summary: {}", e);
        }
    }

    /// Play startup notification
    pub async fn notify_startup(&self) {
        info!("🚀 Sending startup notifications...");
        
        if let Err(e) = self.sound.play_startup_sound().await {
            error!("🔊 Startup sound failed: {}", e);
        }

        // Optionally send a startup message to Telegram
        // self.telegram.send_startup_message().await;
    }
}

// Global notification manager instance
use std::sync::LazyLock;
static GLOBAL_NOTIFICATION_MANAGER: LazyLock<
    std::sync::Mutex<Option<Arc<NotificationManager>>>
> = LazyLock::new(|| std::sync::Mutex::new(None));

pub fn get_global_notification_manager() -> Option<Arc<NotificationManager>> {
    GLOBAL_NOTIFICATION_MANAGER.lock().unwrap().clone()
}

pub fn set_global_notification_manager(manager: Arc<NotificationManager>) {
    *GLOBAL_NOTIFICATION_MANAGER.lock().unwrap() = Some(manager);
}