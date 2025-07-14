// src/bin/dashboard/ui.rs - Complete UI rendering functions with zone ID display
use chrono::Utc;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::app::App;
use crate::notifications::render_enhanced_notifications_page;
use crate::types::{AppPage, TradeNotificationDisplay, ZoneStatus};

pub fn ui(f: &mut Frame, app: &App) {
    match app.current_page {
        AppPage::Dashboard => ui_dashboard(f, app),
        AppPage::NotificationMonitor => {
            let size = f.size();
            render_enhanced_notifications_page(
                f,
                size, // Use the full screen area
                &app.all_notifications,
                &app.validated_trades,
                app.selected_notification_index,
                &app.selected_notification_validation,
                &app.notification_timeframe_filters,
            );
        }
        AppPage::Prices => ui_prices(f, app),
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
            Constraint::Length(3), // Header
            Constraint::Length(6), // Stats (increased for symbol filter)
            Constraint::Min(10),   // Zones table (takes ALL remaining space)
            Constraint::Length(3), // Controls
        ])
        .split(main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(main_chunks[1]);

    render_header(f, app, left_chunks[0], "Dashboard");
    render_stats_and_controls_enhanced(f, app, left_chunks[1], size.width);
    render_zones_table_improved(f, app, left_chunks[2], size.width);
    render_dashboard_help(f, left_chunks[3]);
    render_placed_trades_panel(f, app, right_chunks[0], right_chunks[1]);
    
    // Show trading reminder popup if needed
    if app.show_trading_reminder {
        render_trading_reminder_popup(f, size);
    }
}

