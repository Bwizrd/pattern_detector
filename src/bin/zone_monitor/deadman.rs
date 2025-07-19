use std::fs;
use std::path::Path;

const DEADMAN_FILE: &str = "deadman_switch.txt";

pub fn get_deadman_status() -> String {
    match fs::read_to_string(DEADMAN_FILE) {
        Ok(s) => {
            let trimmed = s.trim().to_lowercase();
            if trimmed == "trading on" || trimmed == "on" {
                "trading on".to_string()
            } else {
                "trading off".to_string()
            }
        }
        Err(_) => "trading on".to_string(),
    }
}

pub fn set_deadman_status(status: &str) -> std::io::Result<()> {
    let normalized = match status.trim().to_lowercase().as_str() {
        "on" | "trading on" => "trading on",
        _ => "trading off",
    };
    fs::write(DEADMAN_FILE, normalized)
}

pub fn is_trading_enabled() -> bool {
    get_deadman_status() == "trading on"
} 