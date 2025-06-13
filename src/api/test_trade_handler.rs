// src/api/test_trade_handler.rs
// Test trade endpoint to verify SL/TP calculation and cTrader API integration

use actix_web::{web, HttpResponse, Error as ActixError};
use serde::{Deserialize, Serialize};
use chrono::Utc;
use log::{info, error};
use reqwest;
use serde_json::json;
use std::collections::HashMap;
use std::env;

#[derive(Debug, Deserialize)]
pub struct TestTradeRequest {
    pub symbol: String,
    pub zone_type: String, // "supply_zone" or "demand_zone"
    pub current_price: f64,
    pub zone_id: Option<String>,
    pub zone_low: Option<f64>,
    pub zone_high: Option<f64>,
    pub strength: Option<f64>,
    pub timeframe: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestTradeResponse {
    pub success: bool,
    pub message: String,
    pub trade_details: Option<TestTradeDetails>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestTradeDetails {
    pub symbol: String,
    pub trade_type: String,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub lot_size: f64,
    pub ctrader_response: Option<String>,
    pub config_sl_pips: f64,
    pub config_tp_pips: f64,
    pub pip_value: f64,
    pub calculated_sl_offset: f64,
    pub calculated_tp_offset: f64,
    pub symbol_id: u32,
    pub trade_side: u32,
}

#[derive(Debug, Serialize)]
struct PlaceOrderRequest {
    #[serde(rename = "symbolId")]
    symbol_id: u32,
    #[serde(rename = "tradeSide")]
    trade_side: u32, // 1 for buy, 2 for sell
    volume: f64,
    #[serde(rename = "stopLoss")]
    stop_loss: f64,
    #[serde(rename = "takeProfit")]
    take_profit: f64,
}

fn get_symbol_mappings() -> HashMap<String, u32> {
    let mut symbol_mappings = HashMap::new();
    symbol_mappings.insert("EURUSD_SB".to_string(), 185);
    symbol_mappings.insert("GBPUSD_SB".to_string(), 199);
    symbol_mappings.insert("USDJPY_SB".to_string(), 226);
    symbol_mappings.insert("USDCHF_SB".to_string(), 222);
    symbol_mappings.insert("AUDUSD_SB".to_string(), 158);
    symbol_mappings.insert("USDCAD_SB".to_string(), 221);
    symbol_mappings.insert("NZDUSD_SB".to_string(), 211);
    symbol_mappings.insert("EURGBP_SB".to_string(), 175);
    symbol_mappings.insert("EURJPY_SB".to_string(), 177);
    symbol_mappings.insert("EURCHF_SB".to_string(), 173);
    symbol_mappings.insert("EURAUD_SB".to_string(), 171);
    symbol_mappings.insert("EURCAD_SB".to_string(), 172);
    symbol_mappings.insert("EURNZD_SB".to_string(), 180);
    symbol_mappings.insert("GBPJPY_SB".to_string(), 192);
    symbol_mappings.insert("GBPCHF_SB".to_string(), 191);
    symbol_mappings.insert("GBPAUD_SB".to_string(), 189);
    symbol_mappings.insert("GBPCAD_SB".to_string(), 190);
    symbol_mappings.insert("GBPNZD_SB".to_string(), 195);
    symbol_mappings.insert("AUDJPY_SB".to_string(), 155);
    symbol_mappings.insert("AUDNZD_SB".to_string(), 156);
    symbol_mappings.insert("AUDCAD_SB".to_string(), 153);
    symbol_mappings.insert("NZDJPY_SB".to_string(), 210);
    symbol_mappings.insert("CADJPY_SB".to_string(), 162);
    symbol_mappings.insert("CHFJPY_SB".to_string(), 163);
    symbol_mappings.insert("NAS100_SB".to_string(), 205);
    symbol_mappings.insert("US500_SB".to_string(), 220);
    symbol_mappings
}

fn get_lot_size_for_symbol(base_symbol: &str, default_lot_size: f64) -> f64 {
    if base_symbol.contains("JPY") {
        env::var("TRADING_JPY_LOT_SIZE")
            .unwrap_or_else(|_| "100".to_string())
            .parse()
            .unwrap_or(100.0)
    } else if base_symbol.contains("NAS100") || base_symbol.contains("US500") {
        env::var("TRADING_INDEX_LOT_SIZE")
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .unwrap_or(10.0)
    } else {
        default_lot_size
    }
}

pub async fn trigger_test_trade(
    req: web::Json<TestTradeRequest>,
) -> Result<HttpResponse, ActixError> {
    info!("🧪 Received test trade request: {:?}", req.0);

    // Load trading configuration from environment
    let stop_loss_pips: f64 = env::var("TRADING_STOP_LOSS_PIPS")
        .unwrap_or_else(|_| "40.0".to_string())
        .parse()
        .unwrap_or(40.0);
    
    let take_profit_pips: f64 = env::var("TRADING_TAKE_PROFIT_PIPS")
        .unwrap_or_else(|_| "10.0".to_string())
        .parse()
        .unwrap_or(10.0);
    
    let lot_size: f64 = env::var("TRADING_LOT_SIZE")
        .unwrap_or_else(|_| "1000".to_string())
        .parse()
        .unwrap_or(1000.0);

    let api_bridge_url = env::var("CTRADER_API_BRIDGE_URL")
        .unwrap_or_else(|_| "http://localhost:8000".to_string());

    // Get pip value for the symbol
    let pip_value = if req.symbol.contains("JPY") {
        0.01
    } else {
        0.0001
    };

    let current_price = req.current_price;

    info!(
        "🔧 TRADE CALCULATION DEBUG: symbol={}, current_price={:.5}, pip_value={}, sl_pips={}, tp_pips={}",
        req.symbol, current_price, pip_value, stop_loss_pips, take_profit_pips
    );

    // Calculate trade parameters - send absolute pip distances (positive values)
    // The TypeScript bridge will convert these to relativeStopLoss/relativeTakeProfit
    let (trade_type, entry_price, stop_loss_distance, take_profit_distance, trade_side) = match req.zone_type.as_str() {
        "demand_zone" => {
            let entry = current_price;
            // For BUY: Send positive pip distances
            let sl_distance = stop_loss_pips * pip_value;  // Always positive (40 pips = 0.004)
            let tp_distance = take_profit_pips * pip_value; // Always positive (10 pips = 0.001)
            info!(
                "🟢 BUY CALCULATION: entry={:.5}, sl_distance={:.5} ({} pips below), tp_distance={:.5} ({} pips above)",
                entry, sl_distance, stop_loss_pips, tp_distance, take_profit_pips
            );
            ("buy".to_string(), entry, sl_distance, tp_distance, 1u32)
        }
        "supply_zone" => {
            let entry = current_price;
            // For SELL: Send positive pip distances  
            let sl_distance = stop_loss_pips * pip_value;  // Always positive (40 pips = 0.004)
            let tp_distance = take_profit_pips * pip_value; // Always positive (10 pips = 0.001)
            info!(
                "🔴 SELL CALCULATION: entry={:.5}, sl_distance={:.5} ({} pips above), tp_distance={:.5} ({} pips below)",
                entry, sl_distance, stop_loss_pips, tp_distance, take_profit_pips
            );
            ("sell".to_string(), entry, sl_distance, tp_distance, 2u32)
        }
        _ => {
            let response = TestTradeResponse {
                success: false,
                message: format!("Unknown zone type: {}", req.zone_type),
                trade_details: None,
                error: Some(format!("unknown_zone_type_{}", req.zone_type)),
            };
            return Ok(HttpResponse::BadRequest().json(response));
        }
    };

    // Get symbol mapping
    let symbol_mappings = get_symbol_mappings();
    let sb_symbol = if req.symbol.ends_with("_SB") {
        req.symbol.clone()
    } else {
        format!("{}_SB", req.symbol)
    };
    
    let symbol_id = match symbol_mappings.get(&sb_symbol) {
        Some(id) => *id,
        None => {
            let response = TestTradeResponse {
                success: false,
                message: format!("Symbol mapping not found for {}", sb_symbol),
                trade_details: None,
                error: Some(format!("symbol_mapping_missing_{}", req.symbol)),
            };
            return Ok(HttpResponse::BadRequest().json(response));
        }
    };

    // Calculate lot size for this symbol
    let calculated_lot_size = get_lot_size_for_symbol(&req.symbol, lot_size);

    info!(
        "🚀 Executing TEST {} trade: {} @ {:.5} (SL distance: {:.5}, TP distance: {:.5})",
        trade_type.to_uppercase(),
        req.symbol,
        entry_price,
        stop_loss_distance,
        take_profit_distance
    );

    // Prepare the API request with positive pip distances
    let api_request = PlaceOrderRequest {
        symbol_id,
        trade_side,
        volume: calculated_lot_size,
        stop_loss: stop_loss_distance,
        take_profit: take_profit_distance,
    };

    let url = format!("{}/placeOrder", api_bridge_url);

    info!(
        "🌐 API call: {} with symbol_id={}, trade_side={}, volume={}, SL_distance={:.5}, TP_distance={:.5}",
        url,
        api_request.symbol_id,
        api_request.trade_side,
        api_request.volume,
        api_request.stop_loss,
        api_request.take_profit
    );

    // Make the API call
    let client = reqwest::Client::new();
    match client
        .post(&url)
        .json(&api_request)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(response) => {
            let response_text = response.text().await.unwrap_or_else(|_| "Failed to read response".to_string());
            info!("📄 API response: {}", response_text);

            let calculated_sl_distance = stop_loss_pips * pip_value;
            let calculated_tp_distance = take_profit_pips * pip_value;

            let response = TestTradeResponse {
                success: true,
                message: "Test trade API call completed".to_string(),
                trade_details: Some(TestTradeDetails {
                    symbol: req.symbol.clone(),
                    trade_type,
                    entry_price,
                    stop_loss: stop_loss_distance,
                    take_profit: take_profit_distance,
                    lot_size: calculated_lot_size,
                    ctrader_response: Some(response_text),
                    config_sl_pips: stop_loss_pips,
                    config_tp_pips: take_profit_pips,
                    pip_value,
                    calculated_sl_offset: calculated_sl_distance,
                    calculated_tp_offset: calculated_tp_distance,
                    symbol_id,
                    trade_side,
                }),
                error: None,
            };

            Ok(HttpResponse::Ok().json(response))
        }
        Err(e) => {
            error!("❌ API call failed: {}", e);
            
            let response = TestTradeResponse {
                success: false,
                message: format!("API call failed: {}", e),
                trade_details: None,
                error: Some(e.to_string()),
            };

            Ok(HttpResponse::InternalServerError().json(response))
        }
    }
}