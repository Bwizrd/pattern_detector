// src/bin/zone_monitor/sound_notifier.rs
// Sound notification system for zone proximity alerts

use crate::types::ZoneAlert;
use std::process::Command;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct SoundNotifier {
    enabled: bool,
    proximity_sound: String,
    supply_sound: String,
    demand_sound: String,
}

impl SoundNotifier {
    pub fn new() -> Self {
        let enabled = std::env::var("ENABLE_SOUND_NOTIFICATIONS")
            .unwrap_or_else(|_| "true".to_string())
            .trim()
            .to_lowercase()
            == "true";

        // macOS system sounds
        let proximity_sound = std::env::var("PROXIMITY_SOUND_FILE")
            .unwrap_or_else(|_| "/System/Library/Sounds/Tink.aiff".to_string());
            
        let supply_sound = std::env::var("SUPPLY_SOUND_FILE")
            .unwrap_or_else(|_| "/System/Library/Sounds/Glass.aiff".to_string());
            
        let demand_sound = std::env::var("DEMAND_SOUND_FILE")
            .unwrap_or_else(|_| "/System/Library/Sounds/Purr.aiff".to_string());

        if enabled {
            info!("🔊 Sound notifier initialized for macOS");
            info!("   Proximity sound: {}", proximity_sound);
            info!("   Supply zone sound: {}", supply_sound);
            info!("   Demand zone sound: {}", demand_sound);
        } else {
            info!("🔊 Sound notifications disabled");
        }

        Self {
            enabled,
            proximity_sound,
            supply_sound,
            demand_sound,
        }
    }

    // Main function to play zone proximity alert sound
    pub async fn play_zone_proximity_alert(&self, alert: &ZoneAlert) {
        if !self.enabled {
            return;
        }

        let sound_file = match alert.zone_type.as_str() {
            "supply" => &self.supply_sound,
            "demand" => &self.demand_sound,
            _ => &self.proximity_sound,
        };

        if let Err(e) = self.play_sound_file(sound_file).await {
            error!("🔊 Failed to play proximity alert sound: {}", e);
        } else {
            info!("🔊 Played {} zone proximity alert for {} @ {:.1} pips (Zone: {})", 
                  alert.zone_type, alert.symbol, alert.distance_pips, alert.zone_id);
        }
    }

    // Play test sound
    pub async fn play_test_sound(&self) -> bool {
        if !self.enabled {
            warn!("🔊 Sound notifications disabled");
            return false;
        }

        let test_sound = "/System/Library/Sounds/Ping.aiff";
        
        match self.play_sound_file(test_sound).await {
            Ok(_) => {
                info!("🔊 Test sound played successfully");
                true
            }
            Err(e) => {
                error!("🔊 Failed to play test sound: {}", e);
                false
            }
        }
    }

    // Play startup sound
    pub async fn play_startup_sound(&self) {
        if !self.enabled {
            return;
        }

        let startup_sound = "/System/Library/Sounds/Ping.aiff";
        
        if let Err(e) = self.play_sound_file(startup_sound).await {
            error!("🔊 Failed to play startup sound: {}", e);
        } else {
            info!("🔊 Zone monitor startup sound played");
        }
    }

    // Internal function to play sound files using macOS afplay
    async fn play_sound_file(&self, sound_file: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        let result = Command::new("afplay")
            .arg(sound_file)
            .output();

        match result {
            Ok(output) => {
                if !output.status.success() {
                    let error = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("Sound playback failed: {}", error).into());
                }
            }
            Err(e) => {
                return Err(format!("Failed to execute afplay command: {}", e).into());
            }
        }

        Ok(())
    }

    // Check if sound notifications are enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    // Get status for health check
    pub fn get_status(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled,
            "platform": "macOS",
            "proximity_sound": self.proximity_sound,
            "supply_sound": self.supply_sound,
            "demand_sound": self.demand_sound,
        })
    }
}