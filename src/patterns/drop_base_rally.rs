use crate::detect::CandleData;
use crate::patterns::PatternRecognizer;
use serde_json::json;

/// Drop-Base-Rally (DBR) Pattern Recognizer based on deep research
/// 
/// As per research, DBR consists of:
/// 1. A bearish impulsive wave (drop) with body/wick ratio > 70%
/// 2. One or more base candles with body/wick ratio < 25%
/// 3. A bullish impulsive wave (rally) with body/wick ratio > 70%
pub struct DropBaseRallyRecognizer {
    /// Minimum body to wick ratio for impulse candles (70% as per research)
    pub min_body_to_wick_ratio_for_impulse: f64,
    /// Maximum body to wick ratio for base candles (25% as per research)
    pub max_body_to_wick_ratio_for_base: f64,
    /// Minimum number of candles required in the base
    pub min_base_candles: usize,
    /// Maximum number of candles allowed in the base
    pub max_base_candles: usize,
    /// Maximum price range percentage allowed in the base
    pub max_base_range_percent: f64,
    /// Minimum distance between patterns (in candles) to avoid overlaps
    pub min_pattern_distance: usize,
    /// Look for quick return to the zone (strong zone)
    pub detect_retest: bool,
    /// Maximum candles to look ahead for retest
    pub max_retest_lookforward: usize,
}

impl Default for DropBaseRallyRecognizer {
    fn default() -> Self {
        Self {
            min_body_to_wick_ratio_for_impulse: 0.7, // 70% as per research
            max_body_to_wick_ratio_for_base: 0.25,   // 25% as per research
            min_base_candles: 1,
            max_base_candles: 5, 
            max_base_range_percent: 2.0,
            min_pattern_distance: 10,
            detect_retest: true,
            max_retest_lookforward: 20,
        }
    }
}

impl PatternRecognizer for DropBaseRallyRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        // Ensure we have enough candles for the minimum pattern detection
        let min_required_candles = 2 + self.min_base_candles; // Drop + Base + Rally
        
        if candles.len() < min_required_candles {
            return json!({
                "pattern": "drop_base_rally",
                "error": "Not enough candles for detection",
                "total_bars": candles.len(),
                "required_bars": min_required_candles,
                "total_detected": 0,
                "datasets": {
                    "Price": 0,
                    "Oscillators": 0,
                    "Lines": 0
                },
                "data": {
                    "price": {
                        "demand_zones": {
                            "Total": 0,
                            "zones": []
                        }
                    },
                    "oscillators": [],
                    "lines": []
                }
            });
        }

        let mut all_patterns = Vec::new();

        // Find all potential DBR patterns
        for i in 0..candles.len() - (min_required_candles - 1) {
            if let Some(dbr_pattern) = self.find_dbr_pattern_starting_at(candles, i) {
                all_patterns.push(dbr_pattern);
            }
        }

        // Filter out overlapping patterns by quality score
        let mut filtered_patterns = Vec::new();
        
        if !all_patterns.is_empty() {
            // Sort patterns by quality score (highest first)
            all_patterns.sort_by(|a, b| {
                let a_score = a["pattern_details"]["quality_score"].as_u64().unwrap_or(0);
                let b_score = b["pattern_details"]["quality_score"].as_u64().unwrap_or(0);
                b_score.cmp(&a_score) // Descending order
            });
            
            // Add the highest quality pattern first
            filtered_patterns.push(all_patterns[0].clone());
            
            // Only add subsequent patterns if they don't overlap with existing ones
            for pattern in &all_patterns[1..] {
                let pattern_start_idx = pattern["pattern_details"]["drop_candle_idx"].as_u64().unwrap_or(0) as usize;
                let pattern_end_idx = pattern["pattern_details"]["rally_candle_idx"].as_u64().unwrap_or(0) as usize;
                
                let mut overlaps = false;
                
                for existing_pattern in &filtered_patterns {
                    let existing_start_idx = existing_pattern["pattern_details"]["drop_candle_idx"].as_u64().unwrap_or(0) as usize;
                    let existing_end_idx = existing_pattern["pattern_details"]["rally_candle_idx"].as_u64().unwrap_or(0) as usize;
                    
                    // Check if patterns are too close to each other - avoiding underflow
                    let existing_min_idx = if existing_start_idx > self.min_pattern_distance {
                        existing_start_idx - self.min_pattern_distance
                    } else {
                        0
                    };
                    
                    let existing_max_idx = existing_end_idx + self.min_pattern_distance;
                    
                    if (pattern_start_idx >= existing_start_idx && pattern_start_idx <= existing_max_idx) ||
                       (pattern_end_idx >= existing_min_idx && pattern_end_idx <= existing_end_idx) ||
                       (pattern_start_idx <= existing_start_idx && pattern_end_idx >= existing_end_idx) {
                        overlaps = true;
                        break;
                    }
                }
                
                if !overlaps {
                    filtered_patterns.push(pattern.clone());
                }
            }
        }

        // Count the number of patterns detected
        let pattern_count = filtered_patterns.len();

        // Return the detected patterns
        json!({
            "pattern": "drop_base_rally",
            "total_bars": candles.len(),
            "total_detected": pattern_count,
            "datasets": {
                "price": 1, // One dataset: demand_zones
                "oscillators": 0,
                "lines": 0
            },
            "data": {
                "price": {
                    "demand_zones": {
                        "total": pattern_count,
                        "zones": filtered_patterns
                    }
                },
                "oscillators": [],
                "lines": []
            }
        })
    }
}

