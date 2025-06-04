// src/sound_notifier.rs
use log::{error, info, warn};
use std::env;
use std::process::Command;
use crate::trade_decision_engine::ValidatedTradeSignal;

pub struct SoundNotifier {
    enabled: bool,
    sound_command: Option<String>,
    buy_sound: String,
    sell_sound: String,
}

impl SoundNotifier {
    pub fn new() -> Self {
        let enabled = env::var("ENABLE_SOUND_NOTIFICATIONS")
            .unwrap_or_else(|_| "true".to_string())
            .trim()
            .to_lowercase()
            == "true";

        // Detect platform and set appropriate sound command
        let sound_command = if cfg!(target_os = "windows") {
            // Windows - uses PowerShell to play system sounds
            Some("powershell".to_string())
        } else if cfg!(target_os = "macos") {
            // macOS - uses afplay
            Some("afplay".to_string())
        } else if cfg!(target_os = "linux") {
            // Linux - try different audio systems
            if which_command_exists("paplay") {
                Some("paplay".to_string())
            } else if which_command_exists("aplay") {
                Some("aplay".to_string())
            } else if which_command_exists("ffplay") {
                Some("ffplay".to_string())
            } else {
                warn!("🔊 No audio player found on Linux system");
                None
            }
        } else {
            None
        };

        let buy_sound = env::var("BUY_SOUND_FILE").unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "SystemAsterisk".to_string() // Windows system sound
            } else {
                "sounds/buy_signal.wav".to_string() // Custom sound file
            }
        });

        let sell_sound = env::var("SELL_SOUND_FILE").unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "SystemExclamation".to_string() // Windows system sound
            } else {
                "sounds/sell_signal.wav".to_string() // Custom sound file
            }
        });

        if enabled && sound_command.is_some() {
            info!("🔊 Sound notifier initialized for {:?}", std::env::consts::OS);
        } else if enabled {
            warn!("🔊 Sound notifier disabled - no audio player available");
        }

        Self {
            enabled: enabled && sound_command.is_some(),
            sound_command,
            buy_sound,
            sell_sound,
        }
    }

    pub async fn play_trade_signal(&self, signal: &ValidatedTradeSignal) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        let sound_file = if signal.action == "BUY" {
            &self.buy_sound
        } else {
            &self.sell_sound
        };

        self.play_sound(sound_file).await?;
        
        info!("🔊 Played {} sound for {} @ {:.5}", 
              signal.action.to_lowercase(), signal.symbol, signal.price);

        Ok(())
    }

    pub async fn play_sound(&self, sound_file: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        let command = self.sound_command.as_ref().unwrap();

        let result = if cfg!(target_os = "windows") {
            // Windows PowerShell command to play system sounds or files
            if sound_file.starts_with("System") {
                Command::new(command)
                    .args(&[
                        "-Command",
                        &format!("[System.Media.SystemSounds]::{}::Play()", sound_file)
                    ])
                    .output()
            } else {
                Command::new(command)
                    .args(&[
                        "-Command",
                        &format!("(New-Object Media.SoundPlayer '{}').PlaySync()", sound_file)
                    ])
                    .output()
            }
        } else if cfg!(target_os = "macos") {
            // macOS afplay command
            Command::new(command)
                .arg(sound_file)
                .output()
        } else {
            // Linux audio players
            match command.as_str() {
                "paplay" => Command::new(command).arg(sound_file).output(),
                "aplay" => Command::new(command).arg(sound_file).output(),
                "ffplay" => Command::new(command)
                    .args(&["-nodisp", "-autoexit", sound_file])
                    .output(),
                _ => return Err("Unsupported audio command".into()),
            }
        };

        match result {
            Ok(output) => {
                if !output.status.success() {
                    let error = String::from_utf8_lossy(&output.stderr);
                    error!("🔊 Sound playback failed: {}", error);
                }
            }
            Err(e) => {
                error!("🔊 Failed to execute sound command: {}", e);
            }
        }

        Ok(())
    }

    pub async fn play_test_sound(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Err("Sound notifier not enabled".into());
        }

        // Play a test sound
        let test_sound = if cfg!(target_os = "windows") {
            "SystemAsterisk"
        } else {
            &self.buy_sound
        };

        self.play_sound(test_sound).await?;
        info!("🔊 Test sound played successfully");
        Ok(())
    }

    pub async fn play_startup_sound(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled {
            return Ok(());
        }

        let startup_sound = if cfg!(target_os = "windows") {
            "SystemStart"
        } else {
            "sounds/startup.wav"
        };

        if let Err(e) = self.play_sound(startup_sound).await {
            warn!("🔊 Failed to play startup sound: {}", e);
        } else {
            info!("🔊 Trading bot startup sound played");
        }

        Ok(())
    }
}

// Helper function to check if a command exists on the system
fn which_command_exists(command: &str) -> bool {
    Command::new("which")
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}