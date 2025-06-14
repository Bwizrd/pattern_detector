// src/bin/zone_monitor/html.rs
// HTML dashboard generation

use crate::state::MonitorState;
use axum::{extract::State, response::Html};
use std::fs;

pub async fn status_page(State(state): State<MonitorState>) -> Html<String> {
    let cache = state.get_zone_cache().await;
    let prices = state.get_latest_prices().await;
    let alerts = state.get_recent_alerts().await;
    let connected = state.get_connection_status().await;
    let attempts = state.get_connection_attempts().await;
    let last_message_time = state.get_last_message_time().await;

    let mut zones_by_symbol = Vec::new();
    for (symbol, zones) in &cache.zones {
        zones_by_symbol.push((symbol.clone(), zones.len()));
    }
    zones_by_symbol.sort_by(|a, b| a.0.cmp(&b.0));

    let mut zones: Vec<_> = cache
        .zones
        .values()
        .flatten()
        .map(|zone| {
            let current_price = prices
                .get(&zone.symbol)
                .map(|p| (p.bid + p.ask) / 2.0)
                .unwrap_or(0.0);
            let distance = if current_price > 0.0 {
                let proximal_line = if zone.zone_type == "supply" { zone.high } else { zone.low };
                let distal_line = if zone.zone_type == "supply" { zone.low } else { zone.high };
                ((current_price - proximal_line).abs() * 10000.0).round()
            } else {
                0.0
            };
            (zone, distance)
        })
        .collect();
        
    zones.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    
    let zone_rows = zones
        .iter()
        .map(|(zone, distance)| {
            format!(
                r#"<tr>
                    <td><strong>{}</strong></td>
                    <td class="{}">{}</td>
                    <td>{}</td>
                    <td class="price">{:.5} - {:.5}</td>
                    <td>{:.0}</td>
                    <td>{:.1} pips</td>
                    <td>{}</td>
                </tr>"#,
                zone.symbol,
                zone.zone_type,
                zone.zone_type.to_uppercase(),
                zone.timeframe,
                zone.low,
                zone.high,
                zone.strength,
                distance,
                zone.created_at.format("%m/%d %H:%M")
            )
        })
        .collect::<Vec<String>>()
        .join("\n");

    let symbols_table = zones_by_symbol
        .iter()
        .map(|(symbol, count)| {
            let price_info = prices.get(symbol);
            let price_str = price_info
                .map(|p| format!("{:.5}", (p.bid + p.ask) / 2.0))
                .unwrap_or_else(|| "No data".to_string());
            let time_str = price_info
                .map(|p| p.timestamp.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "-".to_string());
            format!(
                "<tr><td><strong>{}</strong></td><td>{}</td><td class=\"price\">{}</td><td>{}</td></tr>",
                symbol, count, price_str, time_str
            )
        })
        .collect::<Vec<String>>()
        .join("\n");

    let alerts_section = if alerts.len() > 0 {
        format!(
            r#"
            <div class="card alert-card">
                <h3>🚨 Recent Zone Alerts (Last 5)</h3>
                <table>
                    <tr>
                        <th>Time</th>
                        <th>Symbol</th>
                        <th>Type</th>
                        <th>Price</th>
                        <th>Distance</th>
                        <th>Strength</th>
                    </tr>
                    {}
                </table>
            </div>
            "#,
            alerts
                .iter()
                .rev()
                .take(5)
                .map(|alert| format!(
                    r#"<tr>
                        <td>{}</td>
                        <td><strong>{}</strong></td>
                        <td class="{}">{}</td>
                        <td class="price">{:.5}</td>
                        <td>{:.1} pips</td>
                        <td>{:.2}</td>
                    </tr>"#,
                    alert.timestamp.format("%H:%M:%S"),
                    alert.symbol,
                    alert.zone_type,
                    alert.zone_type.to_uppercase(),
                    alert.current_price,
                    alert.distance_pips,
                    alert.strength
                ))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        "".to_string()
    };

    let template = fs::read_to_string("src/bin/zone_monitor/templates/dashboard.html")
        .expect("Failed to read template file");

    let html = template
        .replace("{connection_status}", if connected { "connected" } else { "disconnected" })
        .replace("{connection_state}", if connected { "Connected ✅" } else { "Disconnected ❌" })
        .replace("{attempts}", &attempts.to_string())
        .replace(
            "{last_message}",
            &last_message_time
                .map(|t| t.format("%H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "Never".to_string()),
        )
        .replace("{current_time}", &chrono::Utc::now().format("%H:%M:%S UTC").to_string())
        .replace("{cache_path}", &state.cache_file_path)
        .replace("{total_zones}", &cache.total_zones.to_string())
        .replace("{active_symbols}", &cache.zones.len().to_string())
        .replace("{live_prices}", &prices.len().to_string())
        .replace("{recent_alerts}", &alerts.len().to_string())
        .replace("{alerts_section}", &alerts_section)
        .replace("{symbols_table}", &symbols_table)
        .replace("{zones_table}", &zone_rows);

    Html(html)
}

pub async fn closest_zones_page(State(state): State<MonitorState>) -> Html<String> {
    let cache = state.get_zone_cache().await;
    let prices = state.get_latest_prices().await;

    // Calculate distance to proximal line (TUI style)
    let mut zones: Vec<_> = cache
        .zones
        .values()
        .flatten()
        .map(|zone| {
            let current_price = prices
                .get(&zone.symbol)
                .map(|p| (p.bid + p.ask) / 2.0)
                .unwrap_or(0.0);
            let (proximal, distal) = if zone.zone_type == "supply" {
                (zone.high, zone.low)
            } else {
                (zone.low, zone.high)
            };
            let is_jpy = zone.symbol.contains("JPY");
            let distance = if current_price > 0.0 {
                let multiplier = if is_jpy { 100.0 } else { 10000.0 };
                (current_price - proximal).abs() * multiplier
            } else {
                0.0
            };
            (zone, distance, current_price, proximal, distal)
        })
        .collect();
    zones.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let zone_rows = zones
        .iter()
        .map(|(zone, distance, price, proximal, distal)| {
            format!(
                r#"<tr>
                    <td>{}</td>
                    <td>{}</td>
                    <td class="{}">{}</td>
                    <td>{:.1}</td>
                    <td>{:.5}</td>
                    <td>{:.5}</td>
                    <td>{:.5}</td>
                    <td>{}</td>
                    <td>{}</td>
                    <td style='font-family:monospace;font-size:0.95em'>{}</td>
                </tr>"#,
                zone.symbol,
                zone.timeframe,
                zone.zone_type,
                zone.zone_type.to_uppercase(),
                distance,
                price,
                proximal,
                distal,
                zone.strength,
                zone.touch_count,
                zone.id
            )
        })
        .collect::<Vec<String>>()
        .join("\n");

    let template = fs::read_to_string("src/bin/zone_monitor/templates/closest_zones.html")
        .expect("Failed to read closest_zones.html template");
    let html = template.replace("{zone_rows}", &zone_rows);
    Html(html)
}