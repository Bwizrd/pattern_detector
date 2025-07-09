// // src/bin/zone_monitor/enriched_trades.rs
// // Module for enriching trade data with booked order information

// use axum::{extract::Path, extract::Query, extract::State, http::StatusCode, Json};
// use serde::{Deserialize, Serialize};
// use std::collections::HashMap;
// use tracing::{info, warn, debug};
// use crate::state::MonitorState;
// use chrono::{DateTime, Utc};

// // Structures for the Node.js API response
// #[derive(Debug, Deserialize, Serialize, Clone)]
// pub struct Deal {
//     #[serde(rename = "dealId")]
//     pub deal_id: i64,
//     #[serde(rename = "orderId")]
//     pub order_id: i64,
//     #[serde(rename = "positionId")]
//     pub position_id: i64,
//     #[serde(rename = "symbolId")]
//     pub symbol_id: i32,
//     pub volume: f64,
//     #[serde(rename = "originalTradeSide")]
//     pub original_trade_side: i32,
//     #[serde(rename = "closingTradeSide")]
//     pub closing_trade_side: i32,
//     #[serde(rename = "volumeInLots")]
//     pub volume_in_lots: f64,
//     #[serde(rename = "entryPrice")]
//     pub entry_price: f64,
//     #[serde(rename = "exitPrice")]
//     pub exit_price: f64,
//     #[serde(rename = "priceDifference")]
//     pub price_difference: f64,
//     #[serde(rename = "pipsProfit")]
//     pub pips_profit: f64,
//     #[serde(rename = "openTime")]
//     pub open_time: i64,
//     #[serde(rename = "closeTime")]
//     pub close_time: i64,
//     pub duration: i64,
//     #[serde(rename = "grossProfit")]
//     pub gross_profit: f64,
//     pub swap: f64,
//     pub commission: f64,
//     #[serde(rename = "pnlConversionFee")]
//     pub pnl_conversion_fee: f64,
//     #[serde(rename = "netProfit")]
//     pub net_profit: f64,
//     #[serde(rename = "balanceAfterTrade")]
//     pub balance_after_trade: f64,
//     #[serde(rename = "balanceVersion")]
//     pub balance_version: i64,
//     #[serde(rename = "quoteToDepositConversionRate")]
//     pub quote_to_deposit_conversion_rate: f64,
//     #[serde(rename = "dealStatus")]
//     pub deal_status: i32,
//     pub label: String,
//     pub comment: String,
// }

// // Update DealsResponse to match the closed trades response
// #[derive(Debug, Deserialize)]
// struct DealsResponse {
//     #[serde(rename = "closedTrades")]
//     deals: Vec<Deal>,
//     #[serde(rename = "totalDeals")]
//     total_deals: i32,
//     #[serde(rename = "hasMore")]
//     has_more: bool,
//     // Ignore the summary field since we calculate our own
// }

// // Booked order information structure
// #[derive(Debug, Deserialize, Serialize, Clone)]
// struct BookedOrder {
//     zone_id: String,
//     symbol: String,
//     timeframe: String,
//     order_type: String,
//     entry_price: f64,
//     lot_size: f64,
//     stop_loss: Option<f64>,
//     take_profit: Option<f64>,
//     ctrader_order_id: String,
//     booked_at: String,
//     status: String,
//     filled_at: Option<String>,
//     filled_price: Option<f64>,
//     closed_at: Option<String>,
//     // Optional zone analysis fields
//     zone_type: Option<String>,
//     zone_high: Option<f64>,
//     zone_low: Option<f64>,
//     zone_strength: Option<f64>,
//     touch_count: Option<i32>,
//     distance_when_placed: Option<f64>,
//     original_zone_id: Option<String>,
// }

// #[derive(Debug, Deserialize)]
// struct BookedOrdersFile {
//     last_updated: String,
//     booked_orders: HashMap<String, BookedOrder>,
// }

