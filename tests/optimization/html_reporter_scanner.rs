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
        let pf_display = if res.profit_factor.is_infinite() { "inf".to_string() } else { format!("{:.2}", res.profit_factor) };
        writeln!(writer, "<tr {}><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            row_class, i + 1, res.symbol, res.timeframe,
            res.net_pips, res.win_rate_percent, pf_display, res.total_trades,
            res.winning_trades, res.losing_trades, res.average_trade_duration_str)?;
    }
    writeln!(writer, "</table>")?;

    // --- Table 2: Sorted by Win Rate ---
    let mut sorted_by_wr = scan_results.to_vec();
    sorted_by_wr.sort_by(|a, b| b.win_rate_percent.partial_cmp(&a.win_rate_percent)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.net_pips.partial_cmp(&a.net_pips).unwrap_or(std::cmp::Ordering::Equal))); // Secondary sort by pips
    writeln!(writer, "<h2>Top Performing Combinations by Win Rate</h2><table>")?;
    writeln!(writer, "<tr><th>Rank</th><th>Symbol</th><th>Pattern TF</th><th>Win Rate (%)</th><th>Net Pips</th><th>Profit Factor</th><th>Trades</th><th>Winners</th><th>Losers</th><th>Avg. Duration</th></tr>")?;
     for (i, res) in sorted_by_wr.iter().take(25).enumerate() { // Show top 25
        let row_class = if i == 0 && res.win_rate_percent > 0.0 { "class='best-row'" } else { "" }; // Highlight best only if WR > 0
        let pf_display = if res.profit_factor.is_infinite() { "inf".to_string() } else { format!("{:.2}", res.profit_factor) };
        writeln!(writer, "<tr {}><td>{}</td><td>{}</td><td>{}</td><td>{:.2}</td><td>{:.1}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            row_class, i + 1, res.symbol, res.timeframe,
            res.win_rate_percent, res.net_pips, pf_display, res.total_trades,
            res.winning_trades, res.losing_trades, res.average_trade_duration_str)?;
    }
    writeln!(writer, "</table>")?;
    
    // --- Table 3: Sorted by Profit Factor ---
    let mut sorted_by_pf = scan_results.to_vec();
    sorted_by_pf.sort_by(|a, b| {
        match (a.profit_factor.is_nan(), b.profit_factor.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater, // NaNs go last
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => b.profit_factor.partial_cmp(&a.profit_factor)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.net_pips.partial_cmp(&a.net_pips).unwrap_or(std::cmp::Ordering::Equal)), // Secondary sort by pips
        }
    });
    writeln!(writer, "<h2>Top Performing Combinations by Profit Factor</h2><table>")?;
    writeln!(writer, "<tr><th>Rank</th><th>Symbol</th><th>Pattern TF</th><th>Profit Factor</th><th>Net Pips</th><th>Win Rate (%)</th><th>Trades</th><th>Winners</th><th>Losers</th><th>Avg. Duration</th></tr>")?;
     for (i, res) in sorted_by_pf.iter().take(25).enumerate() { // Show top 25
        let row_class = if i == 0 && res.profit_factor > 0.0 { "class='best-row'" } else { "" }; // Highlight best only if PF > 0
        let pf_display = if res.profit_factor.is_infinite() { "inf".to_string() } else { format!("{:.2}", res.profit_factor) };
        writeln!(writer, "<tr {}><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            row_class, i + 1, res.symbol, res.timeframe,
            pf_display, res.net_pips, res.win_rate_percent, res.total_trades,
            res.winning_trades, res.losing_trades, res.average_trade_duration_str)?;
    }
    writeln!(writer, "</table>")?;

     // --- Strategy Portfolio Summary ---
    writeln!(writer, "<h2 class='section-header'>Strategy Portfolio Summary (Based on Profitable Combinations)</h2>")?;
    
    let best_by_pips_overall = scan_results.iter()
        .filter(|r| r.net_pips.is_finite())
        .max_by(|a, b| a.net_pips.partial_cmp(&b.net_pips).unwrap_or(std::cmp::Ordering::Equal));

    let profitable_combinations: Vec<&PerformanceScanResult> = scan_results
        .iter()
        .filter(|r| r.net_pips > 1e-9 && r.profit_factor > 1.0)
        .collect();

    if !profitable_combinations.is_empty() {
        writeln!(writer, "<p>The following Symbol/Timeframe combinations were profitable (Net Pips > 0, PF > 1.0) with SL {:.1} / TP {:.1} over the period:</p>", fixed_sl, fixed_tp)?;
        writeln!(writer, "<div class='sub-table'><table>")?;
        writeln!(writer, "<tr><th>Symbol</th><th>Pattern TF</th><th>Net Pips</th><th>Win Rate (%)</th><th>PF</th><th>Trades</th></tr>")?;
        
        let mut total_pips_from_profitable_portfolio: f64 = 0.0;
        let mut total_trades_from_profitable_portfolio: usize = 0;
        let mut total_wins_from_profitable_portfolio: usize = 0;
        // total_losses_from_profitable_portfolio is not directly used for PF calculation here
        // let mut total_losses_from_profitable_portfolio: usize = 0; // Not strictly needed for this summary
        let mut portfolio_gross_profit_sum: f64 = 0.0;
        let mut portfolio_gross_loss_sum: f64 = 0.0;

        for res in &profitable_combinations {
            total_pips_from_profitable_portfolio += res.net_pips;
            total_trades_from_profitable_portfolio += res.total_trades;
            total_wins_from_profitable_portfolio += res.winning_trades;
            // total_losses_from_profitable_portfolio += res.losing_trades;

            // Calculate gross profit and gross loss for each *profitable* combination
            // Gross Profit = Wins * TP_pips
            // Gross Loss = Losses * SL_pips
            // Net Pips = (Wins * TP_pips) - (Losses * SL_pips)
            // PF = (Wins * TP_pips) / (Losses * SL_pips)
            // If PF is inf, gross_loss is 0.
            // If PF is 0, gross_profit is 0.
            let wins_pips = res.winning_trades as f64 * fixed_tp;
            let losses_pips = res.losing_trades as f64 * fixed_sl;
            
            portfolio_gross_profit_sum += wins_pips;
            portfolio_gross_loss_sum += losses_pips;

            let pf_display = if res.profit_factor.is_infinite() { "inf".to_string() } else { format!("{:.2}", res.profit_factor) };
            writeln!(writer, "<tr><td>{}</td><td>{}</td><td>{:.1}</td><td>{:.2}</td><td>{}</td><td>{}</td></tr>",
                res.symbol, res.timeframe, res.net_pips, res.win_rate_percent, pf_display, res.total_trades)?;
        }
        writeln!(writer, "</table></div>")?;

        let portfolio_win_rate = if total_trades_from_profitable_portfolio > 0 {
            (total_wins_from_profitable_portfolio as f64 / total_trades_from_profitable_portfolio as f64) * 100.0
        } else { 0.0 };
        
        let portfolio_pf = if portfolio_gross_loss_sum > 1e-9 {
            portfolio_gross_profit_sum / portfolio_gross_loss_sum
        } else if portfolio_gross_profit_sum > 1e-9 {
            f64::INFINITY
        } else {
            0.0
        };

        writeln!(writer, "<h3>Portfolio Summary (Trading all profitable combinations above):</h3>")?;
        writeln!(writer, "<ul>")?;
        writeln!(writer, "<li><strong>Total Net Pips if all profitable combinations were traded: {:.1}</strong></li>", round_f64_report(total_pips_from_profitable_portfolio, 1))?;
        writeln!(writer, "<li>Total Trades from these combinations: {}</li>", total_trades_from_profitable_portfolio)?;
        writeln!(writer, "<li>Aggregate Win Rate: {:.2}%</li>", round_f64_report(portfolio_win_rate, 2))?;
        let portfolio_pf_display = if portfolio_pf.is_infinite() { "inf".to_string() } else { format!("{:.2}", round_f64_report(portfolio_pf, 2)) };
        writeln!(writer, "<li>Aggregate Profit Factor: {}</li>", portfolio_pf_display)?;
        writeln!(writer, "</ul>")?;

        if let Some(best) = best_by_pips_overall {
             writeln!(writer, "<p>The single best performing Symbol/Timeframe was: <strong>{} on {}</strong>, yielding {:.1} pips.</p>", best.symbol, best.timeframe, best.net_pips)?;
        }

    } else {
        writeln!(writer, "<p>No Symbol/Timeframe combinations were profitable with SL {:.1} / TP {:.1} over the period.</p>", fixed_sl, fixed_tp)?;
    }
    // --- End Strategy Portfolio Summary Section ---

    // --- NEW: Complete List of All Traded Combinations ---
    writeln!(writer, "<h2>Complete List of All Traded Combinations</h2>")?;
    let mut all_results_sorted = scan_results.to_vec();
    all_results_sorted.sort_by(|a, b| {
        a.symbol.cmp(&b.symbol)
         .then_with(|| a.timeframe.cmp(&b.timeframe))
    });

    let mut grand_total_net_pips: f64 = 0.0;
    let mut grand_total_trades: usize = 0;
    let mut grand_total_winners: usize = 0;
    let mut grand_total_gross_profit_pips: f64 = 0.0;
    let mut grand_total_gross_loss_pips: f64 = 0.0;


    writeln!(writer, "<table>")?;
    writeln!(writer, "<tr><th>#</th><th>Symbol</th><th>Pattern TF</th><th>Net Pips</th><th>Win Rate (%)</th><th>Profit Factor</th><th>Trades</th><th>Winners</th><th>Losers</th><th>Avg. Duration</th></tr>")?;
    for (idx, res) in all_results_sorted.iter().enumerate() {
        grand_total_net_pips += res.net_pips; // net_pips is already rounded in PerformanceScanResult
        grand_total_trades += res.total_trades;
        grand_total_winners += res.winning_trades;
        
        // Recalculate gross profit/loss for overall PF
        grand_total_gross_profit_pips += res.winning_trades as f64 * fixed_tp;
        grand_total_gross_loss_pips += res.losing_trades as f64 * fixed_sl;

        let pf_display = if res.profit_factor.is_infinite() { "inf".to_string() } else { format!("{:.2}", res.profit_factor) };
        writeln!(writer, "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            idx + 1, res.symbol, res.timeframe, res.net_pips, res.win_rate_percent, pf_display,
            res.total_trades, res.winning_trades, res.losing_trades, res.average_trade_duration_str)?;
    }
    writeln!(writer, "</table>")?;

    let overall_win_rate = if grand_total_trades > 0 {
        (grand_total_winners as f64 / grand_total_trades as f64) * 100.0
    } else {
        0.0
    };

    let overall_profit_factor = if grand_total_gross_loss_pips > 1e-9 {
        grand_total_gross_profit_pips / grand_total_gross_loss_pips
    } else if grand_total_gross_profit_pips > 1e-9 {
        f64::INFINITY
    } else {
        0.0
    };
    let overall_pf_display = if overall_profit_factor.is_infinite() { "inf".to_string() } else { format!("{:.2}", round_f64_report(overall_profit_factor, 2)) };


    writeln!(writer, "<div class='summary-box' style='margin-top: 20px; border-left-color: #27ae60;'>")?; // Using a different color for distinction
    writeln!(writer, "<h3>Overall Performance (All {} Traded Combinations):</h3>", scan_results.len())?;
    writeln!(writer, "<ul>")?;
    writeln!(writer, "<li><strong>Total Net Pips (if every combination above was traded): {:.1}</strong></li>", round_f64_report(grand_total_net_pips, 1))?;
    writeln!(writer, "<li>Total Trades: {}</li>", grand_total_trades)?;
    writeln!(writer, "<li>Overall Win Rate: {:.2}%</li>", round_f64_report(overall_win_rate, 2))?;
    writeln!(writer, "<li>Overall Profit Factor: {}</li>", overall_pf_display)?;
    writeln!(writer, "</ul>")?;
    writeln!(writer, "</div>")?;
    // --- End NEW Complete List Section ---


    writeln!(writer, "<div class='footer'><p>Report Generated: {} UTC</p></div>", Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true))?;
    writeln!(writer, "</body></html>")?;

    writer.flush()?;
    log::info!("[HTML_SCAN_REPORT] Performance Scan report generated: {}", report_filename);
    Ok(())
}