// analyzer.rs
use crate::config::AnalysisConfig;
use crate::csv_writer::CsvWriter;
use crate::price_fetcher::PriceFetcher;
use crate::trade_validation::BacktestTradeValidator;
use crate::types::*;
use crate::zone_fetcher::ZoneFetcher;
use chrono::{DateTime, Utc};
use log::info;
use std::collections::HashMap;

pub struct TradeAnalyzer {
    config: AnalysisConfig,
    zone_fetcher: ZoneFetcher,
    price_fetcher: PriceFetcher,
    csv_writer: CsvWriter,
    trade_validator: BacktestTradeValidator, // Added trade validator
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
            trade_validator: BacktestTradeValidator::new(), // Initialize trade validator
            zones: Vec::new(),
            price_data: HashMap::new(),
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("🚀 Starting trade analysis for {} to {}", 
              self.config.start_time.format("%Y-%m-%d %H:%M:%S"),
              self.config.end_time.format("%Y-%m-%d %H:%M:%S"));

        // Fetch zones
        self.zones = self.zone_fetcher.fetch_zones(&self.config).await?;
        info!("✅ Loaded {} zones for analysis", self.zones.len());

        // Fetch price data
        self.price_data = self.price_fetcher.fetch_price_data(&self.config, &self.zones).await?;
        info!("✅ Loaded price data for {} symbols", self.price_data.len());

        // Analyze trades
        let trade_results = self.analyze_trades().await?;
        info!("✅ Analysis complete: {} trades identified", trade_results.len());

        // Write results
        self.csv_writer.write_results(&trade_results).await?;
        self.csv_writer.print_summary(&trade_results);

        info!("🎉 Trade analysis completed successfully!");
        Ok(())
    }

    async fn analyze_trades(&mut self) -> Result<Vec<TradeResult>, Box<dyn std::error::Error>> {
        info!("🔍 Starting minute-by-minute trade analysis...");
        
        let mut trade_results = Vec::new();
        let mut open_trades: Vec<OpenTrade> = Vec::new();
        
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
            
            if minute_count % 100 == 0 {
                info!("⏳ Processing minute {}/{} ({})", minute_count, total_minutes, current_time.format("%H:%M"));
            }
            
            let current_prices = self.get_prices_at_timestamp(current_time);
            
            // Check for new trade entries
            for zone in &self.zones {
                if let Some(current_price) = current_prices.get(&zone.symbol) {
                    if open_trades.iter().any(|t| t.zone_id == zone.zone_id) { 
                        continue; 
                    }
                    
                    if self.check_zone_entry(zone, *current_price) {
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
                                let (tp_price, sl_price) = self.calculate_tp_sl_levels(zone, *current_price);
                                
                                let open_trade = OpenTrade {
                                    zone_id: zone.zone_id.clone(),
                                    symbol: zone.symbol.clone(),
                                    timeframe: zone.timeframe.clone(),
                                    action: if zone.zone_type.contains("supply") { "SELL".to_string() } else { "BUY".to_string() },
                                    entry_time: current_time,
                                    entry_price: *current_price,
                                    take_profit: tp_price,
                                    stop_loss: sl_price,
                                    zone_strength: zone.strength_score,
                                };
                                
                                info!("📈 NEW TRADE: {} {} {} @ {:.5} [{}]", 
                                      open_trade.action, open_trade.symbol, open_trade.timeframe, 
                                      open_trade.entry_price, validation_reason);
                                open_trades.push(open_trade);
                            }
                            Err(rejection_reason) => {
                                info!("🚫 TRADE REJECTED: {} {} {} @ {:.5} - {}", 
                                      if zone.zone_type.contains("supply") { "SELL" } else { "BUY" },
                                      zone.symbol, zone.timeframe, current_price, rejection_reason);
                                
                                // Optional: Track rejected signals for analysis
                                let rejected_trade = TradeResult {
                                    entry_time: current_time,
                                    symbol: zone.symbol.clone(),
                                    timeframe: zone.timeframe.clone(),
                                    zone_id: zone.zone_id.clone(),
                                    action: if zone.zone_type.contains("supply") { "SELL".to_string() } else { "BUY".to_string() },
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
                    if let Some(exit_reason) = self.check_trade_exit_with_price(open_trade, *current_price) {
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
                        
                        info!("🎯 TRADE CLOSED: {} {} @ {:.5} -> {:.5} | {:.1} pips | {} | {}min", 
                              completed_trade.action, completed_trade.symbol, completed_trade.entry_price, 
                              current_price, pnl_pips, completed_trade.exit_reason, duration_minutes);
                        
                        trade_results.push(completed_trade);
                        trades_to_close.push(index);
                    }
                    else if let Some(zone) = self.zones.iter().find(|z| z.zone_id == open_trade.zone_id) {
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
                            
                            info!("❌ ZONE INVALIDATED: {} {} @ {:.5} | {:.1} pips | {}min", 
                                  completed_trade.action, completed_trade.symbol, current_price, pnl_pips, duration_minutes);
                            
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
        
        // Handle remaining open trades
        for open_trade in open_trades {
            let final_price = self.price_data.get(&open_trade.symbol)
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
        
        info!("✅ Analysis complete: {} total trades identified", trade_results.len());
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
    
    fn check_trade_exit_with_price(&self, open_trade: &OpenTrade, current_price: f64) -> Option<String> {
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
            current_price >= (proximal_line - proximity_threshold) && 
            current_price <= (proximal_line + proximity_threshold)
        } else {
            let proximal_line = zone.zone_high;
            current_price >= (proximal_line - proximity_threshold) && 
            current_price <= (proximal_line + proximity_threshold)
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

    fn validate_trade_signal(&self, _zone: &ZoneData, _current_time: DateTime<Utc>) -> bool {
        // This method is now replaced by the BacktestTradeValidator
        // Keeping for backward compatibility, but shouldn't be used
        true
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
}