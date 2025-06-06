use crate::api::detect::CandleData;
use crate::zones::patterns::PatternRecognizer;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;

/// Flexible Demand Zone Recognizer
/// 
/// Detects potential demand zones using multiple detection methods:
/// 1. Swing low reversals
/// 2. Strong rejection candles
/// 3. Consolidation breakouts
/// 4. Price range tests
pub struct FlexibleDemandZoneRecognizer {
    /// Minimum number of candles required for pattern detection
    pub min_candles: usize,
    /// Number of candles to look back/forward for swing points
    pub swing_lookback: usize,
    /// Minimum price movement (%) required to consider a swing significant
    pub min_swing_percent: f64,
    /// Minimum volume increase (%) for volume-confirmed zones
    pub min_volume_increase: f64,
    /// Minimum strength score to consider a demand zone valid
    pub min_strength_score: usize,
    /// Minimum distance between zones to avoid overlaps
    pub min_zone_distance: usize,
    /// Maximum zone height as percentage of price
    pub max_zone_height_percent: f64,
}

impl Default for FlexibleDemandZoneRecognizer {
    fn default() -> Self {
        Self {
            min_candles: 10,
            swing_lookback: 5,
            min_swing_percent: 1.0,
            min_volume_increase: 50.0,
            min_strength_score: 40,
            min_zone_distance: 10,
            max_zone_height_percent: 5.0,
        }
    }
}