// // Enriched trade combining both data sources
// #[derive(Debug, Serialize)]
// pub struct EnrichedTrade {
//     // Original closed trade data
//     #[serde(rename = "dealId")]
//     pub deal_id: i64,
//     #[serde(rename = "orderId")]
//     pub order_id: i64,
//     #[serde(rename = "positionId")]
//     pub position_id: i64,
//     #[serde(rename = "symbolId")]
//     pub symbol_id: i32,
//     pub volume: f64,
//     #[serde(rename = "originalTradeSide")]
//     pub original_trade_side: i32,
//     #[serde(rename = "closingTradeSide")]
//     pub closing_trade_side: i32,
//     #[serde(rename = "volumeInLots")]
//     pub volume_in_lots: f64,
//     #[serde(rename = "entryPrice")]
//     pub entry_price: f64,
//     #[serde(rename = "exitPrice")]
//     pub exit_price: f64,
//     #[serde(rename = "priceDifference")]
//     pub price_difference: f64,
//     #[serde(rename = "pipsProfit")]
//     pub pips_profit: f64,
//     #[serde(rename = "openTime")]
//     pub open_time: i64,
//     #[serde(rename = "closeTime")]
//     pub close_time: i64,
//     pub duration: i64,
//     #[serde(rename = "grossProfit")]
//     pub gross_profit: f64,
//     pub swap: f64,
//     pub commission: f64,
//     #[serde(rename = "pnlConversionFee")]
//     pub pnl_conversion_fee: f64,
//     #[serde(rename = "netProfit")]
//     pub net_profit: f64,
//     #[serde(rename = "balanceAfterTrade")]
//     pub balance_after_trade: f64,
//     #[serde(rename = "balanceVersion")]
//     pub balance_version: i64,
//     #[serde(rename = "quoteToDepositConversionRate")]
//     pub quote_to_deposit_conversion_rate: f64,
//     #[serde(rename = "dealStatus")]
//     pub deal_status: i32,
//     pub label: String,
//     pub comment: String,
    
//     // Enriched data from booked orders
//     #[serde(rename = "enrichmentStatus")]
//     pub enrichment_status: String, // "enriched", "partial", "not_found"
//     #[serde(rename = "zoneId")]
//     pub zone_id: Option<String>,
//     pub symbol: Option<String>,
//     pub timeframe: Option<String>,
//     #[serde(rename = "orderType")]
//     pub order_type: Option<String>,
//     #[serde(rename = "entryPricePlanned")]
//     pub entry_price_planned: Option<f64>,
//     #[serde(rename = "lotSize")]
//     pub lot_size: Option<f64>,
//     #[serde(rename = "stopLoss")]
//     pub stop_loss: Option<f64>,
//     #[serde(rename = "takeProfit")]
//     pub take_profit: Option<f64>,
//     #[serde(rename = "bookedAt")]
//     pub booked_at: Option<String>,
//     pub status: Option<String>,
//     #[serde(rename = "filledAt")]
//     pub filled_at: Option<String>,
//     #[serde(rename = "filledPrice")]
//     pub filled_price: Option<f64>,
//     #[serde(rename = "closedAt")]
//     pub closed_at: Option<String>,
    
//     // Zone analysis data (if available)
//     #[serde(rename = "zoneType")]
//     pub zone_type: Option<String>,
//     #[serde(rename = "zoneHigh")]
//     pub zone_high: Option<f64>,
//     #[serde(rename = "zoneLow")]
//     pub zone_low: Option<f64>,
//     #[serde(rename = "zoneStrength")]
//     pub zone_strength: Option<f64>,
//     #[serde(rename = "touchCount")]
//     pub touch_count: Option<i32>,
//     #[serde(rename = "distanceWhenPlaced")]
//     pub distance_when_placed: Option<f64>,
//     #[serde(rename = "originalZoneId")]
//     pub original_zone_id: Option<String>,
// }

// #[derive(Debug, Serialize)]
// pub struct EnrichedTradesResponse {
//     pub deals: Vec<EnrichedTrade>,
//     #[serde(rename = "hasMore")]
//     pub has_more: bool,
//     #[serde(rename = "enrichmentSummary")]
//     pub enrichment_summary: EnrichmentSummary,
// }

