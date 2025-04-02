use chrono::{DateTime, Utc};
use std::collections::HashMap;
/// tests/optimization/html_reporter.rs
use std::fs::File;
use std::io::Write;

use crate::{OptimizationResult, ParameterSearchConfig};

pub fn generate_report(
    results: &[OptimizationResult],
    config: &ParameterSearchConfig,
    symbol: &str,  
    start_datetime: DateTime<Utc>,
    end_datetime: DateTime<Utc>,
    total_days: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    if results.is_empty() {
        println!("No results to report for symbol: {}", symbol);
        return Ok(());
    }

    // Create copies for sorting
    let mut pf_sorted = results.to_vec();
    let mut pips_sorted = results.to_vec();
    let mut winrate_sorted = results.to_vec();

    // Sort by profit factor, net pips, and win rate
    pf_sorted.sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap());
    pips_sorted.sort_by(|a, b| b.net_pips.partial_cmp(&a.net_pips).unwrap());
    winrate_sorted.sort_by(|a, b| b.win_rate.partial_cmp(&a.win_rate).unwrap());

    // Create CSV report
    let date_str = chrono::Utc::now().format("%Y%m%d").to_string();
    let report_dir = format!("reports/{}", date_str);
    std::fs::create_dir_all(&report_dir)?;

    // Create CSV report
    let csv_filename = format!(
        "{}/optimization_results_{}_{}.csv", // <<< ADDED _{}_
        report_dir,
        symbol.replace("_SB", ""), // Add symbol (optionally remove suffix)
        chrono::Utc::now().format("%H%M%S")
    );
    let mut csv_file = File::create(&csv_filename)?;

    // Write CSV header
    writeln!(csv_file, "Symbol,Timeframe,Lot Size,TP Pips,SL Pips,Win Rate,Profit Factor,Total P/L,Total Trades,Net Pips")?;

    // Write all results to CSV
    for result in results {
        writeln!(
            csv_file,
            "{},{},{:.2},{:.1},{:.1},{:.2}%,{:.2},{:.2},{},{}",
            result.symbol,
            result.timeframe,
            result.lot_size,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.total_profit_loss,
            result.total_trades,
            result.net_pips
        )?;
    }

    let date_str = chrono::Utc::now().format("%Y%m%d").to_string();
    let report_dir = format!("reports/{}", date_str);
    std::fs::create_dir_all(&report_dir)?;

    let summary_filename = format!(
        "{}/optimization_report_{}_{}.html", // <<< ADDED _{}_
        report_dir,
        symbol.replace("_SB", ""), // Add symbol (optionally remove suffix)
        chrono::Utc::now().format("%Y%m%d_%H%M%S")
    );
    let mut html_file = File::create(&summary_filename)?;

    // Write HTML header with styles
    write_html_header(
        &mut html_file,
        &config.symbol,
        total_days,
        &start_datetime,
        &end_datetime,
        results.len(),
    )?;

    // Write top results by profit factor
    write_profit_factor_table(&mut html_file, &pf_sorted)?;

    // Write top results by net pips
    write_net_pips_table(&mut html_file, &pips_sorted)?;

    // Write top results by win rate
    write_win_rate_table(&mut html_file, &winrate_sorted)?;

    // Write timeframe-specific results
    write_timeframe_results(&mut html_file, results)?;

    // Write conclusion
    write_conclusion(
        &mut html_file,
        results,
        &pf_sorted,
        &pips_sorted,
        &winrate_sorted,
    )?;

    // Close HTML
    writeln!(
        html_file,
        r#"
    <p>Generated on: {}</p>
</body>
</html>"#,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
    )?;

    // Print summary to console
    print_console_summary(
        &pf_sorted,
        &pips_sorted,
        &winrate_sorted,
        &csv_filename,
        &summary_filename,
        symbol,
    );

    Ok(())
}

