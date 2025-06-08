// src/bin/dashboard/notifications.rs
// Enhanced notifications system with rule validation - FINAL FIXED VERSION

use crate::types::{TradeNotificationDisplay, ValidatedTradeDisplay};
use chrono::Timelike;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap},
    Frame,
};
use std::collections::HashMap;

// Rule validation result for individual trading rules
#[derive(Debug, Clone)]
pub struct RuleValidationResult {
    pub rule_name: String,
    pub description: String,
    pub passed: bool,
    pub value: Option<String>,
    pub threshold: Option<String>,
    pub details: Option<String>,
}

// Complete validation for a notification
#[derive(Debug, Clone)]
pub struct NotificationValidation {
    pub notification_id: usize,
    pub rules: Vec<RuleValidationResult>,
    pub overall_valid: bool,
    pub reason: Option<String>,
}

// Trading configuration that mirrors your zone_monitor TradingConfig
#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub enabled: bool,
    pub lot_size: f64,
    pub stop_loss_pips: f64,
    pub take_profit_pips: f64,
    pub max_daily_trades: i32,
    pub max_trades_per_symbol: i32,
    pub allowed_symbols: Vec<String>,
    pub allowed_timeframes: Vec<String>,
    pub min_risk_reward: f64,
    pub trading_start_hour: u8,
    pub trading_end_hour: u8,
    pub min_zone_strength: f64,
    pub max_touch_count: u32,
    pub min_distance_pips: f64,
    pub trading_threshold_pips: f64,
    pub proximity_threshold_pips: f64,
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            enabled: std::env::var("TRADING_ENABLED")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            lot_size: std::env::var("TRADING_LOT_SIZE")
                .unwrap_or_else(|_| "1000".to_string())
                .parse()
                .unwrap_or(1000.0),
            stop_loss_pips: std::env::var("TRADING_STOP_LOSS_PIPS")
                .unwrap_or_else(|_| "20.0".to_string())
                .parse()
                .unwrap_or(20.0),
            take_profit_pips: std::env::var("TRADING_TAKE_PROFIT_PIPS")
                .unwrap_or_else(|_| "40.0".to_string())
                .parse()
                .unwrap_or(40.0),
            max_daily_trades: std::env::var("TRADING_MAX_DAILY_TRADES")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .unwrap_or(10),
            max_trades_per_symbol: std::env::var("TRADING_MAX_TRADES_PER_SYMBOL")
                .unwrap_or_else(|_| "2".to_string())
                .parse()
                .unwrap_or(2),
            allowed_symbols: std::env::var("TRADING_ALLOWED_SYMBOLS")
                .unwrap_or_else(|_| "EURUSD,GBPUSD,USDJPY,USDCHF,AUDUSD,USDCAD,NZDUSD,EURGBP,EURJPY,EURCHF,EURAUD,EURCAD,EURNZD,GBPJPY,GBPCHF,GBPAUD,GBPCAD,GBPNZD,AUDJPY,AUDNZD,AUDCAD,NZDJPY,CADJPY,CHFJPY".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            allowed_timeframes: std::env::var("TRADING_ALLOWED_TIMEFRAMES")
                .unwrap_or_else(|_| "30m,1h,4h,1d".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            min_risk_reward: std::env::var("TRADING_MIN_RISK_REWARD")
                .unwrap_or_else(|_| "1.5".to_string())
                .parse()
                .unwrap_or(1.5),
            trading_start_hour: std::env::var("TRADING_START_HOUR")
                .unwrap_or_else(|_| "0".to_string())
                .parse()
                .unwrap_or(0),
            trading_end_hour: std::env::var("TRADING_END_HOUR")
                .unwrap_or_else(|_| "23".to_string())
                .parse()
                .unwrap_or(23),
            max_touch_count: std::env::var("MAX_TOUCH_COUNT_FOR_TRADING")
                .unwrap_or_else(|_| "2".to_string())
                .parse()
                .unwrap_or(2),
            proximity_threshold_pips: std::env::var("PROXIMITY_THRESHOLD_PIPS")
                .unwrap_or_else(|_| "10.0".to_string())
                .parse()
                .unwrap_or(10.0),
            trading_threshold_pips: std::env::var("TRADING_THRESHOLD_PIPS")
                .unwrap_or_else(|_| "5.0".to_string())
                .parse()
                .unwrap_or(5.0),
            min_distance_pips: std::env::var("TRADING_THRESHOLD_PIPS")
                .unwrap_or_else(|_| "5.0".to_string())
                .parse()
                .unwrap_or(5.0),
            min_zone_strength: std::env::var("TRADING_MIN_ZONE_STRENGTH")
                .unwrap_or_else(|_| "0.0".to_string())
                .parse()
                .unwrap_or(0.0), // Default to 0.0 - no strength requirement unless set
        }
    }
}

