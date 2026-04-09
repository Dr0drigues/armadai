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

use super::runner::ShellRunner;
use super::session::{SessionMessage, ShellSession};
use super::tui::ShellApp;

/// Helper to save the current session state.
fn save_current_session(
    session_id: &str,
    project_dir: &str,
    provider_name: &str,
    model_name: &str,
    runner: &ShellRunner,
) -> Result<()> {
    let metrics = runner.session_metrics();
    let now = chrono::Utc::now().to_rfc3339();

    // Convert runner history to session messages
    let messages: Vec<SessionMessage> = runner
        .history()
        .iter()
        .map(SessionMessage::from_message)
        .collect();

    // Don't save if there are no messages
    if messages.is_empty() {
        return Ok(());
    }

    let session = ShellSession {
        id: session_id.to_string(),
        name: super::session::generate_session_name(project_dir),
        provider: provider_name.to_string(),
        model: model_name.to_string(),
        project_dir: project_dir.to_string(),
        created_at: now.clone(),
        updated_at: now,
        messages,
        total_tokens_in: metrics.total_tokens_in,
        total_tokens_out: metrics.total_tokens_out,
        total_cost: metrics.total_cost_estimate,
        turn_count: metrics.turn_count,
    };

    super::session::save_session(&session)
}

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
    // Run wizard to ensure project is ready
    let wizard_result = super::wizard::ensure_project_ready()?;

    // Use wizard result for provider config
    let config = super::runner::RunnerConfig {
        command: wizard_result.provider_command.clone(),
        args: wizard_result.provider_args,
        max_history_turns: 5,
        timeout: std::time::Duration::from_secs(120),
    };

    let provider_name = super::detect::provider_display_name(&config.command).to_string();

    // Generate a new session ID
    let session_id = super::session::new_session_id();
    let project_dir = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .to_string_lossy()
        .to_string();

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

    let mut app = ShellApp::new(provider_name.clone());
    app.set_model_name(wizard_result.model_name.clone());
    let mut runner = ShellRunner::new(config);

    // Event loop
    let result = event_loop(
        &mut terminal,
        &mut app,
        &mut runner,
        &session_id,
        &project_dir,
        &provider_name,
        &wizard_result.model_name,
    )
    .await;

    // Final save on exit
    let _ = save_current_session(
        &session_id,
        &project_dir,
        &provider_name,
        &wizard_result.model_name,
        &runner,
    );

    // Cleanup
    restore_terminal();
    println!();

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ShellApp,
    runner: &mut ShellRunner,
    session_id: &str,
    project_dir: &str,
    provider_name: &str,
    model_name: &str,
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

        // Check for slash commands first
        if let Some(result) =
            super::commands::try_execute(&input, runner, app.provider_name(), app.model_name())
        {
            use super::commands::CommandResult;
            match result {
                CommandResult::Display(text) => {
                    app.show_popup(text);
                }
                CommandResult::Clear => {
                    app.clear_conversation();
                    runner.clear();
                }
                CommandResult::Quit => {
                    break;
                }
                CommandResult::SwitchProvider(provider_name_arg) => {
                    if provider_name_arg.is_empty() {
                        app.show_popup(
                            "Usage: /switch <provider>\nExample: /switch claude".to_string(),
                        );
                        continue;
                    }

                    // Find the provider in the registry
                    let providers = super::detect::list_providers();
                    if let Some(provider) = providers.iter().find(|p| {
                        p.command == provider_name_arg
                            || p.display_name.to_lowercase() == provider_name_arg.to_lowercase()
                    }) {
                        if !provider.available {
                            app.show_popup(format!("Provider '{}' is not available. Make sure '{}' is installed and in your PATH.", provider.display_name, provider.command));
                            continue;
                        }

                        // Switch the provider
                        runner.switch_provider(provider.command.clone(), provider.args.clone());
                        app.set_provider_name(provider.display_name.clone());
                        app.set_model_name(provider.model_name.clone());

                        app.show_popup(format!(
                            "Switched to {} ({})\nModel: {}",
                            provider.display_name, provider.command, provider.model_name
                        ));
                    } else {
                        let available: Vec<String> = providers
                            .iter()
                            .filter(|p| p.available)
                            .map(|p| p.command.clone())
                            .collect();
                        app.show_popup(format!("Unknown provider: '{}'\nAvailable providers: {}\nUse /providers to see all options.", provider_name_arg, available.join(", ")));
                    }
                    continue;
                }
                CommandResult::ResumeSession(id) => {
                    match super::session::load_session(&id) {
                        Ok(session) => {
                            // Restore messages to runner
                            let messages: Vec<super::runner::Message> =
                                session.messages.iter().map(|m| m.to_message()).collect();

                            runner.restore_from_session(messages);

                            // Clear and restore UI
                            app.clear_conversation();
                            for msg in &session.messages {
                                match msg.role.as_str() {
                                    "user" => app.add_user_message(&msg.content),
                                    "assistant" => app.add_assistant_message(&msg.content),
                                    _ => {}
                                }
                            }

                            // Restore metrics
                            app.set_session_metrics(
                                session.total_tokens_in,
                                session.total_tokens_out,
                                session.total_cost,
                                session.turn_count,
                                std::time::Duration::from_secs(0),
                            );

                            app.show_popup(format!(
                                "Resumed session: {}\n{} turns, ${:.4}",
                                session.name, session.turn_count, session.total_cost
                            ));
                        }
                        Err(e) => {
                            app.show_popup(format!(
                                "Failed to resume session '{}': {}\n\nUse /sessions to see available sessions.",
                                id, e
                            ));
                        }
                    }
                    continue;
                }
                CommandResult::SaveSession => {
                    match save_current_session(
                        session_id,
                        project_dir,
                        provider_name,
                        model_name,
                        runner,
                    ) {
                        Ok(_) => {
                            app.show_popup(format!(
                                "Session saved: {}\n\nUse /resume {} to restore this session later.",
                                session_id, session_id
                            ));
                        }
                        Err(e) => {
                            app.show_popup(format!("Failed to save session: {}", e));
                        }
                    }
                    continue;
                }
            }
            continue;
        }

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

                    // Auto-save session after each turn
                    let _ = save_current_session(
                        session_id,
                        project_dir,
                        provider_name,
                        model_name,
                        runner,
                    );
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    app.add_assistant_message(&format!(
                        "Error (exit {}): {}",
                        output.status, stderr
                    ));
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
