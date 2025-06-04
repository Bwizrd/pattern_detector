// analyzer.rs - Complete replacement file
use crate::config::AnalysisConfig;
use crate::csv_writer::CsvWriter;
use crate::price_fetcher::PriceFetcher;
use crate::trade_validation::BacktestTradeValidator;
use crate::types::*;
use crate::zone_fetcher::ZoneFetcher;
use crate::zone_proximity_analyzer::ZoneProximityAnalyzer;
use chrono::{DateTime, Utc};
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};

pub struct TradeAnalyzer {
    config: AnalysisConfig,
    zone_fetcher: ZoneFetcher,
    price_fetcher: PriceFetcher,
    csv_writer: CsvWriter,
    trade_validator: BacktestTradeValidator,
    zones: Vec<ZoneData>,
    price_data: HashMap<String, Vec<PriceCandle>>,
}

impl TradeAnalyzer {
    pub fn new(config: AnalysisConfig) -> Self {
        Self {
            config,
            zone_fetcher: ZoneFetcher::new(),
            price_fetcher: PriceFetcher::new(),
            csv_writer: CsvWriter::new(),
            trade_validator: BacktestTradeValidator::new(),
            zones: Vec::new(),
            price_data: HashMap::new(),
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!(
            "🚀 Starting trade analysis for {} to {}",
            self.config.start_time.format("%Y-%m-%d %H:%M:%S"),
            self.config.end_time.format("%Y-%m-%d %H:%M:%S")
        );

        info!("🔥 NEW ENHANCED CODE IS RUNNING - PROXIMITY ANALYZER ENABLED!");

        // Fetch zones
        self.zones = self.zone_fetcher.fetch_zones(&self.config).await?;
        info!("✅ Loaded {} zones for analysis", self.zones.len());

        // Print zone summary for diagnostics
        self.print_zone_diagnostics();

        // Fetch price data
        self.price_data = self
            .price_fetcher
            .fetch_price_data(&self.config, &self.zones)
            .await?;
        info!("✅ Loaded price data for {} symbols", self.price_data.len());

        // Print price data diagnostics
        self.print_price_data_diagnostics();

        // Print trading rules
        self.print_trading_rules();


        // Run proximity analysis
        info!("🔍 STARTING PROXIMITY ANALYSIS...");
        let mut proximity_analyzer = ZoneProximityAnalyzer::new();
        proximity_analyzer.analyze_zone_proximity(&self.zones, &self.price_data, 5.0);

        info!("📝 WRITING PROXIMITY ANALYSIS REPORTS...");
        match proximity_analyzer.write_proximity_summary_only().await {
            Ok(()) => info!("✅ Proximity reports written successfully"),
            Err(e) => warn!("❌ Failed to write proximity reports: {}", e),
        }

        // Analyze trades
        let trade_results = self.analyze_trades().await?;
        info!(
            "✅ Analysis complete: {} trades identified",
            trade_results.len()
        );

        // Print analysis summary
        self.print_analysis_summary(&trade_results);

        // Write results
        self.csv_writer.write_results(&trade_results).await?;
        self.write_clean_trades(&trade_results).await?;
        self.csv_writer.print_summary(&trade_results);

        info!("🎉 Trade analysis completed successfully!");
        Ok(())
    }


    fn print_zone_diagnostics(&self) {
        info!("📊 ZONE DIAGNOSTICS:");

        // Group zones by symbol
        let mut symbol_counts: HashMap<String, usize> = HashMap::new();
        let mut timeframe_counts: HashMap<String, usize> = HashMap::new();
        let mut zone_type_counts: HashMap<String, usize> = HashMap::new();

        for zone in &self.zones {
            *symbol_counts.entry(zone.symbol.clone()).or_insert(0) += 1;
            *timeframe_counts.entry(zone.timeframe.clone()).or_insert(0) += 1;
            *zone_type_counts.entry(zone.zone_type.clone()).or_insert(0) += 1;
        }

        info!("📊 Zones by Symbol:");
        for (symbol, count) in symbol_counts.iter() {
            info!("   {} zones: {}", symbol, count);
        }

        info!("📊 Zones by Timeframe:");
        for (timeframe, count) in timeframe_counts.iter() {
            info!("   {} zones: {}", timeframe, count);
        }

        info!("📊 Zones by Type:");
        for (zone_type, count) in zone_type_counts.iter() {
            info!("   {} zones: {}", zone_type, count);
        }

        // Show first few zones for detailed inspection
        info!("📊 First 5 zones:");
        for (i, zone) in self.zones.iter().take(5).enumerate() {
            info!(
                "   {}: {} {} {} | {:.5}-{:.5} | Strength: {:.1}",
                i + 1,
                zone.symbol,
                zone.timeframe,
                zone.zone_type,
                zone.zone_low,
                zone.zone_high,
                zone.strength_score
            );
        }
    }

    fn print_price_data_diagnostics(&self) {
        info!("📈 PRICE DATA DIAGNOSTICS:");
        for (symbol, candles) in &self.price_data {
            let first_time = candles
                .first()
                .map(|c| c.timestamp.format("%H:%M:%S").to_string())
                .unwrap_or("N/A".to_string());
            let last_time = candles
                .last()
                .map(|c| c.timestamp.format("%H:%M:%S").to_string())
                .unwrap_or("N/A".to_string());
            info!(
                "   {}: {} candles ({} to {})",
                symbol,
                candles.len(),
                first_time,
                last_time
            );
        }
    }

    fn print_trading_rules(&self) {
        let rules = self.trade_validator.get_rules();
        info!("🔧 TRADING RULES:");
        info!("   Enabled: {}", rules.trading_enabled);
        info!("   Allowed Symbols: {:?}", rules.allowed_symbols);
        info!("   Allowed Timeframes: {:?}", rules.allowed_timeframes);
        info!("   Min Zone Strength: {}", rules.min_zone_strength);
        info!("   Max Daily Signals: {}", rules.max_daily_signals);
        info!(
            "   Trading Hours: {:?} - {:?}",
            rules.trading_start_hour_utc, rules.trading_end_hour_utc
        );
    }

    fn print_analysis_summary(&self, trade_results: &[TradeResult]) {
        info!("📊 ANALYSIS SUMMARY:");

        let mut validation_reasons: HashMap<String, usize> = HashMap::new();
        let mut exit_reasons: HashMap<String, usize> = HashMap::new();
        let mut symbol_counts: HashMap<String, usize> = HashMap::new();

        for trade in trade_results {
            if let Some(reason) = &trade.validation_reason {
                *validation_reasons.entry(reason.clone()).or_insert(0) += 1;
            }
            *exit_reasons.entry(trade.exit_reason.clone()).or_insert(0) += 1;
            *symbol_counts.entry(trade.symbol.clone()).or_insert(0) += 1;
        }

        info!("📊 Validation Results:");
        for (reason, count) in validation_reasons.iter() {
            info!("   {}: {}", reason, count);
        }

        info!("📊 Exit Reasons:");
        for (reason, count) in exit_reasons.iter() {
            info!("   {}: {}", reason, count);
        }

        info!("📊 Trades by Symbol:");
        for (symbol, count) in symbol_counts.iter() {
            info!("   {}: {}", symbol, count);
        }
    }

    async fn analyze_trades(&mut self) -> Result<Vec<TradeResult>, Box<dyn std::error::Error>> {
        info!("🔍 Starting minute-by-minute trade analysis...");

        let mut trade_results = Vec::new();
        let mut open_trades: Vec<OpenTrade> = Vec::new();
        let mut used_zones: HashSet<String> = HashSet::new(); // Track zones that have been used

        // Track zone touch attempts for debugging
        let mut zone_touch_counts: HashMap<String, usize> = HashMap::new();

        let mut all_timestamps = std::collections::BTreeSet::new();
        for candles in self.price_data.values() {
            for candle in candles {
                all_timestamps.insert(candle.timestamp);
            }
        }

        let total_minutes = all_timestamps.len();
        info!("📊 Analyzing {} minutes of price data", total_minutes);

        let mut minute_count = 0;

        for current_time in all_timestamps {
            minute_count += 1;

            if minute_count % 20 == 0 {
                info!(
                    "⏳ Processing minute {}/{} ({})",
                    minute_count,
                    total_minutes,
                    current_time.format("%H:%M")
                );
            }

            let current_prices = self.get_prices_at_timestamp(current_time);

            // Check for new trade entries - iterate through ALL zones
            for zone in &self.zones {
                if let Some(current_price) = current_prices.get(&zone.symbol) {
                    // Skip if this zone has already been used (had a trade)
                    if used_zones.contains(&zone.zone_id) {
                        continue;
                    }

                    // Check if this zone is already open
                    if open_trades.iter().any(|t| t.zone_id == zone.zone_id) {
                        continue;
                    }

                    // Check zone entry (price proximity)
                    if self.check_zone_entry(zone, *current_price) {
                        // Track that this zone was touched
                        *zone_touch_counts.entry(zone.zone_id.clone()).or_insert(0) += 1;

                        debug!(
                            "🎯 ZONE TOUCH: {} {} {} @ {:.5} (Touch #{} for this zone)",
                            zone.symbol,
                            zone.timeframe,
                            zone.zone_id,
                            current_price,
                            zone_touch_counts.get(&zone.zone_id).unwrap_or(&0)
                        );

                        // Mark this zone as used forever - NO MORE TRADES ON THIS ZONE
                        used_zones.insert(zone.zone_id.clone());

                        // Use EXACT main app validation logic
                        let touch_count = zone.touch_count.unwrap_or(0);
                        match self.trade_validator.validate_trade_signal(
                            &zone.zone_id,
                            &zone.symbol,
                            &zone.timeframe,
                            zone.strength_score,
                            touch_count,
                            current_time,
                        ) {
                            Ok(validation_reason) => {
                                let (tp_price, sl_price) =
                                    self.calculate_tp_sl_levels(zone, *current_price);

                                let open_trade = OpenTrade {
                                    zone_id: zone.zone_id.clone(),
                                    symbol: zone.symbol.clone(),
                                    timeframe: zone.timeframe.clone(),
                                    action: if zone.zone_type.contains("supply") {
                                        "SELL".to_string()
                                    } else {
                                        "BUY".to_string()
                                    },
                                    entry_time: current_time,
                                    entry_price: *current_price,
                                    take_profit: tp_price,
                                    stop_loss: sl_price,
                                    zone_strength: zone.strength_score,
                                };

                                info!(
                                    "📈 NEW TRADE: {} {} {} @ {:.5} [{}]",
                                    open_trade.action,
                                    open_trade.symbol,
                                    open_trade.timeframe,
                                    open_trade.entry_price,
                                    validation_reason
                                );
                                open_trades.push(open_trade);
                            }
                            Err(rejection_reason) => {
                                info!(
                                    "🚫 TRADE REJECTED: {} {} {} @ {:.5} - {}",
                                    if zone.zone_type.contains("supply") {
                                        "SELL"
                                    } else {
                                        "BUY"
                                    },
                                    zone.symbol,
                                    zone.timeframe,
                                    current_price,
                                    rejection_reason
                                );

                                // Track rejected signals for analysis
                                let rejected_trade = TradeResult {
                                    entry_time: current_time,
                                    symbol: zone.symbol.clone(),
                                    timeframe: zone.timeframe.clone(),
                                    zone_id: zone.zone_id.clone(),
                                    action: if zone.zone_type.contains("supply") {
                                        "SELL".to_string()
                                    } else {
                                        "BUY".to_string()
                                    },
                                    entry_price: *current_price,
                                    exit_time: None,
                                    exit_price: None,
                                    exit_reason: "VALIDATION_FAILED".to_string(),
                                    pnl_pips: None,
                                    duration_minutes: None,
                                    zone_strength: zone.strength_score,
                                    validation_reason: Some(rejection_reason),
                                };
                                trade_results.push(rejected_trade);
                            }
                        }
                    }
                }
            }

            // Check existing trades for exits
            let mut trades_to_close = Vec::new();

            for (index, open_trade) in open_trades.iter().enumerate() {
                if let Some(current_price) = current_prices.get(&open_trade.symbol) {
                    if let Some(exit_reason) =
                        self.check_trade_exit_with_price(open_trade, *current_price)
                    {
                        let duration = current_time - open_trade.entry_time;
                        let duration_minutes = duration.num_minutes();
                        let pnl_pips = self.calculate_pnl_pips(open_trade, *current_price);

                        let completed_trade = TradeResult {
                            entry_time: open_trade.entry_time,
                            symbol: open_trade.symbol.clone(),
                            timeframe: open_trade.timeframe.clone(),
                            zone_id: open_trade.zone_id.clone(),
                            action: open_trade.action.clone(),
                            entry_price: open_trade.entry_price,
                            exit_time: Some(current_time),
                            exit_price: Some(*current_price),
                            exit_reason,
                            pnl_pips: Some(pnl_pips),
                            duration_minutes: Some(duration_minutes),
                            zone_strength: open_trade.zone_strength,
                            validation_reason: Some("Trade executed successfully".to_string()),
                        };

                        info!(
                            "🎯 TRADE CLOSED: {} {} @ {:.5} -> {:.5} | {:.1} pips | {} | {}min",
                            completed_trade.action,
                            completed_trade.symbol,
                            completed_trade.entry_price,
                            current_price,
                            pnl_pips,
                            completed_trade.exit_reason,
                            duration_minutes
                        );

                        trade_results.push(completed_trade);
                        trades_to_close.push(index);
                    } else if let Some(zone) =
                        self.zones.iter().find(|z| z.zone_id == open_trade.zone_id)
                    {
                        if self.check_zone_invalidation(zone, *current_price) {
                            let duration = current_time - open_trade.entry_time;
                            let duration_minutes = duration.num_minutes();
                            let pnl_pips = self.calculate_pnl_pips(open_trade, *current_price);

                            let completed_trade = TradeResult {
                                entry_time: open_trade.entry_time,
                                symbol: open_trade.symbol.clone(),
                                timeframe: open_trade.timeframe.clone(),
                                zone_id: open_trade.zone_id.clone(),
                                action: open_trade.action.clone(),
                                entry_price: open_trade.entry_price,
                                exit_time: Some(current_time),
                                exit_price: Some(*current_price),
                                exit_reason: "ZONE_INVALIDATED".to_string(),
                                pnl_pips: Some(pnl_pips),
                                duration_minutes: Some(duration_minutes),
                                zone_strength: open_trade.zone_strength,
                                validation_reason: Some("Zone invalidated".to_string()),
                            };

                            info!(
                                "❌ ZONE INVALIDATED: {} {} @ {:.5} | {:.1} pips | {}min",
                                completed_trade.action,
                                completed_trade.symbol,
                                current_price,
                                pnl_pips,
                                duration_minutes
                            );

                            trade_results.push(completed_trade);
                            trades_to_close.push(index);
                        }
                    }
                }
            }

            for &index in trades_to_close.iter().rev() {
                open_trades.remove(index);
            }
        }

        // Print final diagnostics
        info!("🎯 ZONE TOUCH SUMMARY:");
        info!("   Total unique zones touched: {}", zone_touch_counts.len());
        for (zone_id, count) in zone_touch_counts.iter().take(10) {
            let zone = self.zones.iter().find(|z| z.zone_id == *zone_id);
            if let Some(z) = zone {
                info!(
                    "   {} {} {}: {} touches",
                    z.symbol, z.timeframe, zone_id, count
                );
            }
        }

        // Handle remaining open trades
        for open_trade in open_trades {
            let final_price = self
                .price_data
                .get(&open_trade.symbol)
                .and_then(|candles| candles.last())
                .map(|candle| candle.close)
                .unwrap_or(open_trade.entry_price);

            let duration = self.config.end_time - open_trade.entry_time;
            let duration_minutes = duration.num_minutes();
            let pnl_pips = self.calculate_pnl_pips(&open_trade, final_price);

            let still_open_trade = TradeResult {
                entry_time: open_trade.entry_time,
                symbol: open_trade.symbol.clone(),
                timeframe: open_trade.timeframe.clone(),
                zone_id: open_trade.zone_id.clone(),
                action: open_trade.action.clone(),
                entry_price: open_trade.entry_price,
                exit_time: None,
                exit_price: None,
                exit_reason: "STILL_OPEN".to_string(),
                pnl_pips: Some(pnl_pips),
                duration_minutes: Some(duration_minutes),
                zone_strength: open_trade.zone_strength,
                validation_reason: Some("Still open at analysis end".to_string()),
            };

            trade_results.push(still_open_trade);
        }

        info!(
            "✅ Analysis complete: {} total trades identified",
            trade_results.len()
        );
        info!("📊 Used {} unique zones", used_zones.len());
        Ok(trade_results)
    }

    fn get_prices_at_timestamp(&self, timestamp: DateTime<Utc>) -> HashMap<String, f64> {
        let mut prices = HashMap::new();
        for (symbol, candles) in &self.price_data {
            if let Some(candle) = candles.iter().find(|c| c.timestamp == timestamp) {
                prices.insert(symbol.clone(), candle.close);
            }
        }
        prices
    }

    fn check_trade_exit_with_price(
        &self,
        open_trade: &OpenTrade,
        current_price: f64,
    ) -> Option<String> {
        let is_buy = open_trade.action == "BUY";

        let tp_hit = if is_buy {
            current_price >= open_trade.take_profit
        } else {
            current_price <= open_trade.take_profit
        };
        if tp_hit {
            return Some("TP_HIT".to_string());
        }

        let sl_hit = if is_buy {
            current_price <= open_trade.stop_loss
        } else {
            current_price >= open_trade.stop_loss
        };
        if sl_hit {
            return Some("SL_HIT".to_string());
        }

        None
    }

    fn calculate_pnl_pips(&self, open_trade: &OpenTrade, current_price: f64) -> f64 {
        let is_buy = open_trade.action == "BUY";
        let pip_value = self.get_pip_value(&open_trade.symbol);
        let price_diff = if is_buy {
            current_price - open_trade.entry_price
        } else {
            open_trade.entry_price - current_price
        };
        price_diff / pip_value
    }

    fn get_pip_value(&self, symbol: &str) -> f64 {
        if symbol.contains("JPY") {
            0.01
        } else {
            0.0001
        }
    }

    fn check_zone_entry(&self, zone: &ZoneData, current_price: f64) -> bool {
        let zone_height = zone.zone_high - zone.zone_low;
        let pip_threshold: f64 = 0.0002;
        let percent_threshold: f64 = zone_height * 0.02;
        let proximity_threshold = pip_threshold.min(percent_threshold);

        let is_supply = zone.zone_type.contains("supply");
        if is_supply {
            let proximal_line = zone.zone_low;
            current_price >= (proximal_line - proximity_threshold)
                && current_price <= (proximal_line + proximity_threshold)
        } else {
            let proximal_line = zone.zone_high;
            current_price >= (proximal_line - proximity_threshold)
                && current_price <= (proximal_line + proximity_threshold)
        }
    }

    fn check_zone_invalidation(&self, zone: &ZoneData, current_price: f64) -> bool {
        let is_supply = zone.zone_type.contains("supply");
        if is_supply {
            current_price > zone.zone_high
        } else {
            current_price < zone.zone_low
        }
    }

    fn calculate_tp_sl_levels(&self, zone: &ZoneData, entry_price: f64) -> (f64, f64) {
        let pip_value = self.get_pip_value(&zone.symbol);
        let is_supply = zone.zone_type.contains("supply");
        let tp_pips = 50.0;
        let sl_pips = 25.0;

        if is_supply {
            let take_profit = entry_price - (tp_pips * pip_value);
            let stop_loss = entry_price + (sl_pips * pip_value);
            (take_profit, stop_loss)
        } else {
            let take_profit = entry_price + (tp_pips * pip_value);
            let stop_loss = entry_price - (sl_pips * pip_value);
            (take_profit, stop_loss)
        }
    }
    // Add this method to your existing TradeAnalyzer impl block:

    async fn write_clean_trades(
        &self,
        trade_results: &[TradeResult],
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Filter out validation failures - only keep actual executed trades
        let clean_trades: Vec<&TradeResult> = trade_results
            .iter()
            .filter(|trade| trade.exit_reason != "VALIDATION_FAILED")
            .collect();

        if clean_trades.is_empty() {
            info!("📝 No executed trades to write to clean trades file");
            return Ok(());
        }

        std::fs::create_dir_all("trades")?;
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("trades/trades-{}.csv", timestamp);

        info!(
            "📝 Writing {} clean trades to: {}",
            clean_trades.len(),
            filename
        );

        let file = std::fs::File::create(&filename)?;
        let mut writer = csv::Writer::from_writer(file);

        writer.write_record(&[
            "entry_time",
            "symbol",
            "timeframe",
            "zone_id",
            "action",
            "entry_price",
            "exit_time",
            "exit_price",
            "exit_reason",
            "pnl_pips",
            "duration_minutes",
            "zone_strength",
        ])?;

        for trade in clean_trades {
            let record = vec![
                trade.entry_time.to_rfc3339(),
                trade.symbol.clone(),
                trade.timeframe.clone(),
                trade.zone_id.clone(),
                trade.action.clone(),
                trade.entry_price.to_string(),
                trade.exit_time.map_or("".to_string(), |t| t.to_rfc3339()),
                trade.exit_price.map_or("".to_string(), |p| p.to_string()),
                trade.exit_reason.clone(),
                trade
                    .pnl_pips
                    .map_or("".to_string(), |p| format!("{:.1}", p)),
                trade
                    .duration_minutes
                    .map_or("".to_string(), |d| d.to_string()),
                format!("{:.1}", trade.zone_strength),
            ];
            writer.write_record(&record)?;
        }

        writer.flush()?;
        info!("✅ Clean trades CSV written successfully: {}", filename);
        println!("📄 Clean trades saved to: {}", filename);
        Ok(())
    }
}