// Rule validator that mirrors your trading engine logic
pub struct NotificationRuleValidator {
    config: TradingConfig,
    symbol_mappings: HashMap<String, u32>,
}

impl NotificationRuleValidator {
    pub fn new() -> Self {
        // Initialize with your symbol mappings from zone_monitor
        let mut symbol_mappings = HashMap::new();
        symbol_mappings.insert("EURUSD_SB".to_string(), 185);
        symbol_mappings.insert("GBPUSD_SB".to_string(), 199);
        symbol_mappings.insert("USDJPY_SB".to_string(), 226);
        symbol_mappings.insert("USDCHF_SB".to_string(), 222);
        symbol_mappings.insert("AUDUSD_SB".to_string(), 158);
        symbol_mappings.insert("USDCAD_SB".to_string(), 221);
        symbol_mappings.insert("NZDUSD_SB".to_string(), 211);
        symbol_mappings.insert("EURGBP_SB".to_string(), 175);
        symbol_mappings.insert("EURJPY_SB".to_string(), 177);
        symbol_mappings.insert("EURCHF_SB".to_string(), 173);
        symbol_mappings.insert("EURAUD_SB".to_string(), 171);
        symbol_mappings.insert("EURCAD_SB".to_string(), 172);
        symbol_mappings.insert("EURNZD_SB".to_string(), 180);
        symbol_mappings.insert("GBPJPY_SB".to_string(), 192);
        symbol_mappings.insert("GBPCHF_SB".to_string(), 191);
        symbol_mappings.insert("GBPAUD_SB".to_string(), 189);
        symbol_mappings.insert("GBPCAD_SB".to_string(), 190);
        symbol_mappings.insert("GBPNZD_SB".to_string(), 195);
        symbol_mappings.insert("AUDJPY_SB".to_string(), 155);
        symbol_mappings.insert("AUDNZD_SB".to_string(), 156);
        symbol_mappings.insert("AUDCAD_SB".to_string(), 153);
        symbol_mappings.insert("NZDJPY_SB".to_string(), 210);
        symbol_mappings.insert("CADJPY_SB".to_string(), 162);
        symbol_mappings.insert("CHFJPY_SB".to_string(), 163);
        symbol_mappings.insert("NAS100_SB".to_string(), 205);
        symbol_mappings.insert("US500_SB".to_string(), 220);

        Self {
            config: TradingConfig::default(),
            symbol_mappings,
        }
    }

