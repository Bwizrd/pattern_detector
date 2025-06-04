// src/trade_decision_engine.rs - Filters and validates trade triggers
use crate::minimal_zone_cache::{TradeNotification, MinimalZoneCache}; // ← Fixed import
use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc; // ← Added import
use tokio::sync::Mutex; // ← Added import

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedTradeSignal {
    pub signal_id: String,
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub action: String, // BUY or SELL
    pub price: f64,
    pub zone_strength: f64,
    pub touch_count: i32,
    pub validation_timestamp: DateTime<Utc>,
    pub validation_reason: String,
}

#[derive(Debug, Clone)]
pub struct TradingRules {
    // Symbol filtering
    pub allowed_symbols: HashSet<String>,

    // Timeframe filtering
    pub allowed_timeframes: HashSet<String>,

    // Day filtering (0 = Sunday, 1 = Monday, etc.)
    pub allowed_weekdays: HashSet<u8>,

    // Zone quality filters
    pub min_touch_count: i32,
    pub max_touch_count: Option<i32>,
    pub min_zone_strength: f64,

    // Timing filters
    pub trading_start_hour_utc: Option<u8>,
    pub trading_end_hour_utc: Option<u8>,

    // Anti-spam protection
    pub zone_cooldown_minutes: u64, // Prevent re-triggering same zone

    // General controls
    pub trading_enabled: bool,
    pub max_daily_signals: u32,
}

impl TradingRules {
    pub fn from_env() -> Self {
        // Parse allowed symbols (comma-separated)
        let allowed_symbols: HashSet<String> = env::var("TRADING_ALLOWED_SYMBOLS")
            .unwrap_or_else(|_| "EURUSD,GBPUSD,USDJPY,USDCHF,AUDUSD,USDCAD".to_string())
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .collect();

        // Parse allowed timeframes (comma-separated)
        let allowed_timeframes: HashSet<String> = env::var("TRADING_ALLOWED_TIMEFRAMES")
            .unwrap_or_else(|_| "1h,4h,1d".to_string())
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .collect();

        // Parse allowed weekdays (comma-separated numbers: 1=Monday, 7=Sunday)
        let allowed_weekdays: HashSet<u8> = env::var("TRADING_ALLOWED_WEEKDAYS")
            .unwrap_or_else(|_| "1,2,3,4,5".to_string()) // Monday-Friday by default
            .split(',')
            .filter_map(|s| s.trim().parse::<u8>().ok())
            .filter(|&day| day >= 1 && day <= 7)
            .collect();

        let min_touch_count = env::var("TRADING_MIN_TOUCH_COUNT")
            .unwrap_or_else(|_| "1".to_string())
            .parse::<i32>()
            .unwrap_or(1);

        let max_touch_count = env::var("TRADING_MAX_TOUCH_COUNT")
            .ok()
            .and_then(|s| s.parse::<i32>().ok());

        let min_zone_strength = env::var("TRADING_MIN_ZONE_STRENGTH")
            .unwrap_or_else(|_| "70.0".to_string())
            .parse::<f64>()
            .unwrap_or(70.0);

        let trading_start_hour_utc = env::var("TRADING_START_HOUR_UTC")
            .ok()
            .and_then(|s| s.parse::<u8>().ok());

        let trading_end_hour_utc = env::var("TRADING_END_HOUR_UTC")
            .ok()
            .and_then(|s| s.parse::<u8>().ok());

        let zone_cooldown_minutes = env::var("TRADING_ZONE_COOLDOWN_MINUTES")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .unwrap_or(60);

        let trading_enabled = env::var("TRADING_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            .trim()
            .to_lowercase()
            == "true";

        let max_daily_signals = env::var("TRADING_MAX_DAILY_SIGNALS")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<u32>()
            .unwrap_or(5);

        Self {
            allowed_symbols,
            allowed_timeframes,
            allowed_weekdays,
            min_touch_count,
            max_touch_count,
            min_zone_strength,
            trading_start_hour_utc,
            trading_end_hour_utc,
            zone_cooldown_minutes,
            trading_enabled,
            max_daily_signals,
        }
    }

    pub fn log_current_settings(&self) {
        info!("🔧 [TRADE_RULES] Current Trading Rules:");
        info!(
            "🔧 [TRADE_RULES]   Trading Enabled: {}",
            self.trading_enabled
        );
        info!(
            "🔧 [TRADE_RULES]   Allowed Symbols: {:?}",
            self.allowed_symbols
        );
        info!(
            "🔧 [TRADE_RULES]   Allowed Timeframes: {:?}",
            self.allowed_timeframes
        );
        info!(
            "🔧 [TRADE_RULES]   Allowed Weekdays: {:?}",
            self.allowed_weekdays
        );
        info!(
            "🔧 [TRADE_RULES]   Touch Count: {} - {:?}",
            self.min_touch_count, self.max_touch_count
        );
        info!(
            "🔧 [TRADE_RULES]   Min Zone Strength: {}",
            self.min_zone_strength
        );
        info!(
            "🔧 [TRADE_RULES]   Trading Hours: {:?} - {:?}",
            self.trading_start_hour_utc, self.trading_end_hour_utc
        );
        info!(
            "🔧 [TRADE_RULES]   Zone Cooldown: {} minutes",
            self.zone_cooldown_minutes
        );
        info!(
            "🔧 [TRADE_RULES]   Max Daily Signals: {}",
            self.max_daily_signals
        );
    }
}

pub struct TradeDecisionEngine {
    rules: TradingRules,
    triggered_zones: HashMap<String, DateTime<Utc>>, // zone_id -> last_trigger_time
    daily_signal_count: u32,
    last_reset_date: DateTime<Utc>,
    validated_signals: Vec<ValidatedTradeSignal>,
    zone_cache: Option<Arc<Mutex<MinimalZoneCache>>>, // ← Fixed with proper imports
}

impl TradeDecisionEngine {
    pub fn new() -> Self {
        let rules = TradingRules::from_env();
        rules.log_current_settings();

        Self {
            rules,
            triggered_zones: HashMap::new(),
            daily_signal_count: 0,
            last_reset_date: Utc::now(),
            validated_signals: Vec::new(),
            zone_cache: None, // No cache by default
        }
    }

