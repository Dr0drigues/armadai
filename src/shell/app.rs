//! Main shell entry point and event loop integration.

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

use super::detect::detect_provider;
use super::runner::ShellRunner;
use super::tui::ShellApp;

/// Restore the terminal to normal state. Called on exit and on panic.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

/// Main shell entry point.
pub async fn run_shell() -> Result<()> {
    let config = detect_provider().ok_or_else(|| {
        anyhow::anyhow!(
            "No supported CLI tool found. Install gemini, claude, or aider.\n\
             Supported tools: gemini, claude, aider, codex"
        )
    })?;

    let provider_name = super::detect::provider_display_name(&config.command).to_string();

    // Install panic hook to restore terminal on crash
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_panic(info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::cursor::Hide,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let model_name = super::detect::detect_model_name(&config.command);
    let mut app = ShellApp::new(provider_name);
    app.set_model_name(model_name);
    let mut runner = ShellRunner::new(config);

    // Event loop
    let result = event_loop(&mut terminal, &mut app, &mut runner).await;

    // Cleanup
    restore_terminal();
    println!();

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ShellApp,
    runner: &mut ShellRunner,
) -> Result<()> {
    loop {
        // Render
        terminal.draw(|f| app.render(f))?;

        // Handle events
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let evt = event::read()?;

        // Handle mouse scroll
        if let Event::Mouse(mouse) = &evt {
            app.handle_mouse(*mouse);
            continue;
        }

        // Handle key events
        let key = match evt {
            Event::Key(key) if key.kind == KeyEventKind::Press => key,
            _ => continue,
        };

        if app.handle_key(key) {
            break;
        }

        // Check if user submitted input
        if key.code != KeyCode::Enter || app.should_quit() {
            continue;
        }
        let Some(input) = app.take_input() else {
            continue;
        };

        app.add_user_message(&input);
        app.set_loading(true);

        let input_clone = input.clone();
        let (tx, mut rx) = tokio::sync::oneshot::channel();

        let cmd = runner.command().to_string();
        let args: Vec<String> = runner.args().to_vec();
        let prompt = runner.build_prompt_for(&input_clone);
        let prompt_for_spawn = prompt.clone();

        // Spawn the CLI call in background
        let handle = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let output = tokio::process::Command::new(&cmd)
                .args(&args)
                .arg(&prompt_for_spawn)
                .output()
                .await;
            let _ = tx.send((output, start.elapsed()));
        });

        // Keep rendering while waiting (spinner animation)
        loop {
            app.tick_spinner();
            terminal.draw(|f| app.render(f))?;

            // Check for cancel during loading
            if event::poll(Duration::from_millis(80))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && (key.code == KeyCode::Esc
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers == crossterm::event::KeyModifiers::CONTROL))
            {
                handle.abort();
                app.set_loading(false);
                app.add_assistant_message("[Cancelled]");
                break;
            }

            // Check if response arrived
            let Ok((output_result, duration)) = rx.try_recv() else {
                continue;
            };

            match output_result {
                Ok(output) if output.status.success() => {
                    let raw = String::from_utf8_lossy(&output.stdout).to_string();
                    let parsed = super::parser::parse_response(&raw);
                    runner.record_turn(&input_clone, &parsed.content, duration);
                    let metrics = runner.session_metrics();
                    app.add_assistant_message(&parsed.content);
                    app.set_session_metrics(
                        metrics.total_tokens_in,
                        metrics.total_tokens_out,
                        metrics.total_cost_estimate,
                        metrics.turn_count,
                        duration,
                    );
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    app.add_assistant_message(&format!("Error (exit {}): {}", output.status, stderr));
                }
                Err(e) => {
                    app.add_assistant_message(&format!("Error: {e}"));
                }
            }
            app.set_loading(false);
            break;
        }
    }
    Ok(())
}
