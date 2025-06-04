// csv_writer.rs
use crate::types::TradeResult;
use chrono::Utc;
use log::info;
use std::fs::File;

pub struct CsvWriter;

impl CsvWriter {
    pub fn new() -> Self {
        Self
    }

    pub async fn write_results(&self, trades: &[TradeResult]) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all("trades")?;
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("trades/trade_analysis_{}.csv", timestamp);
        
        info!("📝 Writing {} trades to CSV: {}", trades.len(), filename);
        
        let file = File::create(&filename)?;
        let mut writer = csv::Writer::from_writer(file);
        
        writer.write_record(&[
            "entry_time", 
            "symbol", 
            "timeframe", 
            "zone_id", 
            "action", 
            "entry_price", 
            "exit_time", 
            "exit_price", 
            "exit_reason", 
            "pnl_pips", 
            "duration_minutes", 
            "zone_strength"
        ])?;
        
        for trade in trades {
            let record = vec![
                trade.entry_time.to_rfc3339(),
                trade.symbol.clone(),
                trade.timeframe.clone(),
                trade.zone_id.clone(),
                trade.action.clone(),
                trade.entry_price.to_string(),
                trade.exit_time.map_or("".to_string(), |t| t.to_rfc3339()),
                trade.exit_price.map_or("".to_string(), |p| p.to_string()),
                trade.exit_reason.clone(),
                trade.pnl_pips.map_or("".to_string(), |p| format!("{:.1}", p)),
                trade.duration_minutes.map_or("".to_string(), |d| d.to_string()),
                format!("{:.1}", trade.zone_strength),
            ];
            writer.write_record(&record)?;
        }
        
        writer.flush()?;
        info!("✅ CSV file written successfully: {}", filename);
        println!("📄 Results saved to: {}", filename);
        Ok(())
    }

    pub fn print_summary(&self, _trades: &[TradeResult]) {
        info!("📊 Analysis summary printing not yet implemented");
    }
}