pub fn ui_notification_monitor(f: &mut Frame, app: &App) {
    let size = f.size();

    // Split into left (50%) and right (50%) panels
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(size);

    // Left side layout - All Notifications
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Notification header + timeframe status
            Constraint::Length(3), // Timeframe controls
            Constraint::Min(1),    // Notifications list
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

    // Add timeframe status display
    render_notification_timeframe_status(f, app, left_chunks[1]);
    render_notification_timeframe_controls(f, app, left_chunks[2]);
    render_all_notifications_panel_enhanced(f, app, Rect::new(0, 0, 0, 0), left_chunks[3]); // Skip header, use full area

    render_notification_help_enhanced(f, right_chunks[1]);
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
    let ws_status = if app.websocket_connected { "🟢 WS" } else { "🔴 WS" };
    
    // ✅ FIX: Use the helper method instead of accessing private field
    let redis_status = if app.is_redis_connected() { "🟢 Redis" } else { "🔴 Redis" };
    
    let trading_status = if app.trading_enabled {
        if app.websocket_connected { "🟢 ON" } else { "🟡 ON (NO WS)" }
    } else {
        "🔴 OFF"
    };
    
    let pending_orders_count = app.pending_orders.len();
    
    let header_text = format!(
        "API: {} | {} | {} | Updates: {} | Last: {}s ago | Time: {} | Page: {} | Prices: {} | Trading: {} | Pending: {}",
        app.api_base_url,
        ws_status,
        redis_status,
        app.update_count,
        elapsed,
        now.format("%H:%M:%S"),
        page_name,
        app.current_prices.len(),
        trading_status,
        pending_orders_count
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
        .title("📊 Stats & Controls")
        .border_style(Style::default().fg(Color::Green));

    // Build stats line with more detail on larger screens
    let stats_line = if screen_width > 120 {
        // Large screens: Show all stats with more detail
        Line::from(vec![
            Span::styled("Total: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{} zones ", total),
                Style::default().fg(Color::White),
            ),
            if triggers > 0 {
                Span::styled(
                    "🚨 TRIGGERS: ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("🚨 Triggers: ", Style::default().fg(Color::Gray))
            },
            Span::styled(
                format!("{} ", triggers),
                if triggers > 0 {
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
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
                Span::styled(
                    "🚨 TRIGGERS: ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("Triggers: ", Style::default().fg(Color::Gray))
            },
            Span::styled(
                format!("{} ", triggers),
                if triggers > 0 {
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
            Span::styled("Inside: ", Style::default().fg(Color::Red)),
            Span::styled(format!("{} ", inside), Style::default().fg(Color::Red)),
            Span::styled("Close: ", Style::default().fg(Color::Yellow)),
            Span::styled(format!("{}", close), Style::default().fg(Color::Yellow)),
        ])
    };

    // Build the timeframes line and append trading plan pairs/timeframes if enabled
    let mut timeframes_line_spans = vec![
        Span::styled("Timeframes: ", Style::default().fg(Color::Cyan)),
        Span::styled(
            format!("{} ", app.get_timeframe_status()),
            Style::default().fg(Color::White),
        ),
        Span::styled("| Breached: ", Style::default().fg(Color::Cyan)),
        Span::styled(
            app.get_breached_status(),
            if app.show_breached {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            },
        ),
        Span::styled("| Indices: ", Style::default().fg(Color::Cyan)),
        Span::styled(
            if app.show_indices { "ON" } else { "OFF" },
            if app.show_indices {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            },
        ),
        Span::styled("| Threshold: ", Style::default().fg(Color::Cyan)),
        Span::styled(
            format!("{:.1}p", app.trading_threshold_pips),
            if app.trading_threshold_pips == 0.0 {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Yellow)
            },
        ),
    ];
    if app.trading_plan_enabled {
        if let Some(plan) = &app.trading_plan {
            let pairs = plan.top_setups.iter()
                .map(|s| format!("{} {}", s.symbol, s.timeframe))
                .collect::<Vec<_>>()
                .join(", ");
            timeframes_line_spans.push(Span::styled(
                format!(" | Plan: {}", pairs),
                Style::default().fg(Color::Green),
            ));
        }
    }
    let timeframes_line = Line::from(timeframes_line_spans);

    let strength_line = Line::from(vec![
        Span::styled(
            app.get_strength_filter_status(),
            if app.strength_input_mode {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            },
        ),
        if app.strength_input_mode {
            Span::styled(
                " (Enter to confirm, Esc to cancel)",
                Style::default().fg(Color::Gray),
            )
        } else {
            Span::styled(" | Press 's' to edit", Style::default().fg(Color::Gray))
        },
    ]);

    let symbol_filter_line = Line::from(vec![
        Span::styled(
            app.get_symbol_filter_status(),
            if app.symbol_filter_mode {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            },
        ),
        if app.symbol_filter_mode {
            Span::styled(
                " (Enter to confirm, Esc to cancel)",
                Style::default().fg(Color::Gray),
            )
        } else {
            Span::styled(" | Press 'f' to edit, 'c' to clear", Style::default().fg(Color::Gray))
        },
    ]);

    let trading_plan_status_span = if app.trading_plan_enabled {
        Span::styled(
            " | Trading Plan: ON",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " | Trading Plan: OFF",
            Style::default().fg(Color::Gray),
        )
    };

    let mut stats_line_spans = stats_line.spans.clone();
    stats_line_spans.push(trading_plan_status_span);
    let stats_line = Line::from(stats_line_spans);

    let selection_line = Line::from(vec![
        if let Some(index) = app.selected_zone_index {
            Span::styled(
                format!("Selected: Zone {} ", index + 1),
                Style::default().fg(Color::Yellow),
            )
        } else {
            Span::styled("Selected: None ", Style::default().fg(Color::Gray))
        },
        if let Some(copied) = &app.last_copied_zone_id {
            Span::styled(format!("| {}", copied), Style::default().fg(Color::Green))
        } else {
            Span::styled(
                "| ↑↓ to select, 'y' zone ID, 'z' zone details",
                Style::default().fg(Color::Gray),
            )
        },
    ]);

    let controls_line = Line::from(vec![
        Span::styled("Toggle: ", Style::default().fg(Color::Gray)),
        Span::styled(
            "[1]5m ",
            if app.is_timeframe_enabled("5m") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[2]15m ",
            if app.is_timeframe_enabled("15m") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[3]30m ",
            if app.is_timeframe_enabled("30m") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[4]1h ",
            if app.is_timeframe_enabled("1h") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[5]4h ",
            if app.is_timeframe_enabled("4h") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[6]1d ",
            if app.is_timeframe_enabled("1d") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[b]breached ",
            if app.show_breached {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            },
        ),
        Span::styled("[s]strength ", Style::default().fg(Color::Cyan)),
        Span::styled("[f]filter ", Style::default().fg(Color::Cyan)),
        Span::styled("[c]clear", Style::default().fg(Color::Cyan)),
    ]);

    let stats_text = Text::from(vec![
        stats_line,
        timeframes_line,
        strength_line,
        symbol_filter_line,
        selection_line,
        controls_line,
    ]);

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

        // Enhanced column configuration with interaction metrics and optional start date
        let (headers, widths, max_rows) = if screen_width > 180 {
            // Extra large screens: Show all columns including interaction metrics
            let mut headers = vec![
                "Symbol/TF",
                "Type", 
                "Distance",
                "Status",
                "Price",
                "Proximal",
                "Distal",
                "Str",
                "Touch",
                "TimeIn", // Time in zone (seconds)
                "Cross", // Proximal crossings  
                "Fresh", // Has ever entered
            ];
            
            let mut base_widths = vec![10, 6, 8, 12, 8, 8, 8, 6, 6, 6, 6, 6];
            
            // Add start date column if enabled
            if app.show_start_dates {
                headers.push("Start");
                base_widths.push(10);
            }
            
            headers.push("P"); // Pending order indicator
            base_widths.push(3);
            
            headers.push("Zone ID");
            base_widths.push(18);
            
            let total_base: u16 = base_widths.iter().sum();

            let widths = base_widths
                .iter()
                .map(|&w| {
                    let proportion = (w as f32 / total_base as f32 * available_width as f32) as u16;
                    Constraint::Length(proportion.max(5))
                })
                .collect();

            (headers, widths, 50)
        } else if screen_width > 160 {
            // Very large screens: Show basic interaction metrics
            let mut headers = vec![
                "Symbol/TF",
                "Type",
                "Distance", 
                "Status",
                "Price",
                "Proximal",
                "Distal",
                "Str",
                "Touch",
                "TimeIn", // Time in zone
                "Cross", // Crossings
            ];
            
            let mut base_widths = vec![12, 6, 8, 12, 10, 10, 10, 6, 6, 6, 6];
            
            // Add start date column if enabled
            if app.show_start_dates {
                headers.push("Start");
                base_widths.push(10);
            }
            
            headers.push("P"); // Pending order indicator
            base_widths.push(3);
            
            headers.push("Zone ID");
            base_widths.push(16);
            
            let total_base: u16 = base_widths.iter().sum();

            let widths = base_widths
                .iter()
                .map(|&w| {
                    let proportion = (w as f32 / total_base as f32 * available_width as f32) as u16;
                    Constraint::Length(proportion.max(6))
                })
                .collect();

            (headers, widths, 50)
        } else if screen_width > 140 {
            // Large screens: Show most columns but truncated Zone ID
            let mut headers = vec![
                "Symbol/TF",
                "Type",
                "S.Dist",
                "Status",
                "Price",
                "Proximal",
                "Distal",
                "Str",
            ];
            
            let mut base_widths = vec![15, 6, 10, 15, 12, 12, 12, 8];
            
            // Add start date column if enabled
            if app.show_start_dates {
                headers.push("Start");
                base_widths.push(8);
            }
            
            headers.push("P"); // Pending order indicator
            base_widths.push(3);
            
            headers.push("Zone ID");
            base_widths.push(16);
            
            let total_base: u16 = base_widths.iter().sum();

            let widths = base_widths
                .iter()
                .map(|&w| {
                    let proportion = (w as f32 / total_base as f32 * available_width as f32) as u16;
                    Constraint::Length(proportion.max(6))
                })
                .collect();

            (headers, widths, 50)
        } else if screen_width > 120 {
            // Medium-large screens: Basic columns + truncated Zone ID
            let mut headers = vec![
                "Symbol/TF",
                "Type",
                "S.Dist",
                "Status",
                "Price",
                "Proximal",
            ];
            
            let mut base_widths = vec![18, 8, 12, 18, 15, 15];
            
            // Add start date column if enabled (condensed)
            if app.show_start_dates {
                headers.push("Start");
                base_widths.push(10);
            }
            
            headers.push("P"); // Pending order indicator
            base_widths.push(3);
            
            headers.push("Zone ID");
            base_widths.push(16);
            
            let total_base: u16 = base_widths.iter().sum();

            let widths = base_widths
                .iter()
                .map(|&w| {
                    let proportion = (w as f32 / total_base as f32 * available_width as f32) as u16;
                    Constraint::Length(proportion.max(8))
                })
                .collect();

            (headers, widths, 45)
        } else if screen_width > 100 {
            // Medium screens: Essential columns only
            (
                vec!["Symbol/TF", "Type", "S.Dist", "Status", "Price", "P", "Zone ID"],
                vec![
                    Constraint::Percentage(18),
                    Constraint::Percentage(10),
                    Constraint::Percentage(12),
                    Constraint::Percentage(18),
                    Constraint::Percentage(18),
                    Constraint::Percentage(4), // P column
                    Constraint::Percentage(20), // Zone ID gets remaining space
                ],
                35,
            )
        } else {
            // Small screens: Minimal columns
            (
                vec!["Symbol/TF", "Type", "S.Dist", "Status"],
                vec![
                    Constraint::Percentage(30),
                    Constraint::Percentage(15),
                    Constraint::Percentage(20),
                    Constraint::Percentage(35),
                ],
                25,
            )
        };

        let header_cells = headers.iter().map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });

        let header = Row::new(header_cells).height(1).bottom_margin(1);

        // Filter zones by trading plan if enabled
        let filtered_zones: Vec<_> = if app.trading_plan_enabled {
            if let Some(plan) = &app.trading_plan {
                let allowed: std::collections::HashSet<_> = plan.top_setups.iter().map(|s| (s.symbol.as_str(), s.timeframe.as_str())).collect();
                app.zones.iter().filter(|zone|
                    allowed.contains(&(zone.symbol.as_str(), zone.timeframe.as_str()))
                ).collect()
            } else {
                vec![]
            }
        } else {
            app.zones.iter().collect()
        };

        let rows: Vec<Row> = filtered_zones
            .iter()
            .enumerate()
            .take(max_rows)
            .map(|(index, zone)| {
                let symbol_tf = format!("{}/{}", zone.symbol, zone.timeframe);
                let zone_type_short = if zone.zone_type.contains("supply") {
                    "SELL"
                } else {
                    "BUY"
                };
                let status_text =
                    format!("{} {}", zone.zone_status.symbol(), zone.zone_status.text());

                // Highlight selected row
                let row_style = if Some(index) == app.selected_zone_index {
                    Style::default()
                        .bg(Color::DarkGray)
                        .fg(zone.zone_status.color())
                } else {
                    Style::default().fg(zone.zone_status.color())
                };

                // Build row cells based on screen size
                let mut cells = vec![
                    Cell::from(symbol_tf),
                    Cell::from(zone_type_short),
                    Cell::from(format!("{:+.1}", zone.signed_distance_pips)),
                    Cell::from(status_text),
                ];

                // Add additional columns for larger screens
                if screen_width > 100 {
                    cells.extend(vec![Cell::from(format!("{:.5}", zone.current_price))]);

                    if screen_width > 120 {
                        // Highlight proximal line with background color if price is inside zone
                        let proximal_cell = if zone.zone_status == ZoneStatus::InsideZone {
                            Cell::from(format!("{:.5}", zone.proximal_line))
                                .style(Style::default().bg(Color::DarkGray).fg(Color::White))
                        } else {
                            Cell::from(format!("{:.5}", zone.proximal_line))
                        };
                        cells.push(proximal_cell);

                        if screen_width > 140 {
                            // Highlight distal line with background color if price is inside zone
                            let distal_cell = if zone.zone_status == ZoneStatus::InsideZone {
                                Cell::from(format!("{:.5}", zone.distal_line))
                                    .style(Style::default().bg(Color::DarkGray).fg(Color::White))
                            } else {
                                Cell::from(format!("{:.5}", zone.distal_line))
                            };
                            
                            cells.extend(vec![
                                distal_cell,
                                Cell::from(format!("{:.0}", zone.strength_score)),
                            ]);

                            if screen_width > 160 {
                                cells.push(Cell::from(format!("{}", zone.touch_count)));
                                
                                // Add interaction metrics for very large screens
                                if screen_width > 180 {
                                    // Time in zone (convert seconds to human readable)
                                    let time_in_display = if zone.total_time_inside_seconds > 0 {
                                        if zone.total_time_inside_seconds < 60 {
                                            format!("{}s", zone.total_time_inside_seconds)
                                        } else if zone.total_time_inside_seconds < 3600 {
                                            format!("{}m", zone.total_time_inside_seconds / 60)
                                        } else {
                                            format!("{}h", zone.total_time_inside_seconds / 3600)
                                        }
                                    } else {
                                        "0".to_string()
                                    };
                                    cells.push(Cell::from(time_in_display));
                                    
                                    // Zone entries (actual entries into the zone)
                                    cells.push(Cell::from(format!("{}", zone.zone_entries)));
                                    
                                    // Fresh zone indicator
                                    let fresh_indicator = if zone.has_ever_entered { "✓" } else { "✨" };
                                    cells.push(Cell::from(fresh_indicator).style(
                                        if zone.has_ever_entered {
                                            Style::default().fg(Color::Green)
                                        } else {
                                            Style::default().fg(Color::Cyan)
                                        }
                                    ));
                                } else {
                                    // For 160+ screens, add basic interaction metrics
                                    let time_in_display = if zone.total_time_inside_seconds > 0 {
                                        if zone.total_time_inside_seconds < 60 {
                                            format!("{}s", zone.total_time_inside_seconds)
                                        } else {
                                            format!("{}m", zone.total_time_inside_seconds / 60)
                                        }
                                    } else {
                                        "0".to_string()
                                    };
                                    cells.push(Cell::from(time_in_display));
                                    
                                    cells.push(Cell::from(format!("{}", zone.zone_entries)));
                                }
                            }
                        }
                    }

                    // Add start date column if enabled
                    if app.show_start_dates && screen_width > 120 {
                        let start_date_display = if let Some(created_at) = zone.created_at {
                            if screen_width > 160 {
                                created_at.format("%m/%d %H:%M").to_string() // MM/DD HH:MM
                            } else {
                                created_at.format("%m/%d").to_string() // Just MM/DD
                            }
                        } else {
                            "N/A".to_string()
                        };
                        cells.push(Cell::from(start_date_display).style(row_style.fg(Color::Gray)));
                    }

                    // Add Pending Order indicator column
                    let pending_indicator = if app.has_pending_order(&zone.zone_id) {
                        "P"
                    } else {
                        ""
                    };
                    cells.push(Cell::from(pending_indicator).style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));

                    // Add Zone ID column (truncated based on available space)
                    let zone_id_display = if screen_width > 160 {
                        zone.zone_id.clone() // Show full ID on very large screens
                    } else if screen_width > 140 {
                        if zone.zone_id.len() > 16 {
                            format!("{}...", &zone.zone_id[..13])
                        } else {
                            zone.zone_id.clone()
                        }
                    } else if screen_width > 120 {
                        if zone.zone_id.len() > 14 {
                            format!("{}...", &zone.zone_id[..11])
                        } else {
                            zone.zone_id.clone()
                        }
                    } else {
                        if zone.zone_id.len() > 12 {
                            format!("{}...", &zone.zone_id[..9])
                        } else {
                            zone.zone_id.clone()
                        }
                    };

                    cells.push(Cell::from(zone_id_display).style(row_style.fg(Color::Magenta)));
                }

                Row::new(cells).style(row_style)
            })
            .collect();

        // Add selection indicator if there are zones
        if !rows.is_empty() && app.zones.len() > 0 {
            let indicator_rows: Vec<Row> = app
                .zones
                .iter()
                .enumerate()
                .take(max_rows)
                .map(|(index, _)| {
                    let indicator = if Some(index) == app.selected_zone_index {
                        "►"
                    } else {
                        " "
                    };
                    let style = if Some(index) == app.selected_zone_index {
                        Style::default().bg(Color::DarkGray).fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![Cell::from(indicator).style(style)])
                })
                .collect();

            // Create side-by-side layout: indicator + main table
            let table_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(2), // Indicator column
                    Constraint::Min(1),    // Main table
                ])
                .split(area);

            // Render indicator column
            let indicator_table = Table::new(indicator_rows)
                .block(Block::default().borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM))
                .widths(&[Constraint::Length(2)]);

            f.render_widget(indicator_table, table_chunks[0]);

            // Render main table
            let main_table_area = Rect {
                x: table_chunks[1].x,
                y: table_chunks[1].y,
                width: table_chunks[1].width,
                height: table_chunks[1].height,
            };

            let main_table = Table::new(rows)
                .header(header)
                .block(Block::default().borders(Borders::RIGHT | Borders::TOP | Borders::BOTTOM))
                .widths(&widths);

            f.render_widget(main_table, main_table_area);
        } else {
            // Fallback if no rows
            let table = Table::new(rows)
                .header(header)
                .block(table_block)
                .widths(&widths);

            f.render_widget(table, area);
        }
    }
}

