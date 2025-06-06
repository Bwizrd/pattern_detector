// src/bin/zone_monitor/notifications.rs
// Simplified sound notification system

use crate::types::ZoneAlert;
use crate::sound_notifier::SoundNotifier;
use tracing::{info, warn};

#[derive(Debug)]
pub struct NotificationManager {
    enabled: bool,
    sound_notifier: SoundNotifier,
}

impl Clone for NotificationManager {
    fn clone(&self) -> Self {
        Self {
            enabled: self.enabled,
            sound_notifier: self.sound_notifier.clone(),
        }
    }
}

impl NotificationManager {
    pub fn new() -> Self {
        // More robust environment variable parsing
        let enabled_str = std::env::var("NOTIFICATIONS_ENABLED").unwrap_or_else(|_| "false".to_string());
        let enabled = enabled_str.to_lowercase() == "true" || enabled_str == "1";

        let sound_notifier = SoundNotifier::new();

        info!("📢 Notification Manager initialized:");
        info!("   Environment NOTIFICATIONS_ENABLED: {:?}", std::env::var("NOTIFICATIONS_ENABLED"));
        info!("   Parsed enabled value: {}", enabled);
        info!("   Sound notifications: {}", sound_notifier.is_enabled());

        Self {
            enabled,
            sound_notifier,
        }
    }

    // Main function to send zone proximity notification
    pub async fn notify_zone_proximity(&self, alert: &ZoneAlert) {
        if !self.enabled {
            return;
        }

        info!("📢 Zone proximity notification: {} {} zone @ {:.1} pips", 
              alert.symbol, alert.zone_type, alert.distance_pips);

        // Play sound notification
        self.sound_notifier.play_zone_proximity_alert(alert).await;
    }

    // Send test notification
    pub async fn send_test_notification(&self) -> bool {
        info!("📢 Test notification called - enabled: {}, sound enabled: {}", 
              self.enabled, self.sound_notifier.is_enabled());
              
        if !self.enabled {
            warn!("📢 Notifications are disabled (enabled={})", self.enabled);
            return false;
        }

        info!("📢 Sending test notification");
        self.sound_notifier.play_test_sound().await
    }

    // Play startup sound
    pub async fn play_startup_sound(&self) {
        if self.enabled {
            self.sound_notifier.play_startup_sound().await;
        }
    }

    // Check if notifications are properly configured
    pub fn is_configured(&self) -> bool {
        self.enabled && self.sound_notifier.is_enabled()
    }

    // Get status for health check
    pub fn get_status(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled,
            "sound_notifier": self.sound_notifier.get_status(),
        })
    }
}