// #[derive(Debug, Serialize)]
// pub struct EnrichmentSummary {
//     #[serde(rename = "totalDeals")]
//     pub total_deals: usize,
//     #[serde(rename = "enrichedCount")]
//     pub enriched_count: usize,
//     #[serde(rename = "partialCount")]
//     pub partial_count: usize,
//     #[serde(rename = "notFoundCount")]
//     pub not_found_count: usize,
//     #[serde(rename = "enrichmentRate")]
//     pub enrichment_rate: f64,
//     #[serde(rename = "matchingDebug")]
//     pub matching_debug: MatchingDebug,
// }

// #[derive(Debug, Serialize)]
// pub struct MatchingDebug {
//     #[serde(rename = "totalBookedOrders")]
//     pub total_booked_orders: usize,
//     #[serde(rename = "sampleOrderIds")]
//     pub sample_order_ids: Vec<String>,
//     #[serde(rename = "samplePositionIds")]
//     pub sample_position_ids: Vec<String>,
//     #[serde(rename = "sampleDealOrderIds")]
//     pub sample_deal_order_ids: Vec<String>,
//     #[serde(rename = "sampleDealPositionIds")]
//     pub sample_deal_position_ids: Vec<String>,
// }

// // API endpoint for enriched trades by date
// pub async fn enriched_trades_by_date_api(
//     State(state): State<MonitorState>,
//     Path(date): Path<String>,
//     Query(params): Query<HashMap<String, String>>,
// ) -> (StatusCode, Json<serde_json::Value>) {
    
//     let max_rows = params.get("maxRows").and_then(|s| s.parse().ok()).unwrap_or(1000);
    
//     info!("📈 Getting enriched trades for date: {} (maxRows: {})", date, max_rows);
    
//     match get_enriched_trades_for_date(&state, &date, max_rows).await {
//         Ok(enriched_response) => {
//             info!("✅ Successfully enriched {} trades for {}", 
//                   enriched_response.deals.len(), date);
//             (StatusCode::OK, Json(serde_json::to_value(enriched_response).unwrap()))
//         },
//         Err(e) => {
//             warn!("❌ Failed to get enriched trades for {}: {}", date, e);
//             let error = serde_json::json!({
//                 "error": "Failed to get enriched trades",
//                 "message": e,
//                 "date": date
//             });
//             (StatusCode::INTERNAL_SERVER_ERROR, Json(error))
//         }
//     }
// }

// // API endpoint for enriched trades by date range
// pub async fn enriched_trades_by_range_api(
//     State(state): State<MonitorState>,
//     Query(params): Query<HashMap<String, String>>,
// ) -> (StatusCode, Json<serde_json::Value>) {
    
//     let start_date = match params.get("startDate") {
//         Some(date) => date,
//         None => {
//             let error = serde_json::json!({
//                 "error": "Missing required parameter",
//                 "message": "startDate parameter is required"
//             });
//             return (StatusCode::BAD_REQUEST, Json(error));
//         }
//     };
    
//     let end_date = match params.get("endDate") {
//         Some(date) => date,
//         None => {
//             let error = serde_json::json!({
//                 "error": "Missing required parameter", 
//                 "message": "endDate parameter is required"
//             });
//             return (StatusCode::BAD_REQUEST, Json(error));
//         }
//     };
    
//     let max_rows = params.get("maxRows").and_then(|s| s.parse().ok()).unwrap_or(1000);
    
//     info!("📈 Getting enriched trades for range: {} to {} (maxRows: {})", 
//           start_date, end_date, max_rows);
    
//     match get_enriched_trades_for_range(&state, start_date, end_date, max_rows).await {
//         Ok(enriched_response) => {
//             info!("✅ Successfully enriched {} trades for range {} to {}", 
//                   enriched_response.deals.len(), start_date, end_date);
//             (StatusCode::OK, Json(serde_json::to_value(enriched_response).unwrap()))
//         },
//         Err(e) => {
//             warn!("❌ Failed to get enriched trades for range {} to {}: {}", 
//                   start_date, end_date, e);
//             let error = serde_json::json!({
//                 "error": "Failed to get enriched trades",
//                 "message": e,
//                 "start_date": start_date,
//                 "end_date": end_date
//             });
//             (StatusCode::INTERNAL_SERVER_ERROR, Json(error))
//         }
//     }
// }

