use crate::api::detect::CandleData;
use crate::zones::patterns::PatternRecognizer;
use serde_json::json;
use serde_json::Value;

pub struct EnhancedSupplyDemandZoneRecognizer {
    // Pattern 1 (original) parameters
    pub min_size_ratio: f64, // Second candle must be this much bigger than first
    pub extend_zone_candles: usize, // Number of additional candles to extend zone box

    // Pattern 2 (swing) parameters
    pub swing_lookback: usize, // Candles to check before/after potential swing point
    pub min_swing_strength: f64, // Minimum percentage move from swing to qualify
    pub min_reversal_candles: usize, // Minimum number of candles moving away from swing
    // rbr
    pub min_rbr_rally_percent: f64, // Minimum size of rally legs (as percentage)
    pub min_rbr_base_candles: usize, // Minimum candles in consolidation/base
    pub max_rbr_base_candles: usize, // Maximum candles in consolidation/base
    pub max_rbr_base_range: f64,    // Maximum price range in base (as percentage)
}

impl Default for EnhancedSupplyDemandZoneRecognizer {
    fn default() -> Self {
        Self {
            min_size_ratio: 1.5,
            extend_zone_candles: 2,
            swing_lookback: 3,
            min_swing_strength: 0.3, // 0.3% for forex, adjust higher for stocks/crypto
            min_reversal_candles: 2,
            // RBR parameters
            min_rbr_rally_percent: 0.4, // 0.3% minimum rally size for forex
            min_rbr_base_candles: 2,    // At least 2 candles in the base
            max_rbr_base_candles: 7,    // Up to 7 candles in the base
            max_rbr_base_range: 0.4,
        }
    }
}