    // Main validation function that mirrors your trading engine logic
    pub fn validate_notification(&self, notification: &TradeNotificationDisplay, id: usize) -> NotificationValidation {
        let mut rules = Vec::new();

        // Rule 1: Symbol Allowlist Check
        let symbol_allowed = self.config.allowed_symbols.contains(&notification.symbol);
        rules.push(RuleValidationResult {
            rule_name: "Symbol Allowlist".to_string(),
            description: "Symbol must be in allowed trading list".to_string(),
            passed: symbol_allowed,
            value: Some(notification.symbol.clone()),
            threshold: Some("Must be in allowed symbols".to_string()),
            details: if !symbol_allowed { 
                Some(format!("Symbol {} not in allowlist", notification.symbol)) 
            } else { None },
        });

        // Rule 2: Symbol Mapping Check (mirrors your can_trade function)
        let sb_symbol = format!("{}_SB", notification.symbol);
        let has_mapping = self.symbol_mappings.contains_key(&sb_symbol);
        rules.push(RuleValidationResult {
            rule_name: "Broker Mapping".to_string(),
            description: "Symbol must have broker mapping".to_string(),
            passed: has_mapping,
            value: Some(sb_symbol.clone()),
            threshold: Some("Must exist in broker mappings".to_string()),
            details: if !has_mapping { 
                Some("No broker symbol mapping found".to_string()) 
            } else { None },
        });

        // Rule 3: Timeframe Check
        let timeframe_allowed = self.config.allowed_timeframes.contains(&notification.timeframe);
        rules.push(RuleValidationResult {
            rule_name: "Timeframe".to_string(),
            description: "Timeframe must be allowed for trading".to_string(),
            passed: timeframe_allowed,
            value: Some(notification.timeframe.clone()),
            threshold: Some(format!("{:?}", self.config.allowed_timeframes)),
            details: if !timeframe_allowed { 
                Some("Timeframe not in allowed list".to_string()) 
            } else { None },
        });

        // Rule 4: Trading Hours Check (mirrors your can_trade function)
        let now = chrono::Utc::now();
        let hour = now.hour() as u8;
        let trading_hours_ok = hour >= self.config.trading_start_hour && hour <= self.config.trading_end_hour;
        rules.push(RuleValidationResult {
            rule_name: "Trading Hours".to_string(),
            description: format!("Must be within {}:00-{}:00 UTC", self.config.trading_start_hour, self.config.trading_end_hour),
            passed: trading_hours_ok,
            value: Some(format!("{}:00", hour)),
            threshold: Some(format!("{}:00-{}:00", self.config.trading_start_hour, self.config.trading_end_hour)),
            details: if !trading_hours_ok { 
                Some("Outside trading hours".to_string()) 
            } else { None },
        });

        // Rule 6: Touch Count Check (if available) - Uses your MAX_TOUCH_COUNT_FOR_TRADING
        if let Some(touch_count) = notification.touch_count {
            let touch_ok = (touch_count as u32) <= self.config.max_touch_count;
            rules.push(RuleValidationResult {
                rule_name: "Touch Count".to_string(),
                description: format!("Touch count must be <= {} (MAX_TOUCH_COUNT_FOR_TRADING)", self.config.max_touch_count),
                passed: touch_ok,
                value: Some(touch_count.to_string()),
                threshold: Some(format!("≤ {}", self.config.max_touch_count)),
                details: if !touch_ok { 
                    Some("Zone touched too many times for trading".to_string()) 
                } else { None },
            });
        }

        // Rule 7: Distance Check (if available) - Uses your TRADING_THRESHOLD_PIPS
        if let Some(distance_pips) = notification.distance_pips {
            let distance_ok = distance_pips <= self.config.trading_threshold_pips;
            rules.push(RuleValidationResult {
                rule_name: "Trading Distance".to_string(),
                description: format!("Distance must be <= {:.1} pips (TRADING_THRESHOLD_PIPS)", self.config.trading_threshold_pips),
                passed: distance_ok,
                value: Some(format!("{:.1} pips", distance_pips)),
                threshold: Some(format!("≤ {:.1} pips", self.config.trading_threshold_pips)),
                details: if !distance_ok { 
                    Some("Must be within trading threshold".to_string()) 
                } else { None },
            });
        }

        // Rule 8: Zone Strength Check (if available and configured)
        if let Some(strength) = notification.strength {
            if self.config.min_zone_strength > 0.0 {
                let strength_ok = strength >= self.config.min_zone_strength;
                rules.push(RuleValidationResult {
                    rule_name: "Zone Strength".to_string(),
                    description: format!("Zone strength must be >= {:.0} (TRADING_MIN_ZONE_STRENGTH)", self.config.min_zone_strength),
                    passed: strength_ok,
                    value: Some(format!("{:.0}", strength)),
                    threshold: Some(format!("{:.0}", self.config.min_zone_strength)),
                    details: if !strength_ok { 
                        Some("Zone strength too low".to_string()) 
                    } else { None },
                });
            } else {
                // Show that strength check is disabled
                rules.push(RuleValidationResult {
                    rule_name: "Zone Strength".to_string(),
                    description: "Zone strength check disabled (TRADING_MIN_ZONE_STRENGTH not set)".to_string(),
                    passed: true,
                    value: Some(format!("{:.0}", strength)),
                    threshold: Some("No requirement".to_string()),
                    details: None,
                });
            }
        }
        // Rule 9: Risk/Reward Ratio Check (mirrors your generate_trade_from_signal function)
        let risk = self.config.stop_loss_pips;
        let reward = self.config.take_profit_pips;
        let rr_ratio = reward / risk;
        let rr_ok = rr_ratio >= self.config.min_risk_reward;
        rules.push(RuleValidationResult {
            rule_name: "Risk/Reward Ratio".to_string(),
            description: format!("R:R must be >= {:.1} (TRADING_MIN_RISK_REWARD)", self.config.min_risk_reward),
            passed: rr_ok,
            value: Some(format!("{:.2}", rr_ratio)),
            threshold: Some(format!("{:.1}", self.config.min_risk_reward)),
            details: if !rr_ok { 
                Some("Risk/reward ratio too low".to_string()) 
            } else { None },
        });

        // Rule 10: Signal Action Validation
        // Rule 10: Signal Action Validation
        let action_ok = notification.action == "BUY" || notification.action == "SELL";
        rules.push(RuleValidationResult {
            rule_name: "Signal Action".to_string(),
            description: "Must have valid BUY or SELL action".to_string(),
            passed: action_ok,
            value: Some(notification.action.clone()),
            threshold: Some("BUY or SELL".to_string()),
            details: if !action_ok { 
                Some("Invalid or missing action".to_string()) 
            } else { None },
        });

        let overall_valid = rules.iter().all(|rule| rule.passed);
        let failed_rules: Vec<&str> = rules.iter()
            .filter(|rule| !rule.passed)
            .map(|rule| rule.rule_name.as_str())
            .collect();

        let reason = if !overall_valid {
            Some(format!("Failed: {}", failed_rules.join(", ")))
        } else {
            Some("✅ All rules passed - Trade executable".to_string())
        };

        NotificationValidation {
            notification_id: id,
            rules,
            overall_valid,
            reason,
        }
    }
}

