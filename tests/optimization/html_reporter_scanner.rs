// tests/optimization/html_reporter_scanner.rs

use super::performance_scanner::PerformanceScanResult; // Ensure this path is correct
use chrono::{DateTime, Utc};
use std::fs::File;
use std::io::{Write, BufWriter};
use log; // For logging

// Helper function for rounding (can be shared if you have a utils mod)
fn round_f64_report(value: f64, decimals: u32) -> f64 {
    if value.is_infinite() || value.is_nan() {
        return value; // Return special numbers as is
    }
    let multiplier = 10f64.powi(decimals as i32);
    (value * multiplier).round() / multiplier
}

pub fn generate_performance_scan_report(
    scan_results: &[PerformanceScanResult],
    report_identifier: &str, // e.g., "LastWeek_SL10_TP50"
    start_dt_obj: DateTime<Utc>,
    end_dt_obj: DateTime<Utc>,
    total_days_in_period: i64,
    fixed_sl: f64, // To display in the report header
    fixed_tp: f64, // To display in the report header
) -> Result<(), Box<dyn std::error::Error>> {
    if scan_results.is_empty() {
        log::info!("[HTML_SCAN_REPORT] No results provided. Skipping report generation for '{}'.", report_identifier);
        return Ok(());
    }

    let date_str = Utc::now().format("%Y%m%d").to_string();
    let report_dir = format!("reports/{}", date_str);
    std::fs::create_dir_all(&report_dir)?;

    let report_filename = format!(
        "{}/perf_scan_{}_{}.html",
        report_dir,
        report_identifier.replace(' ', "_").to_lowercase(),
        Utc::now().format("%H%M%S")
    );
    let file = File::create(&report_filename)?;
    let mut writer = BufWriter::new(file);

    // --- HTML Header & CSS ---
    writeln!(writer, "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\"><title>Performance Scan Report: {}</title>", report_identifier)?;
    writeln!(writer, "<style>")?;
    writeln!(writer, "body {{ font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif; margin: 20px; line-height: 1.6; color: #333; background-color: #f4f7f6; }}")?;
    writeln!(writer, "h1, h2 {{ color: #2c3e50; margin-top: 1.5em; margin-bottom: 0.8em; border-bottom: 1px solid #bdc3c7; padding-bottom: 0.3em; }}")?;
    writeln!(writer, "h1 {{ text-align: center; border-bottom-width: 2px; border-bottom-color: #3498db; }}")?;
    writeln!(writer, "table {{ border-collapse: collapse; width: 100%; margin-bottom: 30px; box-shadow: 0 4px 8px rgba(0,0,0,0.1); background-color: white; }}")?;
    writeln!(writer, "th, td {{ border: 1px solid #e0e0e0; padding: 10px 14px; text-align: left; vertical-align: top; }}")?;
    writeln!(writer, "th {{ background-color: #3498db; color: white; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; }}")?;
    writeln!(writer, "tr:nth-child(even) {{ background-color: #fdfdfd; }}")?;
    writeln!(writer, "tr:hover {{ background-color: #eef5fa; }}")?;
    writeln!(writer, ".summary-box {{ background-color: #e9f5ff; padding: 20px; border-radius: 8px; margin-bottom: 30px; border-left: 5px solid #3498db; box-shadow: 0 2px 4px rgba(0,0,0,0.05); }}")?;
    writeln!(writer, ".summary-box p {{ margin: 0.5em 0; }}")?;
    writeln!(writer, ".best-row {{ background-color: #d5f5e3 !important; font-weight: bold; }}")?;
    writeln!(writer, ".footer {{ text-align: center; margin-top: 40px; padding-top: 20px; border-top: 1px solid #ccc; font-size: 0.9em; color: #777; }}")?;
    writeln!(writer, "</style></head><body>")?;

    writeln!(writer, "<h1>Performance Scan Report: {}</h1>", report_identifier)?;
    writeln!(writer, "<div class='summary-box'>")?;
    writeln!(writer, "<h2>Scan Conditions</h2>")?;
    writeln!(writer, "<p><strong>Pattern:</strong> Fifty Percent Before Big Bar</p>")?; // Assuming fixed pattern
    writeln!(writer, "<p><strong>Period:</strong> {} days (from {} to {})</p>", total_days_in_period, start_dt_obj.format("%Y-%m-%d"), end_dt_obj.format("%Y-%m-%d"))?;
    writeln!(writer, "<p><strong>Fixed Stop Loss:</strong> {:.1} pips</p>", fixed_sl)?;
    writeln!(writer, "<p><strong>Fixed Take Profit:</strong> {:.1} pips</p>", fixed_tp)?;
    let tested_timeframes_str = scan_results.iter()
        .map(|r| r.timeframe.clone())
        .collect::<std::collections::HashSet<String>>() // Get unique timeframes
        .into_iter().collect::<Vec<String>>().join(", ");
    writeln!(writer, "<p><strong>Pattern Timeframes Scanned:</strong> {}</p>", if tested_timeframes_str.is_empty() { "None" } else { &tested_timeframes_str } )?;
    writeln!(writer, "<p><strong>Total Symbol/Timeframe Combinations with Trades:</strong> {}</p>", scan_results.len())?;
    writeln!(writer, "</div>")?;

    // --- Table 1: Sorted by Net Pips ---
    let mut sorted_by_pips = scan_results.to_vec();
    sorted_by_pips.sort_by(|a, b| b.net_pips.partial_cmp(&a.net_pips).unwrap_or(std::cmp::Ordering::Equal));
    
    writeln!(writer, "<h2>Top Performing Combinations by Net Pips</h2><table>")?;
    writeln!(writer, "<tr><th>Rank</th><th>Symbol</th><th>Pattern TF</th><th>Net Pips</th><th>Win Rate (%)</th><th>Profit Factor</th><th>Trades</th><th>Winners</th><th>Losers</th><th>Avg. Duration</th></tr>")?;
    for (i, res) in sorted_by_pips.iter().take(25).enumerate() { // Show top 25 or all if fewer
        let row_class = if i == 0 { "class='best-row'" } else { "" };
        writeln!(writer, "<tr {}><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{:.2}</td><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            row_class, i + 1, res.symbol, res.timeframe,
            res.net_pips, res.win_rate_percent, res.profit_factor, res.total_trades,
            res.winning_trades, res.losing_trades, res.average_trade_duration_str)?;
    }
    writeln!(writer, "</table>")?;

    // --- Table 2: Sorted by Win Rate ---
    let mut sorted_by_wr = scan_results.to_vec();
    sorted_by_wr.sort_by(|a, b| b.win_rate_percent.partial_cmp(&a.win_rate_percent).unwrap_or(std::cmp::Ordering::Equal));
    writeln!(writer, "<h2>Top Performing Combinations by Win Rate</h2><table>")?;
    writeln!(writer, "<tr><th>Rank</th><th>Symbol</th><th>Pattern TF</th><th>Win Rate (%)</th><th>Net Pips</th><th>Profit Factor</th><th>Trades</th><th>Winners</th><th>Losers</th><th>Avg. Duration</th></tr>")?;
     for (i, res) in sorted_by_wr.iter().take(25).enumerate() { // Show top 25
        let row_class = if i == 0 { "class='best-row'" } else { "" };
        writeln!(writer, "<tr {}><td>{}</td><td>{}</td><td>{}</td><td>{:.2}</td><td>{:.1}</td><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            row_class, i + 1, res.symbol, res.timeframe,
            res.win_rate_percent, res.net_pips, res.profit_factor, res.total_trades,
            res.winning_trades, res.losing_trades, res.average_trade_duration_str)?;
    }
    writeln!(writer, "</table>")?;
    
    // --- Table 3: Sorted by Profit Factor ---
    let mut sorted_by_pf = scan_results.to_vec();
    sorted_by_pf.sort_by(|a, b| {
        match (a.profit_factor.is_nan(), b.profit_factor.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal, (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => b.profit_factor.partial_cmp(&a.profit_factor).unwrap_or(std::cmp::Ordering::Equal),
        }
    });
    writeln!(writer, "<h2>Top Performing Combinations by Profit Factor</h2><table>")?;
    writeln!(writer, "<tr><th>Rank</th><th>Symbol</th><th>Pattern TF</th><th>Profit Factor</th><th>Net Pips</th><th>Win Rate (%)</th><th>Trades</th><th>Winners</th><th>Losers</th><th>Avg. Duration</th></tr>")?;
     for (i, res) in sorted_by_pf.iter().take(25).enumerate() { // Show top 25
        let row_class = if i == 0 { "class='best-row'" } else { "" };
        // Handle display of infinity for profit factor
        let pf_display = if res.profit_factor.is_infinite() { "inf".to_string() } else { format!("{:.2}", res.profit_factor) };
        writeln!(writer, "<tr {}><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            row_class, i + 1, res.symbol, res.timeframe,
            pf_display, res.net_pips, res.win_rate_percent, res.total_trades,
            res.winning_trades, res.losing_trades, res.average_trade_duration_str)?;
    }
    writeln!(writer, "</table>")?;

     // --- NEW: Overall Summary of Top Performers ---
    writeln!(writer, "<h2 class='section-header'>Strategy Portfolio Summary (Last Week)</h2>")?;
    
    // Find the single best result (Symbol/TF) by Net Pips
    let best_by_pips_overall = scan_results.iter()
        .filter(|r| r.net_pips.is_finite()) // Exclude NaN/Infinity pips
        .max_by(|a, b| a.net_pips.partial_cmp(&b.net_pips).unwrap_or(std::cmp::Ordering::Equal));

    // Collect all Symbol/Timeframe combinations that were profitable (Net Pips > 0)
    let profitable_combinations: Vec<&PerformanceScanResult> = scan_results
        .iter()
        .filter(|r| r.net_pips > 1e-9 && r.profit_factor > 1.0) // Consider PF > 1 as well
        .collect();

    let mut total_pips_from_profitable_portfolio: f64 = 0.0;
    let mut total_trades_from_profitable_portfolio: usize = 0;
    let mut total_wins_from_profitable_portfolio: usize = 0;
    let mut total_losses_from_profitable_portfolio: usize = 0;

    if !profitable_combinations.is_empty() {
        writeln!(writer, "<p>The following Symbol/Timeframe combinations were profitable (Net Pips > 0, PF > 1.0) with SL {:.1} / TP {:.1} over the period:</p>", fixed_sl, fixed_tp)?;
        writeln!(writer, "<div class='sub-table'><table>")?;
        writeln!(writer, "<tr><th>Symbol</th><th>Pattern TF</th><th>Net Pips</th><th>Win Rate (%)</th><th>PF</th><th>Trades</th></tr>")?;
        for res in &profitable_combinations {
            total_pips_from_profitable_portfolio += res.net_pips; // Summing already rounded net_pips
            total_trades_from_profitable_portfolio += res.total_trades;
            total_wins_from_profitable_portfolio += res.winning_trades;
            total_losses_from_profitable_portfolio += res.losing_trades;
            writeln!(writer, "<tr><td>{}</td><td>{}</td><td>{:.1}</td><td>{:.2}</td><td>{:.2}</td><td>{}</td></tr>",
                res.symbol, res.timeframe, res.net_pips, res.win_rate_percent, res.profit_factor, res.total_trades)?;
        }
        writeln!(writer, "</table></div>")?;

        let portfolio_win_rate = if total_trades_from_profitable_portfolio > 0 {
            (total_wins_from_profitable_portfolio as f64 / total_trades_from_profitable_portfolio as f64) * 100.0
        } else { 0.0 };
        
        let portfolio_gross_profit: f64 = profitable_combinations.iter().map(|r| {
            // Estimate gross profit: NetPips + (Losses * SL) or (Wins * TP / PF) - (Losses * SL)
            // Simpler: If WinRate > 0, (NetPips / (WR * TP - (1-WR)*SL)) * WR * TP
            // For now, let's use a simpler approximation or just sum net pips.
            // To calculate true portfolio PF, we'd need gross profit and gross loss sums.
            // For simplicity in this summary, we'll report summed net pips and an average PF.
            if r.profit_factor.is_finite() && r.profit_factor > 0.0 {
                 r.net_pips * r.profit_factor / (r.profit_factor - 1.0) // Approximate gross profit if PF > 1
            } else if r.net_pips > 0.0 && r.losing_trades == 0 { // All wins
                r.net_pips
            } else {
                0.0
            }
        }).sum();

        let portfolio_gross_loss: f64 = profitable_combinations.iter().map(|r| {
             if r.profit_factor.is_finite() && r.profit_factor > 0.0 && r.profit_factor != 1.0 {
                 r.net_pips / (r.profit_factor - 1.0) // Approximate gross loss if PF > 1 and PF != 1
            } else if r.net_pips > 0.0 && r.losing_trades == 0 {
                0.0
            } else if r.net_pips < 0.0 && r.winning_trades == 0 { // All losses
                r.net_pips.abs()
            }
             else { // Difficult to estimate if PF is 0 or 1 or inf without raw sums
                if r.winning_trades > 0 && r.losing_trades > 0 {
                    (r.winning_trades as f64 * fixed_tp) / r.profit_factor // Very rough
                } else {
                    0.0
                }
            }
        }).sum();


        let portfolio_pf = if portfolio_gross_loss > 1e-9 {
            portfolio_gross_profit / portfolio_gross_loss
        } else if portfolio_gross_profit > 1e-9 {
            f64::INFINITY
        } else {
            0.0
        };


        writeln!(writer, "<h3>Portfolio Summary (Trading all profitable combinations above):</h3>")?;
        writeln!(writer, "<ul>")?;
        writeln!(writer, "<li><strong>Total Net Pips if all profitable combinations were traded: {:.1}</strong></li>", round_f64_report(total_pips_from_profitable_portfolio, 1))?;
        writeln!(writer, "<li>Total Trades from these combinations: {}</li>", total_trades_from_profitable_portfolio)?;
        writeln!(writer, "<li>Aggregate Win Rate: {:.2}%</li>", round_f64_report(portfolio_win_rate, 2))?;
        writeln!(writer, "<li>Approximate Aggregate Profit Factor: {:.2}</li>", round_f64_report(portfolio_pf, 2))?;
        writeln!(writer, "</ul>")?;

        if let Some(best) = best_by_pips_overall {
             writeln!(writer, "<p>The single best performing Symbol/Timeframe was: <strong>{} on {}</strong>, yielding {:.1} pips.</p>", best.symbol, best.timeframe, best.net_pips)?;
        }

    } else {
        writeln!(writer, "<p>No Symbol/Timeframe combinations were profitable with SL {:.1} / TP {:.1} over the period.</p>", fixed_sl, fixed_tp)?;
    }
    // --- End New Summary Section ---


    writeln!(writer, "<div class='footer'><p>Report Generated: {} UTC</p></div>", Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true))?;
    writeln!(writer, "</body></html>")?;

    writer.flush()?;
    log::info!("[HTML_SCAN_REPORT] Performance Scan report generated: {}", report_filename);
    Ok(())
}