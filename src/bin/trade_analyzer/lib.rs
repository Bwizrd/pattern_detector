// lib.rs
pub mod analyzer;
pub mod config;
pub mod csv_writer;
pub mod price_fetcher;
pub mod trade_validation;
pub mod types;
pub mod zone_fetcher;
pub mod zone_proximity_analyzer;

pub use analyzer::TradeAnalyzer;
pub use config::*;
pub use csv_writer::CsvWriter;
pub use price_fetcher::PriceFetcher;
pub use trade_validation::BacktestTradeValidator;
pub use types::*;
pub use zone_fetcher::ZoneFetcher;
pub use zone_proximity_analyzer::ZoneProximityAnalyzer;