fn render_dashboard_help(f: &mut Frame, area: Rect) {
    let help_block = Block::default()
        .borders(Borders::ALL)
        .title("🔧 Controls")
        .border_style(Style::default().fg(Color::Gray));

    let help_text = "'q' quit | 'r' refresh | 'n' Notifications | '1-6' timeframes | 'b' breached | 's' strength | 'f' filter symbol | 'c' clear filter | 'i' indices | 't' trading | 'x' start dates | ↑↓ select zone | 'y' copy zone ID | 'z' copy zone details";
    let help = Paragraph::new(help_text)
        .block(help_block)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);

    f.render_widget(help, area);
}

fn render_placed_trades_panel(f: &mut Frame, app: &App, header_area: Rect, content_area: Rect) {
    // Calculate totals
    let open_trades_count = app.live_trades.iter().filter(|t| t.status == crate::types::TradeStatus::Open).count();
    let total_unrealized = app.get_total_unrealized_pips();
    let total_closed = app.get_closed_trades_total_pips();
    let total_pips = total_unrealized + total_closed;

    // Header
    let trades_header_block = Block::default()
        .borders(Borders::ALL)
        .title("💹 Zone Trigger Trades")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Green));

    let trades_header_text = format!(
        "Open: {} | Total P&L: {:.1} pips", 
        open_trades_count,
        total_pips
    );

    let header_color = if total_pips > 0.0 {
        Color::Green
    } else if total_pips < 0.0 {
        Color::Red
    } else {
        Color::White
    };

    let trades_header = Paragraph::new(trades_header_text)
        .block(trades_header_block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(header_color));

    f.render_widget(trades_header, header_area);

    // Content
    let trades_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    if app.live_trades.is_empty() {
        let empty_text = Paragraph::new("No live trades.\n\nTrades will be created when zones show 🚨 TRIGGER status.\n\nThis is independent from the notifications page ('n').")
            .block(trades_block)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(empty_text, content_area);
    } else {
        let trades_lines: Vec<Line> = app
            .live_trades
            .iter()
            .take(25)
            .map(|trade| {
                let _time_str = trade.timestamp.format("%H:%M:%S").to_string();
                let direction_color = if trade.direction == "BUY" {
                    Color::Green
                } else {
                    Color::Red
                };

                let pips_color = if trade.unrealized_pips > 0.0 {
                    Color::Green
                } else if trade.unrealized_pips < 0.0 {
                    Color::Red
                } else {
                    Color::Gray
                };

                let status_icon = match trade.status {
                    crate::types::TradeStatus::Open => "🔄",
                    crate::types::TradeStatus::TakenProfit => "✅",
                    crate::types::TradeStatus::StoppedOut => "❌",
                    crate::types::TradeStatus::Closed => "⏹️",
                };

                Line::from(vec![
                    Span::styled(status_icon, Style::default().fg(Color::White)),
                    Span::styled(
                        format!(" {} ", trade.direction),
                        Style::default()
                            .fg(direction_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{} ", trade.symbol),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("{} ", trade.timeframe),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(
                        format!("@ {:.4} ", trade.entry_price),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!("{:+.1}p ", trade.unrealized_pips),
                        Style::default()
                            .fg(pips_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("Zone:{}", &trade.zone_id[..8]),
                        Style::default().fg(Color::LightBlue),
                    ),
                ])
            })
            .collect();

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
            .border_style(
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            );

        let trigger_zones: Vec<_> = app
            .zones
            .iter()
            .filter(|z| z.zone_status == ZoneStatus::AtProximal)
            .take(3)
            .collect();

        let alert_lines: Vec<Line> = trigger_zones
            .iter()
            .map(|zone| {
                let action = if zone.zone_type.contains("supply") {
                    "SELL"
                } else {
                    "BUY"
                };
                Line::from(vec![
                    Span::styled(
                        format!("{} ", action),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{}/{} ", zone.symbol, zone.timeframe),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("@ {:.5}", zone.current_price),
                        Style::default().fg(Color::Yellow),
                    ),
                ])
            })
            .collect();

        let alert_text = Text::from(alert_lines);

        let alert = Paragraph::new(alert_text)
            .block(alert_block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Magenta));

        f.render_widget(alert, alert_area);
    }
}

