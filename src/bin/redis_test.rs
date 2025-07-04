// src/bin/redis_test.rs
// Simple test to store just one pending order in Redis

use chrono::{DateTime, Utc};
use redis::{Client, Commands};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOrder {
    zone_id: String,
    symbol: String,
    timeframe: String,
    zone_type: String,
    order_type: String,
    entry_price: f64,
    lot_size: i32,
    stop_loss: f64,
    take_profit: f64,
    ctrader_order_id: String,
    placed_at: DateTime<Utc>,
    status: String,
    zone_high: f64,
    zone_low: f64,
    zone_strength: f64,
    touch_count: i32,
    distance_when_placed: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 Storing simple pending order in Redis...");

    // Connect to Redis
    let client = Client::open("redis://127.0.0.1:6379/")?;
    let mut con = client.get_connection()?;
    
    println!("✅ Connected to Redis");

    // Create your exact pending order
    let pending_order = PendingOrder {
        zone_id: "6a6622d1339cc0d4".to_string(),
        symbol: "GBPUSD".to_string(),
        timeframe: "15m".to_string(),
        zone_type: "supply_zone".to_string(),
        order_type: "SELL_LIMIT".to_string(),
        entry_price: 1.36977599,
        lot_size: 1000,
        stop_loss: 1.3707759899999998,
        take_profit: 1.36877599,
        ctrader_order_id: "47198245".to_string(),
        placed_at: "2025-07-04T05:06:15.162952Z".parse()?,
        status: "PENDING".to_string(),
        zone_high: 1.37095,
        zone_low: 1.36977599,
        zone_strength: 100.0,
        touch_count: 0,
        distance_when_placed: 19.559899999999075,
    };

    // Store the pending order with simple key
    let key = format!("pending_order:{}", pending_order.zone_id);
    let order_json = serde_json::to_string_pretty(&pending_order)?;
    let _: () = con.set(&key, &order_json)?;
    
    println!("📋 Stored pending order at key: {}", key);

    // Verify it was stored
    let retrieved: String = con.get(&key)?;
    let verified_order: PendingOrder = serde_json::from_str(&retrieved)?;
    
    println!("✅ Verified order:");
    println!("   Zone ID: {}", verified_order.zone_id);
    println!("   Symbol: {}", verified_order.symbol);
    println!("   Status: {}", verified_order.status);
    println!("   cTrader ID: {}", verified_order.ctrader_order_id);

    println!("🎉 Done! Check Redis Commander at http://localhost:8082");
    println!("   Look for key: {}", key);

    Ok(())
}