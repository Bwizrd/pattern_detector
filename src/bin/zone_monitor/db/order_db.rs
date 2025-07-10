// src/bin/zone_monitor/db/order_db.rs
// InfluxDB interface for order lifecycle tracking with enriched deal support

use crate::db::schema::OrderEvent;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::error::Error;
use std::fmt;
use tracing::{debug, error, info, warn};

// Add the EnrichedDeal struct here for the database module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedDeal {
    // Basic deal info from broker
    pub deal_id: String,
    pub order_id: Option<String>,
    pub position_id: String,
    pub symbol: String,
    pub symbol_id: Option<i32>,
    pub volume: f64,
    pub volume_in_lots: Option<f64>,
    pub trade_side: i32,
    pub closing_trade_side: Option<i32>,
    pub price: f64,
    pub price_difference: Option<f64>,
    pub pips_profit: Option<f64>,
    pub execution_time: chrono::DateTime<chrono::Utc>,
    pub deal_type: String, // "OPEN" or "CLOSE"
    pub profit: Option<f64>,
    pub gross_profit: Option<f64>,
    pub net_profit: Option<f64>,
    pub swap: Option<f64>,
    pub commission: Option<f64>,
    pub pnl_conversion_fee: Option<f64>,
    pub balance_after_trade: Option<f64>,
    pub balance_version: Option<i64>,
    pub quote_to_deposit_conversion_rate: Option<f64>,
    pub raw_data: Option<serde_json::Value>,
    pub label: Option<String>,
    pub comment: Option<String>,
    pub deal_status: Option<i32>,
    // Timing
    pub open_time: Option<chrono::DateTime<chrono::Utc>>,
    pub close_time: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_minutes: Option<i64>,
    // Enriched data from pending orders (if available)
    pub zone_id: Option<String>,
    pub zone_type: Option<String>, // "supply_zone" or "demand_zone"
    pub zone_strength: Option<f64>,
    pub zone_high: Option<f64>,
    pub zone_low: Option<f64>,
    pub touch_count: Option<i32>,
    pub timeframe: Option<String>,
    pub distance_when_placed: Option<f64>,
    pub original_entry_price: Option<f64>,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub slippage_pips: Option<f64>,
}

#[derive(Debug)]
pub enum OrderDbError {
    InfluxError(String),
    SerializationError(String),
    NetworkError(String),
}

impl fmt::Display for OrderDbError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            OrderDbError::InfluxError(msg) => write!(f, "InfluxDB error: {}", msg),
            OrderDbError::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
            OrderDbError::NetworkError(msg) => write!(f, "Network error: {}", msg),
        }
    }
}

impl Error for OrderDbError {}

#[derive(Debug, Clone)]
pub struct OrderDatabase {
    client: Client,
    host: String,
    org: String,
    token: String,
    bucket: String,
}

impl OrderDatabase {
    pub fn new(host: String, org: String, token: String, bucket: String) -> Self {
        Self {
            client: Client::new(),
            host,
            org,
            token,
            bucket,
        }
    }

