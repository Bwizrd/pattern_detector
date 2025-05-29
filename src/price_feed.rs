// =============================================================================
// src/price_feed.rs - Price feed integration
// =============================================================================

use tokio::sync::mpsc;
use std::collections::HashMap;
use log::{warn, debug};

#[derive(Debug, Clone)]
pub struct PriceUpdate {
    pub symbol_id: u32,
    pub symbol: String,
    pub timeframe: String,
    pub price: f64,
    pub timestamp: u64,
    pub is_new_bar: bool,
}

pub struct PriceFeedBridge {
    price_sender: mpsc::Sender<PriceUpdate>,
    symbol_map: HashMap<u32, String>,
}

impl PriceFeedBridge {
    pub fn new(price_sender: mpsc::Sender<PriceUpdate>) -> Self {
        // Map your cTrader symbol IDs to names
        let mut symbol_map = HashMap::new();
        symbol_map.insert(185, "EURUSD_SB".to_string());
        symbol_map.insert(199, "GBPUSD_SB".to_string());
        symbol_map.insert(226, "USDJPY_SB".to_string());
        symbol_map.insert(222, "USDCHF_SB".to_string());
        symbol_map.insert(158, "AUDUSD_SB".to_string());
        symbol_map.insert(221, "USDCAD_SB".to_string());
        symbol_map.insert(211, "NZDUSD_SB".to_string());
        symbol_map.insert(175, "EURGBP_SB".to_string());
        symbol_map.insert(177, "EURJPY_SB".to_string());
        symbol_map.insert(173, "EURCHF_SB".to_string());
        symbol_map.insert(171, "EURAUD_SB".to_string());
        symbol_map.insert(172, "EURCAD_SB".to_string());
        symbol_map.insert(180, "EURNZD_SB".to_string());
        symbol_map.insert(192, "GBPJPY_SB".to_string());
        symbol_map.insert(191, "GBPCHF_SB".to_string());
        symbol_map.insert(189, "GBPAUD_SB".to_string());
        symbol_map.insert(190, "GBPCAD_SB".to_string());
        symbol_map.insert(195, "GBPNZD_SB".to_string());
        symbol_map.insert(155, "AUDJPY_SB".to_string());
        symbol_map.insert(156, "AUDNZD_SB".to_string());
        symbol_map.insert(153, "AUDCAD_SB".to_string());
        symbol_map.insert(210, "NZDJPY_SB".to_string());
        symbol_map.insert(162, "CADJPY_SB".to_string());
        symbol_map.insert(163, "CHFJPY_SB".to_string());
        symbol_map.insert(205, "NAS100_SB".to_string());
        symbol_map.insert(220, "US500_SB".to_string());
        
        Self {
            price_sender,
            symbol_map,
        }
    }
    
    pub async fn handle_price_update(
        &self,
        symbol_id: u32,
        timeframe: &str,
        price: f64,
        timestamp: u64,
        is_new_bar: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(symbol) = self.symbol_map.get(&symbol_id) {
            let update = PriceUpdate {
                symbol_id,
                symbol: symbol.clone(),
                timeframe: timeframe.to_string(),
                price,
                timestamp,
                is_new_bar,
            };
            
            if let Err(e) = self.price_sender.send(update).await {
                warn!("⚠️  [PRICE_BRIDGE] Failed to send price update: {}", e);
            } else if is_new_bar {
                debug!("📊 [PRICE_BRIDGE] New bar processed: {} {} @ {:.5}", symbol, timeframe, price);
            }
        } else {
            debug!("❓ [PRICE_BRIDGE] Unknown symbol ID: {}", symbol_id);
        }
        
        Ok(())
    }
}