// // Core function to get enriched trades for a specific date
// async fn get_enriched_trades_for_date(state: &MonitorState, date: &str, max_rows: i32) -> Result<EnrichedTradesResponse, String> {
//     // Get the cTrader API bridge URL from environment
//     let ctrader_api_url = std::env::var("CTRADER_API_BRIDGE_URL")
//         .unwrap_or_else(|_| "http://localhost:8000".to_string());
    
//     // Call the Node.js deals API
//     let deals_url = format!("{}/deals/{}?maxRows={}", ctrader_api_url, date, max_rows);
//     let client = reqwest::Client::new();
    
//     info!("🔗 Calling deals API: {}", deals_url);
    
//     let deals_response = client.get(&deals_url)
//         .send()
//         .await
//         .map_err(|e| format!("Failed to call deals API: {}", e))?;
    
//     if !deals_response.status().is_success() {
//         return Err(format!("Deals API returned error: {}", deals_response.status()));
//     }
    
//     let deals_data: DealsResponse = deals_response
//         .json()
//         .await
//         .map_err(|e| format!("Failed to parse deals response: {}", e))?;
    
//     info!("📊 Retrieved {} deals from API", deals_data.deals.len());
    
//     // Load booked orders data from both JSON and InfluxDB
//     let booked_orders = load_combined_order_data(state, date).await?;
//     info!("📋 Loaded {} combined order records for enrichment", booked_orders.len());
    
//     // Enrich the deals
//     let enriched_deals = enrich_deals(deals_data.deals, &booked_orders);
    
//     // Calculate enrichment summary
//     let enrichment_summary = calculate_enrichment_summary(&enriched_deals, &booked_orders);
    
//     info!("💎 Enrichment complete: {:.1}% enrichment rate ({} enriched, {} partial, {} not found)", 
//           enrichment_summary.enrichment_rate,
//           enrichment_summary.enriched_count,
//           enrichment_summary.partial_count,
//           enrichment_summary.not_found_count);
    
//     Ok(EnrichedTradesResponse {
//         deals: enriched_deals,
//         has_more: deals_data.has_more,
//         enrichment_summary,
//     })
// }

// // Core function to get enriched trades for a date range
// async fn get_enriched_trades_for_range(state: &MonitorState, start_date: &str, end_date: &str, max_rows: i32) -> Result<EnrichedTradesResponse, String> {
//     let ctrader_api_url = std::env::var("CTRADER_API_BRIDGE_URL")
//         .unwrap_or_else(|_| "http://localhost:8000".to_string());
    
//     let deals_url = format!("{}/deals?startDate={}&endDate={}&maxRows={}", 
//                            ctrader_api_url, start_date, end_date, max_rows);
//     let client = reqwest::Client::new();
    
//     info!("🔗 Calling deals API: {}", deals_url);
    
//     let deals_response = client.get(&deals_url)
//         .send()
//         .await
//         .map_err(|e| format!("Failed to call deals API: {}", e))?;
    
//     if !deals_response.status().is_success() {
//         return Err(format!("Deals API returned error: {}", deals_response.status()));
//     }
    
//     let deals_data: DealsResponse = deals_response
//         .json()
//         .await
//         .map_err(|e| format!("Failed to parse deals response: {}", e))?;
    
//     info!("📊 Retrieved {} deals from API", deals_data.deals.len());
    
//     let booked_orders = load_combined_order_data_range(state, start_date, end_date).await?;
//     info!("📋 Loaded {} combined order records for enrichment", booked_orders.len());
    
//     let enriched_deals = enrich_deals(deals_data.deals, &booked_orders);
//     let enrichment_summary = calculate_enrichment_summary(&enriched_deals, &booked_orders);
    
//     info!("💎 Enrichment complete: {:.1}% enrichment rate ({} enriched, {} partial, {} not found)", 
//           enrichment_summary.enrichment_rate,
//           enrichment_summary.enriched_count,
//           enrichment_summary.partial_count,
//           enrichment_summary.not_found_count);
    
//     Ok(EnrichedTradesResponse {
//         deals: enriched_deals,
//         has_more: deals_data.has_more,
//         enrichment_summary,
//     })
// }

