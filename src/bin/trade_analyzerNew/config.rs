// config.rs
use chrono::{DateTime, Utc};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "trade-analyzer")]
#[command(about = "Analyze what trades would have been booked for a given period")]
pub struct Args {
    /// Start time for analysis (RFC3339 format, default: today 00:00 UTC)
    #[arg(short, long)]
    pub start_time: Option<String>,
    
    /// End time for analysis (RFC3339 format, default: now)
    #[arg(short, long)]
    pub end_time: Option<String>,
    
    /// Main application URL
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    pub app_url: String,
    
    /// Output CSV file path
    #[arg(short, long, default_value = "analyzed_trades.csv")]
    pub output: String,
    
    /// Minimum timeframe to analyze (default: 30m)
    #[arg(long, default_value = "30m")]
    pub min_timeframe: String,
    
    /// Enable debug logging
    #[arg(short, long)]
    pub debug: bool,
}

#[derive(Debug)]
pub struct AnalysisConfig {
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub app_url: String,
    pub output_file: String,
    pub min_timeframe: String,
    pub debug: bool,
}

pub fn parse_time_or_default(time_str: Option<String>, default_fn: fn() -> DateTime<Utc>) -> DateTime<Utc> {
    match time_str {
        Some(s) => match DateTime::parse_from_rfc3339(&s) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(e) => {
                eprintln!("Warning: Invalid time format '{}': {}. Using default.", s, e);
                default_fn()
            }
        },
        None => default_fn(),
    }
}

pub fn setup_logging(debug: bool) {
    use env_logger::{Builder, Target};
    use log::LevelFilter;
    
    let mut builder = Builder::from_default_env();
    builder.target(Target::Stdout);
    
    if debug {
        builder.filter_level(LevelFilter::Debug);
    } else {
        builder.filter_level(LevelFilter::Info);
    }
    
    builder.init();
}