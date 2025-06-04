// main.rs
mod analyzer;
mod config;
mod csv_writer;
mod price_fetcher;
mod types;
mod zone_fetcher;

use analyzer::TradeAnalyzer;
use chrono::Utc;
use clap::Parser;
use config::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();
    
    let args = Args::parse();
    setup_logging(args.debug);
    
    let start_time = parse_time_or_default(args.start_time, || {
        let now = Utc::now();
        now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
    });
    
    let end_time = parse_time_or_default(args.end_time, || Utc::now());
    
    let config = AnalysisConfig {
        start_time,
        end_time,
        app_url: args.app_url,
        output_file: args.output,
        min_timeframe: args.min_timeframe,
        debug: args.debug,
    };
    
    let mut analyzer = TradeAnalyzer::new(config);
    analyzer.run().await?;
    
    Ok(())
}