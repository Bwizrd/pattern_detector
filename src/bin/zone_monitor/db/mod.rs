// src/bin/zone_monitor/db/mod.rs
// Database module for order lifecycle tracking

pub mod order_db;
pub mod schema;

pub use order_db::OrderDatabase;
pub use schema::{OrderStatus, OrderEventBuilder};