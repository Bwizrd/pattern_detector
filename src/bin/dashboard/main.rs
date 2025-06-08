// src/bin/dashboard/main.rs - Complete main entry point with zone navigation
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::time::{Duration, Instant};

use std::env;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

mod app;
mod notifications;
mod types;
mod ui;
#[path = "../zone_monitor/websocket.rs"]
mod websocket;

use app::App;
use types::{AppPage, PriceUpdate}; // Add PriceUpdate
use ui::ui;
use websocket::WebSocketClient; // Add this

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    price_rx: &mut mpsc::UnboundedReceiver<PriceUpdate>,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(1000);

    loop {
        // CRITICAL: Process WebSocket price updates FIRST
        while let Ok(price_update) = price_rx.try_recv() {
            app.update_price_from_websocket(price_update);
        }

        // Render UI
        terminal.draw(|f| ui(f, app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Handle strength input mode first
                if app.strength_input_mode {
                    match key.code {
                        KeyCode::Enter => {
                            app.toggle_strength_input_mode(); // This will save the input
                        }
                        KeyCode::Esc => {
                            app.cancel_strength_input();
                        }
                        KeyCode::Backspace => {
                            app.handle_strength_backspace();
                        }
                        KeyCode::Char(c) => {
                            app.handle_strength_input(c);
                        }
                        _ => {}
                    }
                } else {
                    // Normal key handling when not in strength input mode
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('r') => {
                            app.update_data().await;
                        }
                        // KeyCode::Char('c') => {
                        //     app.clear_notifications_via_api().await;
                        //     app.update_data().await;
                        // }
                        KeyCode::Char(c)
                            if c.is_ascii_digit()
                                && app.current_page == AppPage::NotificationMonitor =>
                        {
                            let num = c.to_digit(10).unwrap() as usize;
                            app.select_notification_by_number(num);
                        }

                        // ADD this case for manual validation trigger:
                        KeyCode::Enter if app.current_page == AppPage::NotificationMonitor => {
                            app.validate_selected_notification();
                        }
                        KeyCode::Char('d') => {
                            app.switch_page(AppPage::Dashboard);
                        }
                        KeyCode::Char('n') => {
                            app.switch_page(AppPage::NotificationMonitor);
                        }
                        // NEW: Add 'p' key for Prices page
                        KeyCode::Char('p') => {
                            app.switch_to_prices_page();
                        }
                        // Navigation keys - work on all pages but for different content
                        KeyCode::Up => match app.current_page {
                            AppPage::Dashboard => {
                                app.select_previous_zone();
                            }
                            AppPage::NotificationMonitor => {
                                app.select_previous_notification();
                            }
                            AppPage::Prices => {
                                // Could add price selection later
                            }
                        },
                        KeyCode::Down => match app.current_page {
                            AppPage::Dashboard => {
                                app.select_next_zone();
                            }
                            AppPage::NotificationMonitor => {
                                app.select_next_notification();
                            }
                            AppPage::Prices => {
                                // Could add price selection later
                            }
                        },
                        KeyCode::Char('y') => match app.current_page {
                            AppPage::Dashboard => {
                                app.copy_selected_dashboard_zone_id();
                            }
                            AppPage::NotificationMonitor => {
                                app.copy_selected_zone_id();
                            }
                            AppPage::Prices => {
                                // Could add price copying later
                            }
                        },
                        // Strength filter toggle (only on dashboard)
                        KeyCode::Char('s') => {
                            if app.current_page == AppPage::Dashboard {
                                app.toggle_strength_input_mode();
                            }
                        }
                        // Timeframe toggles work on dashboard and notifications
                        KeyCode::Char('1') => {
                            if app.current_page != AppPage::Prices {
                                app.toggle_timeframe("5m");
                            }
                        }
                        KeyCode::Char('2') => {
                            if app.current_page != AppPage::Prices {
                                app.toggle_timeframe("15m");
                            }
                        }
                        KeyCode::Char('3') => {
                            if app.current_page != AppPage::Prices {
                                app.toggle_timeframe("30m");
                            }
                        }
                        KeyCode::Char('4') => {
                            if app.current_page != AppPage::Prices {
                                app.toggle_timeframe("1h");
                            }
                        }
                        KeyCode::Char('5') => {
                            if app.current_page != AppPage::Prices {
                                app.toggle_timeframe("4h");
                            }
                        }
                        KeyCode::Char('6') => {
                            if app.current_page != AppPage::Prices {
                                app.toggle_timeframe("1d");
                            }
                        }
                        // Breached toggle only works on dashboard
                        KeyCode::Char('b') => {
                            if app.current_page == AppPage::Dashboard {
                                app.toggle_breached();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.update_data().await;
            last_tick = Instant::now();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Set up WebSocket connection for live prices
    let ws_url = env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:8081".to_string());
    let (price_tx, mut price_rx) = mpsc::unbounded_channel::<PriceUpdate>();

    // Start WebSocket client in background
    let ws_client = WebSocketClient::new();
    let ws_url_clone = ws_url.clone();
    let price_tx_clone = price_tx.clone();

    tokio::spawn(async move {
        ws_client
            .start_connection_loop(ws_url_clone, move |price_update| {
                if let Err(e) = price_tx_clone.send(price_update) {
                    warn!("Failed to send price update: {}", e);
                }
            })
            .await;
    });

    info!(
        "🚀 Starting Dashboard with WebSocket connection to {}",
        ws_url
    );

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.update_data().await;

    let res = run_app(&mut terminal, &mut app, &mut price_rx).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}
