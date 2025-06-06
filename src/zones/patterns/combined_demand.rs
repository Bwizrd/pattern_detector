use crate::api::detect::CandleData;
use crate::zones::patterns::PatternRecognizer;
use serde_json::json;
use serde_json::Value;

/// Combined Demand Recognizer
/// 
/// Detects both significant bullish moves (demand moves) and their preceding
/// demand zones in a single pattern recognizer.
pub struct CombinedDemandRecognizer {
    // Demand Move parameters
    pub min_candles_required: usize,
    pub atr_period: usize,
    pub max_move_candles: usize,
    pub min_atr_multiple: f64,
    pub min_percentage_move: f64,
    pub lookback_candles: usize,
    
    // Demand Zone parameters
    pub max_lookback: usize,
    pub min_zone_candles: usize,
    pub max_zone_height_percent: f64,
    pub min_move_to_zone_ratio: f64,
    pub consolidation_range_percent: f64,
}

impl Default for CombinedDemandRecognizer {
    fn default() -> Self {
        Self {
            // Demand Move parameters
            min_candles_required: 20,
            atr_period: 14,
            max_move_candles: 5,
            min_atr_multiple: 1.0,
            min_percentage_move: 0.3,
            lookback_candles: 5,
            
            // Demand Zone parameters
            max_lookback: 10,
            min_zone_candles: 1,
            max_zone_height_percent: 5.0,
            min_move_to_zone_ratio: 0.8,
            consolidation_range_percent: 0.3,
        }
    }
}

