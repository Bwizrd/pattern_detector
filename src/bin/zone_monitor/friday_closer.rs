use chrono::{Datelike, Timelike, Utc};
use tokio::time::{sleep, Duration};
use reqwest::Client;
use tracing::{info, error};
use redis::AsyncCommands;
use futures::StreamExt;
use crate::deadman;

pub async fn delete_redis_keys_by_pattern(redis_url: &str, pattern: &str) -> redis::RedisResult<()> {
    let client = redis::Client::open(redis_url)?;
    let mut con = client.get_async_connection().await?;
    let mut iter = con.scan_match::<_, String>(pattern).await?;
    let mut keys = Vec::new();
    while let Some(key) = iter.next().await {
        keys.push(key);
    }
    drop(iter); // Explicitly drop the iterator before using con again
    for key in keys {
        let _: () = con.del(&key).await?;
        info!("Deleted Redis key: {}", key);
    }
    Ok(())
}

pub async fn friday_close_all_positions_task(api_url: &str) {
    let client = Client::new();
    loop {
        let now = Utc::now();
        // Friday = 5 (chrono: Monday=1, Sunday=7)
        if now.weekday().number_from_monday() == 5 && now.hour() == 21 && now.minute() == 55 {
            let endpoint = format!("{}/closeAllPositionsAndOrders", api_url.trim_end_matches('/'));
            match client.post(&endpoint).send().await {
                Ok(resp) if resp.status().is_success() => {
                    info!("Successfully triggered closeAllPositionsAndOrders at Friday 21:55.");
                }
                Ok(resp) => {
                    error!("Failed to close positions: HTTP {}", resp.status());
                }
                Err(e) => {
                    error!("Error sending closeAllPositionsAndOrders request: {}", e);
                }
            }
            // Delete Redis keys for a blank slate
            let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
            for pattern in &["pending_order:*", "order_lookup:*", "position_lookup:*"] {
                match delete_redis_keys_by_pattern(&redis_url, pattern).await {
                    Ok(_) => info!("Deleted all keys for pattern: {}", pattern),
                    Err(e) => error!("Failed to delete Redis keys for pattern {}: {}", pattern, e),
                }
            }
            // Set deadman switch to trading off
            let _ = deadman::set_deadman_status("trading off");
            // Sleep for 61 seconds to avoid double-triggering in the same minute
            sleep(Duration::from_secs(61)).await;
        } else {
            // Check every 30 seconds
            sleep(Duration::from_secs(30)).await;
        }
    }
}

pub async fn sunday_reset_deadman_task() {
    loop {
        let now = Utc::now();
        // Sunday = 7 (chrono: Monday=1, Sunday=7)
        if now.weekday().number_from_monday() == 7 && now.hour() == 22 && now.minute() == 0 {
            match deadman::set_deadman_status("trading on") {
                Ok(_) => info!("Deadman switch automatically reset to trading on at Sunday 22:00."),
                Err(e) => error!("Failed to reset deadman switch on Sunday: {}", e),
            }
            sleep(Duration::from_secs(61)).await;
        } else {
            sleep(Duration::from_secs(30)).await;
        }
    }
} 