// Enhanced notification functions
fn render_all_notifications_panel_enhanced(
    f: &mut Frame,
    app: &App,
    header_area: Rect,
    content_area: Rect,
) {
    // Header with timeframe filter status
    let left_header_block = Block::default()
        .borders(Borders::ALL)
        .title("🔔 All Zone Notifications")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Cyan));

    // Filter notifications by enabled timeframes
    let filtered_notifications: Vec<(usize, &TradeNotificationDisplay)> = app
        .all_notifications
        .iter()
        .enumerate()
        .filter(|(_, notif)| app.is_timeframe_enabled(&notif.timeframe))
        .collect();

    let header_text = format!(
        "Total: {} notifications (filtered: {}){}{}",
        app.all_notifications.len(),
        filtered_notifications.len(),
        if app.selected_notification_index.is_some() {
            format!(
                " | Selected: {}",
                app.selected_notification_index.unwrap() + 1
            )
        } else {
            "".to_string()
        },
        if let Some(copied) = &app.last_copied_zone_id {
            format!(" | {}", copied)
        } else {
            "".to_string()
        }
    );

    let left_header = Paragraph::new(header_text)
        .block(left_header_block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White));

    f.render_widget(left_header, header_area);

    // Content with enhanced layout
    let notif_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if filtered_notifications.is_empty() {
        let empty_text = if app.all_notifications.is_empty() {
            "No zone notifications yet.\n\nProximity alerts and trading signals will appear here.\n\nUse ↑↓ arrows to select, 'y' to copy zone ID\n1-6 to toggle timeframes"
        } else {
            "No notifications for enabled timeframes.\n\nUse 1-6 keys to toggle timeframes:\n[1]5m [2]15m [3]30m [4]1h [5]4h [6]1d"
        };

        let empty_widget = Paragraph::new(empty_text)
            .block(notif_block)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(empty_widget, content_area);
    } else {
        // Calculate available width for better layout
        let available_width = content_area.width.saturating_sub(4);

        // Enhanced headers with more columns
        let headers = if available_width > 120 {
            // Large screens: Show all data
            vec![
                "Time", "Type", "Action", "Pair/TF", "Price", "Dist", "Str", "Touch", "Zone ID",
            ]
        } else if available_width > 90 {
            // Medium screens: Essential columns
            vec![
                "Time", "Type", "Action", "Pair/TF", "Price", "Dist", "Zone ID",
            ]
        } else {
            // Small screens: Minimal columns
            vec!["Time", "Type", "Action", "Pair", "Price"]
        };

        let _header_cells = headers.iter().map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });

        // Enhanced headers with more columns
        let headers = if available_width > 120 {
            // Large screens: Show all data
            vec![
                "Time", "Type", "Action", "Pair/TF", "Price", "Dist", "Str", "Touch", "Zone ID",
            ]
        } else if available_width > 90 {
            // Medium screens: Essential columns
            vec![
                "Time", "Type", "Action", "Pair/TF", "Price", "Dist", "Zone ID",
            ]
        } else {
            // Small screens: Minimal columns
            vec!["Time", "Type", "Action", "Pair", "Price"]
        };

        let header_cells = headers.iter().map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });

        let header = Row::new(header_cells).height(1).bottom_margin(1);

        // Dynamic column widths to use full available space - make ALL columns wider
        let widths = if available_width > 120 {
            vec![
                Constraint::Percentage(10), // Time: HH:MM:SS
                Constraint::Percentage(12), // Type: PROXIMITY/TRADE
                Constraint::Percentage(8),  // Action: BUY/SELL
                Constraint::Percentage(15), // Pair/TF: GBPUSD/1h
                Constraint::Percentage(12), // Price: 1.32250
                Constraint::Percentage(8),  // Distance: 10.0
                Constraint::Percentage(8),  // Strength: 100
                Constraint::Percentage(8),  // Touch: 0
                Constraint::Percentage(19), // Zone ID: Remaining space
            ]
        } else if available_width > 90 {
            vec![
                Constraint::Percentage(12), // Time
                Constraint::Percentage(15), // Type: PROXIMITY/TRADE
                Constraint::Percentage(8),  // Action
                Constraint::Percentage(18), // Pair/TF
                Constraint::Percentage(15), // Price
                Constraint::Percentage(10), // Distance
                Constraint::Percentage(22), // Zone ID: Remaining space
            ]
        } else {
            vec![
                Constraint::Percentage(15), // Time: HH:MM
                Constraint::Percentage(20), // Type: PROXIM
                Constraint::Percentage(10), // Action: B/S
                Constraint::Percentage(25), // Pair: GBPUSD
                Constraint::Percentage(30), // Price: Remaining space
            ]
        };

        let rows: Vec<Row> = filtered_notifications
            .iter()
            .enumerate()
            .take(30)
            .map(|(_display_index, (original_index, notif))| {
                let time_str = if available_width > 90 {
                    notif.timestamp.format("%H:%M:%S").to_string()
                } else {
                    notif.timestamp.format("%H:%M").to_string()
                };

                // Determine notification type and colors
                let (type_str, type_color) = if notif.notification_type == "proximity_alert" {
                    if available_width > 90 {
                        ("PROXIMITY", Color::Yellow)
                    } else {
                        ("PROXIM", Color::Yellow)
                    }
                } else {
                    ("TRADE", Color::Green)
                };

                let action_str = if available_width > 90 {
                    notif.action.clone()
                } else {
                    if notif.action == "BUY" {
                        "B".to_string()
                    } else {
                        "S".to_string()
                    }
                };

                let pair_tf = if available_width > 90 {
                    format!("{}/{}", notif.symbol, notif.timeframe)
                } else {
                    notif.symbol.clone()
                };

                let price_str = format!("{:.5}", notif.price);

                // Use actual data from the notification
                let distance_str = if let Some(dist) = notif.distance_pips {
                    format!("{:.1}", dist)
                } else {
                    "N/A".to_string()
                };
                let strength_str = if let Some(strength) = notif.strength {
                    format!("{:.0}", strength)
                } else {
                    "N/A".to_string()
                };
                let touch_str = if let Some(touch) = notif.touch_count {
                    format!("{}", touch)
                } else {
                    "N/A".to_string()
                };

                // Highlight selected row
                let row_style = if Some(*original_index) == app.selected_notification_index {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };

                let action_color = if notif.action == "BUY" {
                    Color::Green
                } else {
                    Color::Red
                };

                // Show zone ID (truncated if needed)
                let zone_id = notif.signal_id.as_deref().unwrap_or("N/A");
                let display_zone_id = if available_width > 120 && zone_id.len() <= 16 {
                    zone_id.to_string()
                } else if zone_id.len() > 12 {
                    format!("{}...", &zone_id[..9])
                } else {
                    zone_id.to_string()
                };

                // Build row based on screen size
                let mut cells = vec![
                    Cell::from(time_str).style(row_style.fg(Color::Gray)),
                    Cell::from(type_str)
                        .style(row_style.fg(type_color).add_modifier(Modifier::BOLD)),
                    Cell::from(action_str)
                        .style(row_style.fg(action_color).add_modifier(Modifier::BOLD)),
                ];

                if available_width > 90 {
                    cells.extend(vec![
                        Cell::from(pair_tf).style(row_style.fg(Color::Cyan)),
                        Cell::from(price_str).style(row_style.fg(Color::White)),
                        Cell::from(distance_str).style(row_style.fg(Color::Magenta)),
                    ]);

                    if available_width > 120 {
                        cells.extend(vec![
                            Cell::from(strength_str).style(row_style.fg(Color::Blue)),
                            Cell::from(touch_str).style(row_style.fg(Color::Gray)),
                        ]);
                    }

                    cells.push(Cell::from(display_zone_id).style(row_style.fg(Color::Magenta)));
                } else {
                    cells.extend(vec![
                        Cell::from(pair_tf).style(row_style.fg(Color::Cyan)),
                        Cell::from(price_str).style(row_style.fg(Color::White)),
                    ]);
                }

                Row::new(cells)
            })
            .collect();

        // Add selection indicator column
        if !rows.is_empty() {
            let indicator_rows: Vec<Row> = filtered_notifications
                .iter()
                .enumerate()
                .take(30)
                .map(|(_display_index, (original_index, _))| {
                    let indicator = if Some(*original_index) == app.selected_notification_index {
                        "►"
                    } else {
                        " "
                    };
                    let style = if Some(*original_index) == app.selected_notification_index {
                        Style::default().bg(Color::DarkGray).fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![Cell::from(indicator).style(style)])
                })
                .collect();

            // Create side-by-side layout: indicator + main table
            let table_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(2), // Indicator column
                    Constraint::Min(1),    // Main table
                ])
                .split(content_area);

            // Render indicator column
            let indicator_table = Table::new(indicator_rows)
                .block(Block::default().borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM))
                .widths(&[Constraint::Length(2)]);

            f.render_widget(indicator_table, table_chunks[0]);

            // Render main table
            let main_table_area = Rect {
                x: table_chunks[1].x,
                y: table_chunks[1].y,
                width: table_chunks[1].width,
                height: table_chunks[1].height,
            };

            let main_table = Table::new(rows)
                .header(header)
                .block(Block::default().borders(Borders::RIGHT | Borders::TOP | Borders::BOTTOM))
                .widths(&widths);

            f.render_widget(main_table, main_table_area);
        } else {
            // Fallback if no rows
            let table = Table::new(rows)
                .header(header)
                .block(notif_block)
                .widths(&widths);

            f.render_widget(table, content_area);
        }
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
        for trade in app.validated_trades.iter().take(10) {
            // Show fewer with more detail
            let time_str = trade.timestamp.format("%H:%M:%S").to_string();
            let direction_color = if trade.direction == "BUY" {
                Color::Green
            } else {
                Color::Red
            };

            // Trade header line
            detailed_lines.push(Line::from(vec![
                Span::styled(format!("{} ", time_str), Style::default().fg(Color::Gray)),
                Span::styled("✅ ", Style::default().fg(Color::Green)),
                Span::styled(
                    format!("{} ", trade.direction),
                    Style::default()
                        .fg(direction_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}/", trade.symbol),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    format!("{}", trade.timeframe),
                    Style::default().fg(Color::Yellow),
                ),
            ]));

            // Details line
            detailed_lines.push(Line::from(vec![
                Span::styled("   Entry: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{:.5} ", trade.entry_price),
                    Style::default().fg(Color::White),
                ),
                Span::styled("SL: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{:.5} ", trade.stop_loss),
                    Style::default().fg(Color::Red),
                ),
                Span::styled("TP: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{:.5}", trade.take_profit),
                    Style::default().fg(Color::Green),
                ),
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

fn render_notification_help_enhanced(f: &mut Frame, area: Rect) {
    let help_block = Block::default()
        .borders(Borders::ALL)
        .title("🔧 Controls")
        .border_style(Style::default().fg(Color::Gray));

    let help_text = "'q' quit | 'r' refresh | 'd' Dashboard | 'c' clear | ↑↓ select notification | 'y' copy zone ID";
    let help = Paragraph::new(help_text)
        .block(help_block)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);

    f.render_widget(help, area);
}

