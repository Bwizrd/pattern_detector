// src/bin/ratatui_dashboard.rs - Main dashboard entry point
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
                    KeyCode::Char('b') => {
                        app.toggle_breached();
                    }
                    _ => {}
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