// // Load combined order data from both JSON files and InfluxDB for a specific date
// async fn load_combined_order_data(state: &MonitorState, date: &str) -> Result<HashMap<String, BookedOrder>, String> {
//     // Parse the date to create a date range (start of day to end of day)
//     let start_date = if date.contains('T') {
//         format!("{}Z", date)
//     } else {
//         format!("{}T00:00:00Z", date)
//     };
//     let end_date = if date.contains('T') {
//         format!("{}Z", date)
//     } else {
//         format!("{}T23:59:59Z", date)
//     };
    
//     load_combined_order_data_range(state, &start_date, &end_date).await
// }

// // Load combined order data from both JSON files and InfluxDB for a date range
// async fn load_combined_order_data_range(state: &MonitorState, start_date: &str, end_date: &str) -> Result<HashMap<String, BookedOrder>, String> {
//     let mut combined_orders = HashMap::new();
    
//     // First, load from JSON file (legacy fallback)
//     if let Ok(json_orders) = load_booked_orders().await {
//         info!("📄 Loaded {} orders from JSON file", json_orders.len());
//         combined_orders.extend(json_orders);
//     }
    
//     // Then, load from InfluxDB (primary source)
//     if let Ok(influx_orders) = load_orders_from_influxdb(state, start_date, end_date).await {
//         info!("🗄️  Loaded {} orders from InfluxDB", influx_orders.len());
//         // InfluxDB data takes precedence - overwrites JSON data for matching keys
//         combined_orders.extend(influx_orders);
//     } else {
//         warn!("⚠️  Failed to load from InfluxDB, using JSON data only");
//     }
    
//     info!("🔄 Combined total: {} order records", combined_orders.len());
//     Ok(combined_orders)
// }

// // Load orders from InfluxDB for the given date range
// async fn load_orders_from_influxdb(state: &MonitorState, start_date: &str, end_date: &str) -> Result<HashMap<String, BookedOrder>, String> {
//     // Parse dates for InfluxDB query
//     let start_dt = start_date.parse::<DateTime<Utc>>()
//         .map_err(|e| format!("Invalid start date format: {}", e))?;
//     let end_dt = end_date.parse::<DateTime<Utc>>()
//         .map_err(|e| format!("Invalid end date format: {}", e))?;
    
//     // Query InfluxDB for order events in the date range
//     match state.order_database.get_orders_by_date(start_dt, end_dt).await {
//         Ok(order_events) => {
//             info!("📊 Retrieved {} order events from InfluxDB", order_events.len());
            
//             // Convert InfluxDB order events to BookedOrder format
//             let mut orders = HashMap::new();
//             for event in order_events {
//                 let booked_order = convert_order_event_to_booked_order(event);
//                 orders.insert(booked_order.zone_id.clone(), booked_order);
//             }
            
//             Ok(orders)
//         }
//         Err(e) => {
//             warn!("❌ Failed to query InfluxDB: {}", e);
//             Err(format!("InfluxDB query failed: {}", e))
//         }
//     }
// }

// // Convert InfluxDB OrderEvent to BookedOrder format
// fn convert_order_event_to_booked_order(event: crate::db::schema::OrderEvent) -> BookedOrder {
//     let zone_id = event.zone_id.clone(); // Clone early to avoid borrow issues
    
//     BookedOrder {
//         zone_id: event.zone_id,
//         symbol: event.symbol,
//         timeframe: event.timeframe,
//         order_type: event.order_type,
//         entry_price: event.entry_price,
//         lot_size: event.lot_size as f64,
//         stop_loss: Some(event.stop_loss),
//         take_profit: Some(event.take_profit),
//         ctrader_order_id: event.ctrader_order_id.unwrap_or_default(),
//         booked_at: event.timestamp.to_rfc3339(),
//         status: event.status.to_string(),
//         filled_at: event.fill_price.map(|_| event.timestamp.to_rfc3339()),
//         filled_price: event.fill_price,
//         closed_at: event.close_price.map(|_| event.timestamp.to_rfc3339()),
        
//         // Zone metadata from InfluxDB
//         zone_type: Some(event.zone_type),
//         zone_high: event.zone_high,
//         zone_low: event.zone_low,
//         zone_strength: event.zone_strength,
//         touch_count: event.touch_count,
//         distance_when_placed: event.distance_when_placed,
//         original_zone_id: Some(zone_id), // Use the cloned value
//     }
// }

