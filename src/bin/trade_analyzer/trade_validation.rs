// trade_validation.rs - Exact same logic as main app's trade_decision_engine.rs
use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use log::debug;
use std::collections::{HashMap, HashSet};
use std::env;

#[derive(Debug, Clone)]
pub struct TradingRules {
    // Symbol filtering
    pub allowed_symbols: HashSet<String>,

    // Timeframe filtering
    pub allowed_timeframes: HashSet<String>,

    // Day filtering (0 = Sunday, 1 = Monday, etc.)
    pub allowed_weekdays: HashSet<u8>,

    // Zone quality filters
    pub max_touch_count: i32, // Maximum touches allowed (0 and 1 are valid if max is 1)
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
            .unwrap_or_else(|_| "EURUSD,GBPUSD,USDJPY,USDCHF,AUDUSD,USDCAD,NZDUSD,EURGBP,EURJPY,EURCHF,EURAUD,EURCAD,EURNZD,GBPJPY,GBPCHF,GBPAUD,GBPCAD,GBPNZD,AUDJPY,AUDNZD,AUDCAD,NZDJPY,CADJPY,CHFJPY".to_string())
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .collect();

        // Parse allowed timeframes (comma-separated)
        let allowed_timeframes: HashSet<String> = env::var("TRADING_ALLOWED_TIMEFRAMES")
            .unwrap_or_else(|_| "30m,1h,4h,1d".to_string())
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

        let max_touch_count = env::var("MAX_TOUCH_COUNT_FOR_TRADING")
            .unwrap_or_else(|_| "4".to_string()) // Match backtest API default
            .parse::<i32>()
            .unwrap_or(4);

        let min_zone_strength = env::var("TRADING_MIN_ZONE_STRENGTH")
            .unwrap_or_else(|_| "100.0".to_string()) // Match backtest API default
            .parse::<f64>()
            .unwrap_or(100.0);

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
            .unwrap_or_else(|_| "false".to_string())  // Match backtest API default
            .trim()
            .to_lowercase()
            == "true";

        let max_daily_signals = env::var("TRADING_MAX_DAILY_SIGNALS")
            .unwrap_or_else(|_| "10".to_string()) // Match backtest API default
            .parse::<u32>()
            .unwrap_or(10);

        Self {
            allowed_symbols,
            allowed_timeframes,
            allowed_weekdays,
            max_touch_count,
            min_zone_strength,
            trading_start_hour_utc,
            trading_end_hour_utc,
            zone_cooldown_minutes,
            trading_enabled,
            max_daily_signals,
        }
    }
}

pub struct BacktestTradeValidator {
    rules: TradingRules,
    triggered_zones: HashMap<String, DateTime<Utc>>, // zone_id -> last_trigger_time
    daily_signal_count: u32,
    last_reset_date: DateTime<Utc>,
    trades_per_symbol: HashMap<String, u32>, // symbol -> trade count
    trades_per_zone: HashMap<String, usize>, // zone_id -> trade count
}

impl BacktestTradeValidator {
    pub fn new() -> Self {
        let rules = TradingRules::from_env();
        
        Self {
            rules,
            triggered_zones: HashMap::new(),
            daily_signal_count: 0,
            last_reset_date: Utc::now(),
            trades_per_symbol: HashMap::new(),
            trades_per_zone: HashMap::new(),
        }
    }

    /// Validates if a trade should be taken at the given time with the given zone
    /// This is the EXACT same logic as your main app's trade_decision_engine
    pub fn validate_trade_signal(
        &mut self,
        zone_id: &str,
        symbol: &str,
        timeframe: &str,
        zone_strength: f64,
        touch_count: i32,
        current_time: DateTime<Utc>,
    ) -> Result<String, String> {
        // Reset daily counter if new day
        self.reset_daily_counter_if_needed(current_time);

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
        if !self.rules.allowed_symbols.contains(&symbol.to_uppercase()) {
            return Err(format!("Symbol {} not in allowed list", symbol));
        }

        // Check timeframe allowlist
        if !self.rules.allowed_timeframes.contains(&timeframe.to_lowercase()) {
            return Err(format!("Timeframe {} not in allowed list", timeframe));
        }

        // Check weekday
        let current_weekday = current_time.weekday();
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
            let current_hour = current_time.hour() as u8;
            if current_hour < start_hour || current_hour > end_hour {
                return Err(format!(
                    "Outside trading hours: {} (allowed: {}-{})",
                    current_hour, start_hour, end_hour
                ));
            }
        }

        // Check zone strength
        if zone_strength < self.rules.min_zone_strength {
            return Err(format!(
                "Zone strength {:.1} below minimum {:.1}",
                zone_strength, self.rules.min_zone_strength
            ));
        }

        // Check touch count - only reject if above maximum
        if touch_count > self.rules.max_touch_count {
            return Err(format!(
                "Touch count {} above maximum {}",
                touch_count, self.rules.max_touch_count
            ));
        }

        // Check zone cooldown (anti-spam)
        if let Some(last_trigger_time) = self.triggered_zones.get(zone_id) {
            let minutes_since_trigger = (current_time - *last_trigger_time).num_minutes() as u64;
            if minutes_since_trigger < self.rules.zone_cooldown_minutes {
                return Err(format!(
                    "Zone in cooldown: {} minutes left",
                    self.rules.zone_cooldown_minutes - minutes_since_trigger
                ));
            }
        }

        // Check max trades per symbol
        let symbol_trades = self.trades_per_symbol.entry(symbol.to_string()).or_insert(0);
        if *symbol_trades >= 2 { // Default max trades per symbol
            return Err(format!("Max trades per symbol reached for {}", symbol));
        }

        // Check max trades per zone
        let zone_trades = self.trades_per_zone.entry(zone_id.to_string()).or_insert(0);
        if *zone_trades >= 3 { // Default max trades per zone
            return Err(format!("Max trades per zone reached for {}", zone_id));
        }

        // All checks passed - update counters and mark zone as triggered
        self.triggered_zones.insert(zone_id.to_string(), current_time);
        self.daily_signal_count += 1;
        *symbol_trades += 1;
        *zone_trades += 1;

        Ok("All validation criteria passed".to_string())
    }

    fn reset_daily_counter_if_needed(&mut self, current_time: DateTime<Utc>) {
        if current_time.date_naive() != self.last_reset_date.date_naive() {
            self.daily_signal_count = 0;
            self.last_reset_date = current_time;
            self.trades_per_symbol.clear();
            self.trades_per_zone.clear();
            debug!("🔄 [BACKTEST_VALIDATOR] Reset daily counters for new day");
        }
    }

    pub fn get_daily_signal_count(&self) -> u32 {
        self.daily_signal_count
    }

    pub fn get_rules(&self) -> &TradingRules {
        &self.rules
    }
}