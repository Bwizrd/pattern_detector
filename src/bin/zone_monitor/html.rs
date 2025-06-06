// src/bin/zone_monitor/html.rs
// HTML dashboard generation

use crate::state::MonitorState;
use axum::{extract::State, response::Html};

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

    let html = format!(
        r#"
<!DOCTYPE html>
<html>
<head>
    <title>Zone Monitor Dashboard</title>
    <meta http-equiv="refresh" content="5">
    <style>
        body {{ font-family: 'Segoe UI', Arial, sans-serif; margin: 0; padding: 20px; background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); }}
        .container {{ max-width: 1200px; margin: 0 auto; }}
        .card {{ background: rgba(255,255,255,0.95); border-radius: 12px; padding: 20px; margin: 15px 0; box-shadow: 0 8px 32px rgba(0,0,0,0.1); }}
        .status-grid {{ display: grid; grid-template-columns: 1fr 1fr; gap: 20px; margin: 20px 0; }}
        .status-card {{ padding: 20px; border-radius: 8px; text-align: center; }}
        .connected {{ background: linear-gradient(135deg, #d4edda, #c3e6cb); border: 2px solid #28a745; }}
        .disconnected {{ background: linear-gradient(135deg, #f8d7da, #f5c6cb); border: 2px solid #dc3545; }}
        h1 {{ color: white; text-align: center; margin-bottom: 30px; text-shadow: 2px 2px 4px rgba(0,0,0,0.3); font-size: 2.5em; }}
        h3 {{ color: #333; border-bottom: 3px solid #667eea; padding-bottom: 8px; margin-top: 0; }}
        table {{ width: 100%; border-collapse: collapse; background: white; border-radius: 8px; overflow: hidden; }}
        th {{ background: linear-gradient(135deg, #667eea, #764ba2); color: white; padding: 12px; font-weight: 600; }}
        td {{ padding: 10px; border-bottom: 1px solid #eee; }}
        .supply {{ color: #dc3545; font-weight: bold; }}
        .demand {{ color: #28a745; font-weight: bold; }}
        .price {{ font-family: 'Courier New', monospace; font-weight: bold; font-size: 1.1em; }}
        .metric {{ font-size: 1.5em; font-weight: bold; color: #333; }}
        .metric-label {{ font-size: 0.9em; color: #666; margin-top: 5px; }}
        .alert-card {{ background: linear-gradient(135deg, #fff3cd, #ffeaa7); border: 2px solid #ffc107; margin: 10px 0; }}
        @media (max-width: 768px) {{ 
            .status-grid {{ grid-template-columns: 1fr; }}
            .container {{ padding: 10px; }}
            h1 {{ font-size: 2em; }}
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>🎯 Zone Monitor Dashboard</h1>
        
        <div class="status-grid">
            <div class="status-card {}">
                <h3>🔌 WebSocket Connection</h3>
                <div class="metric">{}</div>
                <div class="metric-label">Attempts: {}</div>
                <div class="metric-label">Last Message: {}</div>
            </div>
            
            <div class="status-card connected">
                <h3>📊 System Status</h3>
                <div class="metric">Operational</div>
                <div class="metric-label">Updated: {}</div>
                <div class="metric-label">Cache: {}</div>
            </div>
        </div>

        <div class="card">
            <h3>📈 Live Data Summary</h3>
            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 20px; text-align: center;">
                <div>
                    <div class="metric">{}</div>
                    <div class="metric-label">Total Zones</div>
                </div>
                <div>
                    <div class="metric">{}</div>
                    <div class="metric-label">Active Symbols</div>
                </div>
                <div>
                    <div class="metric">{}</div>
                    <div class="metric-label">Live Prices</div>
                </div>
                <div>
                    <div class="metric">{}</div>
                    <div class="metric-label">Recent Alerts</div>
                </div>
            </div>
        </div>

        {}

        <div class="card">
            <h3>💹 Symbols & Live Prices</h3>
            <table>
                <tr>
                    <th>Symbol</th>
                    <th>Zone Count</th>
                    <th>Latest Price</th>
                    <th>Last Update</th>
                </tr>
                {}
            </table>
        </div>

        <div class="card">
            <h3>🎯 Recent Zone Details (Top 15)</h3>
            <table>
                <tr>
                    <th>Symbol</th>
                    <th>Type</th>
                    <th>Range</th>
                    <th>Strength</th>
                    <th>Current Distance</th>
                    <th>Created</th>
                </tr>
                {}
            </table>
        </div>

        <p style="text-align: center; color: white; margin-top: 30px;">
            <em>🔄 Auto-refresh every 5 seconds</em>
        </p>
    </div>
</body>
</html>
    "#,
        if connected { "connected" } else { "disconnected" },
        if connected { "Connected ✅" } else { "Disconnected ❌" },
        attempts,
        last_message_time
            .map(|t| t.format("%H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "Never".to_string()),
        chrono::Utc::now().format("%H:%M:%S UTC"),
        state.cache_file_path,
        cache.total_zones,
        cache.zones.len(),
        prices.len(),
        alerts.len(),
        if alerts.len() > 0 {
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
        },
        zones_by_symbol
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
            .join("\n"),
        cache
            .zones
            .values()
            .flatten()
            .take(15)
            .map(|zone| {
                let current_price = prices
                    .get(&zone.symbol)
                    .map(|p| (p.bid + p.ask) / 2.0)
                    .unwrap_or(0.0);
                let distance = if current_price > 0.0 {
                    let zone_mid = (zone.high + zone.low) / 2.0;
                    ((current_price - zone_mid).abs() * 10000.0).round()
                } else {
                    0.0
                };
                let strength_class = if zone.strength >= 0.8 {
                    "supply"
                } else if zone.strength >= 0.5 {
                    "demand"
                } else {
                    ""
                };
                format!(
                    r#"<tr>
                        <td><strong>{}</strong></td>
                        <td class="{}">{}</td>
                        <td class="price">{:.5} - {:.5}</td>
                        <td>{:.2}</td>
                        <td>{:.1} pips</td>
                        <td>{}</td>
                    </tr>"#,
                    zone.symbol,
                    zone.zone_type,
                    zone.zone_type.to_uppercase(),
                    zone.low,
                    zone.high,
                    zone.strength,
                    distance,
                    zone.created_at.format("%m/%d %H:%M")
                )
            })
            .collect::<Vec<String>>()
            .join("\n")
    );

    Html(html)
}