fn render_notification_timeframe_status(f: &mut Frame, app: &App, area: Rect) {
    let status_block = Block::default()
        .borders(Borders::ALL)
        .title("📊 Timeframe Filter")
        .border_style(Style::default().fg(Color::Green));

    let enabled_timeframes = app.get_timeframe_status();
    let total_notifications = app.all_notifications.len();
    let filtered_count = app
        .all_notifications
        .iter()
        .filter(|notif| app.is_timeframe_enabled(&notif.timeframe))
        .count();

    let status_text = format!(
        "Active: {} | Showing: {}/{} notifications",
        enabled_timeframes, filtered_count, total_notifications
    );

    let status = Paragraph::new(status_text)
        .block(status_block)
        .style(Style::default().fg(Color::White))
        .alignment(Alignment::Center);

    f.render_widget(status, area);
}

fn render_notification_timeframe_controls(f: &mut Frame, app: &App, area: Rect) {
    let controls_block = Block::default()
        .borders(Borders::ALL)
        .title("⚙️ Toggle Timeframes")
        .border_style(Style::default().fg(Color::Gray));

    let controls_line = Line::from(vec![
        Span::styled(
            "[1]5m ",
            if app.is_timeframe_enabled("5m") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[2]15m ",
            if app.is_timeframe_enabled("15m") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[3]30m ",
            if app.is_timeframe_enabled("30m") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[4]1h ",
            if app.is_timeframe_enabled("1h") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[5]4h ",
            if app.is_timeframe_enabled("4h") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        Span::styled(
            "[6]1d ",
            if app.is_timeframe_enabled("1d") {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
    ]);

    let controls = Paragraph::new(controls_line)
        .block(controls_block)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);

    f.render_widget(controls, area);
}

pub fn ui_prices(f: &mut Frame, app: &App) {
    let size = f.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(4), // Stats
            Constraint::Min(10),   // Price table
            Constraint::Length(3), // Controls
        ])
        .split(size);

    render_header(f, app, chunks[0], "Live Prices");
    render_price_stats(f, app, chunks[1]);
    render_price_table(f, app, chunks[2]);
    render_price_help(f, chunks[3]);
}

