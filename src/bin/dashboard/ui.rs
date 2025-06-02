// src/bin/dashboard/ui.rs - UI rendering functions
use chrono::Utc;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap,
    },
    Frame,
};

use crate::app::App;
use crate::types::{AppPage, ZoneStatus};

pub fn ui(f: &mut Frame, app: &App) {
    match app.current_page {
        AppPage::Dashboard => ui_dashboard(f, app),
        AppPage::NotificationMonitor => ui_notification_monitor(f, app),
    }
}

pub fn ui_dashboard(f: &mut Frame, app: &App) {
    let size = f.size();
    
    // More aggressive space allocation - minimize right panel on large screens
    let (main_constraint, right_constraint) = if size.width > 150 {
        // Very large screens: Tiny right panel, maximize table space
        (Constraint::Min(100), Constraint::Length(30))
    } else if size.width > 120 {
        // Large screens: Small right panel
        (Constraint::Min(85), Constraint::Length(35))
    } else if size.width > 100 {
        // Medium screens: Balanced but favor main area
        (Constraint::Percentage(78), Constraint::Percentage(22))
    } else {
        // Small screens: Original proportions
        (Constraint::Percentage(80), Constraint::Percentage(20))
    };
    
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([main_constraint, right_constraint])
        .split(size);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),    // Header
            Constraint::Length(5),    // Stats
            Constraint::Min(10),      // Zones table (takes ALL remaining space)
            Constraint::Length(3),    // Controls
        ])
        .split(main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),    
            Constraint::Min(5),       
        ])
        .split(main_chunks[1]);

    render_header(f, app, left_chunks[0], "Dashboard");
    render_stats_and_controls_enhanced(f, app, left_chunks[1], size.width);
    render_zones_table_improved(f, app, left_chunks[2], size.width);
    render_dashboard_help(f, left_chunks[3]);
    render_placed_trades_panel(f, app, right_chunks[0], right_chunks[1]);
    render_trade_alert_popup(f, app, size);
}

pub fn ui_notification_monitor(f: &mut Frame, app: &App) {
    let size = f.size();
    
    // Split into left (50%) and right (50%) panels
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(size);

    // Left side layout - All Notifications
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(main_chunks[0]);

    // Right side layout - Validated Trades
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(main_chunks[1]);

    render_header(f, app, left_chunks[0], "Notifications");
    render_all_notifications_panel(f, app, left_chunks[1], left_chunks[2]);
    render_notification_help(f, right_chunks[1]);
    render_validated_trades_panel(f, app, right_chunks[0], right_chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect, page_name: &str) {
    let header_block = Block::default()
        .borders(Borders::ALL)
        .title("🎯 Zone Trading Dashboard")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Cyan));

    let now = Utc::now();
    let elapsed = app.last_update.elapsed().as_secs();
    let header_text = format!(
        "Connected: {} | Updates: {} | Last: {}s ago | Time: {} | Page: {}",
        app.api_base_url,
        app.update_count,
        elapsed,
        now.format("%H:%M:%S"),
        page_name
    );

    let header = Paragraph::new(header_text)
        .block(header_block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White));

    f.render_widget(header, area);
}

