// src/bin/zone_monitor/proximity.rs
// Zone proximity detection logic

use crate::types::{PriceUpdate, Zone, ZoneAlert};
use tracing::{info, debug};

#[derive(Debug)]
pub struct ProximityDetector {
    pub alert_threshold_pips: f64,
}

impl ProximityDetector {
    pub fn new(alert_threshold_pips: f64) -> Self {
        Self {
            alert_threshold_pips,
        }
    }

    pub async fn check_zone_proximity(
        &self,
        price_update: &PriceUpdate,
        zones: &[Zone],
    ) -> Vec<ZoneAlert> {
        let mut alerts = Vec::new();
        let current_price = (price_update.bid + price_update.ask) / 2.0;

        for zone in zones {
            let zone_mid = (zone.high + zone.low) / 2.0;
            let distance_pips = ((current_price - zone_mid).abs() * 10000.0).round();

            // Check if within alert threshold
            if distance_pips <= self.alert_threshold_pips {
                let alert = ZoneAlert {
                    zone_id: zone.id.clone(),
                    symbol: price_update.symbol.clone(),
                    zone_type: zone.zone_type.clone(),
                    current_price,
                    zone_high: zone.high,
                    zone_low: zone.low,
                    distance_pips,
                    strength: zone.strength,
                    timestamp: chrono::Utc::now(),
                };

                // Log the proximity alert
                debug!(
                    "🎯 ZONE PROXIMITY: {} price {:.5} is {:.1} pips from {} zone at {:.5}-{:.5} (strength: {:.2})",
                    price_update.symbol,
                    current_price,
                    distance_pips,
                    zone.zone_type,
                    zone.low,
                    zone.high,
                    zone.strength
                );

                alerts.push(alert);
            }
        }

        alerts
    }
}