// // Load booked orders from the JSON file (legacy fallback)
// async fn load_booked_orders() -> Result<HashMap<String, BookedOrder>, String> {
//     match tokio::fs::read_to_string("shared_booked_orders.json").await {
//         Ok(content) => {
//             let booked_orders_file: BookedOrdersFile = serde_json::from_str(&content)
//                 .map_err(|e| format!("Failed to parse booked orders JSON: {}", e))?;
            
//             info!("📄 Loaded booked orders file with {} orders", booked_orders_file.booked_orders.len());
            
//             // Log sample order IDs for debugging
//             let sample_keys: Vec<String> = booked_orders_file.booked_orders.keys().take(3).cloned().collect();
//             debug!("🔍 Sample booked order keys: {:?}", sample_keys);
            
//             Ok(booked_orders_file.booked_orders)
//         }
//         Err(e) => {
//             warn!("⚠️  Could not load booked orders file: {}", e);
//             Ok(HashMap::new()) // Return empty map if file doesn't exist
//         }
//     }
// }

// // Enrich deals with booked order information - improved matching logic
// fn enrich_deals(deals: Vec<Deal>, booked_orders: &HashMap<String, BookedOrder>) -> Vec<EnrichedTrade> {
//     info!("🔧 Starting enrichment for {} deals with {} booked orders", deals.len(), booked_orders.len());
    
//     deals.into_iter().map(|deal| {
//         // Try multiple matching strategies
//         let matching_order = find_matching_order(&deal, booked_orders);
        
//         let enrichment_status = if let Some(order) = &matching_order {
//             debug!("✅ Found match for deal {} with order {}", deal.deal_id, order.ctrader_order_id);
//             if order.zone_type.is_some() && order.zone_strength.is_some() {
//                 "enriched".to_string()
//             } else {
//                 "partial".to_string()
//             }
//         } else {
//             debug!("❌ No match found for deal {} (order_id: {}, position_id: {})", 
//                   deal.deal_id, deal.order_id, deal.position_id);
//             "not_found".to_string()
//         };
        
//         EnrichedTrade {
//             // Original closed trade data - using the correct field names from Deal
//             deal_id: deal.deal_id,
//             order_id: deal.order_id,
//             position_id: deal.position_id,
//             symbol_id: deal.symbol_id,
//             volume: deal.volume,
//             original_trade_side: deal.original_trade_side,
//             closing_trade_side: deal.closing_trade_side,
//             volume_in_lots: deal.volume_in_lots,
//             entry_price: deal.entry_price,
//             exit_price: deal.exit_price,
//             price_difference: deal.price_difference,
//             pips_profit: deal.pips_profit,
//             open_time: deal.open_time,
//             close_time: deal.close_time,
//             duration: deal.duration,
//             gross_profit: deal.gross_profit,
//             swap: deal.swap,
//             commission: deal.commission,
//             pnl_conversion_fee: deal.pnl_conversion_fee,
//             net_profit: deal.net_profit,
//             balance_after_trade: deal.balance_after_trade,
//             balance_version: deal.balance_version,
//             quote_to_deposit_conversion_rate: deal.quote_to_deposit_conversion_rate,
//             deal_status: deal.deal_status,
//             label: deal.label,
//             comment: deal.comment,
            
//             // Enrichment status
//             enrichment_status,
            
//             // Enriched data from booked orders
//             zone_id: matching_order.as_ref().map(|o| o.zone_id.clone()),
//             symbol: matching_order.as_ref().map(|o| o.symbol.clone()),
//             timeframe: matching_order.as_ref().map(|o| o.timeframe.clone()),
//             order_type: matching_order.as_ref().map(|o| o.order_type.clone()),
//             entry_price_planned: matching_order.as_ref().map(|o| o.entry_price),
//             lot_size: matching_order.as_ref().map(|o| o.lot_size),
//             stop_loss: matching_order.as_ref().and_then(|o| o.stop_loss),
//             take_profit: matching_order.as_ref().and_then(|o| o.take_profit),
//             booked_at: matching_order.as_ref().map(|o| o.booked_at.clone()),
//             status: matching_order.as_ref().map(|o| o.status.clone()),
//             filled_at: matching_order.as_ref().and_then(|o| o.filled_at.clone()),
//             filled_price: matching_order.as_ref().and_then(|o| o.filled_price),
//             closed_at: matching_order.as_ref().and_then(|o| o.closed_at.clone()),
            