impl DropBaseRallyRecognizer {
    /// Calculate the body to wick ratio of a candle
    fn calculate_body_to_wick_ratio(&self, candle: &CandleData) -> f64 {
        let body_size = (candle.close - candle.open).abs();
        let total_range = candle.high - candle.low;
        
        if total_range == 0.0 {
            return 1.0; // Avoid division by zero
        }
        
        body_size / total_range
    }
    
    /// Check if a candle is bearish (close < open)
    fn is_bearish_candle(&self, candle: &CandleData) -> bool {
        candle.close < candle.open
    }
    
    /// Check if a candle is bullish (close > open)
    fn is_bullish_candle(&self, candle: &CandleData) -> bool {
        candle.close > candle.open
    }
    
    /// Check if a candle is a drop candle (bearish impulse with strong body)
    fn is_drop_candle(&self, candle: &CandleData) -> bool {
        self.is_bearish_candle(candle) && 
        self.calculate_body_to_wick_ratio(candle) >= self.min_body_to_wick_ratio_for_impulse
    }
    
    /// Check if a candle is a rally candle (bullish impulse with strong body)
    fn is_rally_candle(&self, candle: &CandleData) -> bool {
        self.is_bullish_candle(candle) && 
        self.calculate_body_to_wick_ratio(candle) >= self.min_body_to_wick_ratio_for_impulse
    }
    
    /// Check if a candle is a base candle (small body relative to wicks)
    fn is_base_candle(&self, candle: &CandleData) -> bool {
        self.calculate_body_to_wick_ratio(candle) <= self.max_body_to_wick_ratio_for_base
    }
    
    /// Find a Drop-Base-Rally pattern starting at the given index
    fn find_dbr_pattern_starting_at(&self, candles: &[CandleData], start_idx: usize) -> Option<serde_json::Value> {
        // Check if we have enough candles left
        let min_required = 2 + self.min_base_candles; // Drop + Base + Rally
        if start_idx + min_required > candles.len() {
            return None;
        }
        
        // First candle must be a drop candle (bearish impulse)
        if !self.is_drop_candle(&candles[start_idx]) {
            return None;
        }
        
        let drop_candle = &candles[start_idx];
        let drop_idx = start_idx;
        
        // Find base candles
        let base_start_idx = drop_idx + 1;
        let mut base_end_idx = base_start_idx;
        let mut base_high = f64::MIN;
        let mut base_low = f64::MAX;
        let mut base_candles = Vec::new();
        
        // Look for consecutive base candles
        for i in base_start_idx..candles.len().min(base_start_idx + self.max_base_candles) {
            let current_candle = &candles[i];
            
            // If not a base candle, stop looking
            if !self.is_base_candle(current_candle) {
                break;
            }
            
            // Add to the base
            base_high = base_high.max(current_candle.high);
            base_low = base_low.min(current_candle.low);
            base_candles.push(i);
            base_end_idx = i;
            
            // Check if base range exceeds maximum
            let base_range_percent = (base_high - base_low) / base_low * 100.0;
            if base_range_percent > self.max_base_range_percent {
                // Remove the last candle that made the range too large
                base_candles.pop();
                if !base_candles.is_empty() {
                    base_end_idx = *base_candles.last().unwrap();
                } else {
                    base_end_idx = base_start_idx - 1; // No valid base
                }
                break;
            }
        }
        
        // Ensure we found enough base candles
        if base_candles.len() < self.min_base_candles {
            return None;
        }
        
        // Recalculate base boundaries
        base_high = f64::MIN;
        base_low = f64::MAX;
        for &idx in &base_candles {
            let candle = &candles[idx];
            base_high = base_high.max(candle.high);
            base_low = base_low.min(candle.low);
        }
        
        // Look for rally candle after the base
        let rally_idx = base_end_idx + 1;
        if rally_idx >= candles.len() {
            return None;
        }
        
        // Rally candle must be a bullish impulse
        if !self.is_rally_candle(&candles[rally_idx]) {
            return None;
        }
        
        let rally_candle = &candles[rally_idx];
        
        // Calculate pattern quality score (0-100)
        let quality_score = self.calculate_pattern_quality(
            drop_candle, 
            rally_candle, 
            &base_candles.iter().map(|&idx| &candles[idx]).collect::<Vec<_>>(),
            candles,
            rally_idx
        );
        
        // Look for retest of the zone if enabled
        let mut retest_detected = false;
        let mut retest_candle_idx = 0;
        let mut retest_quality = 0;
        
        if self.detect_retest && rally_idx + 1 < candles.len() {
            for i in rally_idx + 1..candles.len().min(rally_idx + 1 + self.max_retest_lookforward) {
                let current_candle = &candles[i];
                
                // Check if price has returned to the zone
                if current_candle.low <= base_high && current_candle.high >= base_low {
                    retest_detected = true;
                    retest_candle_idx = i;
                    
                    // Calculate retest quality
                    if self.is_bullish_candle(current_candle) {
                        retest_quality = 100; // Bullish candle at retest is ideal
                    } else {
                        retest_quality = 50;  // Bearish candle at retest is acceptable
                    }
                    
                    break;
                }
            }
        }
        
        // Extend the zone to the right
        let mut zone_end_index = rally_idx;
        
        // Extend the zone until a future candle touches or breaks below the base high
        if rally_idx + 1 < candles.len() {
            for k in (rally_idx + 1)..candles.len() {
                if candles[k].low <= base_high {
                    break;
                }
                zone_end_index = k;
            }
            
            // Extend one more candle to show price entering the box, if possible
            zone_end_index = std::cmp::min(zone_end_index + 2, candles.len() - 1);
        }
        
        // We've found a valid DBR pattern
        Some(json!({
            "category": "Price",
            "type": "demand_zone",
            "start_time": candles[drop_idx].time,
            "end_time": candles[zone_end_index].time,  // Use extended end time
            // The demand zone is defined by the base's boundaries
            "zone_high": base_high,
            "zone_low": base_low,
            "fifty_percent_line": (base_high + base_low) / 2.0,
            // Meta information about the pattern
            "pattern_details": {
                "drop_candle_idx": drop_idx,
                "base_start_idx": base_start_idx,
                "base_end_idx": base_end_idx,
                "rally_candle_idx": rally_idx,
                "zone_end_idx": zone_end_index,
                "base_candle_count": base_candles.len(),
                "base_candle_indices": base_candles,
                "drop_body_to_wick_ratio": self.calculate_body_to_wick_ratio(drop_candle),
                "rally_body_to_wick_ratio": self.calculate_body_to_wick_ratio(rally_candle),
                "base_range_percent": (base_high - base_low) / base_low * 100.0,
                "quality_score": quality_score,
                "retest_detected": retest_detected,
                "retest_candle_idx": retest_candle_idx,
                "retest_quality": retest_quality,
                "zone_strength": if retest_detected { 
                    if retest_candle_idx - rally_idx <= 5 { "Strong" } else { "Moderate" }
                } else { 
                    "Unknown" 
                }
            }
        }))
    }
    
