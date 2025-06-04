// src/notification_manager.rs
use log::{error, info};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::trade_decision_engine::ValidatedTradeSignal;
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

    /// Send all notifications for a validated trade signal
    pub async fn notify_trade_signal(&self, signal: &ValidatedTradeSignal) {
        info!(
            "📢 Sending notifications for {} {} @ {:.5}", 
            signal.action, signal.symbol, signal.price
        );

        // Send notifications concurrently
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