fn write_html_header(
    file: &mut File,
    symbol: &str,
    total_days: i64,
    start_datetime: &DateTime<Utc>,
    end_datetime: &DateTime<Utc>,
    total_results: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(
        file,
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Forex Trading Strategy Optimization Report</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        h1, h2, h3 {{ color: #333366; }}
        table {{ border-collapse: collapse; width: 100%; margin-bottom: 20px; }}
        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
        th {{ background-color: #f2f2f2; }}
        tr:nth-child(even) {{ background-color: #f9f9f9; }}
        tr:hover {{ background-color: #f5f5f5; }}
        .summary {{ background-color: #e6f7ff; padding: 15px; border-radius: 5px; margin-bottom: 20px; }}
        .best {{ background-color: #e6ffe6; }}
        .approach-comparison {{ display: flex; }}
        .approach-card {{ flex: 1; margin: 10px; padding: 15px; border: 1px solid #ddd; border-radius: 5px; }}
        .approach-card h3 {{ margin-top: 0; }}
        .approach-card.pf {{ background-color: #e6f7ff; }}
        .approach-card.pips {{ background-color: #e6ffe6; }}
    </style>
</head>
<body>
    <h1>Forex Trading Strategy Optimization Report</h1>
    <div class="summary">
        <h2>Backtest Summary</h2>
        <p><strong>Pattern:</strong> Fifty Percent Before Big Bar</p>
        <p><strong>Symbol:</strong> {}</p>
        <p><strong>Period:</strong> {} days (from {} to {})</p>
        <p><strong>Total Combinations Tested:</strong> {}</p>
    </div>"#,
        symbol,
        total_days,
        start_datetime.format("%Y-%m-%d"),
        end_datetime.format("%Y-%m-%d"),
        total_results
    )?;

    Ok(())
}

fn write_profit_factor_table(
    file: &mut File,
    results: &[OptimizationResult],
) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(
        file,
        r#"
    <h2>Top 10 Results by Profit Factor</h2>
    <p>Optimizing for risk-adjusted returns (consistency and risk management)</p>
    <table>
        <tr>
            <th>Rank</th>
            <th>Timeframe</th>
            <th>TP (pips)</th>
            <th>SL (pips)</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
            <th>Trades</th>
        </tr>"#
    )?;

    // Get top 10 or all if less than 10
    let top_count = std::cmp::min(10, results.len());
    for i in 0..top_count {
        let result = &results[i];
        writeln!(
            file,
            r#"
        <tr{}>
            <td>{}</td>
            <td>{}</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>"#,
            if i == 0 { " class=\"best\"" } else { "" },
            i + 1,
            result.timeframe,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.net_pips,
            result.total_trades
        )?;
    }

    writeln!(file, "    </table>")?;

    Ok(())
}

fn write_net_pips_table(
    file: &mut File,
    results: &[OptimizationResult],
) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(
        file,
        r#"
    <h2>Top 10 Results by Net Pips</h2>
    <p>Optimizing for total profit (potentially higher returns with higher risk)</p>
    <table>
        <tr>
            <th>Rank</th>
            <th>Timeframe</th>
            <th>TP (pips)</th>
            <th>SL (pips)</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
            <th>Trades</th>
        </tr>"#
    )?;

    // Get top 10 by pips or all if less than 10
    let top_count = std::cmp::min(10, results.len());
    for i in 0..top_count {
        let result = &results[i];
        writeln!(
            file,
            r#"
        <tr{}>
            <td>{}</td>
            <td>{}</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>"#,
            if i == 0 { " class=\"best\"" } else { "" },
            i + 1,
            result.timeframe,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.net_pips,
            result.total_trades
        )?;
    }

    writeln!(file, "    </table>")?;

    Ok(())
}

fn write_win_rate_table(
    file: &mut File,
    results: &[OptimizationResult],
) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(
        file,
        r#"
    <h2>Top 10 Results by Win Rate</h2>
    <p>Optimizing for consistency and psychological comfort (higher percentage of winning trades)</p>
    <table>
        <tr>
            <th>Rank</th>
            <th>Timeframe</th>
            <th>TP (pips)</th>
            <th>SL (pips)</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
            <th>Trades</th>
        </tr>"#
    )?;

    // Get top 10 by win rate or all if less than 10
    let top_count = std::cmp::min(10, results.len());
    for i in 0..top_count {
        let result = &results[i];
        writeln!(
            file,
            r#"
        <tr{}>
            <td>{}</td>
            <td>{}</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>"#,
            if i == 0 { " class=\"best\"" } else { "" },
            i + 1,
            result.timeframe,
            result.take_profit_pips,
            result.stop_loss_pips,
            result.win_rate,
            result.profit_factor,
            result.net_pips,
            result.total_trades
        )?;
    }

    writeln!(file, "    </table>")?;

    Ok(())
}

fn write_timeframe_results(
    file: &mut File,
    results: &[OptimizationResult],
) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(
        file,
        r#"
    <h2>Results by Timeframe</h2>"#
    )?;

    // Group by timeframe
    let mut timeframe_groups = HashMap::new();
    for result in results {
        let key = result.timeframe.clone();
        timeframe_groups
            .entry(key)
            .or_insert_with(Vec::new)
            .push(result.clone());
    }

    // For each timeframe, show best result by PF, pips, and win rate
    for (timeframe, mut group) in timeframe_groups {
        if group.is_empty() {
            continue;
        }

        // Sort by profit factor
        group.sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap());
        let best_pf = &group[0];

        // Sort by net pips
        let mut pips_group = group.clone();
        pips_group.sort_by(|a, b| b.net_pips.partial_cmp(&a.net_pips).unwrap());
        let best_pips = &pips_group[0];

        // Sort by win rate
        let mut winrate_group = group.clone();
        winrate_group.sort_by(|a, b| b.win_rate.partial_cmp(&a.win_rate).unwrap());
        let best_winrate = &winrate_group[0];

        writeln!(
            file,
            r#"
    <h3>Timeframe: {}</h3>
    <table>
        <tr>
            <th>Optimization</th>
            <th>TP (pips)</th>
            <th>SL (pips)</th>
            <th>Win Rate</th>
            <th>Profit Factor</th>
            <th>Net Pips</th>
            <th>Trades</th>
        </tr>
        <tr class="best">
            <td>Best Profit Factor</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>
        <tr>
            <td>Best Net Pips</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>
        <tr>
            <td>Best Win Rate</td>
            <td>{:.1}</td>
            <td>{:.1}</td>
            <td>{:.2}%</td>
            <td>{:.2}</td>
            <td>{:.1}</td>
            <td>{}</td>
        </tr>
    </table>"#,
            timeframe,
            best_pf.take_profit_pips,
            best_pf.stop_loss_pips,
            best_pf.win_rate,
            best_pf.profit_factor,
            best_pf.net_pips,
            best_pf.total_trades,
            best_pips.take_profit_pips,
            best_pips.stop_loss_pips,
            best_pips.win_rate,
            best_pips.profit_factor,
            best_pips.net_pips,
            best_pips.total_trades,
            best_winrate.take_profit_pips,
            best_winrate.stop_loss_pips,
            best_winrate.win_rate,
            best_winrate.profit_factor,
            best_winrate.net_pips,
            best_winrate.total_trades
        )?;
    }

    Ok(())
}

fn write_conclusion(
    file: &mut File,
    results: &[OptimizationResult],
    pf_sorted: &[OptimizationResult],
    pips_sorted: &[OptimizationResult],
    winrate_sorted: &[OptimizationResult],
) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(
        file,
        r#"
    <h2>Conclusion</h2>
    <p>The optimization process tested {} different parameter combinations across various timeframes.</p>"#,
        results.len()
    )?;

    if !pf_sorted.is_empty() && !pips_sorted.is_empty() && !winrate_sorted.is_empty() {
        writeln!(
            file,
            r#"
    <h3>Optimization Approaches Comparison</h3>
    <div class="approach-comparison">
        <div class="approach-card pf">
            <h3>Profit Factor Optimization</h3>
            <p><strong>Timeframe:</strong> {}</p>
            <p><strong>TP/SL:</strong> {:.1}/{:.1} pips (ratio: {:.1})</p>
            <p><strong>Win Rate:</strong> {:.2}%</p>
            <p><strong>Profit Factor:</strong> {:.2}</p>
            <p><strong>Net Pips:</strong> {:.1}</p>
            <p><strong>Trades:</strong> {}</p>
            <p><em>More consistent returns with better risk management. Typically has lower drawdowns.</em></p>
        </div>
        <div class="approach-card pips">
            <h3>Net Pips Optimization</h3>
            <p><strong>Timeframe:</strong> {}</p>
            <p><strong>TP/SL:</strong> {:.1}/{:.1} pips (ratio: {:.1})</p>
            <p><strong>Win Rate:</strong> {:.2}%</p>
            <p><strong>Profit Factor:</strong> {:.2}</p>
            <p><strong>Net Pips:</strong> {:.1}</p>
            <p><strong>Trades:</strong> {}</p>
            <p><em>Higher total profit but potentially higher risk and larger drawdowns.</em></p>
        </div>
        <div class="approach-card winrate">
            <h3>Win Rate Optimization</h3>
            <p><strong>Timeframe:</strong> {}</p>
            <p><strong>TP/SL:</strong> {:.1}/{:.1} pips (ratio: {:.1})</p>
            <p><strong>Win Rate:</strong> {:.2}%</p>
            <p><strong>Profit Factor:</strong> {:.2}</p>
            <p><strong>Net Pips:</strong> {:.1}</p>
            <p><strong>Trades:</strong> {}</p>
            <p><em>Psychologically easier to trade with more winning trades, but may sacrifice overall profitability.</em></p>
        </div>
    </div>
    
    <h3>Making Your Decision</h3>
    <p>When choosing between optimization approaches, consider:</p>
    <ul>
        <li><strong>Risk Tolerance:</strong> Lower risk tolerance favors the profit factor approach</li>
        <li><strong>Account Size:</strong> Smaller accounts may benefit from consistency (profit factor)</li>
        <li><strong>Trading Psychology:</strong> If drawdowns affect your decision-making, prefer profit factor or win rate approach</li>
        <li><strong>Performance Goals:</strong> If maximizing returns is primary, consider the net pips approach</li>
        <li><strong>Emotional Management:</strong> If you need confidence from winning trades, prioritize win rate</li>
    </ul>"#,
            pf_sorted[0].timeframe,
            pf_sorted[0].take_profit_pips,
            pf_sorted[0].stop_loss_pips,
            pf_sorted[0].take_profit_pips / pf_sorted[0].stop_loss_pips,
            pf_sorted[0].win_rate,
            pf_sorted[0].profit_factor,
            pf_sorted[0].net_pips,
            pf_sorted[0].total_trades,
            pips_sorted[0].timeframe,
            pips_sorted[0].take_profit_pips,
            pips_sorted[0].stop_loss_pips,
            pips_sorted[0].take_profit_pips / pips_sorted[0].stop_loss_pips,
            pips_sorted[0].win_rate,
            pips_sorted[0].profit_factor,
            pips_sorted[0].net_pips,
            pips_sorted[0].total_trades,
            winrate_sorted[0].timeframe,
            winrate_sorted[0].take_profit_pips,
            winrate_sorted[0].stop_loss_pips,
            winrate_sorted[0].take_profit_pips / winrate_sorted[0].stop_loss_pips,
            winrate_sorted[0].win_rate,
            winrate_sorted[0].profit_factor,
            winrate_sorted[0].net_pips,
            winrate_sorted[0].total_trades
        )?;
    }

    Ok(())
}

fn print_console_summary(
    pf_sorted: &[OptimizationResult],
    pips_sorted: &[OptimizationResult],
    winrate_sorted: &[OptimizationResult],
    csv_filename: &str,
    html_filename: &str,
    symbol: &str,
) {
    println!("\nOptimization Complete");
    println!("Report files: {} and {}", csv_filename, html_filename);

    if !pf_sorted.is_empty() && !pips_sorted.is_empty() && !winrate_sorted.is_empty() {
        println!("\nOptimization Approaches Comparison:");
        println!("----------------------------------");
        println!("Best by Profit Factor (consistency):");
        println!("  Timeframe: {}", pf_sorted[0].timeframe);
        println!(
            "  TP/SL: {:.1}/{:.1} pips",
            pf_sorted[0].take_profit_pips, pf_sorted[0].stop_loss_pips
        );
        println!("  Win Rate: {:.2}%", pf_sorted[0].win_rate);
        println!("  Profit Factor: {:.2}", pf_sorted[0].profit_factor);
        println!("  Net Pips: {:.1}", pf_sorted[0].net_pips);
        println!("  Total Trades: {}", pf_sorted[0].total_trades);

        println!("\nBest by Net Pips (total profit):");
        println!("  Timeframe: {}", pips_sorted[0].timeframe);
        println!(
            "  TP/SL: {:.1}/{:.1} pips",
            pips_sorted[0].take_profit_pips, pips_sorted[0].stop_loss_pips
        );
        println!("  Win Rate: {:.2}%", pips_sorted[0].win_rate);
        println!("  Profit Factor: {:.2}", pips_sorted[0].profit_factor);
        println!("  Net Pips: {:.1}", pips_sorted[0].net_pips);
        println!("  Total Trades: {}", pips_sorted[0].total_trades);

        println!("\nBest by Win Rate (psychological ease):");
        println!("  Timeframe: {}", winrate_sorted[0].timeframe);
        println!(
            "  TP/SL: {:.1}/{:.1} pips",
            winrate_sorted[0].take_profit_pips, winrate_sorted[0].stop_loss_pips
        );
        println!("  Win Rate: {:.2}%", winrate_sorted[0].win_rate);
        println!("  Profit Factor: {:.2}", winrate_sorted[0].profit_factor);
        println!("  Net Pips: {:.1}", winrate_sorted[0].net_pips);
        println!("  Total Trades: {}", winrate_sorted[0].total_trades);
    }
}
