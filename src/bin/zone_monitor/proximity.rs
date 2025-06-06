// src/bin/zone_monitor/proximity.rs
// Simplified proximity detection - state management moved to ZoneStateManager

use crate::types::{PriceUpdate, Zone, ZoneAlert};
use tracing::info;

#[derive(Debug)]
pub struct ProximityDetector {
    pub alert_threshold_pips: f64,
}

impl ProximityDetector {
    pub fn new(alert_threshold_pips: f64) -> Self {
        info!("🎯 Proximity detector initialized with {:.1} pip threshold", alert_threshold_pips);
        Self {
            alert_threshold_pips,
        }
    }

    // This is now mainly for backwards compatibility
    // The real logic is in ZoneStateManager
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

                alerts.push(alert);
            }
        }

        alerts
    }
}