    /// Calculate the quality score of a pattern (0-100)
    fn calculate_pattern_quality(
        &self,
        drop_candle: &CandleData,
        rally_candle: &CandleData,
        base_candles: &[&CandleData],
        all_candles: &[CandleData],
        rally_idx: usize
    ) -> u64 {
        let mut score = 0;
        
        // 1. Drop candle strength (0-25 points)
        let drop_body_ratio = self.calculate_body_to_wick_ratio(drop_candle);
        // Scale from min ratio (70%) to ideal (100%)
        score += ((drop_body_ratio - self.min_body_to_wick_ratio_for_impulse) / 
                (1.0 - self.min_body_to_wick_ratio_for_impulse) * 25.0) as u64;
        
        // 2. Rally candle strength (0-25 points)
        let rally_body_ratio = self.calculate_body_to_wick_ratio(rally_candle);
        // Scale from min ratio (70%) to ideal (100%)
        score += ((rally_body_ratio - self.min_body_to_wick_ratio_for_impulse) / 
                (1.0 - self.min_body_to_wick_ratio_for_impulse) * 25.0) as u64;
        
        // 3. Base quality (0-25 points)
        let base_score = if base_candles.len() >= self.min_base_candles {
            // Score based on base candle count (ideal is 2-3 candles)
            let count_score = if base_candles.len() <= 3 {
                25
            } else {
                25 - (base_candles.len() - 3) * 5 // Deduct 5 points per additional candle
            };
            
            // Score based on base range (lower is better)
            let base_high = base_candles.iter().map(|c| c.high).fold(f64::MIN, f64::max);
            let base_low = base_candles.iter().map(|c| c.low).fold(f64::MAX, f64::min);
            let base_range_percent = (base_high - base_low) / base_low * 100.0;
            let range_score = ((self.max_base_range_percent - base_range_percent) / 
                             self.max_base_range_percent * 25.0) as u64;
            
            // Average of count score and range score
            (count_score as u64 + range_score) / 2
        } else {
            0
        };
        
        score += base_score;
        
        // 4. Price behavior after rally (0-25 points)
        // Higher score if price continues upward after the rally
        if rally_idx + 1 < all_candles.len() {
            let next_candle = &all_candles[rally_idx + 1];
            
            // Full points if next candle is bullish
            if self.is_bullish_candle(next_candle) {
                score += 25;
            }
            // Half points if next candle closes above previous close
            else if next_candle.close > rally_candle.close {
                score += 12;
            }
        } else {
            // If rally is the last candle, give benefit of doubt
            score += 15;
        }
        
        // Ensure score doesn't exceed 100
        score.min(100)
    }
}