fn render_price_stats(f: &mut Frame, app: &App, area: Rect) {
    let (symbol_count, total_updates, last_update_elapsed) = app.get_price_stats();

    let stats_block = Block::default()
        .borders(Borders::ALL)
        .title("📊 Price Stream Stats")
        .border_style(Style::default().fg(Color::Green));

    let last_update_text = if let Some(elapsed) = last_update_elapsed {
        format!("{}s ago", elapsed.as_secs())
    } else {
        "Never".to_string()
    };

    let stats_line1 = Line::from(vec![
        Span::styled("Symbols: ", Style::default().fg(Color::White)),
        Span::styled(
            format!("{} ", symbol_count),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("| Total Updates: ", Style::default().fg(Color::White)),
        Span::styled(
            format!("{} ", total_updates),
            Style::default().fg(Color::Green),
        ),
        Span::styled("| Last Update: ", Style::default().fg(Color::White)),
        Span::styled(
            last_update_text,
            if last_update_elapsed.map(|d| d.as_secs()).unwrap_or(999) < 5 {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            },
        ),
    ]);

    let websocket_status = Line::from(vec![
        Span::styled("WebSocket Status: ", Style::default().fg(Color::White)),
        if symbol_count > 0 && last_update_elapsed.map(|d| d.as_secs()).unwrap_or(999) < 10 {
            Span::styled(
                "🟢 CONNECTED",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else if symbol_count > 0 {
            Span::styled("🟡 STALE", Style::default().fg(Color::Yellow))
        } else {
            Span::styled("🔴 NO DATA", Style::default().fg(Color::Red))
        },
        Span::styled(" | API: ", Style::default().fg(Color::White)),
        Span::styled(app.api_base_url.clone(), Style::default().fg(Color::Gray)),
    ]);

    let stats_text = Text::from(vec![stats_line1, websocket_status]);

    let stats = Paragraph::new(stats_text)
        .block(stats_block)
        .alignment(Alignment::Left);

    f.render_widget(stats, area);
}

fn render_price_table(f: &mut Frame, app: &App, area: Rect) {
    let table_block = Block::default()
        .borders(Borders::ALL)
        .title("💱 Live Price Feed")
        .border_style(Style::default().fg(Color::Blue));

    if app.current_prices.is_empty() {
        let empty_text = Paragraph::new("No price data available.\n\nCheck:\n• WebSocket connection to price feed\n• API endpoint /current-prices\n• Network connectivity\n\nPress 'r' to refresh")
            .block(table_block)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(empty_text, area);
    } else {
        let header_cells = [
            "Symbol",
            "Bid",
            "Ask",
            "Spread",
            "Pips",
            "Updates",
            "Last Update",
        ]
        .iter()
        .map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });

        let header = Row::new(header_cells).height(1).bottom_margin(1);

        let widths = vec![
            Constraint::Length(8),  // Symbol
            Constraint::Length(10), // Bid
            Constraint::Length(10), // Ask
            Constraint::Length(8),  // Spread
            Constraint::Length(6),  // Pips
            Constraint::Length(8),  // Updates
            Constraint::Min(12),    // Last Update
        ];

        // Sort prices by symbol name
        let mut sorted_prices: Vec<_> = app.current_prices.iter().collect();
        sorted_prices.sort_by(|a, b| a.0.cmp(b.0));

        let rows: Vec<Row> = sorted_prices
            .iter()
            .take(40)
            .map(|(symbol, price_data)| {
                // Calculate spread in pips
                let pip_value = app.pip_values.get(*symbol).cloned().unwrap_or(0.0001);
                let spread_pips = price_data.spread / pip_value;

                // Color based on data freshness
                let time_elapsed = price_data
                    .last_update
                    .signed_duration_since(Utc::now())
                    .num_seconds()
                    .abs();
                let freshness_color = if time_elapsed < 5 {
                    Color::Green
                } else if time_elapsed < 30 {
                    Color::Yellow
                } else {
                    Color::Red
                };

                let last_update_str = price_data.last_update.format("%H:%M:%S").to_string();

                Row::new(vec![
                    Cell::from(symbol.as_str()).style(Style::default().fg(Color::Cyan)),
                    Cell::from(format!("{:.5}", price_data.bid))
                        .style(Style::default().fg(Color::White)),
                    Cell::from(format!("{:.5}", price_data.ask))
                        .style(Style::default().fg(Color::White)),
                    Cell::from(format!("{:.5}", price_data.spread))
                        .style(Style::default().fg(Color::Gray)),
                    Cell::from(format!("{:.1}", spread_pips))
                        .style(Style::default().fg(Color::Magenta)),
                    Cell::from(format!("{}", price_data.update_count))
                        .style(Style::default().fg(Color::Green)),
                    Cell::from(last_update_str).style(Style::default().fg(freshness_color)),
                ])
            })
            .collect();

        let table = Table::new(rows)
            .header(header)
            .block(table_block)
            .widths(&widths);

        f.render_widget(table, area);
    }
}

fn render_price_help(f: &mut Frame, area: Rect) {
    let help_block = Block::default()
        .borders(Borders::ALL)
        .title("🔧 Controls")
        .border_style(Style::default().fg(Color::Gray));

    let help_text = "'q' quit | 'r' refresh | 'd' Dashboard | 'n' Notifications | 'p' Prices | Live price feed from WebSocket";
    let help = Paragraph::new(help_text)
        .block(help_block)
        .style(Style::default().fg(Color::Gray))
        .alignment(Alignment::Center);

    f.render_widget(help, area);
}

fn render_trading_reminder_popup(f: &mut Frame, area: Rect) {
    // Calculate popup size (centered, smaller than screen)
    let popup_width = 50;
    let popup_height = 8;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    
    let popup_area = Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    };

    // Clear the background
    f.render_widget(Clear, popup_area);

    // Create popup block
    let popup_block = Block::default()
        .borders(Borders::ALL)
        .title("🔔 Trading Reminder")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Yellow));

    let popup_content = vec![
        Line::from(vec![
            Span::styled("WebSocket Connected! 🟢", Style::default().fg(Color::Green)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Trading is currently ", Style::default().fg(Color::White)),
            Span::styled("DISABLED", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::White)),
            Span::styled("'t'", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" to enable trading", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::White)),
            Span::styled("Enter", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" to dismiss", Style::default().fg(Color::White)),
        ]),
    ];

    let popup_paragraph = Paragraph::new(popup_content)
        .block(popup_block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });

    f.render_widget(popup_paragraph, popup_area);
}