impl PatternRecognizer for FlexibleDemandZoneRecognizer {
    fn detect(&self, candles: &[CandleData]) -> serde_json::Value {
        if candles.len() < self.min_candles {
            return json!({
                "pattern": "flexible_demand_zone",
                "error": "Not enough candles for detection",
                "total_bars": candles.len(),
                "required_bars": self.min_candles,
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

        let mut all_zones: Vec<Value> = Vec::new();

        // Method 1: Detect swing low reversals
        let swing_low_zones = self.detect_swing_low_zones(candles);
        all_zones.extend(swing_low_zones);

        // Method 2: Detect strong rejection candles
        let rejection_zones = self.detect_rejection_candle_zones(candles);
        all_zones.extend(rejection_zones);

        // Method 3: Detect consolidation breakouts
        let breakout_zones = self.detect_consolidation_breakout_zones(candles);
        all_zones.extend(breakout_zones);

        // Method 4: Detect price range tests
        let range_test_zones = self.detect_range_test_zones(candles);
        all_zones.extend(range_test_zones);

        // Sort zones by strength score (highest first)
        all_zones.sort_by(|a, b| {
            let a_score = a["strength_score"].as_u64().unwrap_or(0);
            let b_score = b["strength_score"].as_u64().unwrap_or(0);
            b_score.cmp(&a_score) // Descending order
        });

        // Filter out overlapping zones
        let mut filtered_zones: Vec<Value> = Vec::new();
        for zone in all_zones {
            let zone_start_idx = zone["start_idx"].as_u64().unwrap_or(0) as usize;
            let zone_end_idx = zone["end_idx"].as_u64().unwrap_or(0) as usize;
            let zone_high = zone["zone_high"].as_f64().unwrap_or(0.0);
            let zone_low = zone["zone_low"].as_f64().unwrap_or(0.0);
            
            let mut overlaps = false;
            
            for existing_zone in &filtered_zones {
                let existing_start_idx = existing_zone["start_idx"].as_u64().unwrap_or(0) as usize;
                let existing_end_idx = existing_zone["end_idx"].as_u64().unwrap_or(0) as usize;
                let existing_high = existing_zone["zone_high"].as_f64().unwrap_or(0.0);
                let existing_low = existing_zone["zone_low"].as_f64().unwrap_or(0.0);
                
                // Check for price and time overlaps
                let price_overlap = 
                    (zone_high >= existing_low && zone_high <= existing_high) ||
                    (zone_low >= existing_low && zone_low <= existing_high) ||
                    (zone_high >= existing_high && zone_low <= existing_low);
                
                let time_overlap = 
                    (zone_start_idx >= existing_start_idx && zone_start_idx <= existing_end_idx) ||
                    (zone_end_idx >= existing_start_idx && zone_end_idx <= existing_end_idx) ||
                    (zone_start_idx <= existing_start_idx && zone_end_idx >= existing_end_idx);
                
                if price_overlap && time_overlap {
                    overlaps = true;
                    break;
                }
                
                // Check if zones are too close
                let distance_check = 
                    zone_start_idx > existing_end_idx && zone_start_idx - existing_end_idx < self.min_zone_distance;
                
                if distance_check && price_overlap {
                    overlaps = true;
                    break;
                }
            }
            
            if !overlaps {
                filtered_zones.push(zone);
            }
        }

        // Prepare final output
        let zones_json: Vec<Value> = filtered_zones.iter().map(|zone| {
            json!({
                "category": "Price",
                "type": "demand_zone",
                "start_time": zone["start_time"],
                "end_time": zone["end_time"],
                "zone_high": zone["zone_high"],
                "zone_low": zone["zone_low"],
                "fifty_percent_line": zone["fifty_percent_line"],
                "detection_method": zone["detection_method"],
                "strength_score": zone["strength_score"],
                "retest_count": zone["retest_count"]
            })
        }).collect();

        let zone_count = zones_json.len();

        json!({
            "pattern": "flexible_demand_zone",
            "total_bars": candles.len(),
            "total_detected": zone_count,
            "datasets": {
                "price": 1,
                "oscillators": 0,
                "lines": 0
            },
            "data": {
                "price": {
                    "demand_zones": {
                        "total": zone_count,
                        "zones": zones_json
                    }
                },
                "oscillators": [],
                "lines": []
            }
        })
    }
}

impl FlexibleDemandZoneRecognizer {
    /// Detect swing low reversals that form demand zones
    fn detect_swing_low_zones(&self, candles: &[CandleData]) -> Vec<Value> {
        let mut zones = Vec::new();
        let lookback = self.swing_lookback;
        
        // Need enough candles to detect swing lows
        if candles.len() < lookback * 2 + 1 {
            return zones;
        }
        
        // Find swing lows
        for i in lookback..candles.len() - lookback {
            // Check if current candle is a potential swing low
            let current_low = candles[i].low;
            let mut is_swing_low = true;
            
            // Check if it's lower than surrounding candles
            for j in i - lookback..i {
                if candles[j].low <= current_low {
                    is_swing_low = false;
                    break;
                }
            }
            
            if !is_swing_low {
                continue;
            }
            
            for j in i + 1..=i + lookback {
                if j >= candles.len() || candles[j].low <= current_low {
                    is_swing_low = false;
                    break;
                }
            }
            
            if !is_swing_low {
                continue;
            }
            
            // Make sure the swing is significant enough
            let pre_swing_high = candles[i - lookback..i].iter().map(|c| c.high).fold(f64::MIN, f64::max);
            let post_swing_high = candles[i + 1..=i + lookback].iter().map(|c| c.high).fold(f64::MIN, f64::max);
            let swing_high = pre_swing_high.max(post_swing_high);
            
            let swing_percent = (swing_high - current_low) / current_low * 100.0;
            if swing_percent < self.min_swing_percent {
                continue;
            }
            
            // Check volume confirmation
            let avg_pre_volume: f64 = candles[i - lookback..i].iter().map(|c| c.volume as f64).sum::<f64>() / lookback as f64;
            let current_volume = candles[i].volume as f64;
            let volume_increase = (current_volume - avg_pre_volume) / avg_pre_volume * 100.0;
            
            // Create zone boundaries
            let zone_low = current_low * 0.998; // Slightly below the swing low
            let zone_height = (swing_high - current_low) * 0.5; // Half the swing height
            let zone_high = current_low + zone_height;
            
            // Check if zone height is reasonable
            let zone_height_percent = zone_height / current_low * 100.0;
            if zone_height_percent > self.max_zone_height_percent {
                continue;
            }
            
            // Extend the zone to the right until price breaks below or above
            let mut zone_end_idx = i + lookback;
            if i + lookback + 1 < candles.len() {
                for k in (i + lookback + 1)..candles.len() {
                    if candles[k].low < zone_low || candles[k].high > zone_high * 1.05 {
                        break;
                    }
                    zone_end_idx = k;
                }
            }
            
            // Calculate zone strength
            let mut strength_score = 50; // Base score
            
            // Add for volume confirmation
            if volume_increase > self.min_volume_increase {
                strength_score += 20;
            }
            
            // Add for strong bounce
            if swing_percent > self.min_swing_percent * 2.0 {
                strength_score += 15;
            }
            
            // Add for multiple tests
            let retest_count = self.count_zone_retests(candles, i, zone_end_idx, zone_high, zone_low);
            strength_score += retest_count.min(3) * 5;
            
            // Only add zones that meet minimum strength criteria
            if strength_score >= self.min_strength_score {
                zones.push(json!({
                    "start_time": candles[i].time,
                    "end_time": candles[zone_end_idx].time,
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                    "start_idx": i,
                    "end_idx": zone_end_idx,
                    "detection_method": "swing_low",
                    "strength_score": strength_score,
                    "swing_percent": swing_percent,
                    "volume_increase": volume_increase,
                    "retest_count": retest_count
                }));
            }
        }
        
        zones
    }
    
    /// Detect strong rejection candles that form demand zones
    fn detect_rejection_candle_zones(&self, candles: &[CandleData]) -> Vec<Value> {
        let mut zones = Vec::new();
        
        // Need enough candles for context
        if candles.len() < self.min_candles {
            return zones;
        }
        
        for i in 5..candles.len() - 3 {
            let candle = &candles[i];
            
            // Look for long lower wicks (rejection candles)
            let body_size = (candle.close - candle.open).abs();
            let lower_wick = candle.open.min(candle.close) - candle.low;
            let upper_wick = candle.high - candle.open.max(candle.close);
            
            // Skip if not a rejection candle
            if lower_wick <= body_size || lower_wick <= upper_wick * 2.0 {
                continue;
            }
            
            // Check if it's a bullish rejection
            let is_bullish = candle.close > candle.open;
            if !is_bullish {
                continue;
            }
            
            // Make sure we have a down move before the rejection
            let prev_candles_bearish = (0..3).any(|offset| {
                let prev_idx = i - offset - 1;
                prev_idx < candles.len() && candles[prev_idx].close < candles[prev_idx].open
            });
            
            if !prev_candles_bearish {
                continue;
            }
            
            // Make sure we have an up move after the rejection
            let next_candle = if i + 1 < candles.len() { &candles[i + 1] } else { continue };
            if next_candle.close <= candle.close {
                continue;
            }
            
            // Create zone boundaries
            let zone_low = candle.low;
            let zone_high = candle.low + (candle.high - candle.low) * 0.382; // Use Fibonacci ratio
            
            // Check if zone height is reasonable
            let zone_height_percent = (zone_high - zone_low) / zone_low * 100.0;
            if zone_height_percent > self.max_zone_height_percent {
                continue;
            }
            
            // Extend the zone to the right until price breaks below
            let mut zone_end_idx = i + 1;
            if i + 2 < candles.len() {
                for k in (i + 2)..candles.len() {
                    if candles[k].low < zone_low {
                        break;
                    }
                    zone_end_idx = k;
                }
            }
            
            // Calculate zone strength
            let mut strength_score = 40; // Base score
            
            // Add for volume on rejection candle
            let avg_volume = candles[i-5..i].iter().map(|c| c.volume as f64).sum::<f64>() / 5.0;
            if candle.volume as f64 > avg_volume * 1.5 {
                strength_score += 20;
            }
            
            // Add for strong close
            if candle.close > (candle.high + candle.low) / 2.0 {
                strength_score += 15;
            }
            
            // Add for multiple tests
            let retest_count = self.count_zone_retests(candles, i, zone_end_idx, zone_high, zone_low);
            strength_score += retest_count.min(3) * 5;
            
            // Only add zones that meet minimum strength criteria
            if strength_score >= self.min_strength_score {
                zones.push(json!({
                    "start_time": candle.time,
                    "end_time": candles[zone_end_idx].time,
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                    "start_idx": i,
                    "end_idx": zone_end_idx,
                    "detection_method": "rejection_candle",
                    "strength_score": strength_score,
                    "lower_wick_ratio": lower_wick / body_size,
                    "volume_ratio": candle.volume as f64 / avg_volume,
                    "retest_count": retest_count
                }));
            }
        }
        
        zones
    }
    
    /// Detect consolidation breakout zones
    fn detect_consolidation_breakout_zones(&self, candles: &[CandleData]) -> Vec<Value> {
        let mut zones = Vec::new();
        
        // Need enough candles for detecting consolidations
        if candles.len() < self.min_candles + 5 {
            return zones;
        }
        
        // Detect consolidation periods (low volatility followed by breakout)
        for i in 10..candles.len() - 5 {
            // Check for low volatility period (at least 5 candles)
            let window_start = i - 10;
            let window_end = i - 1;
            
            // Calculate average true range for the window
            let mut atr_sum = 0.0;
            let mut prev_close = candles[window_start].close;
            
            for j in window_start + 1..=window_end {
                let high = candles[j].high;
                let low = candles[j].low;
                let prev_close_val = prev_close;
                
                let tr = (high - low)
                    .max((high - prev_close_val).abs())
                    .max((low - prev_close_val).abs());
                
                atr_sum += tr;
                prev_close = candles[j].close;
            }
            
            let atr = atr_sum / (window_end - window_start) as f64;
            let avg_price = candles[window_start..=window_end].iter().map(|c| c.close).sum::<f64>() / 
                           (window_end - window_start + 1) as f64;
            
            // Check if the volatility is low enough (less than 0.5% of price)
            let volatility_percent = atr / avg_price * 100.0;
            if volatility_percent > 0.5 {
                continue;
            }
            
            // Find the price range of the consolidation
            let cons_high = candles[window_start..=window_end].iter().map(|c| c.high).fold(f64::MIN, f64::max);
            let cons_low = candles[window_start..=window_end].iter().map(|c| c.low).fold(f64::MAX, f64::min);
            
            // Check if consolidation range is tight enough
            let cons_range_percent = (cons_high - cons_low) / cons_low * 100.0;
            if cons_range_percent > 1.5 {
                continue;
            }
            
            // Look for a breakout after the consolidation
            let breakout_idx = i;
            let breakout_candle = &candles[breakout_idx];
            
            // Skip if not a bullish breakout
            if breakout_candle.close <= cons_high || breakout_candle.close <= breakout_candle.open {
                continue;
            }
            
            // Create zone boundaries (using the consolidation range)
            let zone_low = cons_low;
            let zone_high = cons_high;
            
            // Extend the zone to the right until price breaks below
            let mut zone_end_idx = breakout_idx;
            if breakout_idx + 1 < candles.len() {
                for k in (breakout_idx + 1)..candles.len() {
                    if candles[k].low < zone_low {
                        break;
                    }
                    zone_end_idx = k;
                }
            }
            
            // Calculate zone strength
            let mut strength_score = 45; // Base score
            
            // Add for strong breakout
            let breakout_size = (breakout_candle.close - cons_high) / cons_high * 100.0;
            if breakout_size > 1.0 {
                strength_score += 15;
            }
            
            // Add for volume on breakout
            let avg_volume = candles[window_start..=window_end].iter().map(|c| c.volume as f64).sum::<f64>() / 
                            (window_end - window_start + 1) as f64;
            
            if breakout_candle.volume as f64 > avg_volume * 1.5 {
                strength_score += 20;
            }
            
            // Add for multiple tests
            let retest_count = self.count_zone_retests(candles, breakout_idx, zone_end_idx, zone_high, zone_low);
            strength_score += retest_count.min(3) * 5;
            
            // Only add zones that meet minimum strength criteria
            if strength_score >= self.min_strength_score {
                zones.push(json!({
                    "start_time": candles[window_start].time,
                    "end_time": candles[zone_end_idx].time,
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                    "start_idx": window_start,
                    "end_idx": zone_end_idx,
                    "detection_method": "consolidation_breakout",
                    "strength_score": strength_score,
                    "consolidation_days": window_end - window_start + 1,
                    "breakout_size": breakout_size,
                    "volume_ratio": breakout_candle.volume as f64 / avg_volume,
                    "retest_count": retest_count
                }));
            }
        }
        
        zones
    }
    
    /// Detect price range tests that could form demand zones
    fn detect_range_test_zones(&self, candles: &[CandleData]) -> Vec<Value> {
        let mut zones = Vec::new();
        
        // Need enough candles for context
        if candles.len() < self.min_candles {
            return zones;
        }
        
        // Group candles into potential ranges
        let mut range_map: HashMap<u64, (usize, usize, f64, f64)> = HashMap::new();
        let price_scale = 100.0; // Scale to group nearby prices
        
        for (i, candle) in candles.iter().enumerate() {
            // Create price buckets to find commonly tested areas
            let low_bucket = (candle.low * price_scale).floor() as u64;
            
            // Update or insert the price bucket
            range_map.entry(low_bucket)
                .and_modify(|entry| {
                    // Update count and boundaries
                    entry.0 += 1;
                    entry.1 = i;
                    entry.2 = entry.2.min(candle.low);
                    entry.3 = entry.3.max(candle.high);
                })
                .or_insert((1, i, candle.low, candle.low * 1.01)); // Initial entry
        }
        
        // Find ranges that are tested multiple times
        for (_, (count, last_idx, range_low, range_high)) in range_map.iter() {
            // Skip ranges without multiple tests
            if *count < 3 {
                continue;
            }
            
            // Check if the price moved up significantly after testing this range
            if *last_idx + 5 >= candles.len() {
                continue;
            }
            
            let test_low_idx = *last_idx;
            let after_idx = test_low_idx + 5;
            
            let price_before = candles[test_low_idx].close;
            let price_after = candles[after_idx].close;
            
            let price_change = (price_after - price_before) / price_before * 100.0;
            
            // Skip if price didn't move up after testing the range
            if price_change <= 1.0 {
                continue;
            }
            
            // Create zone boundaries
            let zone_low = *range_low;
            let zone_high = *range_high;
            
            // Check if zone height is reasonable
            let zone_height_percent = (zone_high - zone_low) / zone_low * 100.0;
            if zone_height_percent > self.max_zone_height_percent {
                continue;
            }
            
            // Extend the zone to the right until price breaks below
            let mut zone_end_idx = *last_idx;
            if *last_idx + 1 < candles.len() {
                for k in (*last_idx + 1)..candles.len() {
                    if candles[k].low < zone_low {
                        break;
                    }
                    zone_end_idx = k;
                }
            }
            
            // Calculate zone strength
            let mut strength_score = 35; // Base score
            
            // Add for number of tests
            strength_score += (*count).min(5) * 4;
            
            // Add for price movement after test
            if price_change > 2.0 {
                strength_score += 15;
            }
            
            // Add for multiple tests
            let retest_count = self.count_zone_retests(candles, test_low_idx, zone_end_idx, zone_high, zone_low);
            strength_score += retest_count.min(3) * 5;
            
            // Only add zones that meet minimum strength criteria
            if strength_score >= self.min_strength_score {
                zones.push(json!({
                    "start_time": candles[test_low_idx - 2.min(test_low_idx)].time, // Start a bit before the test
                    "end_time": candles[zone_end_idx].time,
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                    "start_idx": test_low_idx - 2.min(test_low_idx),
                    "end_idx": zone_end_idx,
                    "detection_method": "price_range_test",
                    "strength_score": strength_score,
                    "test_count": count,
                    "price_change_after": price_change,
                    "retest_count": retest_count
                }));
            }
        }
        
        zones
    }
    
    /// Count how many times a zone is retested between start and end indices
    fn count_zone_retests(&self, candles: &[CandleData], start_idx: usize, end_idx: usize, zone_high: f64, zone_low: f64) -> usize {
        if start_idx >= end_idx || start_idx >= candles.len() || end_idx >= candles.len() {
            return 0;
        }
        
        let mut retest_count = 0;
        let mut in_zone = false;
        
        for i in start_idx..=end_idx {
            let candle = &candles[i];
            
            // Check if candle is testing the zone
            let is_in_zone = candle.low <= zone_high && candle.high >= zone_low;
            
            // Count a new retest when price re-enters the zone
            if is_in_zone && !in_zone {
                retest_count += 1;
            }
            
            in_zone = is_in_zone;
        }
        
        retest_count
    }
}