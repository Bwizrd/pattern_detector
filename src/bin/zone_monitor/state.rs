// src/bin/zone_monitor/state.rs
// Core state management with zone state tracking, trading, and CSV logging

use crate::active_order_manager::ActiveOrderManager;
use crate::csv_logger::CsvLogger;
use crate::db::OrderDatabase;
use crate::notifications::NotificationManager;
use crate::pending_order_manager::PendingOrderManager;
use crate::proximity::ProximityDetector;
use crate::proximity_logger::ProximityLogger;
use crate::strategy_manager::StrategyManager;
use crate::telegram_notifier::TelegramNotifier;
use crate::trading_engine::TradingEngine;
use crate::types::{PriceUpdate, ZoneAlert, ZoneCache};
use crate::websocket::WebSocketClient;
use crate::zone_state_manager::ProcessResult;
use crate::zone_state_manager::ZoneStateManager;
use chrono::Timelike;
use chrono::{DateTime, Utc};
use pattern_detector::zone_interactions::{InteractionConfig, ZoneInteractionContainer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Get pip value for a symbol (helper function)
fn get_pip_value(symbol: &str) -> f64 {
    match symbol {
        // JPY pairs use 0.01 as pip value
        s if s.ends_with("JPY") => 0.01,
        // Indices
        "NAS100" => 1.0,
        "US500" => 0.1,
        // All other pairs use 0.0001
        _ => 0.0001,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedNotification {
    pub id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub symbol: String,
    pub timeframe: String,
    pub action: String, // "BUY" or "SELL"
    pub price: f64,
    pub zone_id: String,
    pub zone_type: String, // "proximity_alert" or "trading_signal"
    pub distance_pips: f64,
    pub strength: f64,
    pub touch_count: i32,

    // These are new.
    pub trade_attempted: Option<bool>, // Was a trade attempt made?
    pub trade_executed: Option<bool>,  // Was the trade successfully executed?
    pub execution_result: Option<String>, // "success", "failed", "rejected"
    pub rejection_reason: Option<String>, // "daily_limit", "symbol_limit", "api_error", "rule_failed"
    pub ctrader_order_id: Option<String>, // Order ID from cTrader
    pub attempted_at: Option<DateTime<Utc>>, // When the trade attempt was made

    // NEW: Filtering and notification status fields
    pub telegram_sent: Option<bool>,     // Was this sent to Telegram?
    pub telegram_filtered: Option<bool>, // Was this filtered from Telegram?
    pub telegram_filter_reason: Option<String>, // Why was it filtered?
    pub telegram_response_code: Option<u16>, // HTTP response code from Telegram
    pub telegram_error: Option<String>,  // Error message if Telegram failed

    // Trading rule evaluation (for transparency)
    pub passes_symbol_filter: bool,       // Is symbol allowed?
    pub passes_timeframe_filter: bool,    // Is timeframe allowed?
    pub passes_touch_count_filter: bool,  // Touch count within limits?
    pub passes_trading_hours: bool,       // Within trading hours?
    pub passes_daily_limit: bool,         // Under daily trade limit?
    pub passes_symbol_limit: bool,        // Under per-symbol limit?
    pub risk_reward_ratio: Option<f64>,   // Calculated R:R ratio
    pub passes_risk_reward: Option<bool>, // Meets minimum R:R?
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SharedNotificationsFile {
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub notifications: VecDeque<SharedNotification>,
}

#[derive(Clone)]
pub struct MonitorState {
    pub zone_cache: Arc<RwLock<ZoneCache>>,
    pub latest_prices: Arc<RwLock<HashMap<String, PriceUpdate>>>,
    pub recent_alerts: Arc<RwLock<Vec<ZoneAlert>>>,
    pub websocket_client: Arc<WebSocketClient>,
    pub proximity_detector: Arc<ProximityDetector>,
    pub notification_manager: Arc<NotificationManager>,
    pub zone_state_manager: Arc<ZoneStateManager>,
    pub trading_engine: Arc<TradingEngine>,
    pub csv_logger: Arc<CsvLogger>,
    pub telegram_notifier: Arc<TelegramNotifier>,
    pub proximity_logger: Arc<RwLock<ProximityLogger>>,
    pub pending_order_manager: Arc<RwLock<PendingOrderManager>>,
    pub strategy_manager: Arc<StrategyManager>,
    pub active_order_manager: Arc<ActiveOrderManager>,
    pub zone_interactions: Arc<RwLock<ZoneInteractionContainer>>,
    pub interaction_config: InteractionConfig,
    pub cache_file_path: String,
    pub websocket_url: String,
    pub order_database: Arc<OrderDatabase>,
}

impl MonitorState {
    pub fn new(cache_file_path: String) -> Self {
        let websocket_url =
            std::env::var("PRICE_WS_URL").unwrap_or_else(|_| "ws://localhost:8083".to_string());

        // Get proximity threshold for the detector (this will be deprecated)
        let proximity_threshold = std::env::var("PROXIMITY_THRESHOLD_PIPS")
            .unwrap_or_else(|_| "10.0".to_string())
            .parse()
            .unwrap_or(10.0);

        // Initialize zone interaction tracking
        let interaction_config = InteractionConfig::default();
        let zone_interactions = ZoneInteractionContainer::load_from_file(
            &interaction_config.file_path,
        )
        .unwrap_or_else(|e| {
            tracing::warn!("Failed to load zone interactions: {}, starting fresh", e);
            ZoneInteractionContainer::new()
        });

        // Initialize InfluxDB order database
        let influx_host =
            std::env::var("INFLUXDB_HOST").unwrap_or_else(|_| "http://localhost:8086".to_string());
        let influx_org =
            std::env::var("INFLUXDB_ORG").unwrap_or_else(|_| "pattern_detector".to_string());
        let influx_token = std::env::var("INFLUXDB_TOKEN")
            .unwrap_or_else(|_| "dev-token-please-change".to_string());
        let influx_bucket = "trading_orders".to_string();

        let order_database =
            OrderDatabase::new(influx_host, influx_org, influx_token, influx_bucket);

        Self {
            zone_cache: Arc::new(RwLock::new(ZoneCache::default())),
            latest_prices: Arc::new(RwLock::new(HashMap::new())),
            recent_alerts: Arc::new(RwLock::new(Vec::new())),
            websocket_client: Arc::new(WebSocketClient::new()),
            proximity_detector: Arc::new(ProximityDetector::new(proximity_threshold)),
            notification_manager: Arc::new(NotificationManager::new()),
            zone_state_manager: Arc::new(ZoneStateManager::new(Arc::new(CsvLogger::new()))),
            trading_engine: Arc::new(TradingEngine::new()),
            csv_logger: Arc::new(CsvLogger::new()),
            telegram_notifier: Arc::new(TelegramNotifier::new()),
            proximity_logger: Arc::new(RwLock::new(ProximityLogger::new())),
            pending_order_manager: Arc::new(RwLock::new(PendingOrderManager::new())),
            strategy_manager: Arc::new(StrategyManager::new()),
            active_order_manager: Arc::new(ActiveOrderManager::new()),
            zone_interactions: Arc::new(RwLock::new(zone_interactions)),
            interaction_config,
            cache_file_path,
            websocket_url,
            order_database: Arc::new(order_database),
        }
    }

    pub async fn initialize(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("🚀 MonitorState::initialize() starting...");

        // Load initial zones from file
        info!("📄 Loading zones from cache file...");
        self.load_zones_from_file().await?;
        info!("✅ Zones loaded successfully");

        // Initialize pending order manager with InfluxDB connection (Redis-first, no JSON loading)
        info!("📋 Initializing pending order manager...");
        {
            let mut pending_order_manager = self.pending_order_manager.write().await;
            
            // Connect to InfluxDB for deal enrichment
            pending_order_manager.set_order_database(self.order_database.clone());
            
            // Check Redis stats
            let (pending_orders_count, lookup_keys_count) = pending_order_manager.get_redis_stats().await;
            info!("📊 Redis Stats: {} pending orders, {} lookup keys", pending_orders_count, lookup_keys_count);
            
            if pending_order_manager.is_enabled() {
                info!("✅ Pending Order Manager enabled and connected to Redis");
            } else {
                info!("⏸️ Pending Order Manager disabled");
            }
        }

        // Initialize active order manager (Redis-only, no InfluxDB connection needed)
        info!("📋 Initializing ActiveOrderManager for transitory Redis storage...");
        
        // Check Redis stats for active trades
        let (active_trades_count, _) = self.active_order_manager.get_redis_stats().await;
        info!("📊 Active Trades Redis Stats: {} active trades", active_trades_count);

        if self.active_order_manager.has_active_trades().await {
            info!("✅ ActiveOrderManager has existing active trades");
        } else {
            info!("📋 ActiveOrderManager: No existing active trades");
        }

        // Load initial strategies
        info!("📋 Loading initial strategies...");
        match self.strategy_manager.refresh_strategies().await {
            Ok(count) => {
                info!("✅ Successfully loaded {} strategies", count);
            }
            Err(e) => {
                warn!("⚠️ Failed to load initial strategies: {}", e);
                // Don't fail initialization if strategies can't be loaded
            }
        }

        // Start background tasks
        info!("🔄 Starting background tasks...");
        self.start_cache_refresh_task().await;
        self.start_websocket_task().await;
        self.start_notification_cleanup_task().await;
        self.start_zone_reset_task().await;
        self.start_zone_interaction_save_task().await;
        self.start_deal_enrichment_task().await; // NEW: Deal enrichment instead of order sync
        self.start_strategy_refresh_task().await;
        self.start_active_order_matching_task().await;
        self.start_csv_logger_cleanup_task().await;
        self.start_pending_order_fill_task().await;
        self.start_active_trade_close_task().await;
        info!("✅ Background tasks started");

        // Test notifications if configured
        if self.notification_manager.is_configured() {
            info!("🧪 Testing notification system...");

            // Play startup sound
            self.notification_manager.play_startup_sound().await;

            let test_result = self.notification_manager.send_test_notification().await;
            if test_result {
                info!("✅ Notification system test successful");
            } else {
                warn!("⚠️ Notification system test failed");
            }
        }

        // Test Telegram if configured
        if self.telegram_notifier.is_enabled() {
            info!("🧪 Testing Telegram notifications...");
            if let Err(e) = self.telegram_notifier.send_test_message().await {
                warn!("⚠️ Telegram test failed: {}", e);
            } else {
                info!("✅ Telegram test successful");
            }
        }

        info!("🎉 MonitorState initialization completed successfully!");
        Ok(())
    }

    pub async fn load_zones_from_file(&self) -> Result<(), Box<dyn std::error::Error>> {
        let content = match tokio::fs::read_to_string(&self.cache_file_path).await {
            Ok(content) => content,
            Err(e) => {
                warn!("Could not read cache file {}: {}", self.cache_file_path, e);
                return Ok(()); // Don't error if file doesn't exist yet
            }
        };

        let cache: ZoneCache = serde_json::from_str(&content)?;
        let mut zone_cache = self.zone_cache.write().await;
        *zone_cache = cache;

        info!("📄 Loaded {} zones from cache file", zone_cache.total_zones);
        Ok(())
    }

    async fn start_cache_refresh_task(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));

            loop {
                interval.tick().await;

                if let Err(e) = state.load_zones_from_file().await {
                    error!("❌ Failed to refresh zone cache: {}", e);
                }
            }
        });
    }

    async fn start_zone_reset_task(&self) {
        let zone_state_manager = Arc::clone(&self.zone_state_manager);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300)); // Check every 5 minutes

            loop {
                interval.tick().await;
                zone_state_manager.check_and_reset_expired_zones().await;
            }
        });
    }

    async fn start_websocket_task(&self) {
        let websocket_client = Arc::clone(&self.websocket_client);
        let ws_url = self.websocket_url.clone();
        let state_for_handler = self.clone();

        tokio::spawn(async move {
            let message_handler = move |price_update: PriceUpdate| {
                let state_clone = state_for_handler.clone();
                tokio::spawn(async move {
                    state_clone.handle_price_update(price_update).await;
                });
            };

            websocket_client
                .start_connection_loop(ws_url, message_handler)
                .await;
        });
    }

    async fn start_notification_cleanup_task(&self) {
        let zone_state_manager = Arc::clone(&self.zone_state_manager);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600)); // Clean up every hour

            loop {
                interval.tick().await;
                zone_state_manager.cleanup_old_states(7).await; // Clean up states older than 7 days
            }
        });
    }

    async fn start_zone_interaction_save_task(&self) {
        let zone_interactions = Arc::clone(&self.zone_interactions);
        let interaction_config = self.interaction_config.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // Save every 30 seconds

            loop {
                interval.tick().await;

                let interactions = zone_interactions.read().await;
                if let Err(e) = interactions.save_to_file(&interaction_config.file_path) {
                    tracing::error!("Failed to save zone interactions: {}", e);
                }
            }
        });
    }

    async fn start_zone_interaction_midnight_cleanup(&self) {
        let zone_interactions = Arc::clone(&self.zone_interactions);
        let interaction_config = self.interaction_config.clone();

        tokio::spawn(async move {
            loop {
                // Calculate time until next midnight UTC
                let now = chrono::Utc::now();
                let next_midnight = (now + chrono::Duration::days(1))
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc();
                let duration_until_midnight = (next_midnight - now)
                    .to_std()
                    .unwrap_or(Duration::from_secs(24 * 3600));

                tracing::info!(
                    "🕐 Zone interactions midnight cleanup scheduled in {:.1} hours",
                    duration_until_midnight.as_secs_f64() / 3600.0
                );

                // Wait until midnight
                tokio::time::sleep(duration_until_midnight).await;

                // Reset all zone interaction data
                {
                    let mut interactions = zone_interactions.write().await;
                    interactions.reset_all_data();

                    // Save the reset state
                    if let Err(e) = interactions.save_to_file(&interaction_config.file_path) {
                        tracing::error!("Failed to save reset zone interactions: {}", e);
                    } else {
                        tracing::info!(
                            "🌅 Midnight cleanup: Zone interactions data reset successfully"
                        );
                    }
                }
            }
        });
    }

    // NEW: Deal enrichment task instead of order sync
    async fn start_deal_enrichment_task(&self) {
        let pending_order_manager = Arc::clone(&self.pending_order_manager);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60)); // Check every minute

            loop {
                interval.tick().await;

                // TODO: Implement new polling logic:
                // 1. For all pending orders, poll /order-details/:accountId/:orderId to detect fills and update status/active_orders.
                // 2. For all active trades, poll broker for position status to detect closes, update status, and enrich/save to InfluxDB.
            }
        });
    }

    async fn start_strategy_refresh_task(&self) {
        let strategy_manager = Arc::clone(&self.strategy_manager);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300)); // Check every 5 minutes

            loop {
                interval.tick().await;

                match strategy_manager.refresh_strategies().await {
                    Ok(count) => {
                        info!(
                            "📋 Strategy refresh completed - {} strategies retrieved",
                            count
                        );
                    }
                    Err(e) => {
                        warn!("📋 Strategy refresh failed: {}", e);
                    }
                }
            }
        });
    }

    async fn start_active_order_matching_task(&self) {
        let active_order_manager = Arc::clone(&self.active_order_manager);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds for active trades

            loop {
                interval.tick().await;

                match active_order_manager.process_order_updates().await {
                    Ok(_) => {
                        debug!("📋 Active trade monitoring completed");
                    }
                    Err(e) => {
                        warn!("📋 Active trade monitoring failed: {}", e);
                    }
                }
            }
        });
    }

    async fn start_csv_logger_cleanup_task(&self) {
        let csv_logger = Arc::clone(&self.csv_logger);

        // Start the logging interval (flush buffer every second)
        let csv_logger_for_interval = Arc::clone(&self.csv_logger);
        tokio::spawn(async move {
            csv_logger_for_interval.start_logging_interval().await;
        });

        // Optional cleanup task (can clear buffer if needed)
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600)); // Clean up every hour

            loop {
                interval.tick().await;
                // Clear buffer if it gets too large (safety measure)
                csv_logger.clear_buffer().await;
            }
        });
    }

    async fn start_pending_order_fill_task(&self) {
        let pending_order_manager = Arc::clone(&self.pending_order_manager);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                let manager = pending_order_manager.read().await;
                match manager.poll_pending_orders_for_fills().await {
                    Ok(_) => {
                        tracing::debug!("[Polling] Checked pending orders for fills");
                    }
                    Err(e) => {
                        tracing::warn!("[Polling] Error checking pending orders for fills: {}", e);
                    }
                }
            }
        });
    }

    async fn start_active_trade_close_task(&self) {
        let active_order_manager = Arc::clone(&self.active_order_manager);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                match active_order_manager.poll_active_trades_for_closes().await {
                    Ok(_) => {
                        tracing::debug!("[Polling] Checked active trades for closes");
                    }
                    Err(e) => {
                        tracing::warn!("[Polling] Error checking active trades for closes: {}", e);
                    }
                }
            }
        });
    }

    async fn handle_price_update(&self, price_update: PriceUpdate) {
        // Store the latest price
        {
            let mut prices = self.latest_prices.write().await;
            prices.insert(price_update.symbol.clone(), price_update.clone());
        }

        // Update zone interactions with new price data
        {
            let mut interactions = self.zone_interactions.write().await;
            let timestamp = price_update.timestamp.to_rfc3339();
            let price = (price_update.bid + price_update.ask) / 2.0; // Use mid price

            interactions.update_all_zones_with_price(
                &price_update.symbol,
                price,
                timestamp,
                &self.interaction_config,
            );
        }

        // Get zones for this symbol and ensure zone interactions are initialized
        let zones = {
            let cache = self.zone_cache.read().await;
            let zones = cache
                .zones
                .get(&price_update.symbol)
                .cloned()
                .unwrap_or_default();

            debug!(
                "📋 handle_price_update called - Symbol: {}, Zones found: {}",
                price_update.symbol,
                zones.len()
            );

            // Initialize zone interaction metrics only for zones within 50 pips of proximal line
            if !zones.is_empty() {
                let mut interactions = self.zone_interactions.write().await;
                let current_price = (price_update.bid + price_update.ask) / 2.0;
                let max_distance_pips = 50.0;
                let pip_value = get_pip_value(&price_update.symbol);

                for zone in &zones {
                    let zone_type = if zone.zone_type.contains("supply") {
                        "supply_zone"
                    } else {
                        "demand_zone"
                    };

                    // Calculate proximal line for this zone
                    let proximal_line = if zone_type == "supply_zone" {
                        zone.low // For supply zones, proximal is low
                    } else {
                        zone.high // For demand zones, proximal is high
                    };

                    // Calculate distance from current price to proximal line
                    let distance_to_proximal_pips =
                        (current_price - proximal_line).abs() / pip_value;

                    // Only create/track zones within 50 pips of their proximal line
                    if distance_to_proximal_pips <= max_distance_pips {
                        interactions.get_or_create_zone_metrics(
                            zone.id.clone(),
                            price_update.symbol.clone(),
                            zone.timeframe.clone(),
                            zone_type.to_string(),
                            zone.high,
                            zone.low,
                        );
                    }
                }
            }

            zones
        };

        if !zones.is_empty() {
            // Use the new ZoneStateManager to process the price update
            let zone_interactions_guard = self.zone_interactions.read().await;
            let result = self
                .zone_state_manager
                .process_price_update(&price_update, &zones, Some(&*zone_interactions_guard))
                .await;

            // Log proximity data for debugging and verification
            {
                let mut proximity_logger = self.proximity_logger.write().await;
                proximity_logger
                    .log_proximity(&price_update, &zones, Some(&*zone_interactions_guard))
                    .await;
            }

            // Check and place pending orders for eligible zones
            {
                let mut pending_order_manager = self.pending_order_manager.write().await;
                pending_order_manager
                    .check_and_place_orders(&price_update, &zones, &self.csv_logger)
                    .await;
            }

            self.handle_new_notifications(&result).await;

            // Handle proximity alerts (first time in proximity)
            for alert in result.proximity_alerts {
                // Store alert
                {
                    let mut recent_alerts = self.recent_alerts.write().await;
                    recent_alerts.push(alert.clone());

                    // Keep only the last 50 alerts
                    let len = recent_alerts.len();
                    if len > 50 {
                        recent_alerts.drain(0..len - 50);
                    }
                }

                // Log to CSV
                self.csv_logger.log_proximity_alert(&alert).await;

                // Send proximity notification (sound)
                self.notification_manager
                    .notify_zone_proximity(&alert)
                    .await;
            }

            // Handle trading signals (when price gets close enough to trade)
            for signal in result.trading_signals {
                info!(
                    "🎯 TRADING SIGNAL: {} {} zone @ {:.1} pips - READY TO TRADE!",
                    signal.symbol, signal.zone_type, signal.distance_pips
                );

                // Log to CSV before attempting trade
                self.csv_logger
                    .log_trading_signal(&signal, "SIGNAL_GENERATED")
                    .await;

                // PRE-CHECK: Determine if we should even attempt the trade based on basic rules
                let should_attempt_trade = {
                    let trading_config = self.trading_engine.get_config().await;

                    // Check basic rules that would prevent trade attempt
                    let symbol_allowed = trading_config.allowed_symbols.contains(&signal.symbol);
                    let timeframe_allowed = trading_config
                        .allowed_timeframes
                        .contains(&signal.timeframe);
                    let max_touch_count = std::env::var("MAX_TOUCH_COUNT_FOR_TRADING")
                        .unwrap_or_else(|_| "3".to_string())
                        .parse::<i32>()
                        .unwrap_or(3);
                    let touch_count_ok = signal.touch_count <= max_touch_count;

                    symbol_allowed && timeframe_allowed && touch_count_ok
                };

                if should_attempt_trade {
                    info!(
                        "✅ All pre-checks passed for {} - attempting trade",
                        signal.symbol
                    );

                    // Execute trade through trading engine - get detailed result
                    let trade_result = self
                        .trading_engine
                        .evaluate_and_execute_trade(&signal, &price_update)
                        .await;

                    if trade_result.success {
                        if let Some(trade) = trade_result.trade {
                            info!(
                                "💰 Trade executed: {} {} {:.2} lots @ {:.5}",
                                trade.symbol,
                                trade.trade_type.to_uppercase(),
                                trade.lot_size,
                                trade.entry_price
                            );

                            // Log successful execution
                            self.csv_logger
                                .log_trading_signal(&signal, "TRADE_EXECUTED")
                                .await;

                            // Save to JSON with complete status - no Telegram here, JSON system handles it
                            self.save_trading_signal_with_status(
                                &signal,
                                true, // trade_attempted
                                true, // trade_executed
                                Some(trade_result.execution_result),
                                None, // no rejection reason for successful trades
                                trade.ctrader_order_id,
                                "sent".to_string(), // Will be handled by JSON notification system
                            )
                            .await;

                            // IMPORTANT: Only mark zone as traded AFTER successful execution
                            self.zone_state_manager
                                .mark_zone_as_traded(&signal.zone_id, &signal.symbol)
                                .await;
                        }
                    } else {
                        // Trade was rejected or failed
                        let rejection_reason = trade_result
                            .rejection_reason
                            .clone()
                            .unwrap_or_else(|| "unknown_error".to_string());

                        info!(
                            "🚫 Trade {} for zone {} - reason: {}",
                            trade_result.execution_result, signal.zone_id, rejection_reason
                        );

                        // Log rejection/failure
                        self.csv_logger
                            .log_trading_signal(
                                &signal,
                                &trade_result.execution_result.to_uppercase(),
                            )
                            .await;

                        // Save to JSON with failure status - no Telegram here, JSON system handles it
                        self.save_trading_signal_with_status(
                            &signal,
                            true,  // trade_attempted
                            false, // trade_executed
                            Some(trade_result.execution_result),
                            trade_result.rejection_reason,
                            None,               // no order ID for failed trades
                            "sent".to_string(), // Will be handled by JSON notification system
                        )
                        .await;
                    }
                } else {
                    // Trade attempt was not made due to basic rule violations - determine why
                    let trading_config = self.trading_engine.get_config().await;

                    let mut rejection_reasons = Vec::new();

                    if !trading_config.allowed_symbols.contains(&signal.symbol) {
                        rejection_reasons.push(format!("symbol_not_allowed_{}", signal.symbol));
                    }

                    if !trading_config
                        .allowed_timeframes
                        .contains(&signal.timeframe)
                    {
                        rejection_reasons
                            .push(format!("timeframe_not_allowed_{}", signal.timeframe));
                    }

                    let max_touch_count = std::env::var("MAX_TOUCH_COUNT_FOR_TRADING")
                        .unwrap_or_else(|_| "3".to_string())
                        .parse::<i32>()
                        .unwrap_or(3);

                    if signal.touch_count > max_touch_count {
                        rejection_reasons.push(format!(
                            "touch_count_too_high_{}_{}",
                            signal.touch_count, max_touch_count
                        ));
                    }

                    let rejection_reason = rejection_reasons.join(", ");

                    info!(
                        "🚫 Trade NOT ATTEMPTED for zone {} - reason: {}",
                        signal.zone_id, rejection_reason
                    );

                    // Log that trade was not attempted
                    self.csv_logger
                        .log_trading_signal(&signal, "TRADE_NOT_ATTEMPTED")
                        .await;

                    // Save to JSON with not attempted status
                    self.save_trading_signal_with_status(
                        &signal,
                        false, // trade_attempted
                        false, // trade_executed
                        Some("not_attempted".to_string()),
                        Some(rejection_reason),
                        None, // no order ID
                        "filtered".to_string(),
                    )
                    .await;
                }

                // Store the trading signal as an alert too (for web interface)
                {
                    let mut recent_alerts = self.recent_alerts.write().await;
                    recent_alerts.push(signal.clone());

                    let len = recent_alerts.len();
                    if len > 50 {
                        recent_alerts.drain(0..len - 50);
                    }
                }
            }
        }
    }

    // Getter methods for the web interface
    pub async fn get_connection_status(&self) -> bool {
        *self.websocket_client.connected.read().await
    }

    pub async fn get_connection_attempts(&self) -> u32 {
        *self.websocket_client.connection_attempts.read().await
    }

    pub async fn get_last_message_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.websocket_client.last_message_time.read().await
    }

    pub async fn get_zone_cache(&self) -> ZoneCache {
        self.zone_cache.read().await.clone()
    }

    pub async fn get_latest_prices(&self) -> HashMap<String, PriceUpdate> {
        self.latest_prices.read().await.clone()
    }

    pub async fn get_recent_alerts(&self) -> Vec<ZoneAlert> {
        self.recent_alerts.read().await.clone()
    }

    // New method to get notification status
    pub async fn get_notification_status(&self) -> serde_json::Value {
        self.notification_manager.get_status()
    }

    // Manual test notification trigger
    pub async fn send_test_notification(&self) -> bool {
        self.notification_manager.send_test_notification().await
    }

    // Get cooldown stats - now from zone state manager
    pub async fn get_cooldown_stats(&self) -> serde_json::Value {
        let zone_stats = self.zone_state_manager.get_zone_stats().await;
        let thresholds = self.zone_state_manager.get_thresholds();

        serde_json::json!({
            "zone_stats": zone_stats,
            "proximity_threshold_pips": thresholds.0,
            "trading_threshold_pips": thresholds.1,
        })
    }

    // Check zone cooldown status - now returns zone state info
    pub async fn get_zone_cooldown_status(
        &self,
        zone_id: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let state_info = self.zone_state_manager.get_zone_state_info(zone_id).await;
        state_info.last_proximity_alert
    }

    // Zone state management methods
    pub async fn get_zone_state_info(
        &self,
        zone_id: &str,
    ) -> crate::zone_state_manager::ZoneStateInfo {
        self.zone_state_manager.get_zone_state_info(zone_id).await
    }

    pub async fn get_zone_stats(&self) -> crate::zone_state_manager::ZoneStats {
        self.zone_state_manager.get_zone_stats().await
    }

    pub async fn reset_zone_state(&self, zone_id: &str) {
        self.zone_state_manager.reset_zone_state(zone_id).await
    }

    pub async fn get_thresholds(&self) -> (f64, f64) {
        self.zone_state_manager.get_thresholds()
    }

    // Trading engine methods
    pub async fn get_trading_stats(&self) -> crate::trading_engine::TradingStats {
        self.trading_engine.get_trading_stats().await
    }

    pub async fn get_active_trades(&self) -> HashMap<String, crate::trading_engine::Trade> {
        self.trading_engine.get_active_trades().await
    }

    pub async fn get_trade_history(&self) -> Vec<crate::trading_engine::Trade> {
        self.trading_engine.get_trade_history().await
    }

    pub async fn get_trading_config(&self) -> crate::trading_engine::TradingConfig {
        self.trading_engine.get_config().await
    }

    // Zone interaction methods
    pub async fn get_zone_interactions(&self) -> ZoneInteractionContainer {
        self.zone_interactions.read().await.clone()
    }

    pub async fn close_trade(
        &self,
        trade_id: &str,
        close_price: f64,
        reason: &str,
    ) -> Result<crate::trading_engine::Trade, String> {
        self.trading_engine
            .close_trade(trade_id, close_price, reason)
            .await
    }

    // Redis stats for pending orders
    pub async fn get_pending_order_redis_stats(&self) -> (usize, usize) {
        let manager = self.pending_order_manager.read().await;
        manager.get_redis_stats().await
    }

    // Active trades Redis statistics
    pub async fn get_active_trades_redis_stats(&self) -> (usize, usize) {
        self.active_order_manager.get_redis_stats().await
    }

    // Get all current active trades with enriched data
    pub async fn get_active_trades_enriched(&self) -> Vec<crate::active_order_manager::EnrichedActiveTrade> {
        self.active_order_manager.get_all_active_trades().await
    }

    // Check if there are any active trades
    pub async fn has_active_trades(&self) -> bool {
        self.active_order_manager.has_active_trades().await
    }

    // Get active trades summary for dashboard
    pub async fn get_active_trades_summary(&self) -> serde_json::Value {
        let trades = self.get_active_trades_enriched().await;
        
        let total_trades = trades.len();
        let profitable_trades = trades.iter().filter(|t| t.is_profitable).count();
        let total_unrealized_pnl: f64 = trades.iter().map(|t| t.unrealized_pnl).sum();
        let total_pips: f64 = trades.iter().map(|t| t.pips_profit).sum();
        
        // Group by symbol
        let mut by_symbol: HashMap<String, usize> = HashMap::new();
        for trade in &trades {
            *by_symbol.entry(trade.symbol.clone()).or_insert(0) += 1;
        }
        
        // Group by zone type (if available)
        let mut by_zone_type: HashMap<String, usize> = HashMap::new();
        for trade in &trades {
            if let Some(zone_type) = &trade.zone_type {
                *by_zone_type.entry(zone_type.clone()).or_insert(0) += 1;
            }
        }
        
        serde_json::json!({
            "total_trades": total_trades,
            "profitable_trades": profitable_trades,
            "losing_trades": total_trades - profitable_trades,
            "total_unrealized_pnl": total_unrealized_pnl,
            "total_pips": total_pips,
            "average_pips_per_trade": if total_trades > 0 { total_pips / total_trades as f64 } else { 0.0 },
            "by_symbol": by_symbol,
            "by_zone_type": by_zone_type,
            "enriched_trades": trades.len(),
            "non_enriched_trades": trades.iter().filter(|t| t.zone_id.is_none()).count()
        })
    }

    // Get comprehensive Redis statistics for both managers
    pub async fn get_comprehensive_redis_stats(&self) -> serde_json::Value {
        let (pending_orders, lookup_keys) = self.get_pending_order_redis_stats().await;
        let (active_trades, _) = self.get_active_trades_redis_stats().await;
        
        serde_json::json!({
            "pending_orders": {
                "count": pending_orders,
                "lookup_keys": lookup_keys
            },
            "active_trades": {
                "count": active_trades
            },
            "total_redis_keys": pending_orders + lookup_keys + active_trades
        })
    }

    pub async fn save_notifications_to_shared_file(
        &self,
        new_notifications: &[ZoneAlert],
        notification_type: &str,
    ) {
        let notifications_file = "shared_notifications.json";
        const MAX_NOTIFICATIONS: usize = 100;

        // Read existing notifications or create new file
        let mut shared_file = match fs::read_to_string(notifications_file).await {
            Ok(content) => serde_json::from_str::<SharedNotificationsFile>(&content)
                .unwrap_or_else(|_| SharedNotificationsFile {
                    last_updated: chrono::Utc::now(),
                    notifications: VecDeque::new(),
                }),
            Err(_) => SharedNotificationsFile {
                last_updated: chrono::Utc::now(),
                notifications: VecDeque::new(),
            },
        };

        // Process each notification with full evaluation
        for alert in new_notifications {
            let mut notification = self
                .create_comprehensive_notification(alert, notification_type)
                .await;

            // Determine if this should be sent to Telegram based on type and rules
            let should_send_to_telegram = match notification_type {
                "proximity_alert" => {
                    // For proximity alerts: only send if ALL trading rules pass
                    // (except daily and symbol limits which user wants to see)
                    notification.passes_symbol_filter
                        && notification.passes_timeframe_filter
                        && notification.passes_touch_count_filter
                        && notification.passes_trading_hours
                }
                "trading_signal" => {
                    // For trading signals: only send if trade was actually attempted
                    // (will be updated later when trade is processed)
                    false // Initially false, will be updated after trade attempt
                }
                _ => false,
            };

            if should_send_to_telegram {
                // Send to Telegram and capture result
                match notification_type {
                    "proximity_alert" => {
                        match self.telegram_notifier.send_proximity_alert(alert).await {
                            Ok(_) => {
                                notification.telegram_sent = Some(true);
                                notification.telegram_response_code = Some(200);
                                // Assume success
                            }
                            Err(e) => {
                                notification.telegram_sent = Some(false);
                                notification.telegram_error = Some(e.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                // Determine why it was filtered
                notification.telegram_filtered = Some(true);
                notification.telegram_sent = Some(false);

                let filter_reasons = match notification_type {
                    "proximity_alert" => {
                        let mut reasons = Vec::new();
                        if !notification.passes_symbol_filter {
                            reasons.push(format!("symbol_not_allowed_{}", notification.symbol));
                        }
                        if !notification.passes_timeframe_filter {
                            reasons
                                .push(format!("timeframe_not_allowed_{}", notification.timeframe));
                        }
                        if !notification.passes_touch_count_filter {
                            reasons
                                .push(format!("touch_count_too_high_{}", notification.touch_count));
                        }
                        if !notification.passes_trading_hours {
                            let hour = chrono::Utc::now().hour();
                            reasons.push(format!("outside_trading_hours_{}h", hour));
                        }
                        reasons.join(", ")
                    }
                    "trading_signal" => "awaiting_trade_attempt".to_string(),
                    _ => "unknown".to_string(),
                };

                notification.telegram_filter_reason = Some(filter_reasons);
            }

            shared_file.notifications.push_front(notification);
        }

        // Keep only the most recent notifications
        while shared_file.notifications.len() > MAX_NOTIFICATIONS {
            shared_file.notifications.pop_back();
        }

        shared_file.last_updated = chrono::Utc::now();

        // Write to file
        if let Ok(json_content) = serde_json::to_string_pretty(&shared_file) {
            if let Err(e) = fs::write(notifications_file, json_content).await {
                tracing::error!("Failed to write notifications file: {}", e);
            }
        }
    }

    // New method to update trading execution status for existing notifications
    pub async fn update_notification_trading_status(
        &self,
        zone_id: &str,
        trade_attempted: bool,
        trade_executed: bool,
        execution_result: Option<String>,
        rejection_reason: Option<String>,
        ctrader_order_id: Option<String>,
        telegram_status: Option<String>, // "sent", "filtered", "attempted" for trading signals
    ) {
        let notifications_file = "shared_notifications.json";

        // Read existing notifications
        let mut shared_file = match fs::read_to_string(notifications_file).await {
            Ok(content) => match serde_json::from_str::<SharedNotificationsFile>(&content) {
                Ok(file) => file,
                Err(e) => {
                    tracing::error!("Failed to parse notifications file: {}", e);
                    return;
                }
            },
            Err(e) => {
                tracing::error!("Failed to read notifications file: {}", e);
                return;
            }
        };

        let now = chrono::Utc::now();
        let mut updated = false;

        // Find and update the most recent trading_signal notification for this zone
        for notification in shared_file.notifications.iter_mut() {
            if notification.zone_id == zone_id && notification.zone_type == "trading_signal" {
                // Update trading fields
                if trade_attempted && notification.trade_attempted != Some(true) {
                    notification.trade_attempted = Some(true);
                    notification.attempted_at = Some(now);
                    updated = true;
                }

                if trade_executed {
                    notification.trade_executed = Some(true);
                    notification.execution_result = execution_result.clone();
                    notification.ctrader_order_id = ctrader_order_id.clone();
                    updated = true;
                } else if trade_attempted {
                    notification.trade_executed = Some(false);
                    notification.execution_result = execution_result.clone();
                    notification.rejection_reason = rejection_reason.clone();
                    updated = true;
                }

                // Handle Telegram status for trading signals
                if let Some(status) = &telegram_status {
                    match status.as_str() {
                        "attempted" => {
                            // Trade was attempted, now decide if we should send to Telegram
                            if trade_executed {
                                // Successful trade - send to Telegram
                                notification.telegram_filtered = Some(false);
                                // Will be updated with actual send result separately
                            } else {
                                // Failed/rejected trade - still send to Telegram for transparency
                                notification.telegram_filtered = Some(false);
                                // Will be updated with actual send result separately
                            }
                        }
                        "sent" => {
                            notification.telegram_sent = Some(true);
                            notification.telegram_response_code = Some(200);
                            updated = true;
                        }
                        "send_failed" => {
                            notification.telegram_sent = Some(false);
                            notification.telegram_error = Some("send_failed".to_string());
                            updated = true;
                        }
                        "filtered" => {
                            notification.telegram_filtered = Some(true);
                            notification.telegram_sent = Some(false);
                            notification.telegram_filter_reason =
                                Some("trade_not_attempted".to_string());
                            updated = true;
                        }
                        _ => {}
                    }
                }

                break; // Only update the first (most recent) match
            }
        }

        if updated {
            shared_file.last_updated = now;

            // Write updated file
            if let Ok(json_content) = serde_json::to_string_pretty(&shared_file) {
                if let Err(e) = fs::write(notifications_file, json_content).await {
                    tracing::error!("Failed to write updated notifications file: {}", e);
                } else {
                    tracing::debug!("Updated trading status for zone {}", zone_id);
                }
            }
        }
    }

    // Call this method whenever you have new proximity alerts or trading signals
    pub async fn handle_new_notifications(&self, process_result: &ProcessResult) {
        if !process_result.proximity_alerts.is_empty() {
            self.save_notifications_to_shared_file(
                &process_result.proximity_alerts,
                "proximity_alert",
            )
            .await;
        }
    }

    // Enhanced notification creation with full rule evaluation
    pub async fn create_comprehensive_notification(
        &self,
        alert: &ZoneAlert,
        notification_type: &str,
    ) -> SharedNotification {
        let action = if alert.zone_type.contains("supply") {
            "SELL"
        } else {
            "BUY"
        };

        // Evaluate all trading rules for transparency
        let trading_config = self.trading_engine.get_config().await;
        let trading_stats = self.trading_engine.get_trading_stats().await;
        let active_trades = self.trading_engine.get_active_trades().await;

        let passes_symbol_filter = trading_config.allowed_symbols.contains(&alert.symbol);
        let passes_timeframe_filter = trading_config.allowed_timeframes.contains(&alert.timeframe);
        let passes_touch_count_filter = {
            let max_touch_count = std::env::var("MAX_TOUCH_COUNT_FOR_TRADING")
                .unwrap_or_else(|_| "3".to_string())
                .parse::<i32>()
                .unwrap_or(3);
            alert.touch_count <= max_touch_count
        };

        let now = chrono::Utc::now();
        let hour = now.hour() as u8;
        let passes_trading_hours =
            hour >= trading_config.trading_start_hour && hour <= trading_config.trading_end_hour;

        let passes_daily_limit = trading_stats.daily_trades < trading_config.max_daily_trades;

        let symbol_trade_count = active_trades
            .values()
            .filter(|t| t.symbol == alert.symbol && t.status == "open")
            .count() as i32;
        let passes_symbol_limit = symbol_trade_count < trading_config.max_trades_per_symbol;

        // Calculate R:R ratio if it's a trading signal
        let (risk_reward_ratio, passes_risk_reward) = if notification_type == "trading_signal" {
            // Simplified R:R calculation
            let rr_ratio = trading_config.take_profit_pips / trading_config.stop_loss_pips;
            (
                Some(rr_ratio),
                Some(rr_ratio >= trading_config.min_risk_reward),
            )
        } else {
            (None, None)
        };

        SharedNotification {
            id: format!("{}_{}", alert.zone_id, alert.timestamp.timestamp_millis()),
            timestamp: alert.timestamp,
            symbol: alert.symbol.clone(),
            timeframe: alert.timeframe.clone(),
            action: action.to_string(),
            price: alert.current_price,
            zone_id: alert.zone_id.clone(),
            zone_type: notification_type.to_string(),
            distance_pips: alert.distance_pips,
            strength: alert.strength,
            touch_count: alert.touch_count,

            // Initialize trading fields
            trade_attempted: match notification_type {
                "trading_signal" => Some(false),
                _ => None,
            },
            trade_executed: match notification_type {
                "trading_signal" => Some(false),
                _ => None,
            },
            execution_result: None,
            rejection_reason: None,
            ctrader_order_id: None,
            attempted_at: None,

            // Initialize notification status
            telegram_sent: None,
            telegram_filtered: None,
            telegram_filter_reason: None,
            telegram_response_code: None,
            telegram_error: None,

            // Rule evaluation results
            passes_symbol_filter,
            passes_timeframe_filter,
            passes_touch_count_filter,
            passes_trading_hours,
            passes_daily_limit,
            passes_symbol_limit,
            risk_reward_ratio,
            passes_risk_reward,
        }
    }

    // Add a new method to save trading signals with their execution status:
    pub async fn save_trading_signal_with_status(
        &self,
        signal: &ZoneAlert,
        trade_attempted: bool,
        trade_executed: bool,
        execution_result: Option<String>,
        rejection_reason: Option<String>,
        ctrader_order_id: Option<String>,
        _telegram_status: String,
    ) {
        let notifications_file = "shared_notifications.json";
        const MAX_NOTIFICATIONS: usize = 100;

        // Read existing notifications or create new file
        let mut shared_file = match fs::read_to_string(notifications_file).await {
            Ok(content) => serde_json::from_str::<SharedNotificationsFile>(&content)
                .unwrap_or_else(|_| SharedNotificationsFile {
                    last_updated: chrono::Utc::now(),
                    notifications: VecDeque::new(),
                }),
            Err(_) => SharedNotificationsFile {
                last_updated: chrono::Utc::now(),
                notifications: VecDeque::new(),
            },
        };

        // Create the comprehensive notification with complete status
        let mut notification = self
            .create_comprehensive_notification(signal, "trading_signal")
            .await;

        // Set the actual execution status
        notification.trade_attempted = Some(trade_attempted);
        notification.trade_executed = Some(trade_executed);
        notification.execution_result = execution_result;
        notification.rejection_reason = rejection_reason;
        notification.ctrader_order_id = ctrader_order_id;

        if trade_attempted {
            notification.attempted_at = Some(chrono::Utc::now());
        }

        // Send to Telegram if trade was attempted (successful or failed)
        if trade_attempted {
            let telegram_status_msg = if trade_executed {
                "TRADE_EXECUTED"
            } else {
                match notification.execution_result.as_deref() {
                    Some("rejected") => "TRADE_REJECTED",
                    Some("failed") => "TRADE_FAILED",
                    _ => "TRADE_ERROR",
                }
            };

            match self
                .telegram_notifier
                .send_trading_signal(signal, telegram_status_msg)
                .await
            {
                Ok(_) => {
                    notification.telegram_sent = Some(true);
                    notification.telegram_filtered = Some(false);
                    notification.telegram_response_code = Some(200);
                }
                Err(e) => {
                    notification.telegram_sent = Some(false);
                    notification.telegram_filtered = Some(false);
                    notification.telegram_error = Some(e.to_string());
                }
            }
        } else {
            // Trade not attempted - filtered
            notification.telegram_sent = Some(false);
            notification.telegram_filtered = Some(true);
            notification.telegram_filter_reason = Some("trade_not_attempted".to_string());
        }

        shared_file.notifications.push_front(notification);

        // Keep only the most recent notifications
        while shared_file.notifications.len() > MAX_NOTIFICATIONS {
            shared_file.notifications.pop_back();
        }

        shared_file.last_updated = chrono::Utc::now();

        // Write to file
        if let Ok(json_content) = serde_json::to_string_pretty(&shared_file) {
            if let Err(e) = fs::write(notifications_file, json_content).await {
                tracing::error!("Failed to write notifications file: {}", e);
            }
        }
    }
}