fn render_stats_and_controls_enhanced(f: &mut Frame, app: &App, area: Rect, screen_width: u16) {
    let (total, triggers, inside, close, watch) = app.get_stats();
    
    let stats_block = Block::default()
        .borders(Borders::ALL)
        .title("📊 Stats & Timeframes")
        .border_style(Style::default().fg(Color::Green));

    // Build stats line with more detail on larger screens
    let stats_line = if screen_width > 120 {
        // Large screens: Show all stats with more detail
        Line::from(vec![
            Span::styled("Total: ", Style::default().fg(Color::White)),
            Span::styled(format!("{} zones ", total), Style::default().fg(Color::White)),
            if triggers > 0 {
                Span::styled("🚨 TRIGGERS: ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("🚨 Triggers: ", Style::default().fg(Color::Gray))
            },
            Span::styled(format!("{} ", triggers), if triggers > 0 { Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::Gray) }),
            Span::styled("📍 Inside: ", Style::default().fg(Color::Red)),
            Span::styled(format!("{} ", inside), Style::default().fg(Color::Red)),
            Span::styled("🔴 Close (<10p): ", Style::default().fg(Color::Yellow)),
            Span::styled(format!("{} ", close), Style::default().fg(Color::Yellow)),
            Span::styled("🟢 Watch (<25p): ", Style::default().fg(Color::Green)),
            Span::styled(format!("{}", watch), Style::default().fg(Color::Green)),
        ])
    } else {
        // Smaller screens: Condensed stats
        Line::from(vec![
            Span::styled("Total: ", Style::default().fg(Color::White)),
            Span::styled(format!("{} ", total), Style::default().fg(Color::White)),
            if triggers > 0 {
                Span::styled("🚨 TRIGGERS: ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("Triggers: ", Style::default().fg(Color::Gray))
            },
            Span::styled(format!("{} ", triggers), if triggers > 0 { Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::Gray) }),
            Span::styled("Inside: ", Style::default().fg(Color::Red)),
            Span::styled(format!("{} ", inside), Style::default().fg(Color::Red)),
            Span::styled("Close: ", Style::default().fg(Color::Yellow)),
            Span::styled(format!("{}", close), Style::default().fg(Color::Yellow)),
        ])
    };

    let timeframes_line = Line::from(vec![
        Span::styled("Timeframes: ", Style::default().fg(Color::Cyan)),
        Span::styled(format!("{} ", app.get_timeframe_status()), Style::default().fg(Color::White)),
        Span::styled("| Breached: ", Style::default().fg(Color::Cyan)),
        Span::styled(app.get_breached_status(), if app.show_breached { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
    ]);

    let controls_line = Line::from(vec![
        Span::styled("Toggle: ", Style::default().fg(Color::Gray)),
        Span::styled("[1]5m ", if app.is_timeframe_enabled("5m") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[2]15m ", if app.is_timeframe_enabled("15m") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[3]30m ", if app.is_timeframe_enabled("30m") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[4]1h ", if app.is_timeframe_enabled("1h") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[5]4h ", if app.is_timeframe_enabled("4h") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[6]1d ", if app.is_timeframe_enabled("1d") { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
        Span::styled("[b]breached", if app.show_breached { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
    ]);

    let stats_text = Text::from(vec![stats_line, timeframes_line, controls_line]);

    let stats = Paragraph::new(stats_text)
        .block(stats_block)
        .alignment(Alignment::Left);

    f.render_widget(stats, area);
}

fn render_zones_table_improved(f: &mut Frame, app: &App, area: Rect, screen_width: u16) {
    if let Some(error) = &app.error_message {
        let error_block = Block::default()
            .borders(Borders::ALL)
            .title("❌ Error")
            .border_style(Style::default().fg(Color::Red));

        let error_widget = Paragraph::new(error.as_str())
            .block(error_block)
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });

        f.render_widget(error_widget, area);
    } else {
        let table_block = Block::default()
            .borders(Borders::ALL)
            .title("🎯 Active Zones")
            .border_style(Style::default().fg(Color::Blue));

        // Calculate available width (subtract borders and padding)
        let available_width = area.width.saturating_sub(4); // 2 for borders + 2 for padding
        
        // Dynamic column configuration that uses ALL available space
        let (headers, widths, max_rows) = if screen_width > 140 {
            // Large screens: Show all columns with proportional spacing
            let base_widths = [15, 6, 10, 15, 12, 12, 12, 8, 10]; // Base proportions
            let total_base: u16 = base_widths.iter().sum();
            
            let widths = base_widths.iter().map(|&w| {
                let proportion = (w as f32 / total_base as f32 * available_width as f32) as u16;
                Constraint::Length(proportion.max(6)) // Minimum 6 chars per column
            }).collect();
            
            (
                vec!["Symbol/TF", "Type", "S.Dist", "Status", "Price", "Proximal", "Distal", "Str", "Touches"],
                widths,
                50  // More rows on large screens
            )
        } else if screen_width > 120 {
            // Medium-large screens: Hide touches column but use full width
            let base_widths = [18, 8, 12, 18, 15, 15, 15, 10]; // Larger proportions
            let total_base: u16 = base_widths.iter().sum();
            
            let widths = base_widths.iter().map(|&w| {
                let proportion = (w as f32 / total_base as f32 * available_width as f32) as u16;
                Constraint::Length(proportion.max(8))
            }).collect();
            
            (
                vec!["Symbol/TF", "Type", "S.Dist", "Status", "Price", "Proximal", "Distal", "Str"],
                widths,
                45
            )
        } else if screen_width > 100 {
            // Medium screens: Use percentage-based widths for flexibility
            (
                vec!["Symbol/TF", "Type", "S.Dist", "Status", "Price", "Proximal"],
                vec![
                    Constraint::Percentage(20),  // Symbol/TF gets 20%
                    Constraint::Percentage(10),  // Type gets 10%
                    Constraint::Percentage(12),  // S.Dist gets 12%
                    Constraint::Percentage(18),  // Status gets 18%
                    Constraint::Percentage(20),  // Price gets 20%
                    Constraint::Percentage(20),  // Proximal gets 20%
                ],
                35
            )
        } else {
            // Small screens: Use all available space with minimal columns
            (
                vec!["Symbol/TF", "Type", "S.Dist", "Status"],
                vec![
                    Constraint::Percentage(30),  // Symbol/TF
                    Constraint::Percentage(15),  // Type
                    Constraint::Percentage(20),  // S.Dist
                    Constraint::Percentage(35),  // Status
                ],
                25
            )
        };

        let header_cells = headers.iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));

        let header = Row::new(header_cells).height(1).bottom_margin(1);

        let rows: Vec<Row> = app.zones.iter().take(max_rows).map(|zone| {
            let symbol_tf = format!("{}/{}", zone.symbol, zone.timeframe);
            let zone_type_short = if zone.zone_type.contains("supply") { "SELL" } else { "BUY" };
            let status_text = format!("{} {}", zone.zone_status.symbol(), zone.zone_status.text());
            
            let row_style = Style::default().fg(zone.zone_status.color());
            
            // Build row cells based on screen size
            let mut cells = vec![
                Cell::from(symbol_tf),
                Cell::from(zone_type_short),
                Cell::from(format!("{:+.1}", zone.signed_distance_pips)),
                Cell::from(status_text),
            ];
            
            // Add additional columns for larger screens
            if screen_width > 100 {
                cells.extend(vec![
                    Cell::from(format!("{:.5}", zone.current_price)),
                    Cell::from(format!("{:.5}", zone.proximal_line)),
                ]);
                
                if screen_width > 120 {
                    cells.extend(vec![
                        Cell::from(format!("{:.5}", zone.distal_line)),
                        Cell::from(format!("{:.0}", zone.strength_score)),
                    ]);
                    
                    if screen_width > 140 {
                        cells.push(Cell::from(format!("{}", zone.touch_count)));
                    }
                }
            }
            
            Row::new(cells).style(row_style)
        }).collect();

        let table = Table::new(rows)
            .header(header)
            .block(table_block)
            .widths(&widths);

        f.render_widget(table, area);
    }
}

fn render_dashboard_help(f: &mut Frame, area: Rect) {
    let help_block = Block::default()
        .borders(Borders::ALL)
        .title("🔧 Controls")
        .border_style(Style::default().fg(Color::Gray));

    let help_text = "Press 'q' to quit | 'r' to refresh | 'n' for Notification Monitor | '1-6' toggle timeframes | 'b' toggle breached | 'c' clear notifications";
    let help = Paragraph::new(help_text)
        .block(help_block)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);

    f.render_widget(help, area);
}