    pub fn new_with_cache(zone_cache: Arc<Mutex<MinimalZoneCache>>) -> Self {
        let rules = TradingRules::from_env();
        rules.log_current_settings();

        Self {
            rules,
            triggered_zones: HashMap::new(),
            daily_signal_count: 0,
            last_reset_date: Utc::now(),
            validated_signals: Vec::new(),
            zone_cache: Some(zone_cache), // ← Fixed
        }
    }

    pub fn reload_rules(&mut self) {
        self.rules = TradingRules::from_env();
        self.rules.log_current_settings();
        info!("🔄 [TRADE_ENGINE] Trading rules reloaded from environment");
    }

    // ← Fixed: Made async to handle the touch count check
    pub async fn process_notification(
        &mut self,
        notification: TradeNotification,
    ) -> Option<ValidatedTradeSignal> {
        // Reset daily counter if new day
        self.reset_daily_counter_if_needed();

        let validation_result = self.validate_notification(&notification).await; // ← Added await

        match validation_result {
            Ok(reason) => {
                let touch_count = self.get_zone_touch_count(&notification.zone_id).await; // ← Fixed
                let signal = self.create_validated_signal(notification, reason, touch_count); // ← Pass touch count
                info!(
                    "✅ [TRADE_ENGINE] VALIDATED SIGNAL: {} {} {}/{} @ {:.5}",
                    signal.action, signal.symbol, signal.timeframe, signal.zone_id, signal.price
                );

                // Track this zone as triggered
                self.triggered_zones
                    .insert(signal.zone_id.clone(), Utc::now());

                // Increment daily counter
                self.daily_signal_count += 1;

                // Store the signal
                self.validated_signals.insert(0, signal.clone());

                // Keep only last 50 signals
                if self.validated_signals.len() > 50 {
                    self.validated_signals.truncate(50);
                }

                Some(signal)
            }
            Err(reason) => {
                debug!(
                    "❌ [TRADE_ENGINE] REJECTED: {} {} {}/{} - {}",
                    notification.action,
                    notification.symbol,
                    notification.timeframe,
                    notification.zone_id,
                    reason
                );
                None
            }
        }
    }

