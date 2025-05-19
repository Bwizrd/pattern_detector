// tests/optimization/html_reporter_advanced.rs

use crate::advanced_optimizer::AdvancedOptimizationResult; // Assuming this is where the struct is
use chrono::{DateTime, Utc, Weekday};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Write, BufWriter}; // Use BufWriter for better performance

// Helper function for rounding (can be shared if you have a utils mod)
fn round_f64_report(value: f64, decimals: u32) -> f64 {
    if value.is_infinite() || value.is_nan() { return value; }
    let multiplier = 10f64.powi(decimals as i32);
    (value * multiplier).round() / multiplier
}

// Helper to format Weekday or Hour into a presentable string for table keys
fn format_map_key_to_string<T: ToString>(key: T) -> String {
    key.to_string()
}


pub fn generate_advanced_report(
    all_results: &[AdvancedOptimizationResult],
    report_identifier: &str, // e.g., "ALL_SYMBOLS_ADVANCED" or a specific symbol if filtered before calling
    start_dt_obj: DateTime<Utc>,
    end_dt_obj: DateTime<Utc>,
    total_days_in_period: i64,
    top_n_configs_to_detail: usize, // How many top configs to show day/hour details for
) -> Result<(), Box<dyn std::error::Error>> {
    if all_results.is_empty() {
        log::info!("No results provided to generate_advanced_report. Skipping.");
        return Ok(());
    }

    let date_str = Utc::now().format("%Y%m%d").to_string();
    let report_dir = format!("reports/{}", date_str);
    std::fs::create_dir_all(&report_dir)?;

    let report_filename = format!(
        "{}/adv_opt_report_{}_{}.html",
        report_dir,
        report_identifier.replace(" ", "_").to_lowercase(),
        Utc::now().format("%H%M%S")
    );
    let file = File::create(&report_filename)?;
    let mut writer = BufWriter::new(file);

    // --- HTML Header ---
    writeln!(writer, "<!DOCTYPE html><html><head><title>Advanced Optimization Report: {}</title>", report_identifier)?;
    writeln!(writer, "<style>")?;
    writeln!(writer, "body {{ font-family: Arial, sans-serif; margin: 20px; line-height: 1.6; }}")?;
    writeln!(writer, "h1, h2, h3 {{ color: #2c3e50; }}")?;
    writeln!(writer, "table {{ border-collapse: collapse; width: 100%; margin-bottom: 25px; box-shadow: 0 2px 3px rgba(0,0,0,0.1); }}")?;
    writeln!(writer, "th, td {{ border: 1px solid #ddd; padding: 10px 12px; text-align: left; }}")?;
    writeln!(writer, "th {{ background-color: #3498db; color: white; font-weight: bold; text-transform: uppercase; letter-spacing: 0.05em; }}")?;
    writeln!(writer, "tr:nth-child(even) {{ background-color: #f8f9f9; }}")?;
    writeln!(writer, "tr:hover {{ background-color: #eaf2f8; }}")?;
    writeln!(writer, ".summary-box {{ background-color: #ecf0f1; padding: 20px; border-radius: 5px; margin-bottom: 30px; border-left: 5px solid #3498db; }}")?;
    writeln!(writer, ".best-overall {{ background-color: #d4efdf; font-weight: bold; }}")?;
    writeln!(writer, ".config-details-section {{ margin-top: 20px; padding: 15px; border: 1px solid #bdc3c7; border-radius: 4px; background-color: #ffffff; }}")?;
    writeln!(writer, ".sub-table table {{ width: auto; margin-left: 20px; font-size: 0.9em; box-shadow: none; }}")?;
    writeln!(writer, ".sub-table th {{ background-color: #95a5a6; }}")?;
    writeln!(writer, "</style></head><body>")?;

    writeln!(writer, "<h1>Advanced Optimization Report: {}</h1>", report_identifier)?;
    writeln!(writer, "<div class='summary-box'>")?;
    writeln!(writer, "<h2>Overall Backtest Conditions</h2>")?;
    writeln!(writer, "<p><strong>Pattern:</strong> Fifty Percent Before Big Bar</p>")?; // Assuming this is fixed for now
    writeln!(writer, "<p><strong>Period:</strong> {} days (from {} to {})</p>", total_days_in_period, start_dt_obj.format("%Y-%m-%d"), end_dt_obj.format("%Y-%m-%d"))?;
    writeln!(writer, "<p><strong>Total Unique Configurations Tested (with >=5 trades):</strong> {}</p>", all_results.len())?;
    writeln!(writer, "</div>")?;

    // --- Section 1: Top N Configurations by Net Pips ---
    let mut sorted_by_pips = all_results.to_vec(); // Clone for sorting
    sorted_by_pips.sort_by(|a, b| b.net_pips.partial_cmp(&a.net_pips).unwrap_or(std::cmp::Ordering::Equal));
    
    writeln!(writer, "<h2>Top {} Configurations by Net Pips</h2>", top_n_configs_to_detail)?;
    generate_main_results_table(&mut writer, &sorted_by_pips, top_n_configs_to_detail, "pips")?;

    // --- Section 2: Top N Configurations by Profit Factor ---
    let mut sorted_by_pf = all_results.to_vec();
    sorted_by_pf.sort_by(|a, b| {
        // Handle NaN/Infinity for profit factor sorting (treat NaN as worse, Infinity as best)
        match (a.profit_factor.is_nan(), b.profit_factor.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater, // NaN is worse
            (false, true) => std::cmp::Ordering::Less,    // NaN is worse
            (false, false) => b.profit_factor.partial_cmp(&a.profit_factor).unwrap_or(std::cmp::Ordering::Equal),
        }
    });
    writeln!(writer, "<h2>Top {} Configurations by Profit Factor</h2>", top_n_configs_to_detail)?;
    generate_main_results_table(&mut writer, &sorted_by_pf, top_n_configs_to_detail, "pf")?;


    // --- Section 3: Detailed Day/Hour Breakdown for selected Top N configurations ---
    // We'll take the top N by Net Pips for this detailed breakdown, or you could choose others.
    writeln!(writer, "<h2>Detailed Performance Breakdown for Top Configurations (by Net Pips)</h2>")?;
    for (rank, result) in sorted_by_pips.iter().take(top_n_configs_to_detail).enumerate() {
        writeln!(writer, "<div class='config-details-section'>")?;
        writeln!(writer, "<h3>Rank {} (Pips): Symbol: {}, Timeframe: {}, SL: {:.1}, TP: {:.1}</h3>",
            rank + 1, result.symbol, result.timeframe, result.stop_loss_pips, result.take_profit_pips)?;
        writeln!(writer, "<p>Overall for this config: Net Pips: {:.1}, PF: {:.2}, Win Rate: {:.2}%, Trades: {}, Avg Duration: {}</p>",
            result.net_pips, result.profit_factor, result.win_rate, result.total_trades, result.average_trade_duration_str)?;

        // Day of Week Breakdown
        if !result.trades_by_day_of_week.is_empty() {
            writeln!(writer, "<h4>Performance by Day of Week:</h4>")?;
            writeln!(writer, "<div class='sub-table'><table><tr><th>Day</th><th>Trades</th><th>Net Pips</th></tr>")?;
            // Sort days for consistent display order
            let mut days: Vec<_> = result.trades_by_day_of_week.keys().collect();
            days.sort_by_key(|day_str| { // Custom sort for days of the week
                match day_str.as_str() {
                    "Mon" => 1, "Tue" => 2, "Wed" => 3, "Thu" => 4, "Fri" => 5, "Sat" => 6, "Sun" => 7,
                    _ => 8 // Unknown last
                }
            });
            for day_key_str in days {
                let trades = result.trades_by_day_of_week.get(day_key_str).unwrap_or(&0);
                let pips = result.pnl_by_day_of_week.get(day_key_str).unwrap_or(&0.0);
                writeln!(writer, "<tr><td>{}</td><td>{}</td><td>{:.1}</td></tr>", day_key_str, trades, pips)?;
            }
            writeln!(writer, "</table></div>")?;
        }

        // Hour of Day Breakdown
        if !result.trades_by_hour_of_day.is_empty() {
            writeln!(writer, "<h4>Performance by Hour of Day (UTC):</h4>")?;
            writeln!(writer, "<div class='sub-table'><table><tr><th>Hour</th><th>Trades</th><th>Net Pips</th></tr>")?;
            let mut hours: Vec<_> = result.trades_by_hour_of_day.keys().cloned().collect();
            hours.sort(); // Sort numerically
            for hour_key in hours {
                let trades = result.trades_by_hour_of_day.get(&hour_key).unwrap_or(&0);
                let pips = result.pnl_by_hour_of_day.get(&hour_key).unwrap_or(&0.0);
                writeln!(writer, "<tr><td>{:02}:00</td><td>{}</td><td>{:.1}</td></tr>", hour_key, trades, pips)?;
            }
            writeln!(writer, "</table></div>")?;
        }
        writeln!(writer, "</div>")?; // End config-details-section
    }


    // --- HTML Footer ---
    writeln!(writer, "<hr><p>Report Generated: {} UTC</p>", Utc::now().to_rfc3339())?;
    writeln!(writer, "</body></html>")?;

    writer.flush()?;
    log::info!("Advanced optimization report generated: {}", report_filename);
    Ok(())
}