fn render_placed_trades_panel(f: &mut Frame, app: &App, header_area: Rect, content_area: Rect) {
    // Header
    let trades_header_block = Block::default()
        .borders(Borders::ALL)
        .title("📋 Placed Trades")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Green));

    let trades_count = app.placed_trades.len();
    let trades_header_text = format!("Total: {} placed trades", trades_count);

    let trades_header = Paragraph::new(trades_header_text)
        .block(trades_header_block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White));

    f.render_widget(trades_header, header_area);

    // Content
    let trades_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    if app.placed_trades.is_empty() {
        let empty_text = Paragraph::new("No trades placed yet.\n\nActual trade orders will appear here once order placement is implemented.")
            .block(trades_block)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(empty_text, content_area);
    } else {
        let trades_lines: Vec<Line> = app.placed_trades.iter().take(25).map(|trade| {
            let time_str = trade.timestamp.format("%m-%d %H:%M:%S").to_string();
            let direction_color = if trade.direction == "BUY" { Color::Green } else { Color::Red };
            
            Line::from(vec![
                Span::styled(format!("{} ", time_str), Style::default().fg(Color::Gray)),
                Span::styled("📋 ", Style::default().fg(Color::White)),
                Span::styled(format!("{} ", trade.direction), Style::default().fg(direction_color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}/", trade.symbol), Style::default().fg(Color::Cyan)),
                Span::styled(format!("{}", trade.timeframe), Style::default().fg(Color::Yellow)),
                Span::styled(format!(" @ {:.5}", trade.entry_price), Style::default().fg(Color::White)),
            ])
        }).collect();

        let trades_text = Text::from(trades_lines);

        let trades_display = Paragraph::new(trades_text)
            .block(trades_block)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });

        f.render_widget(trades_display, content_area);
    }
}

