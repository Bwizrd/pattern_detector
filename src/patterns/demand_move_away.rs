use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;
use serde_json::Value;

/// Demand Move Away Recognizer
/// 
/// Detects significant bullish price moves that may indicate movement away from a demand zone.
/// Focuses on pure price movement without volume requirements.
/// Detects both single-candle and multi-candle (cumulative) bullish moves.
pub struct DemandMoveAwayRecognizer {
    /// Minimum number of candles required for pattern detection and ATR calculation
    pub min_candles_required: usize,
    /// Number of candles to use for ATR calculation
    pub atr_period: usize,
    /// Maximum number of candles to consider for a single price move
    pub max_move_candles: usize,
    /// Minimum move size as a multiple of ATR to be considered significant
    pub min_atr_multiple: f64,
    /// Minimum percentage price increase to be considered significant
    pub min_percentage_move: f64,
    /// Minimum candles before a move to look for potential demand zones
    pub lookback_candles: usize,
}

impl Default for DemandMoveAwayRecognizer {
    fn default() -> Self {
        Self {
            min_candles_required: 20,
            atr_period: 14,
            max_move_candles: 5,      // Up to 5 candles for multi-candle moves
            min_atr_multiple: 1.0,    // 1.0× ATR minimum
            min_percentage_move: 0.3, // 0.3% minimum for forex
            lookback_candles: 5,
        }
    }
}

impl PatternRecognizer for DemandMoveAwayRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Value {
        if candles.len() < self.min_candles_required {
            return json!({
                "pattern": "demand_move_away",
                "error": "Not enough candles for detection",
                "total_bars": candles.len(),
                "required_bars": self.min_candles_required,
                "total_detected": 0,
                "datasets": {
                    "Price": 0,
                    "Oscillators": 0,
                    "Lines": 0
                },
                "data": {
                    "price": {
                        "demand_zones": {
                            "total": 0,
                            "zones": []
                        }
                    },
                    "oscillators": [],
                    "lines": []
                }
            });
        }

        // Calculate ATR
        let atr = self.calculate_atr(candles, self.atr_period);
        
        // Find significant moves
        let demand_moves = self.detect_significant_moves(candles, atr);
        
        let move_count = demand_moves.len();
        
        // Return the detected moves
        json!({
            "pattern": "demand_move_away",
            "total_bars": candles.len(),
            "total_detected": move_count,
            "average_true_range": atr,
            "datasets": {
                "price": 1,
                "oscillators": 0,
                "lines": 0
            },
            "data": {
                "price": {
                    "demand_zones": {
                        "total": move_count,
                        "zones": demand_moves
                    }
                },
                "oscillators": [],
                "lines": []
            }
        })
    }
}

impl DemandMoveAwayRecognizer {
    /// Calculate Average True Range (ATR) for volatility reference
    fn calculate_atr(&self, candles: &[CandleData], period: usize) -> f64 {
        if candles.len() < period + 1 {
            // Not enough data for ATR calculation
            return 0.0;
        }
        
        let mut tr_sum = 0.0;
        let mut prev_close = candles[0].close;
        
        for i in 1..=period {
            let candle = &candles[i];
            
            // True Range calculation
            let tr = (candle.high - candle.low)
                .max((candle.high - prev_close).abs())
                .max((candle.low - prev_close).abs());
                
            tr_sum += tr;
            prev_close = candle.close;
        }
        
        // Return Average True Range
        tr_sum / period as f64
    }
    
