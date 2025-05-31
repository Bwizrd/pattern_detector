// src/realtime_cache_updater.rs - Real-time cache update service
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::interval;
use log::{info, warn, error};

use crate::minimal_zone_cache::MinimalZoneCache;

pub struct RealtimeCacheUpdater {
    cache: Arc<Mutex<MinimalZoneCache>>,
    update_interval: Duration,
    is_running: Arc<tokio::sync::Mutex<bool>>,
}

impl RealtimeCacheUpdater {
    pub fn new(cache: Arc<Mutex<MinimalZoneCache>>) -> Self {
        Self {
            cache,
            update_interval: Duration::from_secs(60), // 1 minute
            is_running: Arc::new(tokio::sync::Mutex::new(false)),
        }
    }

    pub fn with_interval(mut self, seconds: u64) -> Self {
        self.update_interval = Duration::from_secs(seconds);
        self
    }

    /// Start the real-time cache update loop
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut is_running = self.is_running.lock().await;
        if *is_running {
            warn!("🔄 [CACHE_UPDATER] Already running");
            return Ok(());
        }
        *is_running = true;
        drop(is_running);

        info!("🚀 [CACHE_UPDATER] Starting real-time cache updates every {:?}", self.update_interval);

        let cache_clone = Arc::clone(&self.cache);
        let interval_duration = self.update_interval;
        let running_flag = Arc::clone(&self.is_running);

        tokio::spawn(async move {
            let mut ticker = interval(interval_duration);
            let mut update_count = 0u64;

            loop {
                // Check if we should stop
                {
                    let is_running = running_flag.lock().await;
                    if !*is_running {
                        info!("🛑 [CACHE_UPDATER] Stopping update loop");
                        break;
                    }
                }

                ticker.tick().await;
                update_count += 1;

                let start_time = std::time::Instant::now();
                info!("🔄 [CACHE_UPDATER] Starting update #{}", update_count);

                // Acquire cache lock with timeout
                match tokio::time::timeout(Duration::from_secs(30), cache_clone.lock()).await {
                    Ok(mut cache_guard) => {
                        match cache_guard.refresh_zones().await {
                            Ok(()) => {
                                let duration = start_time.elapsed();
                                let (total, supply, demand) = cache_guard.get_stats();
                                info!(
                                    "✅ [CACHE_UPDATER] Update #{} completed in {:.2}s: {} zones ({} supply, {} demand)",
                                    update_count,
                                    duration.as_secs_f64(),
                                    total,
                                    supply,
                                    demand
                                );
                            }
                            Err(e) => {
                                error!("❌ [CACHE_UPDATER] Update #{} failed: {}", update_count, e);
                            }
                        }
                        drop(cache_guard); // Explicitly release lock
                    }
                    Err(_) => {
                        error!("⏰ [CACHE_UPDATER] Update #{} timeout - cache lock unavailable", update_count);
                    }
                }
            }

            info!("🏁 [CACHE_UPDATER] Update loop ended after {} updates", update_count);
        });

        Ok(())
    }

    /// Stop the real-time cache updates
    pub async fn stop(&self) {
        let mut is_running = self.is_running.lock().await;
        *is_running = false;
        info!("🛑 [CACHE_UPDATER] Stop requested");
    }

    /// Check if the updater is currently running
    pub async fn is_running(&self) -> bool {
        *self.is_running.lock().await
    }

    /// Force an immediate cache update
    pub async fn update_now(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let start_time = std::time::Instant::now();
        info!("⚡ [CACHE_UPDATER] Manual update requested");

        match tokio::time::timeout(Duration::from_secs(30), self.cache.lock()).await {
            Ok(mut cache_guard) => {
                cache_guard.refresh_zones().await?;
                let duration = start_time.elapsed();
                let (total, supply, demand) = cache_guard.get_stats();
                info!(
                    "✅ [CACHE_UPDATER] Manual update completed in {:.2}s: {} zones ({} supply, {} demand)",
                    duration.as_secs_f64(),
                    total,
                    supply,
                    demand
                );
                Ok(())
            }
            Err(_) => {
                let error_msg = "Cache lock timeout during manual update";
                error!("⏰ [CACHE_UPDATER] {}", error_msg);
                Err(error_msg.into())
            }
        }
    }
}