// Helper function to generate the main summary tables
fn generate_main_results_table(
    writer: &mut BufWriter<File>,
    results_slice: &[AdvancedOptimizationResult],
    top_n: usize,
    best_metric_id: &str, // Just for CSS class
) -> std::io::Result<()> {
    writeln!(writer, "<table>")?;
    writeln!(writer, "<tr><th>Rank</th><th>Symbol</th><th>Pattern TF</th><th>SL (pips)</th><th>TP (pips)</th><th>Net Pips</th><th>Profit Factor</th><th>Win Rate (%)</th><th>Trades</th><th>Avg. Duration</th></tr>")?;

    for (i, result) in results_slice.iter().take(top_n).enumerate() {
        let row_class = if i == 0 { format!("class='best-overall best-{}'", best_metric_id) } else { "".to_string() };
        writeln!(writer, "<tr {}>", row_class)?;
        writeln!(writer, "<td>{}</td>", i + 1)?;
        writeln!(writer, "<td>{}</td>", result.symbol)?;
        writeln!(writer, "<td>{}</td>", result.timeframe)?;
        writeln!(writer, "<td>{:.1}</td>", result.stop_loss_pips)?;
        writeln!(writer, "<td>{:.1}</td>", result.take_profit_pips)?;
        writeln!(writer, "<td>{:.1}</td>", result.net_pips)?;
        writeln!(writer, "<td>{:.2}</td>", result.profit_factor)?; // Profit factor to 2 decimals
        writeln!(writer, "<td>{:.2}</td>", result.win_rate)?;    // Win rate to 2 decimals
        writeln!(writer, "<td>{}</td>", result.total_trades)?;
        writeln!(writer, "<td>{}</td>", result.average_trade_duration_str)?;
        writeln!(writer, "</tr>")?;
    }
    writeln!(writer, "</table>")?;
    Ok(())
}