    /// Write an order event to InfluxDB
    pub async fn write_order_event(&self, event: &OrderEvent) -> Result<(), OrderDbError> {
        let line_protocol = self.event_to_line_protocol(event)?;

        debug!("Writing order event to InfluxDB: {}", line_protocol);

        let url = format!(
            "{}/api/v2/write?org={}&bucket={}",
            self.host, self.org, self.bucket
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Token {}", self.token))
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(line_protocol)
            .send()
            .await
            .map_err(|e| OrderDbError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(OrderDbError::InfluxError(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        info!(
            "Successfully wrote order event for zone_id: {}, status: {}",
            event.zone_id, event.status
        );
        Ok(())
    }

    // /// Write enriched deal to InfluxDB with full zone data and duplicate prevention
    // pub async fn write_enriched_deal(&self, deal: &EnrichedDeal) -> Result<(), OrderDbError> {
    //     info!(
    //         "🔄 ATTEMPTING to write enriched deal {} to InfluxDB",
    //         deal.deal_id
    //     );

    //     // Check for duplicates first
    //     info!("🔍 Checking if deal {} already exists...", deal.deal_id);
    //     match self.deal_already_exists(&deal.deal_id).await {
    //         Ok(exists) => {
    //             if exists {
    //                 info!(
    //                     "⏭️ Deal {} already exists in database, skipping",
    //                     deal.deal_id
    //                 );
    //                 return Ok(());
    //             } else {
    //                 info!("✅ Deal {} is new, proceeding with write", deal.deal_id);
    //             }
    //         }
    //         Err(e) => {
    //             error!(
    //                 "❌ Failed to check duplicate for deal {}: {}",
    //                 deal.deal_id, e
    //             );
    //             // Continue anyway - better to risk duplicate than lose data
    //             warn!("⚠️ Proceeding with write despite duplicate check failure");
    //         }
    //     }

    //     // Generate line protocol
    //     info!("📝 Generating line protocol for deal {}...", deal.deal_id);
    //     let line_protocol = match self.enriched_deal_to_line_protocol(deal) {
    //         Ok(line) => {
    //             info!(
    //                 "✅ Line protocol generated for deal {}: {}",
    //                 deal.deal_id, line
    //             );
    //             line
    //         }
    //         Err(e) => {
    //             error!(
    //                 "❌ Failed to generate line protocol for deal {}: {}",
    //                 deal.deal_id, e
    //             );
    //             return Err(e);
    //         }
    //     };

    //     info!(
    //         "🌐 Writing enriched deal {} to InfluxDB at {}",
    //         deal.deal_id, self.host
    //     );

    //     let url = format!(
    //         "{}/api/v2/write?org={}&bucket={}",
    //         self.host, self.org, self.bucket
    //     );
    //     info!("📍 InfluxDB write URL: {}", url);

    //     let response = self
    //         .client
    //         .post(&url)
    //         .header("Authorization", format!("Token {}", self.token))
    //         .header("Content-Type", "text/plain; charset=utf-8")
    //         .body(line_protocol.clone())
    //         .send()
    //         .await
    //         .map_err(|e| {
    //             error!("❌ HTTP request failed for deal {}: {}", deal.deal_id, e);
    //             OrderDbError::NetworkError(e.to_string())
    //         })?;

    //     let status = response.status();
    //     info!(
    //         "📡 InfluxDB response status for deal {}: {}",
    //         deal.deal_id, status
    //     );

    //     if !status.is_success() {
    //         let error_text = response
    //             .text()
    //             .await
    //             .unwrap_or_else(|_| "Unknown error".to_string());
    //         error!(
    //             "❌ InfluxDB write FAILED for deal {}: HTTP {} - {}",
    //             deal.deal_id, status, error_text
    //         );
    //         error!("❌ Failed line protocol was: {}", line_protocol);
    //         return Err(OrderDbError::InfluxError(format!(
    //             "HTTP {}: {}",
    //             status, error_text
    //         )));
    //     } else {
    //         info!(
    //             "✅ InfluxDB write SUCCESS for deal {}: HTTP {}",
    //             deal.deal_id, status
    //         );
    //     }

    //     let is_enriched = deal.zone_id.is_some();
    //     if is_enriched {
    //         info!(
    //         "🎯 SUCCESSFULLY saved ENRICHED deal {} to InfluxDB: {} {} @ {:.5} (Zone: {}, Slippage: {:.1}p)",
    //         deal.deal_id,
    //         deal.symbol,
    //         deal.deal_type,
    //         deal.price,
    //         deal.zone_id.as_ref().unwrap(),
    //         deal.slippage_pips.unwrap_or(0.0)
    //     );
    //     } else {
    //         info!(
    //             "💾 SUCCESSFULLY saved NON-ENRICHED deal {} to InfluxDB: {} {} @ {:.5}",
    //             deal.deal_id, deal.symbol, deal.deal_type, deal.price
    //         );
    //     }

    //     Ok(())
    // }

    pub async fn write_enriched_deal(&self, deal: &EnrichedDeal) -> Result<(), OrderDbError> {
        info!(
            "🔄 ATTEMPTING to write enriched deal {} to InfluxDB",
            deal.deal_id
        );

        // DUPLICATE CHECK DISABLED - broken logic was preventing all writes
        info!(
            "⚠️ DUPLICATE CHECK DISABLED - writing deal {} directly",
            deal.deal_id
        );

        // Generate line protocol
        info!("📝 Generating line protocol for deal {}...", deal.deal_id);
        let line_protocol = match self.enriched_deal_to_line_protocol(deal) {
            Ok(line) => {
                info!(
                    "✅ Line protocol generated for deal {}: {}",
                    deal.deal_id, line
                );
                line
            }
            Err(e) => {
                error!(
                    "❌ Failed to generate line protocol for deal {}: {}",
                    deal.deal_id, e
                );
                return Err(e);
            }
        };

        info!(
            "🌐 Writing enriched deal {} to InfluxDB at {}",
            deal.deal_id, self.host
        );

        let url = format!(
            "{}/api/v2/write?org={}&bucket={}",
            self.host, self.org, self.bucket
        );
        info!("📍 InfluxDB write URL: {}", url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Token {}", self.token))
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(line_protocol.clone())
            .send()
            .await
            .map_err(|e| {
                error!("❌ HTTP request failed for deal {}: {}", deal.deal_id, e);
                OrderDbError::NetworkError(e.to_string())
            })?;

        let status = response.status();
        info!(
            "📡 InfluxDB response status for deal {}: {}",
            deal.deal_id, status
        );

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!(
                "❌ InfluxDB write FAILED for deal {}: HTTP {} - {}",
                deal.deal_id, status, error_text
            );
            error!("❌ Failed line protocol was: {}", line_protocol);
            return Err(OrderDbError::InfluxError(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        } else {
            info!(
                "✅ InfluxDB write SUCCESS for deal {}: HTTP {}",
                deal.deal_id, status
            );
        }

        let is_enriched = deal.zone_id.is_some();
        if is_enriched {
            info!(
            "🎯 SUCCESSFULLY saved ENRICHED deal {} to InfluxDB: {} {} @ {:.5} (Zone: {}, Slippage: {:.1}p)",
            deal.deal_id,
            deal.symbol,
            deal.deal_type,
            deal.price,
            deal.zone_id.as_ref().unwrap(),
            deal.slippage_pips.unwrap_or(0.0)
        );
        } else {
            info!(
                "💾 SUCCESSFULLY saved NON-ENRICHED deal {} to InfluxDB: {} {} @ {:.5}",
                deal.deal_id, deal.symbol, deal.deal_type, deal.price
            );
        }

        Ok(())
    }

    /// Check if a deal already exists in the database to prevent duplicates
    async fn deal_already_exists(&self, deal_id: &str) -> Result<bool, OrderDbError> {
        let flux_query = format!(
            r#"from(bucket: "{}")
               |> range(start: -7d)
               |> filter(fn: (r) => r._measurement == "enriched_deals")
               |> filter(fn: (r) => r.deal_id == "{}")
               |> count()
               |> yield(name: "count")"#,
            self.bucket, deal_id
        );

        match self.execute_flux_query_simple(&flux_query).await {
            Ok(count) => Ok(count > 0),
            Err(e) => {
                // If query fails, assume deal doesn't exist to avoid blocking
                warn!("⚠️ Failed to check for duplicate deal {}: {}", deal_id, e);
                Ok(false)
            }
        }
    }

    /// Convert EnrichedDeal to InfluxDB line protocol
    fn enriched_deal_to_line_protocol(&self, deal: &EnrichedDeal) -> Result<String, OrderDbError> {
        let timestamp_ns = deal
            .execution_time
            .timestamp_nanos_opt()
            .ok_or_else(|| OrderDbError::SerializationError("Invalid timestamp".to_string()))?;

        // Build tags (indexed fields for querying)
        let mut tags = vec![
            format!("symbol={}", escape_tag(&deal.symbol)),
            format!("deal_type={}", escape_tag(&deal.deal_type)),
            format!("deal_id={}", escape_tag(&deal.deal_id)),
            format!("position_id={}", escape_tag(&deal.position_id)),
        ];

        // Add zone tags if available (for efficient zone-based queries)
        if let Some(ref zone_id) = deal.zone_id {
            tags.push(format!("zone_id={}", escape_tag(zone_id)));
        }
        if let Some(ref zone_type) = deal.zone_type {
            tags.push(format!("zone_type={}", escape_tag(zone_type)));
        }
        if let Some(ref timeframe) = deal.timeframe {
            tags.push(format!("timeframe={}", escape_tag(timeframe)));
        }

        // Build fields (actual data values)
        let mut fields = vec![
            format!("volume={}", deal.volume),
            format!("trade_side={}i", deal.trade_side),
            format!("price={}", deal.price),
        ];

        // Add profit if available
        if let Some(profit) = deal.profit {
            fields.push(format!("profit={}", profit));
        }

        // Add all enriched zone data as fields
        if let Some(zone_strength) = deal.zone_strength {
            fields.push(format!("zone_strength={}", zone_strength));
        }
        if let Some(zone_high) = deal.zone_high {
            fields.push(format!("zone_high={}", zone_high));
        }
        if let Some(zone_low) = deal.zone_low {
            fields.push(format!("zone_low={}", zone_low));
        }
        if let Some(touch_count) = deal.touch_count {
            fields.push(format!("touch_count={}i", touch_count));
        }
        if let Some(distance_when_placed) = deal.distance_when_placed {
            fields.push(format!("distance_when_placed={}", distance_when_placed));
        }
        if let Some(original_entry_price) = deal.original_entry_price {
            fields.push(format!("original_entry_price={}", original_entry_price));
        }
        if let Some(stop_loss) = deal.stop_loss {
            fields.push(format!("stop_loss={}", stop_loss));
        }
        if let Some(take_profit) = deal.take_profit {
            fields.push(format!("take_profit={}", take_profit));
        }
        if let Some(slippage_pips) = deal.slippage_pips {
            fields.push(format!("slippage_pips={}", slippage_pips));
        }

        // Add enrichment indicator
        let is_enriched = deal.zone_id.is_some();
        fields.push(format!("enriched={}", is_enriched));

        let line = format!(
            "enriched_deals,{} {} {}",
            tags.join(","),
            fields.join(","),
            timestamp_ns
        );

        Ok(line)
    }

    /// Get enriched deals statistics
    pub async fn get_enriched_deals_stats(
        &self,
        days: u32,
    ) -> Result<serde_json::Value, OrderDbError> {
        // Proper Flux query for statistics
        let flux_query = format!(
            r#"
            import "experimental"
            
            data = from(bucket: "{}")
                |> range(start: -{}d)
                |> filter(fn: (r) => r._measurement == "enriched_deals")
            
            total_deals = data 
                |> filter(fn: (r) => r._field == "volume")
                |> count()
                |> findRecord(fn: (key) => true, idx: 0)
                |> getColumn(column: "_value")
            
            enriched_deals = data 
                |> filter(fn: (r) => r._field == "enriched" and r._value == true)
                |> count()
                |> findRecord(fn: (key) => true, idx: 0)
                |> getColumn(column: "_value")
            
            avg_slippage = data 
                |> filter(fn: (r) => r._field == "slippage_pips" and r._value > 0.0)
                |> mean()
                |> findRecord(fn: (key) => true, idx: 0)
                |> getColumn(column: "_value")
            
            total_profit = data 
                |> filter(fn: (r) => r._field == "profit")
                |> sum()
                |> findRecord(fn: (key) => true, idx: 0)
                |> getColumn(column: "_value")
            
            // Return results as a table
            array.from(rows: [{{
                metric: "total_deals",
                value: total_deals
            }}, {{
                metric: "enriched_deals", 
                value: enriched_deals
            }}, {{
                metric: "avg_slippage",
                value: avg_slippage
            }}, {{
                metric: "total_profit",
                value: total_profit
            }}])
            "#,
            self.bucket, days
        );

        // Execute the query (simplified response for now)
        match self.execute_flux_query_simple(&flux_query).await {
            Ok(_) => {
                // Return basic response since full CSV parsing isn't implemented
                Ok(serde_json::json!({
                    "period_days": days,
                    "note": "Basic stats - check InfluxDB UI for detailed metrics",
                    "query_executed": true
                }))
            }
            Err(e) => Ok(serde_json::json!({
                "error": format!("Failed to execute stats query: {}", e),
                "period_days": days
            })),
        }
    }

    /// Query order events for a specific zone_id
    pub async fn get_order_lifecycle(
        &self,
        zone_id: &str,
    ) -> Result<Vec<OrderEvent>, OrderDbError> {
        let flux_query = format!(
            r#"from(bucket: "{}")
               |> range(start: -30d)
               |> filter(fn: (r) => r._measurement == "order_events")
               |> filter(fn: (r) => r.zone_id == "{}")
               |> sort(columns: ["_time"])"#,
            self.bucket, zone_id
        );

        self.execute_flux_query(&flux_query).await
    }

    /// Query order events for a specific date range
    pub async fn get_orders_by_date(
        &self,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<Vec<OrderEvent>, OrderDbError> {
        let flux_query = format!(
            r#"from(bucket: "{}")
               |> range(start: {}, stop: {})
               |> filter(fn: (r) => r._measurement == "order_events")
               |> sort(columns: ["_time"])"#,
            self.bucket,
            start_date.to_rfc3339(),
            end_date.to_rfc3339()
        );

        self.execute_flux_query(&flux_query).await
    }

    /// Query enriched deals for a specific date range
    pub async fn get_enriched_deals_by_date(
        &self,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> Result<Vec<EnrichedDeal>, OrderDbError> {
        let flux_query = format!(
            r#"from(bucket: "{}")
               |> range(start: {}, stop: {})
               |> filter(fn: (r) => r._measurement == "enriched_deals")
               |> sort(columns: ["_time"])"#,
            self.bucket,
            start_date.to_rfc3339(),
            end_date.to_rfc3339()
        );

        // For now, return empty - full implementation would parse CSV to EnrichedDeal
        warn!("get_enriched_deals_by_date not fully implemented - CSV parsing needed");
        Ok(Vec::new())
    }

    /// Query enriched deals by zone_id
    pub async fn get_enriched_deals_by_zone(
        &self,
        zone_id: &str,
    ) -> Result<Vec<EnrichedDeal>, OrderDbError> {
        let flux_query = format!(
            r#"from(bucket: "{}")
               |> range(start: -30d)
               |> filter(fn: (r) => r._measurement == "enriched_deals")
               |> filter(fn: (r) => r.zone_id == "{}")
               |> sort(columns: ["_time"])"#,
            self.bucket, zone_id
        );

        // For now, return empty - full implementation would parse CSV to EnrichedDeal
        warn!("get_enriched_deals_by_zone not fully implemented - CSV parsing needed");
        Ok(Vec::new())
    }

    /// Query orders by cTrader order ID
    pub async fn get_order_by_ctrader_id(
        &self,
        ctrader_order_id: &str,
    ) -> Result<Vec<OrderEvent>, OrderDbError> {
        let flux_query = format!(
            r#"from(bucket: "{}")
               |> range(start: -30d)
               |> filter(fn: (r) => r._measurement == "order_events")
               |> filter(fn: (r) => r.ctrader_order_id == "{}")
               |> sort(columns: ["_time"])"#,
            self.bucket, ctrader_order_id
        );

        self.execute_flux_query(&flux_query).await
    }

    /// Query orders by cTrader position ID
    pub async fn get_order_by_position_id(
        &self,
        position_id: &str,
    ) -> Result<Vec<OrderEvent>, OrderDbError> {
        let flux_query = format!(
            r#"from(bucket: "{}")
               |> range(start: -30d)
               |> filter(fn: (r) => r._measurement == "order_events")
               |> filter(fn: (r) => r.ctrader_position_id == "{}")
               |> sort(columns: ["_time"])"#,
            self.bucket, position_id
        );

        self.execute_flux_query(&flux_query).await
    }

    /// Convert OrderEvent to InfluxDB line protocol
    fn event_to_line_protocol(&self, event: &OrderEvent) -> Result<String, OrderDbError> {
        let timestamp_ns = event
            .timestamp
            .timestamp_nanos_opt()
            .ok_or_else(|| OrderDbError::SerializationError("Invalid timestamp".to_string()))?;

        // Build tags (escaped for line protocol)
        let mut tags = vec![
            format!("zone_id={}", escape_tag(&event.zone_id)),
            format!("symbol={}", escape_tag(&event.symbol)),
            format!("timeframe={}", escape_tag(&event.timeframe)),
            format!("status={}", escape_tag(&event.status.to_string())),
            format!("zone_type={}", escape_tag(&event.zone_type)),
            format!("order_type={}", escape_tag(&event.order_type)),
        ];

        // Add optional cTrader IDs as tags if present
        if let Some(ref ctrader_order_id) = event.ctrader_order_id {
            tags.push(format!("ctrader_order_id={}", escape_tag(ctrader_order_id)));
        }
        if let Some(ref position_id) = event.ctrader_position_id {
            tags.push(format!("ctrader_position_id={}", escape_tag(position_id)));
        }
        if let Some(ref deal_id) = event.ctrader_deal_id {
            tags.push(format!("ctrader_deal_id={}", escape_tag(deal_id)));
        }

        // Build fields
        let mut fields = vec![
            format!("entry_price={}", event.entry_price),
            format!("lot_size={}i", event.lot_size),
            format!("stop_loss={}", event.stop_loss),
            format!("take_profit={}", event.take_profit),
            format!(
                "status=\"{}\"",
                escape_field_string(&event.status.to_string())
            ),
        ];

        // Add optional fields
        if let Some(zone_high) = event.zone_high {
            fields.push(format!("zone_high={}", zone_high));
        }
        if let Some(zone_low) = event.zone_low {
            fields.push(format!("zone_low={}", zone_low));
        }
        if let Some(zone_strength) = event.zone_strength {
            fields.push(format!("zone_strength={}", zone_strength));
        }
        if let Some(touch_count) = event.touch_count {
            fields.push(format!("touch_count={}i", touch_count));
        }
        if let Some(distance) = event.distance_when_placed {
            fields.push(format!("distance_when_placed={}", distance));
        }
        if let Some(fill_price) = event.fill_price {
            fields.push(format!("fill_price={}", fill_price));
        }
        if let Some(close_price) = event.close_price {
            fields.push(format!("close_price={}", close_price));
        }
        if let Some(pnl) = event.actual_pnl_pips {
            fields.push(format!("actual_pnl_pips={}", pnl));
        }
        if let Some(ref reason) = event.close_reason {
            fields.push(format!("close_reason=\"{}\"", escape_field_string(reason)));
        }

        let line = format!(
            "order_events,{} {} {}",
            tags.join(","),
            fields.join(","),
            timestamp_ns
        );

        Ok(line)
    }

    /// Execute Flux query and parse results to OrderEvent
    async fn execute_flux_query(&self, query: &str) -> Result<Vec<OrderEvent>, OrderDbError> {
        debug!("Executing Flux query: {}", query);

        let url = format!("{}/api/v2/query?org={}", self.host, self.org);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Token {}", self.token))
            .json(&json!({
                "query": query,
                "type": "flux"
            }))
            .send()
            .await
            .map_err(|e| OrderDbError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(OrderDbError::InfluxError(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let response_text = response
            .text()
            .await
            .map_err(|e| OrderDbError::NetworkError(e.to_string()))?;

        // Parse CSV response into OrderEvent structs
        self.parse_flux_response(&response_text)
    }

    /// Execute simple Flux query that returns a count
    async fn execute_flux_query_simple(&self, query: &str) -> Result<i64, OrderDbError> {
        debug!("Executing simple Flux query: {}", query);

        let url = format!("{}/api/v2/query?org={}", self.host, self.org);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Token {}", self.token))
            .json(&json!({
                "query": query,
                "type": "flux"
            }))
            .send()
            .await
            .map_err(|e| OrderDbError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(OrderDbError::InfluxError(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let response_text = response
            .text()
            .await
            .map_err(|e| OrderDbError::NetworkError(e.to_string()))?;

        // Simple parsing for count queries - look for numeric values in response
        // This is a simplified implementation
        if response_text.contains("0") {
            Ok(0)
        } else {
            Ok(1) // Assume found if not 0
        }
    }

    /// Parse InfluxDB CSV response into OrderEvent structs
    fn parse_flux_response(&self, _csv_data: &str) -> Result<Vec<OrderEvent>, OrderDbError> {
        // TODO: Implement CSV parsing to OrderEvent conversion
        // This would parse the InfluxDB CSV format and reconstruct OrderEvent objects
        warn!("CSV parsing not yet implemented - returning empty result");
        Ok(Vec::new())
    }

    pub async fn get_enriched_deal_by_id(&self, _deal_id: &str) -> Result<Option<EnrichedDeal>, OrderDbError> {
        // TODO: Implement real query and parsing
        Ok(None)
    }
}

/// Escape tag values for InfluxDB line protocol
fn escape_tag(value: &str) -> String {
    value
        .replace(' ', "\\ ")
        .replace(',', "\\,")
        .replace('=', "\\=")
}

/// Escape string field values for InfluxDB line protocol
fn escape_field_string(value: &str) -> String {
    value.replace('"', "\\\"").replace('\\', "\\\\")
}