    /// Detect significant bullish price moves
    fn detect_significant_moves(&self, candles: &[CandleData], atr: f64) -> Vec<Value> {
        let mut moves: Vec<Value> = Vec::new();
        
        // Skip the first few candles for ATR context
        let start_idx = self.atr_period.max(5);
        
        if atr == 0.0 || start_idx >= candles.len() {
            return moves;
        }
        
        // 1. First pass: Detect single bullish candles with significant body size
        for i in start_idx..candles.len() {
            let candle = &candles[i];
            
            // Skip if not a bullish candle
            if candle.close <= candle.open {
                continue;
            }
            
            // Calculate candle body size
            let body_size = candle.close - candle.open;
            
            // Calculate as multiple of ATR
            let atr_multiple = body_size / atr;
            
            // Calculate as percentage
            let percentage_move = (body_size / candle.open) * 100.0;
            
            // Skip if the move isn't significant enough
            if atr_multiple < self.min_atr_multiple * 1.5 && percentage_move < self.min_percentage_move * 1.5 {
                continue;
            }
            
            // Calculate body to range ratio
            let body_to_range_ratio = body_size / (candle.high - candle.low);
            
            // Skip candles with small bodies relative to their range
            if body_to_range_ratio < 0.6 {
                continue;
            }
            
            // Single-candle move detected
            let strength_score = 
                (atr_multiple / self.min_atr_multiple) * 40.0 + 
                (percentage_move / self.min_percentage_move) * 40.0 + 
                body_to_range_ratio * 20.0;
            
            moves.push(json!({
                "category": "Price",
                "type": "demand_move",
                "start_time": candle.time,
                "end_time": candle.time,
                "zone_high": candle.close,
                "zone_low": candle.open,
                "fifty_percent_line": (candle.open + candle.close) / 2.0,
                "start_idx": i,
                "end_idx": i,
                "atr_multiple": atr_multiple,
                "percentage_move": percentage_move,
                "move_candles": 1,
                "move_speed": percentage_move,
                "body_to_range_ratio": body_to_range_ratio,
                "strength_score": strength_score,
                "detection_method": "single_candle"
            }));
        }
        
        // 2. Second pass: Detect multi-candle cumulative moves (2-5 candles)
        for window_size in 2..=self.max_move_candles {
            if start_idx + window_size >= candles.len() {
                continue;
            }
            
            for i in start_idx..candles.len() - window_size {
                // Calculate cumulative price change over the window
                let window_start = i;
                let window_end = i + window_size - 1;
                
                let start_price = candles[window_start].open;
                let end_price = candles[window_end].close;
                
                // Skip if not a net bullish move
                if end_price <= start_price {
                    continue;
                }
                
                // Calculate the size of the cumulative move
                let move_size = end_price - start_price;
                
                // Calculate as multiple of ATR
                let atr_multiple = move_size / atr;
                
                // Calculate as percentage
                let percentage_move = (move_size / start_price) * 100.0;
                
                // Skip if the move isn't significant enough
                if atr_multiple < self.min_atr_multiple && percentage_move < self.min_percentage_move {
                    continue;
                }
                
                // Count bearish and bullish candles in the window
                let mut bullish_count = 0;
                let mut bearish_count = 0;
                let mut move_high = start_price;
                let mut move_low = start_price;
                
                for j in window_start..=window_end {
                    let c = &candles[j];
                    if c.close > c.open {
                        bullish_count += 1;
                    } else if c.close < c.open {
                        bearish_count += 1;
                    }
                    move_high = move_high.max(c.high);
                    move_low = move_low.min(c.low);
                }
                
                // Skip if not predominantly bullish
                if bullish_count <= bearish_count {
                    continue;
                }
                
                // Calculate move speed (percentage gain per candle)
                let move_speed = percentage_move / window_size as f64;
                
                // Calculate directness (how direct the move was)
                let theoretical_direct_move = end_price - start_price;
                let actual_path = (move_high - move_low) + (end_price - start_price);
                let directness = theoretical_direct_move / actual_path;
                
                // Calculate a "strength score" for this move
                let strength_score = 
                    (atr_multiple / self.min_atr_multiple) * 35.0 + 
                    (percentage_move / self.min_percentage_move) * 35.0 + 
                    directness * 20.0 +
                    (move_speed / (self.min_percentage_move / 2.0)) * 10.0;
                
                // Multi-candle move detected
                moves.push(json!({
                    "category": "Price",
                    "type": "demand_move",
                    "start_time": candles[window_start].time,
                    "end_time": candles[window_end].time,
                    "zone_high": end_price,
                    "zone_low": start_price,
                    "fifty_percent_line": (start_price + end_price) / 2.0,
                    "start_idx": window_start,
                    "end_idx": window_end,
                    "atr_multiple": atr_multiple,
                    "percentage_move": percentage_move,
                    "move_candles": window_size,
                    "move_speed": move_speed,
                    "bullish_count": bullish_count,
                    "bearish_count": bearish_count,
                    "directness": directness,
                    "strength_score": strength_score,
                    "detection_method": "multi_candle"
                }));
            }
        }
        
        // Sort moves by strength score (highest first)
        moves.sort_by(|a, b| {
            let a_score = a["strength_score"].as_f64().unwrap_or(0.0);
            let b_score = b["strength_score"].as_f64().unwrap_or(0.0);
            b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        // Remove overlapping moves, keeping only the strongest
        let mut filtered_moves: Vec<Value> = Vec::new();
        for m in moves {
            let start_idx = m["start_idx"].as_u64().unwrap_or(0) as usize;
            let end_idx = m["end_idx"].as_u64().unwrap_or(0) as usize;
            
            let mut overlaps = false;
            for existing in &filtered_moves {
                let existing_start = existing["start_idx"].as_u64().unwrap_or(0) as usize;
                let existing_end = existing["end_idx"].as_u64().unwrap_or(0) as usize;
                
                // Check for significant overlap
                if (start_idx >= existing_start && start_idx <= existing_end) ||
                   (end_idx >= existing_start && end_idx <= existing_end) ||
                   (start_idx <= existing_start && end_idx >= existing_end) {
                    
                    // If they overlap substantially, keep only the stronger one
                    let current_score = m["strength_score"].as_f64().unwrap_or(0.0);
                    let existing_score = existing["strength_score"].as_f64().unwrap_or(0.0);
                    
                    overlaps = true;
                    break;
                }
            }
            
            if !overlaps {
                filtered_moves.push(m);
            }
        }
        
        filtered_moves
    }
}