// Enhanced notification rendering functions
pub fn render_enhanced_notifications_page(
    f: &mut Frame,
    area: Rect,
    notifications: &[TradeNotificationDisplay],
    _validated_trades: &[ValidatedTradeDisplay],
    selected_index: Option<usize>,
    validation: &Option<NotificationValidation>,
    timeframe_filters: &HashMap<String, bool>,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // Left panel: Split into notifications (top) and trading rules (bottom)
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(chunks[0]);

    // Left top: Enhanced notifications table with numbers
    render_numbered_notifications_table(f, left_chunks[0], notifications, selected_index, timeframe_filters);

    // Left bottom: Trading rules configuration
    render_trading_rules_panel(f, left_chunks[1]);

    // Right panel: Rule validation for selected notification
    render_notification_rule_validation(f, chunks[1], validation);
}

fn render_numbered_notifications_table(
    f: &mut Frame,
    area: Rect,
    notifications: &[TradeNotificationDisplay],
    selected_index: Option<usize>,
    timeframe_filters: &HashMap<String, bool>,
) {
    let notif_block = Block::default()
        .borders(Borders::ALL)
        .title("🔔 Notifications (↑↓ select, 1-9 direct, Enter validate)")
        .title_alignment(ratatui::layout::Alignment::Center)
        .border_style(Style::default().fg(Color::Cyan));

    // Filter notifications by enabled timeframes and create numbered list
    let filtered_notifications: Vec<(usize, &TradeNotificationDisplay)> = notifications
        .iter()
        .enumerate()
        .filter(|(_, notif)| {
            timeframe_filters.get(&notif.timeframe).copied().unwrap_or(true)
        })
        .collect();

    if filtered_notifications.is_empty() {
        let empty_text = if notifications.is_empty() {
            "No zone notifications yet.\n\nProximity alerts and trading signals will appear here.\n\nUse ↑↓ arrows to select, 'y' to copy zone ID\n1-6 to toggle timeframes"
        } else {
            "No notifications for enabled timeframes.\n\nUse 1-6 keys to toggle timeframes:\n[1]5m [2]15m [3]30m [4]1h [5]4h [6]1d"
        };

        let empty_widget = Paragraph::new(empty_text)
            .block(notif_block)
            .style(Style::default().fg(Color::Gray))
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(empty_widget, area);
    } else {
        // Calculate available width for better layout
        let available_width = area.width.saturating_sub(4);

        // Enhanced headers with more columns based on your original design
        let headers = if available_width > 120 {
            // Large screens: Show all data including number column
            vec!["#", "Time", "Type", "Action", "Pair/TF", "Price", "Dist", "Str", "Touch", "Zone ID"]
        } else if available_width > 90 {
            // Medium screens: Essential columns with number
            vec!["#", "Time", "Type", "Action", "Pair/TF", "Price", "Dist", "Zone ID"]
        } else {
            // Small screens: Minimal columns with number
            vec!["#", "Time", "Type", "Action", "Pair", "Price"]
        };

        let header_cells = headers.iter().map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });

        let header = Row::new(header_cells).height(1).bottom_margin(1);

        // Dynamic column widths to use full available space - restored from your original
        let widths = if available_width > 120 {
            vec![
                Constraint::Length(3),   // # - number column
                Constraint::Percentage(10),  // Time: HH:MM:SS
                Constraint::Percentage(12),  // Type: PROXIMITY/TRADE
                Constraint::Percentage(8),   // Action: BUY/SELL
                Constraint::Percentage(15),  // Pair/TF: GBPUSD/1h
                Constraint::Percentage(12),  // Price: 1.32250
                Constraint::Percentage(8),   // Distance: 10.0
                Constraint::Percentage(8),   // Strength: 100
                Constraint::Percentage(8),   // Touch: 0
                Constraint::Percentage(16),  // Zone ID: Remaining space
            ]
        } else if available_width > 90 {
            vec![
                Constraint::Length(3),   // # - number column
                Constraint::Percentage(12),  // Time
                Constraint::Percentage(15),  // Type: PROXIMITY/TRADE
                Constraint::Percentage(8),   // Action
                Constraint::Percentage(18),  // Pair/TF
                Constraint::Percentage(15),  // Price
                Constraint::Percentage(10),  // Distance
                Constraint::Percentage(19),  // Zone ID: Remaining space
            ]
        } else {
            vec![
                Constraint::Length(3),   // # - number column
                Constraint::Percentage(15),  // Time: HH:MM
                Constraint::Percentage(20),  // Type: PROXIM
                Constraint::Percentage(10),  // Action: B/S
                Constraint::Percentage(25),  // Pair: GBPUSD
                Constraint::Percentage(27),  // Price: Remaining space
            ]
        };

        let rows: Vec<Row> = filtered_notifications
            .iter()
            .enumerate()
            .rev() // Reverse to show latest notifications first
            .take(30)
            .rev() // Reverse back to maintain proper display order
            .map(|(display_number, (original_index, notif))| {
                let time_str = if available_width > 90 {
                    notif.timestamp.format("%H:%M:%S").to_string()
                } else {
                    notif.timestamp.format("%H:%M").to_string()
                };

                // Determine notification type and colors (restored from your original)
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
                    if notif.action == "BUY" { "B".to_string() } else { "S".to_string() }
                };

                let pair_tf = if available_width > 90 {
                    format!("{}/{}", notif.symbol, notif.timeframe)
                } else {
                    notif.symbol.clone()
                };

                let price_str = format!("{:.5}", notif.price);

                // Use actual data from the notification (restored colors)
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

                // Highlight selected row (restored from your original style)
                let row_style = if Some(*original_index) == selected_index {
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

                // Build row based on screen size with restored colors
                let mut cells = vec![
                    Cell::from(format!("{}", display_number + 1)).style(row_style.fg(Color::White).add_modifier(Modifier::BOLD)), // Number
                    Cell::from(time_str).style(row_style.fg(Color::Gray)),
                    Cell::from(type_str).style(row_style.fg(type_color).add_modifier(Modifier::BOLD)),
                    Cell::from(action_str).style(row_style.fg(action_color).add_modifier(Modifier::BOLD)),
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

        // Add selection indicator column (restored from your original)
        if !rows.is_empty() {
            let indicator_rows: Vec<Row> = filtered_notifications
                .iter()
                .enumerate()
                .rev() // Reverse to match the main table
                .take(30)
                .rev() // Reverse back to maintain proper display order
                .map(|(_display_index, (original_index, _))| {
                    let indicator = if Some(*original_index) == selected_index {
                        "►"
                    } else {
                        " "
                    };
                    let style = if Some(*original_index) == selected_index {
                        Style::default().bg(Color::DarkGray).fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![Cell::from(indicator).style(style)])
                })
                .collect();

            // Create side-by-side layout: indicator + main table (restored layout)
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
                .block(notif_block)
                .widths(&widths);

            f.render_widget(table, area);
        }
    }
}

fn render_trading_rules_panel(f: &mut Frame, area: Rect) {
    let rules_block = Block::default()
        .borders(Borders::ALL)
        .title("⚙️ Trading Rules Configuration")
        .title_alignment(ratatui::layout::Alignment::Center)
        .border_style(Style::default().fg(Color::Green));

    // Read current config from environment variables
    let config = TradingConfig::default();

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Trading Status: ", Style::default().fg(Color::White)),
            if config.enabled {
                Span::styled("ENABLED", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("DISABLED", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            },
        ]),
        Line::from(""),
        
        Line::from(vec![
            Span::styled("Position Sizing:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Lot Size: ", Style::default().fg(Color::White)),
            Span::styled(format!("{:.0}", config.lot_size), Style::default().fg(Color::Yellow)),
            Span::styled(" | Stop Loss: ", Style::default().fg(Color::White)),
            Span::styled(format!("{:.1} pips", config.stop_loss_pips), Style::default().fg(Color::Red)),
            Span::styled(" | Take Profit: ", Style::default().fg(Color::White)),
            Span::styled(format!("{:.1} pips", config.take_profit_pips), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("  Risk/Reward: ", Style::default().fg(Color::White)),
            Span::styled(format!("{:.1}:1", config.min_risk_reward), Style::default().fg(Color::Magenta)),
        ]),
        Line::from(""),
        
        Line::from(vec![
            Span::styled("Trading Limits:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Max Daily Trades: ", Style::default().fg(Color::White)),
            Span::styled(format!("{}", config.max_daily_trades), Style::default().fg(Color::Yellow)),
            Span::styled(" | Max Per Symbol: ", Style::default().fg(Color::White)),
            Span::styled(format!("{}", config.max_trades_per_symbol), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("  Trading Hours: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{}:00 - {}:00 UTC", config.trading_start_hour, config.trading_end_hour), 
                Style::default().fg(Color::Cyan)
            ),
        ]),
        Line::from(""),
        
        Line::from(vec![
            Span::styled("Quality Filters:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Max Touch Count: ", Style::default().fg(Color::White)),
            Span::styled(format!("{}", config.max_touch_count), Style::default().fg(Color::Gray)),
            Span::styled(" | Trading Distance: ", Style::default().fg(Color::White)),
            Span::styled(format!("≤{:.1} pips", config.trading_threshold_pips), Style::default().fg(Color::Magenta)),
        ]),
        Line::from(vec![
            Span::styled("  Proximity Distance: ", Style::default().fg(Color::White)),
            Span::styled(format!("≤{:.1} pips", config.proximity_threshold_pips), Style::default().fg(Color::Yellow)),
            if config.min_zone_strength > 0.0 {
                Span::styled(format!(" | Min Strength: {:.0}", config.min_zone_strength), Style::default().fg(Color::Blue))
            } else {
                Span::styled(" | Min Strength: Disabled", Style::default().fg(Color::Gray))
            },
        ]),
        Line::from(""),
        
        Line::from(vec![
            Span::styled("Allowed Timeframes:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(config.allowed_timeframes.join(", "), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        
        Line::from(vec![
            Span::styled("Symbols: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} allowed", config.allowed_symbols.len()), Style::default().fg(Color::White)),
        ]),
    ];

    // Show first few symbols to give an idea
    let symbols_preview = if config.allowed_symbols.len() > 8 {
        format!("  {}... +{} more", 
            config.allowed_symbols[..8].join(", "),
            config.allowed_symbols.len() - 8
        )
    } else {
        format!("  {}", config.allowed_symbols.join(", "))
    };
    
    lines.push(Line::from(vec![
        Span::styled(symbols_preview, Style::default().fg(Color::Gray)),
    ]));

    let rules_text = Text::from(lines);

    let rules_display = Paragraph::new(rules_text)
        .block(rules_block)
        .wrap(Wrap { trim: true });

    f.render_widget(rules_display, area);
}

fn render_notification_rule_validation(
    f: &mut Frame,
    area: Rect,
    validation: &Option<NotificationValidation>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("🔍 Rule Validation Results")
        .title_alignment(ratatui::layout::Alignment::Center)
        .border_style(Style::default().fg(Color::Blue));

    if let Some(validation) = validation {
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("Notification #{} Analysis", validation.notification_id + 1),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        for rule in &validation.rules {
            let status_color = if rule.passed { Color::Green } else { Color::Red };
            let status_symbol = if rule.passed { "✓" } else { "✗" };

            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", status_symbol),
                    Style::default().fg(status_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}: ", rule.rule_name),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if rule.passed { "PASS" } else { "FAIL" },
                    Style::default().fg(status_color).add_modifier(Modifier::BOLD),
                ),
            ]));

            if let (Some(value), Some(threshold)) = (&rule.value, &rule.threshold) {
                lines.push(Line::from(vec![
                    Span::raw("  Value: "),
                    Span::styled(value.clone(), Style::default().fg(Color::Yellow)),
                    Span::raw(" | Req: "),
                    Span::styled(threshold.clone(), Style::default().fg(Color::Cyan)),
                ]));
            }

            if let Some(details) = &rule.details {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(details.clone(), Style::default().fg(Color::Red)),
                ]));
            }

            lines.push(Line::from(""));
        }

        // Overall result
        let overall_color = if validation.overall_valid { Color::Green } else { Color::Red };
        let overall_text = if validation.overall_valid { 
            "✓ TRADE EXECUTABLE" 
        } else { 
            "✗ TRADE BLOCKED" 
        };
        
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "Final Decision: ",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                overall_text,
                Style::default().fg(overall_color).add_modifier(Modifier::BOLD),
            ),
        ]));

        if let Some(reason) = &validation.reason {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(reason.clone(), Style::default().fg(Color::Gray)),
            ]));
        }

        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    } else {
        let help_lines = vec![
            Line::from(vec![
                Span::styled("Select a notification to analyze", Style::default().fg(Color::Gray)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Navigation:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("  ↑↓ ", Style::default().fg(Color::Yellow)),
                Span::styled("Navigate notifications", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  1-9 ", Style::default().fg(Color::Yellow)),
                Span::styled("Jump to notification number", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  Enter ", Style::default().fg(Color::Yellow)),
                Span::styled("Force validation refresh", Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("The validation will show if each", Style::default().fg(Color::Gray)),
            ]),
            Line::from(vec![
                Span::styled("trading rule passes or fails based", Style::default().fg(Color::Gray)),
            ]),
            Line::from(vec![
                Span::styled("on your current configuration.", Style::default().fg(Color::Gray)),
            ]),
        ];

        let help_text = Text::from(help_lines);
        let paragraph = Paragraph::new(help_text)
            .block(block)
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    }
}

fn render_enhanced_validated_trades(
    f: &mut Frame,
    area: Rect,
    validated_trades: &[ValidatedTradeDisplay],
) {
    let items: Vec<ListItem> = validated_trades
        .iter()
        .map(|trade| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} ", trade.timestamp.format("%H:%M:%S")),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    format!("{} {} ", trade.symbol, trade.direction),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("@ {:.5}", trade.entry_price),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    format!(" SL:{:.5}", trade.stop_loss),
                    Style::default().fg(Color::Red),
                ),
                Span::styled(
                    format!(" TP:{:.5}", trade.take_profit),
                    Style::default().fg(Color::Green),
                ),
            ]))
        })
        .collect();

    let trades_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Validated Trades")
        );

    f.render_widget(trades_list, area);
}