    // ← Fixed: Made async
    async fn validate_notification(&self, notification: &TradeNotification) -> Result<String, String> {
        // Check if trading is enabled
        if !self.rules.trading_enabled {
            return Err("Trading disabled".to_string());
        }

        // Check daily limit
        if self.daily_signal_count >= self.rules.max_daily_signals {
            return Err(format!(
                "Daily limit reached: {}/{}",
                self.daily_signal_count, self.rules.max_daily_signals
            ));
        }

        // Check symbol allowlist
        if !self
            .rules
            .allowed_symbols
            .contains(&notification.symbol.to_uppercase())
        {
            return Err(format!(
                "Symbol {} not in allowed list",
                notification.symbol
            ));
        }

        // Check timeframe allowlist
        if !self
            .rules
            .allowed_timeframes
            .contains(&notification.timeframe.to_lowercase())
        {
            return Err(format!(
                "Timeframe {} not in allowed list",
                notification.timeframe
            ));
        }

        // Check weekday
        let current_weekday = Utc::now().weekday();
        let weekday_num = match current_weekday {
            Weekday::Mon => 1,
            Weekday::Tue => 2,
            Weekday::Wed => 3,
            Weekday::Thu => 4,
            Weekday::Fri => 5,
            Weekday::Sat => 6,
            Weekday::Sun => 7,
        };

        if !self.rules.allowed_weekdays.contains(&weekday_num) {
            return Err(format!("Trading not allowed on {}", current_weekday));
        }

        // Check trading hours
        if let (Some(start_hour), Some(end_hour)) = (
            self.rules.trading_start_hour_utc,
            self.rules.trading_end_hour_utc,
        ) {
            let current_hour = Utc::now().hour() as u8;
            if current_hour < start_hour || current_hour > end_hour {
                return Err(format!(
                    "Outside trading hours: {} (allowed: {}-{})",
                    current_hour, start_hour, end_hour
                ));
            }
        }

        // Check zone strength
        if notification.strength < self.rules.min_zone_strength {
            return Err(format!(
                "Zone strength {:.1} below minimum {:.1}",
                notification.strength, self.rules.min_zone_strength
            ));
        }

        // Check touch count - now properly async
        let touch_count = self.get_zone_touch_count(&notification.zone_id).await; // ← Fixed

        if touch_count < self.rules.min_touch_count {
            return Err(format!(
                "Touch count {} below minimum {}",
                touch_count, self.rules.min_touch_count
            ));
        }

        if let Some(max_touches) = self.rules.max_touch_count {
            if touch_count > max_touches {
                return Err(format!(
                    "Touch count {} above maximum {}",
                    touch_count, max_touches
                ));
            }
        }

        // Check zone cooldown (anti-spam)
        if let Some(last_trigger_time) = self.triggered_zones.get(&notification.zone_id) {
            let minutes_since_trigger = (Utc::now() - *last_trigger_time).num_minutes() as u64;
            if minutes_since_trigger < self.rules.zone_cooldown_minutes {
                return Err(format!(
                    "Zone in cooldown: {} minutes left",
                    self.rules.zone_cooldown_minutes - minutes_since_trigger
                ));
            }
        }

        Ok(format!("Validated: {} criteria passed", "all"))
    }

    // ← Fixed: Now properly async
    async fn get_zone_touch_count(&self, zone_id: &str) -> i32 {
        if let Some(cache) = &self.zone_cache {
            let cache_guard = cache.lock().await;
            // Search through zones to find matching zone_id
            let zones = cache_guard.get_all_zones();
            for zone in zones {
                if let Some(id) = &zone.zone_id {
                    if id == zone_id {
                        return zone.touch_count.unwrap_or(0) as i32;
                    }
                }
            }
        }
        0 // Zone not found or no cache
    }

    // ← Fixed: Added touch_count parameter
    fn create_validated_signal(
        &self,
        notification: TradeNotification,
        reason: String,
        touch_count: i32, // ← Added parameter
    ) -> ValidatedTradeSignal {
        ValidatedTradeSignal {
            signal_id: format!("SIGNAL_{}_{}", notification.zone_id, Utc::now().timestamp()),
            zone_id: notification.zone_id.clone(),
            symbol: notification.symbol.clone(),
            timeframe: notification.timeframe.clone(),
            action: notification.action.clone(),
            price: notification.price,
            zone_strength: notification.strength,
            touch_count, // ← Use the passed value
            validation_timestamp: Utc::now(),
            validation_reason: reason,
        }
    }

    fn reset_daily_counter_if_needed(&mut self) {
        let now = Utc::now();
        if now.date_naive() != self.last_reset_date.date_naive() {
            self.daily_signal_count = 0;
            self.last_reset_date = now;
            info!("🔄 [TRADE_ENGINE] Reset daily signal counter for new day");
        }
    }

    // Public getters
    pub fn get_validated_signals(&self) -> &Vec<ValidatedTradeSignal> {
        &self.validated_signals
    }

    pub fn get_daily_signal_count(&self) -> u32 {
        self.daily_signal_count
    }

    pub fn get_rules(&self) -> &TradingRules {
        &self.rules
    }

    pub fn clear_signals(&mut self) {
        self.validated_signals.clear();
    }

    // Clean up old triggered zones (call periodically)
    pub fn cleanup_old_triggers(&mut self) {
        let cutoff_time = Utc::now() - chrono::Duration::hours(24);
        self.triggered_zones
            .retain(|_, &mut trigger_time| trigger_time > cutoff_time);
    }

    pub async fn process_notification_with_reason(
        &mut self,
        notification: TradeNotification,
    ) -> Result<ValidatedTradeSignal, String> {
        // Reset daily counter if new day
        self.reset_daily_counter_if_needed();

        let validation_result = self.validate_notification(&notification).await;

        match validation_result {
            Ok(reason) => {
                let touch_count = self.get_zone_touch_count(&notification.zone_id).await;
                let signal = self.create_validated_signal(notification, reason, touch_count);
                
                // Track this zone as triggered
                self.triggered_zones
                    .insert(signal.zone_id.clone(), Utc::now());

                // Increment daily counter
                self.daily_signal_count += 1;

                // Store the signal
                self.validated_signals.insert(0, signal.clone());

                // Keep only last 50 signals
                if self.validated_signals.len() > 50 {
                    self.validated_signals.truncate(50);
                }

                Ok(signal)
            }
            Err(reason) => Err(reason),
        }
    }
}