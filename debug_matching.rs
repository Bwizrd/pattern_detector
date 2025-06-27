use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BookedOrder {
    pub zone_id: String,
    pub symbol: String,
    pub timeframe: String,
    pub order_type: String,
    pub entry_price: f64,
    pub lot_size: u32,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub ctrader_order_id: String,
    pub booked_at: DateTime<Utc>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filled_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_entry_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_exit_price: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BookedOrdersData {
    pub last_updated: DateTime<Utc>,
    pub booked_orders: HashMap<String, BookedOrder>,
}

#[derive(Debug, Deserialize)]
pub struct Position {
    #[serde(rename = "positionId")]
    pub position_id: String,
    #[serde(rename = "symbolId")]
    pub symbol_id: String,
    #[serde(rename = "tradeSide")]
    pub trade_side: String,
    pub volume: f64,
    #[serde(rename = "entryPrice")]
    pub entry_price: f64,
    #[serde(rename = "currentPrice")]
    pub current_price: f64,
    pub pips: f64,
    #[serde(rename = "grossUnrealizedPL")]
    pub gross_unrealized_pl: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load current booked orders
    let booked_orders_content = std::fs::read_to_string("shared_booked_orders.json")?;
    let mut booked_data: BookedOrdersData = serde_json::from_str(&booked_orders_content)?;
    
    println!("Current booked orders:");
    for (zone_id, order) in &booked_data.booked_orders {
        println!("  Zone: {}, Symbol: {}, Status: {}, Entry: {}, Order ID: {}", 
                 zone_id, order.symbol, order.status, order.entry_price, order.ctrader_order_id);
    }
    
    // Mock some positions based on what we know from your previous tests
    let mock_positions = vec![
        Position {
            position_id: "12345".to_string(),
            symbol_id: "1".to_string(), // EURUSD
            trade_side: "BUY".to_string(),
            volume: 1000.0,
            entry_price: 1.0123,
            current_price: 1.0125,
            pips: 2.0,
            gross_unrealized_pl: 20.0,
        },
        Position {
            position_id: "12346".to_string(),
            symbol_id: "2".to_string(), // USDCAD  
            trade_side: "BUY".to_string(),
            volume: 1000.0,
            entry_price: 1.3600, // Close to our pending order price
            current_price: 1.3605,
            pips: 5.0,
            gross_unrealized_pl: 50.0,
        }
    ];
    
    // Symbol mapping (add more as needed)
    let mut symbol_map = HashMap::new();
    symbol_map.insert("1", "EURUSD");
    symbol_map.insert("2", "USDCAD");
    
    println!("\nMock positions:");
    for pos in &mock_positions {
        let symbol_name = symbol_map.get(pos.symbol_id.as_str()).unwrap_or(&pos.symbol_id);
        println!("  Position: {}, Symbol: {} ({}), Side: {}, Entry: {}, Volume: {}", 
                 pos.position_id, symbol_name, pos.symbol_id, pos.trade_side, pos.entry_price, pos.volume);
    }
    
    // Test the matching logic
    println!("\nTesting matching logic:");
    let mut matches_found = 0;
    
    for pos in &mock_positions {
        let position_symbol = symbol_map.get(pos.symbol_id.as_str()).unwrap_or(&pos.symbol_id);
        let position_side = if pos.trade_side == "BUY" { "BUY_LIMIT" } else { "SELL_LIMIT" };
        
        println!("\nChecking position {} ({}, {}, entry: {})", 
                 pos.position_id, position_symbol, position_side, pos.entry_price);
        
        for (zone_id, order) in &booked_data.booked_orders {
            if order.status == "PENDING" {
                let symbol_match = order.symbol == position_symbol;
                let side_match = order.order_type == position_side;
                let price_diff = (order.entry_price - pos.entry_price).abs();
                let price_match = price_diff < 0.001; // 0.1 pip tolerance
                let volume_match = order.lot_size as f64 == pos.volume;
                
                println!("  Checking order {}: symbol={} side={} price_diff={:.6} volume={}",
                         zone_id, symbol_match, side_match, price_diff, volume_match);
                
                if symbol_match && side_match && price_match && volume_match {
                    println!("    ✓ MATCH FOUND! This order should be updated to FILLED");
                    matches_found += 1;
                }
            }
        }
    }
    
    println!("\nTotal matches found: {}", matches_found);
    
    if matches_found > 0 {
        println!("\nThe matching logic should work. If orders aren't being updated,");
        println!("the issue might be in the API call, authentication, or file writing.");
    } else {
        println!("\nNo matches found. This suggests either:");
        println!("1. The positions data doesn't match the pending orders");
        println!("2. The matching criteria are too strict");
        println!("3. The symbol mapping is incorrect");
    }
    
    Ok(())
}
