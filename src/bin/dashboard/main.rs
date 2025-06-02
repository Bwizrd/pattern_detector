// src/bin/dashboard/main.rs - Complete main entry point with zone navigation
use std::io;
use tokio::time::{Duration, Instant};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};

mod types;
mod app;
mod ui;

use app::App;
use types::AppPage;
use ui::ui;

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> io::Result<()> {
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(1000);

    loop {
        terminal.draw(|f| ui(f, &app))?;

        let timeout = tick_rate.checked_sub(last_tick.elapsed()).unwrap_or_else(|| Duration::from_secs(0));

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
                        KeyCode::Char('c') => {
                            app.clear_notifications_via_api().await;
                            app.update_data().await;
                        }
                        KeyCode::Char('d') => {
                            app.switch_page(AppPage::Dashboard);
                        }
                        KeyCode::Char('n') => {
                            app.switch_page(AppPage::NotificationMonitor);
                        }
                        // Navigation keys - work on both pages but for different content
                        KeyCode::Up => {
                            match app.current_page {
                                AppPage::Dashboard => {
                                    app.select_previous_zone();
                                }
                                AppPage::NotificationMonitor => {
                                    app.select_previous_notification();
                                }
                            }
                        }
                        KeyCode::Down => {
                            match app.current_page {
                                AppPage::Dashboard => {
                                    app.select_next_zone();
                                }
                                AppPage::NotificationMonitor => {
                                    app.select_next_notification();
                                }
                            }
                        }
                        KeyCode::Char('y') => {
                            match app.current_page {
                                AppPage::Dashboard => {
                                    app.copy_selected_dashboard_zone_id();
                                }
                                AppPage::NotificationMonitor => {
                                    app.copy_selected_zone_id();
                                }
                            }
                        }
                        // Strength filter toggle (only on dashboard)
                        KeyCode::Char('s') => {
                            if app.current_page == AppPage::Dashboard {
                                app.toggle_strength_input_mode();
                            }
                        }
                        // Timeframe toggles work on both pages
                        KeyCode::Char('1') => {
                            app.toggle_timeframe("5m");
                        }
                        KeyCode::Char('2') => {
                            app.toggle_timeframe("15m");
                        }
                        KeyCode::Char('3') => {
                            app.toggle_timeframe("30m");
                        }
                        KeyCode::Char('4') => {
                            app.toggle_timeframe("1h");
                        }
                        KeyCode::Char('5') => {
                            app.toggle_timeframe("4h");
                        }
                        KeyCode::Char('6') => {
                            app.toggle_timeframe("1d");
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

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.update_data().await;

    let res = run_app(&mut terminal, app).await;

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