fn render_trade_alert_popup(f: &mut Frame, app: &App, size: Rect) {
    let (_, triggers, _, _, _) = app.get_stats();
    
    if triggers > 0 {
        let alert_area = Rect {
            x: size.width / 4,
            y: size.height / 4,
            width: size.width / 2,
            height: 7,
        };

        f.render_widget(Clear, alert_area);

        let alert_block = Block::default()
            .borders(Borders::ALL)
            .title("🚨 TRADE ALERT 🚨")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD));

        let trigger_zones: Vec<_> = app.zones.iter()
            .filter(|z| z.zone_status == ZoneStatus::AtProximal)
            .take(3)
            .collect();

        let alert_lines: Vec<Line> = trigger_zones.iter().map(|zone| {
            let action = if zone.zone_type.contains("supply") { "SELL" } else { "BUY" };
            Line::from(vec![
                Span::styled(format!("{} ", action), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}/{} ", zone.symbol, zone.timeframe), Style::default().fg(Color::Cyan)),
                Span::styled(format!("@ {:.5}", zone.current_price), Style::default().fg(Color::Yellow)),
            ])
        }).collect();

        let alert_text = Text::from(alert_lines);

        let alert = Paragraph::new(alert_text)
            .block(alert_block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Magenta));

        f.render_widget(alert, alert_area);
    }
}