//             // Zone analysis data
//             zone_type: matching_order.as_ref().and_then(|o| o.zone_type.clone()),
//             zone_high: matching_order.as_ref().and_then(|o| o.zone_high),
//             zone_low: matching_order.as_ref().and_then(|o| o.zone_low),
//             zone_strength: matching_order.as_ref().and_then(|o| o.zone_strength),
//             touch_count: matching_order.as_ref().and_then(|o| o.touch_count),
//             distance_when_placed: matching_order.as_ref().and_then(|o| o.distance_when_placed),
//             original_zone_id: matching_order.as_ref().and_then(|o| o.original_zone_id.clone()),
//         }
//     }).collect()
// }

// // Improved matching logic with multiple strategies
// fn find_matching_order(deal: &Deal, booked_orders: &HashMap<String, BookedOrder>) -> Option<BookedOrder> {
//     let order_id_str = deal.order_id.to_string();
//     let position_id_str = deal.position_id.to_string();
    
//     // Strategy 1: Match by order_id against ctrader_order_id
//     if let Some(order) = booked_orders.values().find(|order| order.ctrader_order_id == order_id_str) {
//         debug!("🎯 Strategy 1 match: order_id {} = ctrader_order_id {}", order_id_str, order.ctrader_order_id);
//         return Some(order.clone());
//     }
    
//     // Strategy 2: Match by position_id against ctrader_order_id
//     if let Some(order) = booked_orders.values().find(|order| order.ctrader_order_id == position_id_str) {
//         debug!("🎯 Strategy 2 match: position_id {} = ctrader_order_id {}", position_id_str, order.ctrader_order_id);
//         return Some(order.clone());
//     }
    
//     // Strategy 3: Try matching by the map key (which might be order_id or position_id)
//     if let Some(order) = booked_orders.get(&order_id_str) {
//         debug!("🎯 Strategy 3 match: order_id {} found as map key", order_id_str);
//         return Some(order.clone());
//     }
    
//     if let Some(order) = booked_orders.get(&position_id_str) {
//         debug!("🎯 Strategy 4 match: position_id {} found as map key", position_id_str);
//         return Some(order.clone());
//     }
    
//     None
// }

// // Calculate enrichment statistics with debug info
// fn calculate_enrichment_summary(enriched_deals: &[EnrichedTrade], booked_orders: &HashMap<String, BookedOrder>) -> EnrichmentSummary {
//     let total = enriched_deals.len();
//     let enriched = enriched_deals.iter().filter(|d| d.enrichment_status == "enriched").count();
//     let partial = enriched_deals.iter().filter(|d| d.enrichment_status == "partial").count();
//     let not_found = enriched_deals.iter().filter(|d| d.enrichment_status == "not_found").count();
    
//     let enrichment_rate = if total > 0 {
//         (enriched + partial) as f64 / total as f64 * 100.0
//     } else {
//         0.0
//     };
    
//     // Create debug info
//     let sample_order_ids: Vec<String> = booked_orders.keys().take(3).cloned().collect();
//     let sample_position_ids: Vec<String> = booked_orders.values()
//         .take(3)
//         .map(|o| o.ctrader_order_id.clone())
//         .collect();
    
//     let sample_deal_order_ids: Vec<String> = enriched_deals.iter()
//         .take(3)
//         .map(|d| d.order_id.to_string())
//         .collect();
    
//     let sample_deal_position_ids: Vec<String> = enriched_deals.iter()
//         .take(3)
//         .map(|d| d.position_id.to_string())
//         .collect();
    
//     let matching_debug = MatchingDebug {
//         total_booked_orders: booked_orders.len(),
//         sample_order_ids,
//         sample_position_ids,
//         sample_deal_order_ids,
//         sample_deal_position_ids,
//     };
    
//     EnrichmentSummary {
//         total_deals: total,
//         enriched_count: enriched,
//         partial_count: partial,
//         not_found_count: not_found,
//         enrichment_rate,
//         matching_debug,
//     }
// }