// src/bin/zone_monitor/trade_rules.rs
// Centralized trade rules engine used by all services for consistency

use crate::types::{Zone, PriceUpdate};
use std::env;
use tracing::{debug, info};
use chrono::Timelike;

#[derive(Debug, Clone)]
pub struct TradeRulesConfig {
    pub proximity_threshold_pips: f64,
    pub trading_threshold_pips: f64,
    pub max_touch_count: i32,
    pub min_zone_strength: f64,
    pub allowed_symbols: Vec<String>,
    pub allowed_timeframes: Vec<String>,
    pub excluded_timeframes: Vec<String>,
    pub trading_start_hour: u8,
    pub trading_end_hour: u8,
    pub min_risk_reward: f64,
}

impl TradeRulesConfig {
    pub fn from_env() -> Self {
        let allowed_symbols = env::var("TRADING_ALLOWED_SYMBOLS")
            .unwrap_or_else(|_| "EURUSD,GBPUSD,USDJPY,USDCHF,AUDUSD,USDCAD,NZDUSD,EURGBP,EURJPY,EURCHF,EURAUD,EURCAD,EURNZD,GBPJPY,GBPCHF,GBPAUD,GBPCAD,GBPNZD,AUDJPY,AUDNZD,AUDCAD,NZDJPY,CADJPY,CHFJPY".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let allowed_timeframes = env::var("TRADING_ALLOWED_TIMEFRAMES")
            .unwrap_or_else(|_| "5m,15m,30m,1h,4h,1d".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let excluded_timeframes = env::var("EXCLUDED_PROXIMITY_TIMEFRAMES")
            .unwrap_or_else(|_| "".to_string())
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .collect();

        Self {
            proximity_threshold_pips: env::var("PROXIMITY_THRESHOLD_PIPS")
                .unwrap_or_else(|_| "10.0".to_string())
                .parse()
                .unwrap_or(10.0),
            trading_threshold_pips: env::var("TRADING_THRESHOLD_PIPS")
                .unwrap_or_else(|_| "0.0".to_string())
                .parse()
                .unwrap_or(0.0),
            max_touch_count: env::var("MAX_TOUCH_COUNT_FOR_TRADING")
                .unwrap_or_else(|_| "4".to_string())
                .parse()
                .unwrap_or(4),
            min_zone_strength: env::var("TRADING_MIN_ZONE_STRENGTH")
                .unwrap_or_else(|_| "80.0".to_string())
                .parse()
                .unwrap_or(80.0),
            allowed_symbols,
            allowed_timeframes,
            excluded_timeframes,
            trading_start_hour: env::var("TRADING_START_HOUR")
                .unwrap_or_else(|_| "0".to_string())
                .parse()
                .unwrap_or(0),
            trading_end_hour: env::var("TRADING_END_HOUR")
                .unwrap_or_else(|_| "24".to_string())
                .parse()
                .unwrap_or(24),
            min_risk_reward: env::var("TRADING_MIN_RISK_REWARD")
                .unwrap_or_else(|_| "0.1".to_string())
                .parse()
                .unwrap_or(0.1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeEvaluation {
    pub should_trigger_trade: bool,
    pub is_in_proximity: bool,
    pub distance_pips: f64,
    pub rejection_reasons: Vec<String>,
    pub trade_direction: Option<String>, // "BUY" or "SELL"
}

#[derive(Debug)]
pub struct TradeRulesEngine {
    config: TradeRulesConfig,
}

impl TradeRulesEngine {
    pub fn new() -> Self {
        let config = TradeRulesConfig::from_env();
        info!("🎯 Trade Rules Engine initialized:");
        info!("   Proximity threshold: {:.1} pips", config.proximity_threshold_pips);
        info!("   Trading threshold: {:.1} pips", config.trading_threshold_pips);
        info!("   Max touch count: {}", config.max_touch_count);
        info!("   Min zone strength: {:.1}", config.min_zone_strength);
        info!("   Allowed symbols: {}", config.allowed_symbols.len());
        info!("   Allowed timeframes: {:?}", config.allowed_timeframes);
        info!("   Excluded timeframes: {:?}", config.excluded_timeframes);

        Self { config }
    }

    /// Evaluate if a zone should trigger a trade given current price and zone interaction data
    pub fn evaluate_trade_opportunity(
        &self,
        price_update: &PriceUpdate,
        zone: &Zone,
        zone_interactions: Option<&pattern_detector::zone_interactions::ZoneInteractionContainer>,
    ) -> TradeEvaluation {
        let mut rejection_reasons = Vec::new();
        let current_price = (price_update.bid + price_update.ask) / 2.0;

        // Determine zone type and direction
        let is_supply = zone.zone_type.to_lowercase().contains("supply");
        let trade_direction = if is_supply { "SELL" } else { "BUY" };

        // Calculate proximal line (entry point)
        let proximal_price = if is_supply {
            zone.low  // Supply zones: enter when price reaches low (proximal)
        } else {
            zone.high // Demand zones: enter when price reaches high (proximal)
        };

        // Calculate distance in pips
        let distance_pips = self.calculate_pips_distance(&price_update.symbol, current_price, proximal_price);

        // Check if in proximity
        let is_in_proximity = distance_pips <= self.config.proximity_threshold_pips;

        // Check basic zone eligibility
        if !zone.is_active {
            rejection_reasons.push("zone_not_active".to_string());
        }

        // Check symbol allowlist
        if !self.config.allowed_symbols.contains(&price_update.symbol) {
            rejection_reasons.push(format!("symbol_not_allowed_{}", price_update.symbol));
        }

        // Check timeframe allowlist
        if !self.config.allowed_timeframes.contains(&zone.timeframe) {
            rejection_reasons.push(format!("timeframe_not_allowed_{}", zone.timeframe));
        }

        // Check excluded timeframes
        if self.config.excluded_timeframes.contains(&zone.timeframe) {
            rejection_reasons.push(format!("timeframe_excluded_{}", zone.timeframe));
        }

        // Check touch count limit
        if zone.touch_count > self.config.max_touch_count {
            rejection_reasons.push(format!("touch_count_too_high_{}_{}", zone.touch_count, self.config.max_touch_count));
        }

        // Check minimum zone strength
        if zone.strength < self.config.min_zone_strength {
            rejection_reasons.push(format!("zone_strength_too_low_{:.1}_{:.1}", zone.strength, self.config.min_zone_strength));
        }

        // Check trading hours
        let now = chrono::Utc::now();
        let hour = now.hour() as u8;
        if hour < self.config.trading_start_hour || hour > self.config.trading_end_hour {
            rejection_reasons.push(format!("outside_trading_hours_{}h", hour));
        }

        // Check if within trading threshold
        let within_trading_threshold = distance_pips <= self.config.trading_threshold_pips;
        if !within_trading_threshold {
            rejection_reasons.push(format!("not_within_trading_threshold_{:.1}_{:.1}", distance_pips, self.config.trading_threshold_pips));
        }

        // Check first-time entry using zone interaction data
        let (has_ever_entered, zone_entries) = if let Some(interactions) = zone_interactions {
            if let Some(metrics) = interactions.metrics.get(&zone.id) {
                (metrics.has_ever_entered, metrics.zone_entries)
            } else {
                (false, 0)
            }
        } else {
            (false, 0)
        };

        let is_first_time_entry = !has_ever_entered && zone_entries == 0;
        if !is_first_time_entry {
            rejection_reasons.push(format!("not_first_time_entry_{}_{}", has_ever_entered, zone_entries));
        }

        // Trade should trigger if all conditions are met
        let should_trigger_trade = rejection_reasons.is_empty() && within_trading_threshold && is_first_time_entry;

        debug!("🎯 Trade evaluation for {} {} zone {}: trigger={}, proximity={}, distance={:.1}pips, reasons={:?}",
               price_update.symbol, zone.zone_type, zone.id, should_trigger_trade, is_in_proximity, distance_pips, rejection_reasons);

        TradeEvaluation {
            should_trigger_trade,
            is_in_proximity,
            distance_pips,
            rejection_reasons,
            trade_direction: Some(trade_direction.to_string()),
        }
    }

    /// Get pip value for a symbol
    fn calculate_pips_distance(&self, symbol: &str, price1: f64, price2: f64) -> f64 {
        let pip_value = self.get_pip_value_for_symbol(symbol);
        (price1 - price2).abs() / pip_value
    }

    /// Get pip value for different currency pairs
    fn get_pip_value_for_symbol(&self, symbol: &str) -> f64 {
        // JPY pairs use 0.01 as pip value
        if symbol.contains("JPY") {
            0.01
        } else if symbol.ends_with("_SB") {
            // All SB suffixed pairs use 0.0001 (except JPY which is handled above)
            0.0001
        } else {
            // Legacy support for non-SB symbols
            match symbol {
                // Indices
                "NAS100" => 1.0,
                "US500" => 0.1,
                // All other pairs use 0.0001
                _ => 0.0001,
            }
        }
    }

    /// Get the current configuration
    pub fn get_config(&self) -> &TradeRulesConfig {
        &self.config
    }

    /// Reload configuration from environment variables
    pub fn reload_config(&mut self) {
        self.config = TradeRulesConfig::from_env();
        info!("🔄 Trade rules configuration reloaded from environment");
    }
}