fn render_all_notifications_panel(f: &mut Frame, app: &App, header_area: Rect, content_area: Rect) {
    // Header
    let left_header_block = Block::default()
        .borders(Borders::ALL)
        .title("🔔 All Zone Notifications")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Cyan));

    let left_header_text = format!("Total: {} notifications", app.all_notifications.len());

    let left_header = Paragraph::new(left_header_text)
        .block(left_header_block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White));

    f.render_widget(left_header, header_area);

    // Content
    let notif_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if app.all_notifications.is_empty() {
        let empty_text = Paragraph::new("No zone notifications yet.\n\nAll zone touches will appear here.")
            .block(notif_block)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(empty_text, content_area);
    } else {
        let notif_lines: Vec<Line> = app.all_notifications.iter().take(30).map(|notif| {
            let time_str = notif.timestamp.format("%m-%d %H:%M:%S").to_string();
            let action_color = if notif.action == "BUY" { Color::Green } else { Color::Red };
            
            Line::from(vec![
                Span::styled(format!("{} ", time_str), Style::default().fg(Color::Gray)),
                Span::styled("🔔 ", Style::default().fg(Color::White)),
                Span::styled(format!("{} ", notif.action), Style::default().fg(action_color)),
                Span::styled(format!("{}/", notif.symbol), Style::default().fg(Color::Cyan)),
                Span::styled(format!("{} ", notif.timeframe), Style::default().fg(Color::Yellow)),
                Span::styled(format!("@ {:.5}", notif.price), Style::default().fg(Color::White)),
            ])
        }).collect();

        let notif_text = Text::from(notif_lines);

        let notifications = Paragraph::new(notif_text)
            .block(notif_block)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });

        f.render_widget(notifications, content_area);
    }
}

fn render_validated_trades_panel(f: &mut Frame, app: &App, header_area: Rect, content_area: Rect) {
    // Header
    let right_header_block = Block::default()
        .borders(Borders::ALL)
        .title("✅ Validated Trade Signals")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Green));

    let right_header_text = format!("Total: {} validated trades", app.validated_trades.len());

    let right_header = Paragraph::new(right_header_text)
        .block(right_header_block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White));

    f.render_widget(right_header, header_area);

    // Content
    let trades_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    if app.validated_trades.is_empty() {
        let empty_text = Paragraph::new("No validated trades yet.\n\nTrades that pass all trading rules will appear here with:\n• Direction (BUY/SELL)\n• Entry Price\n• Time of Entry\n• Stop Loss\n• Take Profit")
            .block(trades_block)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(empty_text, content_area);
    } else {
        let mut detailed_lines = Vec::new();
        for trade in app.validated_trades.iter().take(10) { // Show fewer with more detail
            let time_str = trade.timestamp.format("%m-%d %H:%M:%S").to_string();
            let direction_color = if trade.direction == "BUY" { Color::Green } else { Color::Red };
            
            // Trade header line
            detailed_lines.push(Line::from(vec![
                Span::styled(format!("{} ", time_str), Style::default().fg(Color::Gray)),
                Span::styled("✅ ", Style::default().fg(Color::Green)),
                Span::styled(format!("{} ", trade.direction), Style::default().fg(direction_color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}/", trade.symbol), Style::default().fg(Color::Cyan)),
                Span::styled(format!("{}", trade.timeframe), Style::default().fg(Color::Yellow)),
            ]));
            
            // Details line
            detailed_lines.push(Line::from(vec![
                Span::styled("   Entry: ", Style::default().fg(Color::Gray)),
                Span::styled(format!("{:.5} ", trade.entry_price), Style::default().fg(Color::White)),
                Span::styled("SL: ", Style::default().fg(Color::Gray)),
                Span::styled(format!("{:.5} ", trade.stop_loss), Style::default().fg(Color::Red)),
                Span::styled("TP: ", Style::default().fg(Color::Gray)),
                Span::styled(format!("{:.5}", trade.take_profit), Style::default().fg(Color::Green)),
            ]));
            
            // Add empty line for spacing
            detailed_lines.push(Line::from(""));
        }

        let trades_text = Text::from(detailed_lines);

        let trades_display = Paragraph::new(trades_text)
            .block(trades_block)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });

        f.render_widget(trades_display, content_area);
    }
}

fn render_notification_help(f: &mut Frame, area: Rect) {
    let help_block = Block::default()
        .borders(Borders::ALL)
        .title("🔧 Controls")
        .border_style(Style::default().fg(Color::Gray));

    let help_text = "Press 'q' to quit | 'r' to refresh | 'd' for Dashboard | 'c' clear notifications";
    let help = Paragraph::new(help_text)
        .block(help_block)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);

    f.render_widget(help, area);
}