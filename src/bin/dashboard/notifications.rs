// src/bin/dashboard/notifications.rs
// Enhanced notifications system with rule validation - FINAL FIXED VERSION

use crate::types::{TradeNotificationDisplay, ValidatedTradeDisplay};
use chrono::Timelike;
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
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
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            lot_size: 1000.0,
            stop_loss_pips: 20.0,
            take_profit_pips: 40.0,
            max_daily_trades: 10,
            max_trades_per_symbol: 2,
            allowed_symbols: vec![
                "EURUSD".to_string(), "GBPUSD".to_string(), "USDJPY".to_string(),
                "USDCHF".to_string(), "AUDUSD".to_string(), "USDCAD".to_string(),
                "NZDUSD".to_string(), "EURGBP".to_string(), "EURJPY".to_string(),
                "EURCHF".to_string(), "EURAUD".to_string(), "EURCAD".to_string(),
                "EURNZD".to_string(), "GBPJPY".to_string(), "GBPCHF".to_string(),
                "GBPAUD".to_string(), "GBPCAD".to_string(), "GBPNZD".to_string(),
                "AUDJPY".to_string(), "AUDNZD".to_string(), "AUDCAD".to_string(),
                "NZDJPY".to_string(), "CADJPY".to_string(), "CHFJPY".to_string(),
            ],
            allowed_timeframes: vec!["30m".to_string(), "1h".to_string(), "4h".to_string()],
            min_risk_reward: 1.5,
            trading_start_hour: 8,
            trading_end_hour: 17,
            min_zone_strength: 0.7,
            max_touch_count: 3,
            min_distance_pips: 10.0,
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

        // Rule 5: Zone Strength Check (if available)
        if let Some(strength) = notification.strength {
            let strength_ok = strength >= self.config.min_zone_strength;
            rules.push(RuleValidationResult {
                rule_name: "Zone Strength".to_string(),
                description: format!("Zone strength must be >= {:.2}", self.config.min_zone_strength),
                passed: strength_ok,
                value: Some(format!("{:.3}", strength)),
                threshold: Some(format!("{:.2}", self.config.min_zone_strength)),
                details: if !strength_ok { 
                    Some("Zone strength too low".to_string()) 
                } else { None },
            });
        }

        // Rule 6: Touch Count Check (if available)
        if let Some(touch_count) = notification.touch_count {
            let touch_ok = (touch_count as u32) <= self.config.max_touch_count;
            rules.push(RuleValidationResult {
                rule_name: "Touch Count".to_string(),
                description: format!("Touch count must be <= {}", self.config.max_touch_count),
                passed: touch_ok,
                value: Some(touch_count.to_string()),
                threshold: Some(format!("≤ {}", self.config.max_touch_count)),
                details: if !touch_ok { 
                    Some("Zone touched too many times".to_string()) 
                } else { None },
            });
        }

        // Rule 7: Distance Check (if available)
        if let Some(distance_pips) = notification.distance_pips {
            let distance_ok = distance_pips >= self.config.min_distance_pips;
            rules.push(RuleValidationResult {
                rule_name: "Minimum Distance".to_string(),
                description: format!("Distance must be >= {:.1} pips", self.config.min_distance_pips),
                passed: distance_ok,
                value: Some(format!("{:.1} pips", distance_pips)),
                threshold: Some(format!("{:.1} pips", self.config.min_distance_pips)),
                details: if !distance_ok { 
                    Some("Too close to zone".to_string()) 
                } else { None },
            });
        }

        // Rule 8: Risk/Reward Ratio Check (mirrors your generate_trade_from_signal function)
        let risk = self.config.stop_loss_pips;
        let reward = self.config.take_profit_pips;
        let rr_ratio = reward / risk;
        let rr_ok = rr_ratio >= self.config.min_risk_reward;
        rules.push(RuleValidationResult {
            rule_name: "Risk/Reward Ratio".to_string(),
            description: format!("R:R must be >= {:.1}", self.config.min_risk_reward),
            passed: rr_ok,
            value: Some(format!("{:.2}", rr_ratio)),
            threshold: Some(format!("{:.1}", self.config.min_risk_reward)),
            details: if !rr_ok { 
                Some("Risk/reward ratio too low".to_string()) 
            } else { None },
        });

        // Rule 9: Signal Action Validation
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

        // Rule 10: Notification Type Check (prefer TRADE over PROXIMITY)
        let type_score = match notification.notification_type.as_str() {
            "TRADE" => 100,
            "zone_trigger" => 90,
            "PROXIMITY" => 70,
            _ => 50,
        };
        let type_ok = type_score >= 70;
        rules.push(RuleValidationResult {
            rule_name: "Signal Quality".to_string(),
            description: "Signal type quality assessment".to_string(),
            passed: type_ok,
            value: Some(notification.notification_type.clone()),
            threshold: Some("High quality signal".to_string()),
            details: if !type_ok { 
                Some("Low quality signal type".to_string()) 
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
    validated_trades: &[ValidatedTradeDisplay],
    selected_index: Option<usize>,
    validation: &Option<NotificationValidation>,
    timeframe_filters: &HashMap<String, bool>,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // Left panel: Enhanced notifications table with numbers
    render_numbered_notifications_table(f, chunks[0], notifications, selected_index, timeframe_filters);

    // Right panel: Split into validation and trades
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(chunks[1]);

    // Top right: Rule validation
    render_notification_rule_validation(f, right_chunks[0], validation);

    // Bottom right: Validated trades
    render_enhanced_validated_trades(f, right_chunks[1], validated_trades);
}

fn render_numbered_notifications_table(
    f: &mut Frame,
    area: Rect,
    notifications: &[TradeNotificationDisplay],
    selected_index: Option<usize>,
    timeframe_filters: &HashMap<String, bool>,
) {
    let header_cells = ["#", "Time", "Type", "Action", "Pair/TF", "Price", "Dist", "Str", "Touch", "Zone ID"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));
    let header = Row::new(header_cells)
        .style(Style::default().bg(Color::DarkGray))
        .height(1)
        .bottom_margin(1);

    // Filter notifications by enabled timeframes and create numbered list
    let filtered_notifications: Vec<(usize, &TradeNotificationDisplay)> = notifications
        .iter()
        .enumerate()
        .filter(|(_, notif)| {
            timeframe_filters.get(&notif.timeframe).copied().unwrap_or(true)
        })
        .collect();

    let rows: Vec<Row> = filtered_notifications
        .iter()
        .enumerate()
        .map(|(display_number, (actual_index, notification))| {
            let cells = vec![
                Cell::from(format!("{}", display_number + 1)), // Display number (1-based)
                Cell::from(notification.timestamp.format("%H:%M:%S").to_string()),
                Cell::from(notification.notification_type.clone()),
                Cell::from(notification.action.clone()),
                Cell::from(format!("{}/{}", notification.symbol, notification.timeframe)),
                Cell::from(format!("{:.5}", notification.price)),
                Cell::from(
                    notification.distance_pips
                        .map(|d| format!("{:.1}", d))
                        .unwrap_or_else(|| "-".to_string())
                ),
                Cell::from(
                    notification.strength
                        .map(|s| format!("{:.2}", s))
                        .unwrap_or_else(|| "-".to_string())
                ),
                Cell::from(
                    notification.touch_count
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "-".to_string())
                ),
                Cell::from(
                    notification.signal_id
                        .as_ref()
                        .map(|id| id.clone())
                        .unwrap_or_else(|| "-".to_string())
                ),
            ];

            let style = if Some(*actual_index) == selected_index {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                Style::default()
            };

            Row::new(cells).style(style).height(1)
        })
        .collect();

    let table = Table::new(rows)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Notifications (↑↓ select, 1-9 direct, Enter validate)")
        )
        .widths(&[
            Constraint::Length(3),  // #
            Constraint::Length(8),  // Time
            Constraint::Length(8),  // Type
            Constraint::Length(6),  // Action
            Constraint::Length(10), // Pair/TF
            Constraint::Length(8),  // Price
            Constraint::Length(5),  // Dist
            Constraint::Length(4),  // Str
            Constraint::Length(5),  // Touch
            Constraint::Length(8),  // Zone ID
        ])
        .column_spacing(1);

    f.render_widget(table, area);
}

fn render_notification_rule_validation(
    f: &mut Frame,
    area: Rect,
    validation: &Option<NotificationValidation>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Trading Rule Validation");

    if let Some(validation) = validation {
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("Notification #{} Validation", validation.notification_id + 1),
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
            "✓ EXECUTABLE" 
        } else { 
            "✗ NOT EXECUTABLE" 
        };
        
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "Final Result: ",
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
        let paragraph = Paragraph::new("Select a notification number to view trading rule validation")
            .block(block)
            .style(Style::default().fg(Color::Gray));

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