impl PatternRecognizer for EnhancedSupplyDemandZoneRecognizer {
    fn detect(&self, candles: &[CandleData]) -> Value {
        // Ensure we have at least 2 candles for pattern detection
        if candles.len() < 2 {
            return json!({
                "pattern": "supply_demand_zone",
                "error": "Not enough candles for detection",
                "total_bars": candles.len(),
                "required_bars": 2,
                "total_detected": 0,
                "datasets": {
                    "Price": 0,
                    "Oscillators": 0,
                    "Lines": 0
                },
                "data": {
                    "price": {
                        "supply_zones": {
                            "Total": 0,
                            "zones": []
                        },
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

        let mut supply_zones: Vec<Value> = Vec::new();
        let mut demand_zones: Vec<Value> = Vec::new();

        // Pattern 1: Original Supply/Demand Zone detection
        self.detect_original_pattern(candles, &mut supply_zones, &mut demand_zones);

        // Pattern 2: Swing High/Low detection
        self.detect_swing_pattern(candles, &mut supply_zones, &mut demand_zones);

        // Pattern 3: RBR pattern detection (new)
        self.detect_rbr_pattern(candles, &mut demand_zones);

        // Pattern 4: Momentum shift detection
        self.detect_momentum_shift_zones(candles, &mut demand_zones);

        // Filter out overlapping zones, keeping the higher quality ones
        let demand_zones = self.filter_overlapping_zones(demand_zones, "demand");
        let supply_zones = self.filter_overlapping_zones(supply_zones, "supply");

        // Count the number of zones
        let supply_zone_count = supply_zones.len();
        let demand_zone_count = demand_zones.len();

        // Return the detected zones and dataset information
        json!({
            "pattern": "supply_demand_zone",
            "total_bars": candles.len(),
            "total_detected": supply_zone_count + demand_zone_count,
            "datasets": {
                "price": 2, // Two datasets: supply_zones and demand_zones
                "oscillators": 0, // No oscillators in this pattern
                "lines": 0        // No trendlines in this pattern
            },
            "data": {
                "price": {
                    "supply_zones": {
                        "total": supply_zone_count,
                        "zones": supply_zones
                    },
                    "demand_zones": {
                        "total": demand_zone_count,
                        "zones": demand_zones
                    }
                },
                "oscillators": [], // Empty array for oscillators
                "lines": []        // Empty array for trendlines
            }
        })
    }
}

impl EnhancedSupplyDemandZoneRecognizer {
    /// Detect the original pattern (bearish/bullish sequence)
    fn detect_original_pattern(
        &self,
        candles: &[CandleData],
        supply_zones: &mut Vec<Value>,
        demand_zones: &mut Vec<Value>,
    ) {
        for i in 0..candles.len().saturating_sub(2) {
            let c1 = &candles[i];
            let c2 = &candles[i + 1];
            let c3 = &candles[i + 2];

            let c1_body = (c1.close - c1.open).abs();
            let c2_body = (c2.close - c2.open).abs();

            // Demand zone: first candle bearish, second candle bullish & bigger, third candle's low > first candle's high
            if c1.close < c1.open
                && c2.close > c2.open
                && c2_body >= self.min_size_ratio * c1_body
                && c3.low > c1.high
            {
                let zone_high = c1.high;
                let zone_low = c1.low;
                let mut zone_end_index = i + 2;

                // Extend the zone until a future candle's low touches or goes below zone_high
                for k in (i + 3)..candles.len() {
                    if candles[k].low <= zone_high {
                        break;
                    }
                    zone_end_index = k;
                }

                // Extend additional candles to show price entering the box, if possible
                zone_end_index =
                    std::cmp::min(zone_end_index + self.extend_zone_candles, candles.len() - 1);

                // Calculate quality score based on candle sizes and follow-through
                let quality_score =
                    self.calculate_demand_quality(c1, c2, c3, i, zone_end_index, candles);

                demand_zones.push(json!({
                    "category": "Price",
                    "type": "demand_zone",
                    "start_time": c1.time,
                    "end_time": candles[zone_end_index].time,
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                    "detection_method": "original",
                    "quality_score": quality_score,
                    "size_ratio": c2_body / c1_body,
                    "start_idx": i,
                    "end_idx": zone_end_index,
                    "strength": self.zone_strength_label(quality_score)
                }));
            }

            // Supply zone: first candle bullish, second candle bearish & bigger, third candle's high < first candle's low
            if c1.close > c1.open
                && c2.close < c2.open
                && c2_body >= self.min_size_ratio * c1_body
                && c3.high < c1.low
            {
                let zone_high = c1.high;
                let zone_low = c1.low;
                let mut zone_end_index = i + 2;

                // Extend the zone until a future candle's high touches or exceeds zone_low
                for k in (i + 3)..candles.len() {
                    if candles[k].high >= zone_low {
                        break;
                    }
                    zone_end_index = k;
                }

                // Extend additional candles to show price entering the box, if possible
                zone_end_index =
                    std::cmp::min(zone_end_index + self.extend_zone_candles, candles.len() - 1);

                // Calculate quality score based on candle sizes and follow-through
                let quality_score =
                    self.calculate_supply_quality(c1, c2, c3, i, zone_end_index, candles);

                supply_zones.push(json!({
                    "category": "Price",
                    "type": "supply_zone",
                    "start_time": c1.time,
                    "end_time": candles[zone_end_index].time,
                    "zone_high": zone_high,
                    "zone_low": zone_low,
                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                    "detection_method": "original",
                    "quality_score": quality_score,
                    "size_ratio": c2_body / c1_body,
                    "start_idx": i,
                    "end_idx": zone_end_index,
                    "strength": self.zone_strength_label(quality_score)
                }));
            }
        }
    }

    /// Detect swing high/low patterns
    fn detect_swing_pattern(
        &self,
        candles: &[CandleData],
        supply_zones: &mut Vec<Value>,
        demand_zones: &mut Vec<Value>,
    ) {
        let lookback = self.swing_lookback;

        // Need enough candles for swing detection
        if candles.len() < lookback * 2 + 1 {
            return;
        }

        // Detect swing lows (demand zones)
        for i in lookback..candles.len() - lookback {
            // Check if current candle is a potential swing low
            let is_swing_low = self.is_swing_low(candles, i);

            if is_swing_low {
                // Check for bullish follow-through
                let mut reversal_confirmed = false;
                let mut highest_after = candles[i].high;

                for j in i + 1..std::cmp::min(i + 7, candles.len()) {
                    highest_after = highest_after.max(candles[j].high);
                    // We want to see price move decisively higher
                    if j >= i + self.min_reversal_candles
                        && (highest_after - candles[i].low) / candles[i].low * 100.0
                            >= self.min_swing_strength
                    {
                        reversal_confirmed = true;
                        break;
                    }
                }

                if reversal_confirmed {
                    let zone_low = candles[i].low * 0.9998; // Just below the low

                    // For zone height, take the average of nearby candles
                    let nearby_highs: Vec<f64> = (i.saturating_sub(2)..=i)
                        .chain(i + 1..=std::cmp::min(i + 2, candles.len() - 1))
                        .map(|idx| candles[idx].high)
                        .collect();

                    let mut zone_high = if nearby_highs.is_empty() {
                        candles[i].high
                    } else {
                        nearby_highs.iter().sum::<f64>() / nearby_highs.len() as f64
                    };

                    // Check if zone height is reasonable (less than 1% of price)
                    if (zone_high - zone_low) / zone_low > 0.01 {
                        // Zone too tall, adjust height
                        zone_high = zone_low * 1.01;
                    }

                    // Find how far price moved away before returning
                    let mut zone_end_index = i;
                    for k in (i + 1)..candles.len() {
                        if candles[k].low <= zone_high {
                            break;
                        }
                        zone_end_index = k;
                    }

                    // Extend additional candles
                    zone_end_index =
                        std::cmp::min(zone_end_index + self.extend_zone_candles, candles.len() - 1);

                    // Calculate quality score
                    let swing_depth = self.calculate_swing_depth(candles, i, "low");
                    let follow_through = (highest_after - candles[i].low) / candles[i].low * 100.0;
                    let quality_score = (swing_depth * 40.0) + (follow_through * 10.0);

                    demand_zones.push(json!({
                        "category": "Price",
                        "type": "demand_zone",
                        "start_time": candles[i].time,
                        "end_time": candles[zone_end_index].time,
                        "zone_high": zone_high,
                        "zone_low": zone_low,
                        "fifty_percent_line": (zone_high + zone_low) / 2.0,
                        "detection_method": "swing_low",
                        "quality_score": quality_score,
                        "swing_depth": swing_depth,
                        "follow_through": follow_through,
                        "start_idx": i,
                        "end_idx": zone_end_index,
                        "strength": self.zone_strength_label(quality_score)
                    }));
                }
            }

            // Check if current candle is a potential swing high
            let is_swing_high = self.is_swing_high(candles, i);

            if is_swing_high {
                // Check for bearish follow-through
                let mut reversal_confirmed = false;
                let mut lowest_after = candles[i].low;

                for j in i + 1..std::cmp::min(i + 7, candles.len()) {
                    lowest_after = lowest_after.min(candles[j].low);
                    // We want to see price move decisively lower
                    if j >= i + self.min_reversal_candles
                        && (candles[i].high - lowest_after) / candles[i].high * 100.0
                            >= self.min_swing_strength
                    {
                        reversal_confirmed = true;
                        break;
                    }
                }

                if reversal_confirmed {
                    let zone_high = candles[i].high * 1.0002; // Just above the high

                    // For zone height, take the average of nearby candles
                    let nearby_lows: Vec<f64> = (i.saturating_sub(2)..=i)
                        .chain(i + 1..=std::cmp::min(i + 2, candles.len() - 1))
                        .map(|idx| candles[idx].low)
                        .collect();

                    let mut zone_low = if nearby_lows.is_empty() {
                        candles[i].low
                    } else {
                        nearby_lows.iter().sum::<f64>() / nearby_lows.len() as f64
                    };

                    // Check if zone height is reasonable (less than 1% of price)
                    if (zone_high - zone_low) / zone_low > 0.01 {
                        // Zone too tall, adjust height
                        zone_low = zone_high * 0.99;
                    }

                    // Find how far price moved away before returning
                    let mut zone_end_index = i;
                    for k in (i + 1)..candles.len() {
                        if candles[k].high >= zone_low {
                            break;
                        }
                        zone_end_index = k;
                    }

                    // Extend additional candles
                    zone_end_index =
                        std::cmp::min(zone_end_index + self.extend_zone_candles, candles.len() - 1);

                    // Calculate quality score
                    let swing_depth = self.calculate_swing_depth(candles, i, "high");
                    let follow_through = (candles[i].high - lowest_after) / candles[i].high * 100.0;
                    let quality_score = (swing_depth * 40.0) + (follow_through * 10.0);

                    supply_zones.push(json!({
                        "category": "Price",
                        "type": "supply_zone",
                        "start_time": candles[i].time,
                        "end_time": candles[zone_end_index].time,
                        "zone_high": zone_high,
                        "zone_low": zone_low,
                        "fifty_percent_line": (zone_high + zone_low) / 2.0,
                        "detection_method": "swing_high",
                        "quality_score": quality_score,
                        "swing_depth": swing_depth,
                        "follow_through": follow_through,
                        "start_idx": i,
                        "end_idx": zone_end_index,
                        "strength": self.zone_strength_label(quality_score)
                    }));
                }
            }
        }
    }

    /// Check if a candle is a swing low
    fn is_swing_low(&self, candles: &[CandleData], idx: usize) -> bool {
        let current_low = candles[idx].low;
        let lookback = self.swing_lookback;

        // Check if it's lower than surrounding candles
        let lower_than_before =
            (idx.saturating_sub(lookback)..idx).all(|i| candles[i].low > current_low);
        let lower_than_after = (idx + 1..std::cmp::min(idx + lookback + 1, candles.len()))
            .all(|i| candles[i].low > current_low);

        lower_than_before && lower_than_after
    }

    /// Check if a candle is a swing high
    fn is_swing_high(&self, candles: &[CandleData], idx: usize) -> bool {
        let current_high = candles[idx].high;
        let lookback = self.swing_lookback;

        // Check if it's higher than surrounding candles
        let higher_than_before =
            (idx.saturating_sub(lookback)..idx).all(|i| candles[i].high < current_high);
        let higher_than_after = (idx + 1..std::cmp::min(idx + lookback + 1, candles.len()))
            .all(|i| candles[i].high < current_high);

        higher_than_before && higher_than_after
    }

    /// Calculate the depth of a swing point compared to surrounding prices
    fn calculate_swing_depth(&self, candles: &[CandleData], idx: usize, swing_type: &str) -> f64 {
        let lookback = self.swing_lookback;

        if swing_type == "low" {
            let swing_low = candles[idx].low;
            let pre_swing_high = (idx.saturating_sub(lookback)..idx)
                .map(|i| candles[i].high)
                .fold(f64::MIN, f64::max);
            let post_swing_high = (idx + 1..std::cmp::min(idx + lookback + 1, candles.len()))
                .map(|i| candles[i].high)
                .fold(f64::MIN, f64::max);

            let swing_high = pre_swing_high.max(post_swing_high);
            // Return depth as a percentage
            (swing_high - swing_low) / swing_low * 100.0
        } else {
            let swing_high = candles[idx].high;
            let pre_swing_low = (idx.saturating_sub(lookback)..idx)
                .map(|i| candles[i].low)
                .fold(f64::MAX, f64::min);
            let post_swing_low = (idx + 1..std::cmp::min(idx + lookback + 1, candles.len()))
                .map(|i| candles[i].low)
                .fold(f64::MAX, f64::min);

            let swing_low = pre_swing_low.min(post_swing_low);
            // Return depth as a percentage
            (swing_high - swing_low) / swing_low * 100.0
        }
    }

    /// Calculate quality score for demand zones
    fn calculate_demand_quality(
        &self,
        c1: &CandleData,
        c2: &CandleData,
        _c3: &CandleData,
        start_idx: usize,
        end_idx: usize,
        candles: &[CandleData],
    ) -> f64 {
        let c1_body = (c1.close - c1.open).abs();
        let c2_body = (c2.close - c2.open).abs();

        // Factor 1: Size ratio between candles (higher is better)
        let size_ratio_score = (c2_body / c1_body - self.min_size_ratio) * 20.0;

        // Factor 2: How far price moved away before returning
        let distance_away = end_idx - start_idx;
        let distance_score = std::cmp::min(distance_away, 10) as f64 * 5.0;

        // Factor 3: Time in market - zones that have been in the market longer are likely stronger
        let age_score = if end_idx == candles.len() - 1 {
            // Zone still active until the end of the chart
            30.0
        } else {
            0.0
        };

        // Factor 4: Gap detection - zones that form after gaps are important
        let gap_score = if start_idx > 0 && c1.low > candles[start_idx - 1].high * 1.001 {
            20.0 // Gap up before zone
        } else {
            0.0
        };

        // Combine scores and cap at 100
        let total_score = (size_ratio_score + distance_score + age_score + gap_score)
            .min(100.0)
            .max(0.0);

        total_score
    }

    /// Calculate quality score for supply zones
    fn calculate_supply_quality(
        &self,
        c1: &CandleData,
        c2: &CandleData,
        _c3: &CandleData,
        start_idx: usize,
        end_idx: usize,
        candles: &[CandleData],
    ) -> f64 {
        let c1_body = (c1.close - c1.open).abs();
        let c2_body = (c2.close - c2.open).abs();

        // Factor 1: Size ratio between candles (higher is better)
        let size_ratio_score = (c2_body / c1_body - self.min_size_ratio) * 20.0;

        // Factor 2: How far price moved away before returning
        let distance_away = end_idx - start_idx;
        let distance_score = std::cmp::min(distance_away, 10) as f64 * 5.0;

        // Factor 3: Time in market - zones that have been in the market longer are likely stronger
        let age_score = if end_idx == candles.len() - 1 {
            // Zone still active until the end of the chart
            30.0
        } else {
            0.0
        };

        // Factor 4: Gap detection - zones that form after gaps are important
        let gap_score = if start_idx > 0 && c1.high < candles[start_idx - 1].low * 0.999 {
            20.0 // Gap down before zone
        } else {
            0.0
        };

        // Combine scores and cap at 100
        let total_score = (size_ratio_score + distance_score + age_score + gap_score)
            .min(100.0)
            .max(0.0);

        total_score
    }

    fn detect_rbr_pattern(&self, candles: &[CandleData], demand_zones: &mut Vec<Value>) {
        // Need enough candles for the pattern
        let min_pattern_size = self.min_rbr_base_candles + 4; // First leg + base + second leg
        if candles.len() < min_pattern_size {
            return;
        }

        // Look for RBR patterns throughout the chart
        for i in 0..candles.len().saturating_sub(min_pattern_size) {
            // Look for initial rally (bullish move)
            let mut rally_end_idx = 0;
            let mut rally_detected = false;
            let mut rally_high: f64 = 0.0;
            let mut rally_low: f64 = f64::MAX;

            // Scan for a series of predominantly bullish candles forming a rally
            for j in i..std::cmp::min(i + 10, candles.len() - min_pattern_size + 1) {
                // Track high and low of this potential rally
                rally_high = rally_high.max(candles[j].high);
                rally_low = rally_low.min(candles[j].low);

                // Measure the rally from low to high
                let start_price = candles[i].low;
                let end_price = candles[j].high;
                let percent_move = (end_price - start_price) / start_price * 100.0;

                // Check if we have a significant rally
                if percent_move >= self.min_rbr_rally_percent {
                    // Additional check: At least 50% of candles should be bullish (less strict than before)
                    let bullish_candles = (i..=j)
                        .filter(|&idx| candles[idx].close > candles[idx].open)
                        .count();
                    let total_candles = j - i + 1;

                    if bullish_candles as f64 / total_candles as f64 >= 0.5 {
                        rally_detected = true;
                        rally_end_idx = j;
                        break;
                    }
                }
            }

            // Skip if no initial rally found
            if !rally_detected {
                continue;
            }

            // Look for a consolidation/base after the rally
            let base_start_idx = rally_end_idx + 1;
            if base_start_idx >= candles.len() - 3 {
                continue; // Not enough candles left
            }

            // Find the base/consolidation area
            let mut base_end_idx = base_start_idx;
            let mut base_high = candles[base_start_idx].high;
            let mut base_low = candles[base_start_idx].low;
            let mut base_detected = false;

            // Look for consecutive candles with tight range
            for j in base_start_idx + 1
                ..std::cmp::min(
                    base_start_idx + self.max_rbr_base_candles,
                    candles.len() - 2,
                )
            {
                // Calculate potential base boundaries if we include this candle
                let potential_high = base_high.max(candles[j].high);
                let potential_low = base_low.min(candles[j].low);

                // Check if range is still acceptable
                let range_percent = (potential_high - potential_low) / potential_low * 100.0;

                if range_percent > self.max_rbr_base_range {
                    // Don't immediately break, check if we already have enough candles for a base
                    if j - base_start_idx >= self.min_rbr_base_candles {
                        base_detected = true;
                        break;
                    } else {
                        break; // Base is getting too wide and we don't have enough candles yet
                    }
                }

                // Update base boundaries
                base_high = potential_high;
                base_low = potential_low;
                base_end_idx = j;

                // Check if we have enough candles for a valid base
                if j - base_start_idx + 1 >= self.min_rbr_base_candles {
                    // Additional check: Base should be relatively horizontal
                    // Using a more lenient approach for trending bases
                    let first_close = candles[base_start_idx].close;
                    let last_close = candles[j].close;
                    let base_trend_percent = (last_close - first_close).abs() / first_close * 100.0;

                    // More lenient condition for base trend
                    if base_trend_percent <= self.max_rbr_base_range * 0.8 {
                        base_detected = true;
                    }
                }
            }

            // Skip if no valid base found
            if !base_detected {
                continue;
            }

            // More flexible condition: Base should generally stay below the rally high,
            // but we allow some minor violations (up to 10% above)
            if base_high > rally_high * 1.1 {
                continue;
            }

            // Look for second rally after the base
            let second_rally_start_idx = base_end_idx + 1;
            if second_rally_start_idx >= candles.len() - 1 {
                continue; // Not enough candles left
            }

            // Check for second bullish move
            let mut second_rally_detected = false;
            let mut second_rally_end_idx = second_rally_start_idx;

            for j in second_rally_start_idx + 1
                ..std::cmp::min(second_rally_start_idx + 10, candles.len())
            {
                let start_price = candles[second_rally_start_idx].low;
                let end_price = candles[j].high;
                let percent_move = (end_price - start_price) / start_price * 100.0;

                if percent_move >= self.min_rbr_rally_percent {
                    // Check for bullish candles but with more lenient criteria
                    let bullish_candles = (second_rally_start_idx..=j)
                        .filter(|&idx| candles[idx].close > candles[idx].open)
                        .count();
                    let total_candles = j - second_rally_start_idx + 1;

                    // More lenient: At least 50% should be bullish
                    if bullish_candles as f64 / total_candles as f64 >= 0.5 {
                        second_rally_detected = true;
                        second_rally_end_idx = j;
                        break;
                    }
                }
            }

            // Skip if no second rally found
            if !second_rally_detected {
                continue;
            }

            // More flexible breakout criteria:
            // The second rally should either:
            // 1. Break above the first rally's high, OR
            // 2. Show significant momentum (at least 75% of the first rally's size)
            let first_rally_size = (rally_high - candles[i].low) / candles[i].low * 100.0;
            let second_rally_size = (candles[second_rally_end_idx].high
                - candles[second_rally_start_idx].low)
                / candles[second_rally_start_idx].low
                * 100.0;

            let significant_momentum = second_rally_size >= first_rally_size * 0.75;
            let breaks_above = candles[second_rally_end_idx].high > rally_high;

            if !breaks_above && !significant_momentum {
                continue;
            }

            // Valid RBR pattern found - create a demand zone at the base
            let zone_high = base_high;
            let zone_low = base_low;

            // Extend zone to the right until price returns
            let mut zone_end_idx = second_rally_end_idx;
            for k in (second_rally_end_idx + 1)..candles.len() {
                if candles[k].low <= zone_high {
                    break;
                }
                zone_end_idx = k;
            }

            // Extend zone by a few more candles to show price entering
            zone_end_idx =
                std::cmp::min(zone_end_idx + self.extend_zone_candles, candles.len() - 1);

            // Calculate quality score
            let base_tightness =
                self.max_rbr_base_range - (zone_high - zone_low) / zone_low * 100.0;

            // Factor in the breakout strength (how much the second rally exceeds the first rally high)
            let breakout_strength = if breaks_above {
                (candles[second_rally_end_idx].high - rally_high) / rally_high * 100.0
            } else {
                0.0 // No breakout, but still a valid momentum-based RBR
            };

            // Calculate special characteristics that could make this a "perfect" RBR
            // 1. Clear rally -> base -> rally structure with minimal noise
            // 2. Tight consolidation base
            // 3. Strong second rally

            let base_candle_count = base_end_idx - base_start_idx + 1;
            let ideal_base_candles =
                self.min_rbr_base_candles <= base_candle_count && base_candle_count <= 5; // Ideal is 3-5 candles

            let clear_structure = ideal_base_candles
                && (base_high - base_low) / base_low * 100.0 <= self.max_rbr_base_range * 0.7;

            // Calculate quality score with more weight on clear structure
            let structure_bonus = if clear_structure { 30.0 } else { 0.0 };

            let quality_score = (first_rally_size * 10.0)
                + (second_rally_size * 20.0)
                + (base_tightness * 20.0)
                + (breakout_strength * 20.0)
                + structure_bonus;

            // Cap the quality score at 100
            let final_quality_score = quality_score.min(100.0);

            // Add the demand zone with improved metadata
            demand_zones.push(json!({
                "category": "Price",
                "type": "demand_zone",
                "start_time": candles[base_start_idx].time,
                "end_time": candles[zone_end_idx].time,
                "zone_high": zone_high,
                "zone_low": zone_low,
                "fifty_percent_line": (zone_high + zone_low) / 2.0,
                "detection_method": "rbr_pattern",
                "quality_score": final_quality_score,
                "first_rally_size": first_rally_size,
                "second_rally_size": second_rally_size,
                "breakout_strength": breakout_strength,
                "base_range_percent": (zone_high - zone_low) / zone_low * 100.0,
                "base_candles": base_candle_count,
                "clear_structure": clear_structure,
                "start_idx": base_start_idx,
                "end_idx": zone_end_idx,
                "strength": self.zone_strength_label(final_quality_score)
            }));
        }
    }

    fn detect_momentum_shift_zones(&self, candles: &[CandleData], demand_zones: &mut Vec<Value>) {
        // Need at least a few candles for momentum analysis
        if candles.len() < 4 {
            return;
        }

        for i in 1..candles.len().saturating_sub(2) {
            // Look for bearish-to-bullish momentum shifts

            // Pattern 1: Strong bearish candle(s) followed by strong bullish reversal
            if i >= 1 && i + 1 < candles.len() {
                let prev_candle = &candles[i - 1];
                let current_candle = &candles[i];
                let next_candle = &candles[i + 1];

                // Check for a bearish candle followed by a bullish candle
                let prev_bearish = prev_candle.close < prev_candle.open;
                let current_bullish = current_candle.close > current_candle.open;

                if prev_bearish && current_bullish {
                    // Calculate candle sizes
                    let prev_body_size = (prev_candle.open - prev_candle.close).abs();
                    let current_body_size = (current_candle.close - current_candle.open).abs();

                    // Check if the bullish candle is significant (at least 50% of the bearish candle)
                    let significant_reversal = current_body_size >= prev_body_size * 0.5;

                    // Check if next candle continues the bullish move or at least doesn't fully reverse it
                    let momentum_continues = next_candle.low > current_candle.low * 0.998;

                    if significant_reversal && momentum_continues {
                        // Found a potential momentum shift demand zone
                        let zone_low = current_candle.low.min(prev_candle.low);
                        let zone_high = current_candle.high.max(prev_candle.high);

                        // Apply a quality score based on the strength of the reversal
                        let reversal_strength = current_body_size / prev_body_size * 100.0;
                        let follow_through = if next_candle.close > next_candle.open {
                            (next_candle.close - next_candle.open) / next_candle.open * 100.0
                        } else {
                            0.0
                        };

                        // Look forward to see if price respects this zone
                        let mut zone_respected = false;
                        let mut zone_end_idx = i + 2;
                        let look_ahead = std::cmp::min(i + 10, candles.len() - 1);

                        for j in i + 2..=look_ahead {
                            if candles[j].low > zone_low * 0.998 {
                                zone_respected = true;
                                zone_end_idx = j;
                            } else {
                                // Zone violated, stop looking
                                break;
                            }
                        }

                        let respect_bonus = if zone_respected { 30.0 } else { 0.0 };

                        // Calculate quality score
                        let quality_score =
                            (reversal_strength * 0.5) + (follow_through * 10.0) + respect_bonus;

                        // Add the demand zone if it meets minimum quality
                        if quality_score >= 30.0 {
                            // Extend zone to the right to show area of interest
                            let display_end_idx = std::cmp::min(
                                zone_end_idx + self.extend_zone_candles,
                                candles.len() - 1,
                            );

                            demand_zones.push(json!({
                                "category": "Price",
                                "type": "demand_zone",
                                "start_time": candles[i-1].time,
                                "end_time": candles[display_end_idx].time,
                                "zone_high": zone_high,
                                "zone_low": zone_low,
                                "fifty_percent_line": (zone_high + zone_low) / 2.0,
                                "detection_method": "momentum_shift",
                                "quality_score": quality_score.min(100.0),
                                "reversal_strength": reversal_strength,
                                "follow_through": follow_through,
                                "zone_respected": zone_respected,
                                "start_idx": i-1,
                                "end_idx": display_end_idx,
                                "strength": self.zone_strength_label(quality_score.min(100.0))
                            }));
                        }
                    }
                }
            }

            // Pattern 2: Downtrend reversal with small base
            if i >= 3 && i + 2 < candles.len() {
                // Look for a sequence of bearish candles followed by a small consolidation and bullish breakout
                let is_prior_downtrend = candles[i - 3].close < candles[i - 3].open
                    && candles[i - 2].close < candles[i - 2].open
                    && candles[i - 1].close <= candles[i - 1].open;

                // Check for a base (small range candle compared to previous)
                let prev_range = (candles[i - 2].high - candles[i - 2].low)
                    + (candles[i - 3].high - candles[i - 3].low);
                let base_range =
                    (candles[i].high - candles[i].low) + (candles[i - 1].high - candles[i - 1].low);

                let is_compressed_base = base_range < prev_range * 0.7;

                // Check for a bullish breakout
                let is_bullish_breakout = candles[i + 1].close > candles[i + 1].open
                    && candles[i + 1].high > candles[i].high;

                if is_prior_downtrend && is_compressed_base && is_bullish_breakout {
                    // Create zone at the base level
                    let zone_low = candles[i - 1].low.min(candles[i].low);
                    let zone_high = candles[i - 1].high.max(candles[i].high);

                    // Look ahead to see if price respects this zone
                    let mut zone_respected = false;
                    let mut zone_end_idx = i + 2;
                    let look_ahead = std::cmp::min(i + 10, candles.len() - 1);

                    for j in i + 2..=look_ahead {
                        if candles[j].low > zone_low * 0.998 {
                            zone_respected = true;
                            zone_end_idx = j;
                        } else {
                            break;
                        }
                    }

                    // Calculate quality metrics
                    let breakout_strength =
                        (candles[i + 1].close - candles[i + 1].open) / candles[i + 1].open * 100.0;
                    let compression_ratio = base_range / prev_range * 100.0;
                    let respect_bonus = if zone_respected { 30.0 } else { 0.0 };

                    // Calculate quality score
                    let quality_score = (breakout_strength * 10.0)
                        + ((100.0 - compression_ratio) * 0.5)
                        + respect_bonus;

                    // Add the demand zone if it meets minimum quality
                    if quality_score >= 30.0 {
                        // Extend zone to the right to show area of interest
                        let display_end_idx = std::cmp::min(
                            zone_end_idx + self.extend_zone_candles,
                            candles.len() - 1,
                        );

                        demand_zones.push(json!({
                            "category": "Price",
                            "type": "demand_zone",
                            "start_time": candles[i-1].time,
                            "end_time": candles[display_end_idx].time,
                            "zone_high": zone_high,
                            "zone_low": zone_low,
                            "fifty_percent_line": (zone_high + zone_low) / 2.0,
                            "detection_method": "downtrend_reversal",
                            "quality_score": quality_score.min(100.0),
                            "breakout_strength": breakout_strength,
                            "compression_ratio": compression_ratio,
                            "zone_respected": zone_respected,
                            "start_idx": i-1,
                            "end_idx": display_end_idx,
                            "strength": self.zone_strength_label(quality_score.min(100.0))
                        }));
                    }
                }
            }
            // Pattern 3: Strong Bearish-to-Bullish Reversal (Single Candle Pattern)
            for i in 0..candles.len().saturating_sub(2) {
                if i + 1 < candles.len() {
                    let current_candle = &candles[i];
                    let next_candle = &candles[i + 1];

                    // Check for a large bearish candle followed by a large bullish candle
                    let current_bearish = current_candle.close < current_candle.open;
                    let next_bullish = next_candle.close > next_candle.open;

                    if current_bearish && next_bullish {
                        // Calculate candle sizes (magnitudes)
                        let current_body_size = (current_candle.open - current_candle.close).abs();
                        let next_body_size = (next_candle.close - next_candle.open).abs();

                        // Both should be relatively large candles
                        let curr_avg_size = if i >= 5 {
                            let mut sum = 0.0;
                            for j in i - 5..i {
                                sum += (candles[j].open - candles[j].close).abs();
                            }
                            sum / 5.0
                        } else {
                            current_body_size * 0.5 // fallback if not enough prior candles
                        };

                        let is_large_bearish = current_body_size > curr_avg_size * 1.5;
                        let is_substantial_bullish = next_body_size > curr_avg_size;

                        // Check for significant reversal - either:
                        // 1. Both candles are large, or
                        // 2. The bullish candle retraces at least 40% of the bearish candle
                        let significant_reversal = (is_large_bearish && is_substantial_bullish)
                            || (next_body_size >= current_body_size * 0.4);

                        // Check if the candles are properly aligned - critical check to avoid false detections
                        // The close of the bearish candle should be close to the open of the bullish candle
                        let price_difference = (current_candle.close - next_candle.open).abs();
                        let price_difference_percent =
                            price_difference / current_candle.close * 100.0;
                        let candles_aligned = price_difference_percent <= 0.2; // Max 0.2% gap/overlap

                        // Additional check: the bullish candle's low shouldn't be significantly below the bearish candle's low
                        let lows_aligned = next_candle.low >= current_candle.low * 0.998;

                        if significant_reversal && lows_aligned && candles_aligned {
                            // Define the zone with proper height
                            // Zone low is at the lowest low of both candles
                            let zone_low = current_candle.low.min(next_candle.low);

                            // Calculate zone height as 40-50% of the average body size
                            // (making it relative to candle size but not too tall)
                            let avg_body_size = (current_body_size + next_body_size) / 2.0;
                            let zone_height = avg_body_size * 0.50; // 50% of average body size

                            // Set zone high based on the calculated height, starting from zone_low
                            let zone_high = zone_low + zone_height;

                            // Look ahead to see if price respects this zone
                            let mut zone_respected = false;
                            let mut zone_end_idx = i + 2;
                            let look_ahead = std::cmp::min(i + 10, candles.len() - 1);

                            for j in i + 2..=look_ahead {
                                if candles[j].low > zone_low * 0.998 {
                                    zone_respected = true;
                                    zone_end_idx = j;
                                } else {
                                    break;
                                }
                            }

                            // Calculate quality metrics
                            let reversal_ratio = next_body_size / current_body_size * 100.0;
                            let alignment_score = (0.2 - price_difference_percent) * 100.0; // Higher score for better alignment
                            let size_impact = if is_large_bearish { 30.0 } else { 0.0 };
                            let respect_bonus = if zone_respected { 40.0 } else { 0.0 };

                            // Calculate quality score with high emphasis on alignment
                            let quality_score = (reversal_ratio * 0.3)
                                + (alignment_score * 0.2)
                                + size_impact
                                + respect_bonus;

                            // Add the demand zone if it meets minimum quality
                            if quality_score >= 25.0 {
                                // Extend zone to the right to show area of interest
                                let display_end_idx = std::cmp::min(
                                    zone_end_idx + self.extend_zone_candles,
                                    candles.len() - 1,
                                );

                                demand_zones.push(json!({
                                    "category": "Price",
                                    "type": "demand_zone",
                                    "start_time": candles[i].time,
                                    "end_time": candles[display_end_idx].time,
                                    "zone_high": zone_high,
                                    "zone_low": zone_low,
                                    "fifty_percent_line": (zone_high + zone_low) / 2.0,
                                    "detection_method": "strong_reversal",
                                    "quality_score": quality_score.min(100.0),
                                    "reversal_ratio": reversal_ratio,
                                    "candle_alignment": price_difference_percent,
                                    "large_bearish": is_large_bearish,
                                    "zone_respected": zone_respected,
                                    "start_idx": i,
                                    "end_idx": display_end_idx,
                                    "strength": self.zone_strength_label(quality_score.min(100.0))
                                }));
                            }
                        }
                    }
                }
            }
        }
    }
    /// Apply additional quality filters to remove low-quality zones
    fn apply_quality_filters(&self, zones: Vec<Value>, _zone_type: &str) -> Vec<Value> {
        zones
            .into_iter()
            .filter(|zone| {
                let quality_score = zone["quality_score"].as_f64().unwrap_or(0.0);
                let detection_method = zone["detection_method"].as_str().unwrap_or("");

                // Apply stricter filtering criteria for RBR patterns
                if detection_method == "rbr_pattern" {
                    // Get RBR-specific metrics
                    let second_rally_size = zone["second_rally_size"].as_f64().unwrap_or(0.0);
                    let base_range_percent = zone["base_range_percent"].as_f64().unwrap_or(1.0);

                    // For a valid RBR:
                    // 1. Second rally should be substantial
                    // 2. Base must be reasonably tight
                    let valid_rbr = second_rally_size >= self.min_rbr_rally_percent
                        && base_range_percent <= self.max_rbr_base_range * 1.2; // More lenient

                    // More lenient quality threshold for RBR patterns
                    quality_score >= 35.0 && valid_rbr
                } else if detection_method.contains("swing") {
                    // Get swing-specific metrics
                    let swing_depth = zone["swing_depth"].as_f64().unwrap_or(0.0);
                    let follow_through = zone["follow_through"].as_f64().unwrap_or(0.0);

                    // For a valid swing pattern:
                    // 1. Swing should have decent depth
                    // 2. Follow-through must be substantial
                    swing_depth >= self.min_swing_strength * 1.2
                        && follow_through >= self.min_swing_strength * 1.5
                        && quality_score >= 35.0
                } else {
                    // Default criteria for other patterns
                    quality_score >= 25.0
                }
            })
            .collect()
    }

    /// Filter out overlapping zones, keeping only the highest quality ones
    fn filter_overlapping_zones(&self, zones: Vec<Value>, zone_type: &str) -> Vec<Value> {
        // First apply quality filters to remove low-quality zones
        let filtered_by_quality = self.apply_quality_filters(zones, zone_type);

        let mut filtered_zones: Vec<Value> = Vec::new();

        // Sort zones by quality score (highest first)
        let mut sorted_zones = filtered_by_quality;
        sorted_zones.sort_by(|a, b| {
            let a_score = a["quality_score"].as_f64().unwrap_or(0.0);
            let b_score = b["quality_score"].as_f64().unwrap_or(0.0);
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for zone in sorted_zones {
            let zone_high = zone["zone_high"].as_f64().unwrap_or(0.0);
            let zone_low = zone["zone_low"].as_f64().unwrap_or(0.0);
            let zone_midpoint = (zone_high + zone_low) / 2.0;
            let start_idx = zone["start_idx"].as_u64().unwrap_or(0) as usize;
            let end_idx = zone["end_idx"].as_u64().unwrap_or(0) as usize;

            let mut overlaps = false;
            for existing in &filtered_zones {
                let existing_high = existing["zone_high"].as_f64().unwrap_or(0.0);
                let existing_low = existing["zone_low"].as_f64().unwrap_or(0.0);
                let existing_midpoint = (existing_high + existing_low) / 2.0;

                // Check for significant price overlap
                let price_overlap = (zone_midpoint >= existing_low
                    && zone_midpoint <= existing_high)
                    || (existing_midpoint >= zone_low && existing_midpoint <= zone_high);

                // Check for time overlap (if zones are close enough in time)
                let existing_start = existing["start_idx"].as_u64().unwrap_or(0) as usize;
                let existing_end = existing["end_idx"].as_u64().unwrap_or(0) as usize;

                let time_overlap = (start_idx >= existing_start && start_idx <= existing_end)
                    || (end_idx >= existing_start && end_idx <= existing_end)
                    || (existing_start >= start_idx && existing_start <= end_idx);

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
    fn zone_strength_label(&self, quality_score: f64) -> &str {
        if quality_score >= 75.0 {
            "Strong"
        } else if quality_score >= 50.0 {
            "Moderate"
        } else {
            "Weak"
        }
    }
}
