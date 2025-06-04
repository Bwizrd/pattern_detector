// src/trade_tracker.rs - Track open trades and calculate P&L
use chrono::{DateTime, Utc};
use log::info;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedTrade {
    pub trade_id: String,
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub timeframe: String,
    pub action: String, // BUY or SELL
    pub entry_price: f64,
    pub zone_strength: f64,
    pub zone_id: String,
    pub take_profit: Option<f64>,
    pub stop_loss: Option<f64>,
    pub position_size: Option<f64>,
    pub status: TradeStatus,
    pub current_price: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    pub unrealized_pnl_pips: Option<f64>,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeStatus {
    Open,
    TakeProfitHit,
    StopLossHit,
    ManualClose,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TradeTrackingRequest {
    pub trades: Vec<TradeWithLevels>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TradeWithLevels {
    pub timestamp: String,
    pub symbol: String,
    pub timeframe: String,
    pub action: String,
    pub entry_price: f64,
    pub zone_strength: f64,
    pub zone_id: String,
    pub take_profit: Option<f64>,
    pub stop_loss: Option<f64>,
    pub position_size: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct TradeTrackingResponse {
    pub status: String,
    pub total_trades: usize,
    pub open_trades: usize,
    pub closed_trades: usize,
    pub total_pnl: f64,
    pub total_pnl_pips: f64,
    pub trades: Vec<TrackedTrade>,
    pub summary: TradingSummary,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TradingSummary {
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub break_even_trades: usize,
    pub win_rate: f64,
    pub average_win: f64,
    pub average_loss: f64,
    pub largest_win: f64,
    pub largest_loss: f64,
    pub profit_factor: f64,
}

pub struct TradeTracker {
    tracked_trades: HashMap<String, TrackedTrade>,
}

impl TradeTracker {
    pub fn new() -> Self {
        Self {
            tracked_trades: HashMap::new(),
        }
    }

    /// Load trades from CSV file
    pub async fn load_trades_from_csv(
        &mut self,
        csv_file_path: &PathBuf,
        default_tp_pips: Option<f64>,
        default_sl_pips: Option<f64>,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        info!("📈 Loading trades from CSV: {:?}", csv_file_path);

        let content = fs::read_to_string(csv_file_path)?;
        let mut csv_reader = csv::Reader::from_reader(content.as_bytes());

        let mut loaded_count = 0;

        for result in csv_reader.records() {
            let record = result?;
            
            if record.len() < 8 {
                continue;
            }

            // Parse CSV record
            let timestamp_str = &record[0];
            let symbol = &record[1];
            let timeframe = &record[2];
            let action = &record[3];
            let entry_price: f64 = record[4].parse()?;
            let zone_strength: f64 = record[6].parse()?;
            let zone_id = &record[7];

            let timestamp = DateTime::parse_from_rfc3339(timestamp_str)?
                .with_timezone(&Utc);

            // Calculate TP/SL if not provided
            let (take_profit, stop_loss) = self.calculate_default_levels(
                action,
                entry_price,
                symbol,
                default_tp_pips,
                default_sl_pips,
            );

            let trade_id = format!("{}_{}", zone_id, timestamp.timestamp());

            let tracked_trade = TrackedTrade {
                trade_id: trade_id.clone(),
                timestamp,
                symbol: symbol.to_string(),
                timeframe: timeframe.to_string(),
                action: action.to_string(),
                entry_price,
                zone_strength,
                zone_id: zone_id.to_string(),
                take_profit,
                stop_loss,
                position_size: Some(1.0), // Default position size
                status: TradeStatus::Open,
                current_price: None,
                unrealized_pnl: None,
                unrealized_pnl_pips: None,
                last_updated: Utc::now(),
            };

            self.tracked_trades.insert(trade_id, tracked_trade);
            loaded_count += 1;
        }

        info!("📈 Loaded {} trades from CSV", loaded_count);
        Ok(loaded_count)
    }

    /// Track trades from request with custom TP/SL levels
    pub fn track_trades_from_request(&mut self, request: TradeTrackingRequest) {
        for trade_data in request.trades {
            let timestamp = DateTime::parse_from_rfc3339(&trade_data.timestamp)
                .unwrap_or_else(|_| Utc::now().into())
                .with_timezone(&Utc);

            let trade_id = format!("{}_{}", trade_data.zone_id, timestamp.timestamp());

            let tracked_trade = TrackedTrade {
                trade_id: trade_id.clone(),
                timestamp,
                symbol: trade_data.symbol,
                timeframe: trade_data.timeframe,
                action: trade_data.action,
                entry_price: trade_data.entry_price,
                zone_strength: trade_data.zone_strength,
                zone_id: trade_data.zone_id,
                take_profit: trade_data.take_profit,
                stop_loss: trade_data.stop_loss,
                position_size: trade_data.position_size,
                status: TradeStatus::Open,
                current_price: None,
                unrealized_pnl: None,
                unrealized_pnl_pips: None,
                last_updated: Utc::now(),
            };

            self.tracked_trades.insert(trade_id, tracked_trade);
        }
    }

    /// Update all trades with current prices and calculate P&L
    pub async fn update_with_current_prices(
        &mut self,
        current_prices: &HashMap<String, f64>,
    ) -> TradeTrackingResponse {
        let mut total_pnl = 0.0;
        let mut total_pnl_pips = 0.0;
        let mut open_trades = 0;
        let mut closed_trades = 0;

        // Collect trades to update (to avoid borrowing issues)
        let mut trade_updates = Vec::new();
        
        for (trade_id, trade) in &self.tracked_trades {
            if let Some(&current_price) = current_prices.get(&trade.symbol) {
                let mut updated_trade = trade.clone();
                updated_trade.current_price = Some(current_price);
                updated_trade.last_updated = Utc::now();

                // Check if TP or SL hit
                Self::check_trade_closure_static(&mut updated_trade, current_price);

                // Calculate P&L
                let (pnl, pnl_pips) = Self::calculate_pnl_static(&updated_trade, current_price);
                updated_trade.unrealized_pnl = Some(pnl);
                updated_trade.unrealized_pnl_pips = Some(pnl_pips);

                if updated_trade.status == TradeStatus::Open {
                    open_trades += 1;
                    total_pnl += pnl;
                    total_pnl_pips += pnl_pips;
                } else {
                    closed_trades += 1;
                    // For closed trades, use the closing P&L
                    total_pnl += pnl;
                    total_pnl_pips += pnl_pips;
                }

                trade_updates.push((trade_id.clone(), updated_trade));
            }
        }

        // Apply updates
        for (trade_id, updated_trade) in trade_updates {
            self.tracked_trades.insert(trade_id, updated_trade);
        }

        let summary = self.calculate_summary();

        TradeTrackingResponse {
            status: "success".to_string(),
            total_trades: self.tracked_trades.len(),
            open_trades,
            closed_trades,
            total_pnl,
            total_pnl_pips,
            trades: self.tracked_trades.values().cloned().collect(),
            summary,
            timestamp: Utc::now(),
        }
    }

    fn check_trade_closure_static(trade: &mut TrackedTrade, current_price: f64) {
        if trade.status != TradeStatus::Open {
            return; // Already closed
        }

        let is_buy = trade.action == "BUY";

        // Check Take Profit
        if let Some(tp) = trade.take_profit {
            let tp_hit = if is_buy {
                current_price >= tp
            } else {
                current_price <= tp
            };

            if tp_hit {
                trade.status = TradeStatus::TakeProfitHit;
                return;
            }
        }

        // Check Stop Loss
        if let Some(sl) = trade.stop_loss {
            let sl_hit = if is_buy {
                current_price <= sl
            } else {
                current_price >= sl
            };

            if sl_hit {
                trade.status = TradeStatus::StopLossHit;
            }
        }
    }

    fn calculate_pnl_static(trade: &TrackedTrade, current_price: f64) -> (f64, f64) {
        let is_buy = trade.action == "BUY";
        let position_size = trade.position_size.unwrap_or(1.0);

        // If trade is closed, use the TP/SL price instead of current price
        let effective_price = match trade.status {
            TradeStatus::TakeProfitHit => trade.take_profit.unwrap_or(current_price),
            TradeStatus::StopLossHit => trade.stop_loss.unwrap_or(current_price),
            _ => current_price,
        };

        let price_diff = if is_buy {
            effective_price - trade.entry_price
        } else {
            trade.entry_price - effective_price
        };

        let pip_value = Self::get_pip_value_static(&trade.symbol);
        let pnl_pips = price_diff / pip_value;
        let pnl_dollars = pnl_pips * position_size * 10.0; // Assuming $10 per pip

        (pnl_dollars, pnl_pips)
    }

    fn get_pip_value_static(symbol: &str) -> f64 {
        // Standard pip values for major pairs
        match symbol {
            "USDJPY" => 0.01,
            s if s.contains("JPY") => 0.01,
            _ => 0.0001,
        }
    }

    fn calculate_default_levels(
        &self,
        action: &str,
        entry_price: f64,
        symbol: &str,
        default_tp_pips: Option<f64>,
        default_sl_pips: Option<f64>,
    ) -> (Option<f64>, Option<f64>) {
        let pip_value = Self::get_pip_value_static(symbol);
        let is_buy = action == "BUY";

        let take_profit = default_tp_pips.map(|pips| {
            if is_buy {
                entry_price + (pips * pip_value)
            } else {
                entry_price - (pips * pip_value)
            }
        });

        let stop_loss = default_sl_pips.map(|pips| {
            if is_buy {
                entry_price - (pips * pip_value)
            } else {
                entry_price + (pips * pip_value)
            }
        });

        (take_profit, stop_loss)
    }

    fn calculate_summary(&self) -> TradingSummary {
        let mut winning_trades = 0;
        let mut losing_trades = 0;
        let mut break_even_trades = 0;
        let mut total_wins = 0.0f64;
        let mut total_losses = 0.0f64;
        let mut largest_win = 0.0f64;
        let mut largest_loss = 0.0f64;

        for trade in self.tracked_trades.values() {
            if let Some(pnl) = trade.unrealized_pnl {
                if pnl > 0.0 {
                    winning_trades += 1;
                    total_wins += pnl;
                    largest_win = largest_win.max(pnl);
                } else if pnl < 0.0 {
                    losing_trades += 1;
                    total_losses += pnl.abs();
                    largest_loss = largest_loss.max(pnl.abs());
                } else {
                    break_even_trades += 1;
                }
            }
        }

        let total_trades = self.tracked_trades.len() as f64;
        let win_rate = if total_trades > 0.0 {
            winning_trades as f64 / total_trades * 100.0
        } else {
            0.0
        };

        let average_win = if winning_trades > 0 {
            total_wins / winning_trades as f64
        } else {
            0.0
        };

        let average_loss = if losing_trades > 0 {
            total_losses / losing_trades as f64
        } else {
            0.0
        };

        let profit_factor = if total_losses > 0.0 {
            total_wins / total_losses
        } else if total_wins > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        TradingSummary {
            winning_trades,
            losing_trades,
            break_even_trades,
            win_rate,
            average_win,
            average_loss,
            largest_win,
            largest_loss,
            profit_factor,
        }
    }

    pub fn get_tracked_trades(&self) -> &HashMap<String, TrackedTrade> {
        &self.tracked_trades
    }
}