impl PatternRecognizer for CombinedDemandRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Value {
        if candles.len() < self.min_candles_required {
            return json!({
                "pattern": "combined_demand",
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

        // Calculate ATR for move detection
        let atr = self.calculate_atr(candles, self.atr_period);
        
        // 1. Detect demand moves
        let demand_moves = self.detect_significant_moves(candles, atr);
        
        // 2. Detect demand zones based on those moves
        let demand_zones = self.find_demand_zones(candles, &demand_moves);
        
        // 3. Combine both into a single result
        let combined_zones = self.combine_moves_and_zones(demand_moves, demand_zones);
        
        let zone_count = combined_zones.len();
        
        // Return the results in the expected format
        json!({
            "pattern": "combined_demand",
            "total_bars": candles.len(),
            "total_detected": zone_count,
            "average_true_range": atr,
            "datasets": {
                "price": 1,
                "oscillators": 0,
                "lines": 0
            },
            "data": {
                "price": {
                    "demand_zones": {
                        "total": zone_count,
                        "zones": combined_zones
                    }
                },
                "oscillators": [],
                "lines": []
            }
        })
    }
}

impl CombinedDemandRecognizer {
    /// Combine demand moves and zones into a single list for display
    fn combine_moves_and_zones(&self, moves: Vec<Value>, zones: Vec<Value>) -> Vec<Value> {
        let mut combined = Vec::new();
        
        // Add all moves
        for m in moves {
            combined.push(m);
        }
        
        // Add all zones
        for z in zones {
            combined.push(z);
        }
        
        combined
    }
    
    /// Calculate Average True Range (ATR) for volatility reference
    fn calculate_atr(&self, candles: &[CandleData], period: usize) -> f64 {
        if candles.len() < period + 1 {
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
                    directness * 30.0;
                
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
                    // let current_score = m["strength_score"].as_f64().unwrap_or(0.0);
                    // let existing_score = existing["strength_score"].as_f64().unwrap_or(0.0);
                    
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
    
    /// Find demand zones that preceded significant bullish moves
    fn find_demand_zones(&self, candles: &[CandleData], demand_moves: &[Value]) -> Vec<Value> {
        let mut zones = Vec::new();
        
        for move_data in demand_moves {
            let start_idx = move_data["start_idx"].as_u64().unwrap_or(0) as usize;
            let end_idx = move_data["end_idx"].as_u64().unwrap_or(0) as usize;
            let detection_method = move_data["detection_method"].as_str().unwrap_or("");
            
            // Skip if not enough candles before the move
            if start_idx < 1 {
                continue;
            }
            
            // Detect the appropriate zone based on the move type
            match detection_method {
                "single_candle" => {
                    if let Some(zone) = self.detect_zone_before_single_candle(candles, start_idx) {
                        zones.push(zone);
                    }
                },
                "multi_candle" => {
                    if let Some(zone) = self.detect_zone_before_multi_candle(candles, start_idx, end_idx) {
                        zones.push(zone);
                    }
                },
                _ => continue
            }
        }
        
        // Filter out overlapping zones
        self.filter_overlapping_zones(zones)
    }
    
    /// Detect a demand zone before a single strong bullish candle
    fn detect_zone_before_single_candle(&self, candles: &[CandleData], move_idx: usize) -> Option<Value> {
        // Can't look back if we're at the beginning
        if move_idx < 1 {
            return None;
        }
        
        let move_candle = &candles[move_idx];
        let lookback_start = if move_idx > self.max_lookback {
            move_idx - self.max_lookback
        } else {
            0
        };
        
        // Try different detection methods and take the best result
        let mut potential_zones = Vec::new();
        
        // Method 1: Look for a recent swing low
        if let Some(zone) = self.detect_swing_low_zone(candles, lookback_start, move_idx, move_candle) {
            potential_zones.push(zone);
        }
        
        // Method 2: Look for a base/consolidation pattern
        if let Some(zone) = self.detect_base_consolidation_zone(candles, lookback_start, move_idx, move_candle) {
            potential_zones.push(zone);
        }
        
        // Method 3: Look for a bearish candle followed by a consolidation
        if let Some(zone) = self.detect_dbr_pattern_zone(candles, lookback_start, move_idx, move_candle) {
            potential_zones.push(zone);
        }
        
        // Choose the best zone based on quality score
        potential_zones.sort_by(|a, b| {
            let a_score = a["quality_score"].as_f64().unwrap_or(0.0);
            let b_score = b["quality_score"].as_f64().unwrap_or(0.0);
            b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        potential_zones.first().cloned()
    }
    
    /// Detect a demand zone before a multi-candle bullish move
    fn detect_zone_before_multi_candle(&self, candles: &[CandleData], start_idx: usize, end_idx: usize) -> Option<Value> {
        // Can't look back if we're at the beginning
        if start_idx < 1 {
            return None;
        }
        
        let lookback_start = if start_idx > self.max_lookback {
            start_idx - self.max_lookback
        } else {
            0
        };
        
        // For multi-candle moves, we treat the first candle as the reference
        let move_candle = &candles[start_idx];
        
        // Try different detection methods and take the best result
        let mut potential_zones = Vec::new();
        
        // Method 1: Look for a recent swing low
        if let Some(zone) = self.detect_swing_low_zone(candles, lookback_start, start_idx, move_candle) {
            potential_zones.push(zone);
        }
        
        // Method 2: Look for a base/consolidation pattern
        if let Some(zone) = self.detect_base_consolidation_zone(candles, lookback_start, start_idx, move_candle) {
            potential_zones.push(zone);
        }
        
        // Method 3: Look for a bearish candle followed by a consolidation
        if let Some(zone) = self.detect_dbr_pattern_zone(candles, lookback_start, start_idx, move_candle) {
            potential_zones.push(zone);
        }
        
        // Choose the best zone based on quality score
        potential_zones.sort_by(|a, b| {
            let a_score = a["quality_score"].as_f64().unwrap_or(0.0);
            let b_score = b["quality_score"].as_f64().unwrap_or(0.0);
            b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        potential_zones.first().cloned()
    }
    
    /// Detect a swing low demand zone
    fn detect_swing_low_zone(&self, candles: &[CandleData], start_idx: usize, end_idx: usize, move_candle: &CandleData) -> Option<Value> {
        if end_idx <= start_idx || end_idx >= candles.len() {
            return None;
        }
        
        // Find the lowest low in the lookback period
        let mut min_idx = start_idx;
        let mut min_low = candles[start_idx].low;
        
        for i in start_idx + 1..end_idx {
            if candles[i].low < min_low {
                min_low = candles[i].low;
                min_idx = i;
            }
        }
        
        // Check if it's a genuine swing low
        let is_swing_low = (min_idx > start_idx) && (min_idx < end_idx - 1);
        if !is_swing_low {
            return None;
        }
        
        // Create the zone boundaries
        let swing_candle = &candles[min_idx];
        let zone_low = swing_candle.low * 0.9995; // Just below the low
        
        // For the zone height, we use the preceding candle's high
        let zone_high = if min_idx > 0 {
            candles[min_idx - 1].high
        } else {
            swing_candle.high
        };
        
        // Check if zone height is reasonable
        let zone_height_percent = (zone_high - zone_low) / zone_low * 100.0;
        if zone_height_percent > self.max_zone_height_percent {
            return None;
        }
        
        // Calculate quality score based on how clearly it's a swing low
        let pre_swing_high = if min_idx > 1 {
            candles[min_idx - 1].high
        } else if min_idx > 0 {
            candles[min_idx].high
        } else {
            swing_candle.high
        };
        
        let post_swing_high = candles[min_idx..end_idx].iter().map(|c| c.high).fold(f64::MIN, f64::max);
        
        let move_size = move_candle.close - swing_candle.low;
        let swing_depth = (pre_swing_high.min(post_swing_high) - swing_candle.low) / swing_candle.low * 100.0;
        
        let quality_score = (swing_depth / 1.0) * 50.0 + (move_size / swing_candle.low * 100.0 / 1.0) * 50.0;
        
        Some(json!({
            "category": "Price",
            "type": "demand_zone",
            "start_time": candles[min_idx].time,
            "end_time": candles[end_idx].time,
            "zone_high": zone_high,
            "zone_low": zone_low,
            "fifty_percent_line": (zone_high + zone_low) / 2.0,
            "start_idx": min_idx,
            "end_idx": end_idx,
            "detection_method": "swing_low",
            "quality_score": quality_score,
            "move_idx": end_idx
        }))
    }
    
    /// Detect a base/consolidation demand zone
    fn detect_base_consolidation_zone(&self, candles: &[CandleData], start_idx: usize, end_idx: usize, move_candle: &CandleData) -> Option<Value> {
        if end_idx <= start_idx + 1 || end_idx >= candles.len() {
            return None;
        }
        
        // Look for a tight consolidation (base) before the move
        let max_consolidation_candles = 5.min(end_idx - start_idx);
        let mut best_base_start = 0;
        let mut best_base_end = 0;
        let mut best_base_range = f64::MAX;
        let mut best_base_high = 0.0;
        let mut best_base_low = 0.0;
        
        // Try different consolidation lengths
        for base_length in 2..=max_consolidation_candles {
            let base_end_idx = end_idx - 1;
            let base_start_idx = base_end_idx.saturating_sub(base_length - 1);
            
            // Skip if not enough candles
            if base_start_idx < start_idx {
                continue;
            }
            
            // Calculate the price range of this potential base
            let base_high = candles[base_start_idx..=base_end_idx].iter().map(|c| c.high).fold(f64::MIN, f64::max);
            let base_low = candles[base_start_idx..=base_end_idx].iter().map(|c| c.low).fold(f64::MAX, f64::min);
            let base_range_percent = (base_high - base_low) / base_low * 100.0;
            
            // Check if this is a tighter range than previous candidates
            if base_range_percent < best_base_range && base_range_percent < self.consolidation_range_percent {
                best_base_start = base_start_idx;
                best_base_end = base_end_idx;
                best_base_range = base_range_percent;
                best_base_high = base_high;
                best_base_low = base_low;
            }
        }
        
        // If no valid base found
        if best_base_range == f64::MAX || best_base_end < best_base_start {
            return None;
        }
        
        // Calculate quality score
        let move_size = move_candle.close - best_base_low;
        let move_to_base_ratio = move_size / (best_base_high - best_base_low);
        
        // Require the move to be significant compared to the base size
        if move_to_base_ratio < self.min_move_to_zone_ratio {
            return None;
        }
        
        let quality_score = 
            (1.0 / best_base_range * 50.0) + // Lower range is better
            (move_to_base_ratio * 50.0);     // Higher ratio is better
        
        Some(json!({
            "category": "Price",
            "type": "demand_zone",
            "start_time": candles[best_base_start].time,
            "end_time": candles[end_idx].time,
            "zone_high": best_base_high,
            "zone_low": best_base_low * 0.9995, // Slightly below for buffer
            "fifty_percent_line": (best_base_high + best_base_low) / 2.0,
            "start_idx": best_base_start,
            "end_idx": end_idx,
            "detection_method": "base_consolidation",
            "quality_score": quality_score,
            "base_range_percent": best_base_range,
            "move_to_base_ratio": move_to_base_ratio,
            "move_idx": end_idx
        }))
    }
    
    /// Detect a DBR (Drop-Base-Rally) pattern demand zone
    fn detect_dbr_pattern_zone(&self, candles: &[CandleData], start_idx: usize, end_idx: usize, move_candle: &CandleData) -> Option<Value> {
        if end_idx <= start_idx + 2 || end_idx >= candles.len() {
            return None;
        }
        
        // Look back to find a bearish candle (drop)
        let mut drop_idx = 0;
        let mut found_drop = false;
        
        for i in (start_idx..end_idx-1).rev() {
            if candles[i].close < candles[i].open {
                drop_idx = i;
                found_drop = true;
                break;
            }
        }
        
        if !found_drop {
            return None;
        }
        
        // Look for a base between the drop and the move
        let base_start_idx = drop_idx + 1;
        let base_end_idx = end_idx - 1;
        
        // Need at least one candle for the base
        if base_end_idx < base_start_idx {
            return None;
        }
        
        // Calculate base boundaries
        let base_high = candles[base_start_idx..=base_end_idx].iter().map(|c| c.high).fold(f64::MIN, f64::max);
        let base_low = candles[base_start_idx..=base_end_idx].iter().map(|c| c.low).fold(f64::MAX, f64::min);
        let base_range_percent = (base_high - base_low) / base_low * 100.0;
        
        // Check if base is tight enough
        if base_range_percent > self.consolidation_range_percent * 1.5 {
            return None;
        }
        
        // Create zone boundaries - Drop's low to Base's high
        let zone_low = candles[drop_idx].low * 0.9995; // Slightly below drop's low
        let zone_high = base_high;
        
        // Calculate quality score
        let drop_size = (candles[drop_idx].open - candles[drop_idx].close).abs();
        let move_size = move_candle.close - zone_low;
        let move_to_drop_ratio = move_size / drop_size;
        
        let quality_score = 
            (move_to_drop_ratio * 40.0) +
            (1.0 / base_range_percent * 30.0) +
            ((base_end_idx - base_start_idx + 1) as f64 / 3.0 * 30.0); // 2-3 base candles is optimal
        
        Some(json!({
            "category": "Price",
            "type": "demand_zone",
            "start_time": candles[drop_idx].time,
            "end_time": candles[end_idx].time,
            "zone_high": zone_high,
            "zone_low": zone_low,
            "fifty_percent_line": (zone_high + zone_low) / 2.0,
            "start_idx": drop_idx,
            "end_idx": end_idx,
            "detection_method": "dbr_pattern",
            "quality_score": quality_score,
            "drop_idx": drop_idx,
            "base_start_idx": base_start_idx,
            "base_end_idx": base_end_idx,
            "base_range_percent": base_range_percent,
            "move_to_drop_ratio": move_to_drop_ratio,
            "move_idx": end_idx
        }))
    }
    
    /// Filter out overlapping demand zones, keeping only the highest quality ones
    fn filter_overlapping_zones(&self, zones: Vec<Value>) -> Vec<Value> {
        let mut filtered_zones: Vec<Value> = Vec::new();
        
        // Sort zones by quality score (highest first)
        let mut sorted_zones = zones;
        sorted_zones.sort_by(|a, b| {
            let a_score = a["quality_score"].as_f64().unwrap_or(0.0);
            let b_score = b["quality_score"].as_f64().unwrap_or(0.0);
            b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        for zone in sorted_zones {
            let zone_high = zone["zone_high"].as_f64().unwrap_or(0.0);
            let zone_low = zone["zone_low"].as_f64().unwrap_or(0.0);
            let start_idx = zone["start_idx"].as_u64().unwrap_or(0) as usize;
            let end_idx = zone["end_idx"].as_u64().unwrap_or(0) as usize;
            
            let mut overlaps = false;
            for existing in &filtered_zones {
                let existing_high = existing["zone_high"].as_f64().unwrap_or(0.0);
                let existing_low = existing["zone_low"].as_f64().unwrap_or(0.0);
                let existing_start = existing["start_idx"].as_u64().unwrap_or(0) as usize;
                let existing_end = existing["end_idx"].as_u64().unwrap_or(0) as usize;
                
                // Check for price overlap
                let price_overlap = 
                    (zone_high >= existing_low && zone_high <= existing_high) ||
                    (zone_low >= existing_low && zone_low <= existing_high) ||
                    (zone_high >= existing_high && zone_low <= existing_low);
                
                // Check for time overlap
                let time_overlap = 
                    (start_idx >= existing_start && start_idx <= existing_end) ||
                    (end_idx >= existing_start && end_idx <= existing_end) ||
                    (start_idx <= existing_start && end_idx >= existing_end);
                
                if price_overlap && time_overlap {
                    overlaps = true;
                    break;
                }
            }
            
            if !overlaps {
                filtered_zones.push(zone);
            }
        }
        
        filtered_zones
    }
}