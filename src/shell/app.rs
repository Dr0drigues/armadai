//! Main shell entry point and event loop integration.
//!
//! Ties together the runner (will be created in parallel) and TUI.

#![cfg(feature = "tui")]

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::time::Duration;

use super::tui::ShellApp;

/// Main shell entry point.
pub async fn run_shell() -> Result<()> {
    // Detect provider (for now, hardcode gemini check)
    let provider_name = "Gemini";
    let command = "gemini";

    // Check if gemini is available
    let which_result = std::process::Command::new("which").arg(command).output();

    if which_result.is_err() || !which_result.unwrap().status.success() {
        anyhow::bail!(
            "No supported CLI tool found. Install gemini, claude, or aider. \
             To start the shell: armadai shell"
        );
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = ShellApp::new(provider_name.to_string());

    // Event loop
    let result = event_loop(&mut terminal, &mut app).await;

    // Cleanup
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ShellApp,
) -> Result<()> {
    loop {
        // Render
        terminal.draw(|f| app.render(f))?;

        // Handle events
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            // Only process Press events, ignore repeats
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if app.handle_key(key) {
                break; // quit
            }

            // Check if user submitted input (Enter key)
            if key.code == KeyCode::Enter
                && !app.should_quit()
                && let Some(input) = app.take_input()
            {
                app.add_user_message(&input);
                app.set_loading(true);

                // Force redraw to show "thinking..."
                terminal.draw(|f| app.render(f))?;

                // TODO: Execute turn here when runner module is ready
                // For now, just simulate a response
                tokio::time::sleep(Duration::from_millis(500)).await;

                let response = format!("Echo: {}", input);
                app.add_assistant_message(&response);
                app.update_metrics(10, 20, 0.0, Duration::from_millis(500));
                app.set_loading(false);
            }
        }
    }

    Ok(())
}
