// src/bin/zone_monitor/db/order_db.rs
// InfluxDB interface for order lifecycle tracking

use crate::db::schema::OrderEvent;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::json;
use std::error::Error;
use std::fmt;
use tracing::{debug, info, warn};

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

#[derive(Debug)]
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
        
        let url = format!("{}/api/v2/write?org={}&bucket={}", self.host, self.org, self.bucket);
        
        let response = self.client
            .post(&url)
            .header("Authorization", format!("Token {}", self.token))
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(line_protocol)
            .send()
            .await
            .map_err(|e| OrderDbError::NetworkError(e.to_string()))?;
            
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(OrderDbError::InfluxError(format!(
                "HTTP {}: {}", status, error_text
            )));
        }
        
        info!("Successfully wrote order event for zone_id: {}, status: {}", event.zone_id, event.status);
        Ok(())
    }
    
    /// Query order events for a specific zone_id
    pub async fn get_order_lifecycle(&self, zone_id: &str) -> Result<Vec<OrderEvent>, OrderDbError> {
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
    
    /// Query orders by cTrader order ID
    pub async fn get_order_by_ctrader_id(&self, ctrader_order_id: &str) -> Result<Vec<OrderEvent>, OrderDbError> {
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
    pub async fn get_order_by_position_id(&self, position_id: &str) -> Result<Vec<OrderEvent>, OrderDbError> {
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
        let timestamp_ns = event.timestamp.timestamp_nanos_opt()
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
            format!("status=\"{}\"", escape_field_string(&event.status.to_string())),
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
    
    /// Execute Flux query and parse results
    async fn execute_flux_query(&self, query: &str) -> Result<Vec<OrderEvent>, OrderDbError> {
        debug!("Executing Flux query: {}", query);
        
        let url = format!("{}/api/v2/query?org={}", self.host, self.org);
        
        let response = self.client
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
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(OrderDbError::InfluxError(format!(
                "HTTP {}: {}", status, error_text
            )));
        }
        
        let response_text = response.text().await
            .map_err(|e| OrderDbError::NetworkError(e.to_string()))?;
            
        // Parse CSV response into OrderEvent structs
        self.parse_flux_response(&response_text)
    }
    
    /// Parse InfluxDB CSV response into OrderEvent structs
    fn parse_flux_response(&self, _csv_data: &str) -> Result<Vec<OrderEvent>, OrderDbError> {
        // TODO: Implement CSV parsing to OrderEvent conversion
        // This would parse the InfluxDB CSV format and reconstruct OrderEvent objects
        warn!("CSV parsing not yet implemented - returning empty result");
        Ok(Vec::new())
    }
}

/// Escape tag values for InfluxDB line protocol
fn escape_tag(value: &str) -> String {
    value.replace(' ', "\\ ")
         .replace(',', "\\,")
         .replace('=', "\\=")
}

/// Escape string field values for InfluxDB line protocol
fn escape_field_string(value: &str) -> String {
    value.replace('"', "\\\"")
         .replace('\\', "\\\\")
}