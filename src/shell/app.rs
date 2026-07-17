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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::runner::ShellRunner;
use super::session::{SessionMessage, ShellSession};
use super::tui::ShellApp;

/// Check if a key event is a cancel key (Esc or Ctrl+C).
fn is_cancel_key(key: &event::KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c')
            && key.modifiers == crossterm::event::KeyModifiers::CONTROL)
}

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
    if let Err(e) = disable_raw_mode() {
        tracing::warn!("Failed to disable raw mode: {:?}", e);
    }
    if let Err(e) = execute!(
        io::stdout(),
        crossterm::event::DisableBracketedPaste,
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    ) {
        tracing::warn!("Failed to restore terminal state: {:?}", e);
    }
}

/// Main shell entry point.
pub async fn run_shell() -> Result<()> {
    // Run wizard to ensure project is ready
    let wizard_result = super::wizard::ensure_project_ready()?;

    // Load shell config from project config (if available)
    let shell_config = crate::core::project::find_project_config()
        .and_then(|(_, cfg)| cfg.shell)
        .unwrap_or_default();

    // Build runner config: project config overrides wizard defaults
    let command = shell_config
        .default_provider
        .clone()
        .unwrap_or(wizard_result.provider_command.clone());
    let base_args = super::detect::args_for_provider(&command);

    // Resolve model and inject CLI flags if needed
    let model_str = shell_config
        .default_model
        .as_deref()
        .unwrap_or("latest:pro");
    let resolved_model = super::config::resolve_shell_model(&command, model_str);
    let mut args = base_args;
    args.extend(super::config::model_cli_args(&command, &resolved_model));

    let config = super::runner::RunnerConfig {
        command: command.clone(),
        args,
        max_history_turns: shell_config.effective_max_history(),
        timeout: shell_config.effective_timeout(),
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
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = ShellApp::new(provider_name.clone());
    app.set_model_name(resolved_model.clone());

    // Initialize workroom from project orchestration config
    if let Ok(config_content) = std::fs::read_to_string(".armadai/config.yaml")
        .or_else(|_| std::fs::read_to_string("armadai.yaml"))
    {
        app.workroom.init_from_config(&config_content);
    }

    let mut runner = ShellRunner::new(config);

    // Event loop
    let result = event_loop(
        &mut terminal,
        &mut app,
        &mut runner,
        &session_id,
        &project_dir,
        &provider_name,
        &resolved_model,
        &shell_config,
    )
    .await;

    // Final save on exit
    if let Err(e) = save_current_session(
        &session_id,
        &project_dir,
        &provider_name,
        &wizard_result.model_name,
        &runner,
    ) {
        tracing::warn!("Failed to save session on exit: {:?}", e);
    }

    // Cleanup
    restore_terminal();
    println!();

    result
}

#[allow(clippy::too_many_arguments)]
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ShellApp,
    runner: &mut ShellRunner,
    session_id: &str,
    project_dir: &str,
    provider_name: &str,
    model_name: &str,
    shell_config: &super::config::ShellConfig,
) -> Result<()> {
    loop {
        // Render
        terminal.draw(|f| app.render(f))?;

        // Handle events
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let evt = event::read()?;

        // Handle paste (bracketed paste mode)
        if let Event::Paste(text) = &evt {
            // Replace newlines with spaces for single-line input
            let clean = text.replace('\n', " ").replace('\r', "");
            for c in clean.chars() {
                let byte_idx = app.char_to_byte_pub(app.cursor_pos());
                app.insert_char_at(byte_idx, c);
            }
            continue;
        }

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
        if let Some(result) = super::commands::try_execute(
            &input,
            runner,
            app.provider_name(),
            app.model_name(),
            shell_config,
        ) {
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
                CommandResult::Tandem(providers) => {
                    app.set_tandem(providers.clone());
                    app.show_popup(format!(
                        "# Tandem Mode\n\nNext message will be sent to **{}** in parallel.\n\nType your message and press Enter.",
                        providers.join(", ")
                    ));
                    continue;
                }
                CommandResult::Pipeline(providers) => {
                    app.set_pipeline(providers.clone());
                    app.show_popup(format!(
                        "# Pipeline Mode\n\nNext message: **{}** generates → **{}** reviews.\n\nType your message and press Enter.",
                        providers.first().unwrap_or(&"?".to_string()),
                        providers.get(1).unwrap_or(&"?".to_string()),
                    ));
                    continue;
                }
                CommandResult::ToggleWorkroom => {
                    app.workroom.toggle_pin();
                    let status = if app.workroom.is_pinned() {
                        "pinned (always visible)"
                    } else {
                        "auto (visible during orchestration)"
                    };
                    app.show_popup(format!("# Workroom\n\nPanel is now **{}**.", status));
                    continue;
                }
                CommandResult::TogglePty => {
                    app.toggle_pty_mode();
                    if app.is_pty_mode() {
                        app.show_popup("# PTY Mode Enabled\n\nMessages will be sent through interactive CLI.\nThe CLI reads project agents and can delegate natively.\n\n**Note:** Response parsing may include CLI UI artifacts.".to_string());
                    } else {
                        app.show_popup(
                            "# PTY Mode Disabled\n\nBack to one-shot mode with JSON metrics."
                                .to_string(),
                        );
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

        // PTY mode execution
        if app.is_pty_mode() {
            execute_pty_turn(
                terminal,
                app,
                runner,
                &input,
                session_id,
                project_dir,
                provider_name,
                model_name,
            )
            .await?;
            continue;
        }

        // Check for explicit tandem/pipeline mode (from /tandem or /pipeline command)
        if let Some(provider_names) = app.take_tandem() {
            execute_tandem(
                terminal,
                app,
                runner,
                &input,
                &provider_names,
                session_id,
                project_dir,
                provider_name,
                model_name,
            )
            .await?;
            continue;
        }
        if let Some(provider_names) = app.take_pipeline() {
            // Build implicit steps from provider names (one step per provider, no agent, no custom prompt)
            let steps: Vec<super::config::PipelineStep> = provider_names
                .iter()
                .map(|p| super::config::PipelineStep {
                    name: p.clone(),
                    prompt: None,
                    providers: vec![super::config::ShellProviderEntry {
                        provider: p.clone(),
                        model: None,
                        agent: None,
                    }],
                })
                .collect();
            execute_pipeline_steps(
                terminal,
                app,
                runner,
                &input,
                &steps,
                session_id,
                project_dir,
                provider_name,
                model_name,
            )
            .await?;
            continue;
        }

        // Auto-pipeline from config (if pipeline steps are configured)
        if let Some(ref pipeline) = shell_config.pipeline
            && !pipeline.steps.is_empty()
        {
            execute_pipeline_steps(
                terminal,
                app,
                runner,
                &input,
                &pipeline.steps,
                session_id,
                project_dir,
                provider_name,
                model_name,
            )
            .await?;
            continue;
        }

        // Normal single-provider execution
        app.set_loading(true);
        app.start_streaming_response();

        let input_clone = input.clone();
        let cmd = runner.command().to_string();
        let args: Vec<String> = runner.args().to_vec();
        let prompt = runner.build_prompt_for(&input_clone);
        let is_json_mode = super::json_runner::supports_json(&cmd);

        // Spawn CLI with piped stdout for streaming
        let start_time = std::time::Instant::now();
        let mut child = match tokio::process::Command::new(&cmd)
            .args(&args)
            .arg(&prompt)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                app.update_last_assistant(&format!("Error spawning {}: {}", cmd, e));
                app.set_loading(false);
                continue;
            }
        };

        // Stream stdout line by line via channel
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let _ = stream_tx.send(line.clone());
                    }
                    Err(_) => break,
                }
            }
        });

        // Buffer stderr to prevent blocking (>64KB would cause hang)
        let stderr_buffer = Arc::new(Mutex::new(String::new()));
        let stderr_buf = stderr_buffer.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut stderr = stderr;
            let mut buf = vec![0u8; 4096];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(s) = String::from_utf8(buf[..n].to_vec()) {
                            stderr_buf.lock().unwrap().push_str(&s);
                        }
                    }
                }
            }
        });

        // Render loop: drain stream chunks + handle cancel
        loop {
            app.tick_spinner();
            app.workroom.tick();
            terminal.draw(|f| app.render(f))?;

            // Check for cancel
            if event::poll(Duration::from_millis(30))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && is_cancel_key(&key)
            {
                if let Err(e) = child.kill().await {
                    tracing::debug!("Failed to kill cancelled command: {:?}", e);
                }
                app.append_to_streaming("\n\n[Cancelled]");
                app.set_loading(false);
                break;
            }

            // Drain all available lines and parse as stream events
            let mut got_data = false;
            let mut result_event: Option<super::json_runner::CliResponse> = None;

            while let Ok(line) = stream_rx.try_recv() {
                // Log raw stream event for debugging
                super::session::log_stream_event(session_id, line.trim());

                if is_json_mode {
                    use super::json_runner::{StreamEvent, parse_stream_event};
                    match parse_stream_event(&cmd, &line) {
                        StreamEvent::Init { model, agents } => {
                            if let Some(m) = model {
                                app.set_model_name(m);
                            }
                            // Set agents from init (filtered, not all set to Working)
                            app.workroom.set_agents_from_init(&agents);
                            app.workroom.set_visible(true);
                        }
                        StreamEvent::Delta(text) => {
                            // Detect agent mentions in streamed text
                            app.workroom.detect_mentions(&text);
                            app.append_to_streaming(&text);
                            got_data = true;
                        }
                        StreamEvent::Message(text) => {
                            app.workroom.detect_mentions(&text);
                            app.append_to_streaming(&text);
                            got_data = true;
                        }
                        StreamEvent::Result(resp) => {
                            result_event = Some(resp);
                        }
                        StreamEvent::Error(msg) => {
                            app.append_to_streaming(&format!("\n\nError: {}", msg));
                            got_data = true;
                        }
                        StreamEvent::Ignored => {}
                    }
                } else {
                    // Text mode fallback
                    app.workroom.parse_streaming_line(&line);
                    app.append_to_streaming(&line);
                    got_data = true;
                }
            }

            if got_data {
                terminal.draw(|f| app.render(f))?;
            }

            // Check if child process has finished
            if let Ok(Some(status)) = child.try_wait() {
                // Drain remaining
                while let Ok(line) = stream_rx.try_recv() {
                    if is_json_mode {
                        use super::json_runner::{StreamEvent, parse_stream_event};
                        match parse_stream_event(&cmd, &line) {
                            StreamEvent::Delta(text) | StreamEvent::Message(text) => {
                                app.append_to_streaming(&text);
                            }
                            StreamEvent::Result(resp) => {
                                result_event = Some(resp);
                            }
                            _ => {}
                        }
                    } else {
                        app.append_to_streaming(&line);
                    }
                }

                let duration = start_time.elapsed();
                let content = app.get_last_assistant_content();

                // Clean markers from content
                let parsed = super::parser::parse_response(&content);
                app.update_last_assistant(&parsed.content);

                // Check for failure and append stderr if present
                if !status.success() {
                    let stderr_content = stderr_buffer.lock().unwrap();
                    if !stderr_content.is_empty() {
                        app.append_to_streaming(&format!(
                            "\n\n[Failed with status: {}]\n{}",
                            status, stderr_content
                        ));
                    } else {
                        app.append_to_streaming(&format!("\n\n[Failed with status: {}]", status));
                    }
                }

                if let Some(resp) = result_event {
                    // Use real metrics from stream result event
                    let tokens_in = resp.tokens_in.unwrap_or_else(|| {
                        super::runner::ShellRunner::estimate_tokens(&prompt) as u64
                    });
                    let tokens_out = resp.tokens_out.unwrap_or_else(|| {
                        super::runner::ShellRunner::estimate_tokens(&parsed.content) as u64
                    });
                    let cost = resp.cost_usd.unwrap_or(0.0);
                    let real_duration = resp
                        .duration_ms
                        .map(Duration::from_millis)
                        .unwrap_or(duration);

                    if let Some(ref model) = resp.model {
                        app.set_model_name(model.clone());
                    }

                    runner.record_turn_exact(
                        &input_clone,
                        &parsed.content,
                        real_duration,
                        tokens_in,
                        tokens_out,
                        cost,
                    );
                } else {
                    runner.record_turn(&input_clone, &parsed.content, duration);
                }

                let metrics = runner.session_metrics();
                app.set_session_metrics(
                    metrics.total_tokens_in,
                    metrics.total_tokens_out,
                    metrics.total_cost_estimate,
                    metrics.turn_count,
                    duration,
                );

                if let Err(e) =
                    save_current_session(session_id, project_dir, provider_name, model_name, runner)
                {
                    tracing::warn!("Failed to save session: {:?}", e);
                }

                app.workroom.on_complete();
                app.set_loading(false);
                break;
            }
        }
        // Reset workroom for next turn
        app.workroom.reset();
    }
    Ok(())
}

/// Execute tandem mode: send to N providers in parallel, show all responses.
#[allow(clippy::too_many_arguments)]
async fn execute_tandem(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ShellApp,
    runner: &mut ShellRunner,
    input: &str,
    provider_names: &[String],
    session_id: &str,
    project_dir: &str,
    provider_name: &str,
    model_name: &str,
) -> Result<()> {
    use super::detect::list_providers;

    let all_providers = list_providers();
    let start_time = std::time::Instant::now();

    // Resolve provider infos
    let mut resolved = Vec::new();
    for name in provider_names {
        if let Some(p) = all_providers
            .iter()
            .find(|p| p.command == *name || p.display_name.to_lowercase() == name.to_lowercase())
        {
            if p.available {
                resolved.push(p.clone());
            } else {
                app.add_system_message(&format!("Provider '{}' not installed — skipped", name));
            }
        } else {
            app.add_system_message(&format!("Unknown provider '{}' — skipped", name));
        }
    }

    if resolved.is_empty() {
        app.add_system_message(
            "No valid providers for tandem. Use /providers to see available ones.",
        );
        return Ok(());
    }

    app.set_loading(true);
    let prompt = runner.build_prompt_for(input);

    // Spawn all providers in parallel with streaming
    struct ProviderStream {
        display_name: String,
        stream_id: String, // Unique ID to prevent collision when same provider appears twice
        cmd: String,
        child: tokio::process::Child,
        stream_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
        result_event: Option<super::json_runner::CliResponse>,
        stderr_buffer: Arc<Mutex<String>>,
    }

    let mut streams = Vec::new();
    for provider in &resolved {
        let cmd = provider.command.clone();
        let args = provider.args.clone();
        let prompt = prompt.clone();
        let display_name = provider.display_name.clone();

        let mut child = match tokio::process::Command::new(&cmd)
            .args(&args)
            .arg(&prompt)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                app.add_system_message(&format!("Failed to spawn {}: {}", display_name, e));
                continue;
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (stream_tx, stream_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        // Spawn reader task for stdout
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let _ = stream_tx.send(line.clone());
                    }
                    Err(_) => break,
                }
            }
        });

        // Buffer to capture stderr for error reporting
        let stderr_buffer = Arc::new(Mutex::new(String::new()));
        let stderr_buf = stderr_buffer.clone();

        // Spawn reader task for stderr (to prevent buffer deadlock)
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut stderr = stderr;
            let mut buf = vec![0u8; 4096];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(s) = String::from_utf8(buf[..n].to_vec()) {
                            stderr_buf.lock().unwrap().push_str(&s);
                        }
                    }
                }
            }
        });

        // Create empty streaming message for this provider and get unique stream ID
        let stream_id = app.start_tandem_stream(&display_name);

        streams.push(ProviderStream {
            display_name,
            stream_id,
            cmd,
            child,
            stream_rx,
            result_event: None,
            stderr_buffer,
        });
    }

    if streams.is_empty() {
        app.add_system_message("No providers could be started for tandem.");
        app.set_loading(false);
        return Ok(());
    }

    // Show spinner while waiting
    app.add_system_message(&format!(
        "⚡ Tandem: sending to {} in parallel...",
        streams
            .iter()
            .map(|s| s.display_name.as_str())
            .collect::<Vec<_>>()
            .join(" + ")
    ));
    terminal.draw(|f| app.render(f))?;

    // Stream loop: drain all channels and render incrementally
    let mut active_streams = streams.len();
    while active_streams > 0 {
        app.tick_spinner();
        app.workroom.tick();
        terminal.draw(|f| app.render(f))?;

        // Check for cancel
        if event::poll(Duration::from_millis(30))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && is_cancel_key(&key)
        {
            for stream in &mut streams {
                let _ = stream.child.kill().await;
            }
            app.add_system_message("[Tandem cancelled]");
            app.set_loading(false);
            return Ok(());
        }

        // Drain all provider streams and append progressively
        let mut got_data = false;
        for stream in &mut streams {
            while let Ok(line) = stream.stream_rx.try_recv() {
                super::session::log_stream_event(session_id, line.trim());

                let is_json_mode = super::json_runner::supports_json(&stream.cmd);
                if is_json_mode {
                    use super::json_runner::{StreamEvent, parse_stream_event};
                    match parse_stream_event(&stream.cmd, &line) {
                        StreamEvent::Init { model, agents } => {
                            if let Some(m) = model {
                                app.set_model_name(m);
                            }
                            // Set agents from init (filtered, not all set to Working)
                            app.workroom.set_agents_from_init(&agents);
                            app.workroom.set_visible(true);
                        }
                        StreamEvent::Delta(text) => {
                            app.workroom.detect_mentions(&text);
                            app.append_to_tandem_stream(&stream.stream_id, &text);
                            got_data = true;
                        }
                        StreamEvent::Message(text) => {
                            app.workroom.detect_mentions(&text);
                            app.append_to_tandem_stream(&stream.stream_id, &text);
                            got_data = true;
                        }
                        StreamEvent::Result(resp) => {
                            // Store result event for metrics, do NOT append content
                            stream.result_event = Some(resp);
                        }
                        StreamEvent::Error(msg) => {
                            app.append_to_tandem_stream(
                                &stream.stream_id,
                                &format!("\n\nError: {}", msg),
                            );
                            got_data = true;
                        }
                        StreamEvent::Ignored => {}
                    }
                } else {
                    // Text mode fallback
                    app.workroom.parse_streaming_line(&line);
                    app.append_to_tandem_stream(&stream.stream_id, &line);
                    got_data = true;
                }
            }
        }

        if got_data {
            terminal.draw(|f| app.render(f))?;
        }

        // Check if any child has finished
        active_streams = 0;
        for stream in &mut streams {
            match stream.child.try_wait() {
                Ok(Some(_status)) => {
                    // Child finished, continue to next
                }
                Ok(None) => {
                    // Still running
                    active_streams += 1;
                }
                Err(_) => {
                    // Error checking status, assume finished
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // Finalize: drain remaining events and wait for all children to complete
    let mut combined_content = String::new();
    for mut stream in streams {
        // Drain any remaining stream events
        let is_json_mode = super::json_runner::supports_json(&stream.cmd);
        while let Ok(line) = stream.stream_rx.try_recv() {
            if is_json_mode {
                use super::json_runner::{StreamEvent, parse_stream_event};
                match parse_stream_event(&stream.cmd, &line) {
                    StreamEvent::Delta(text) | StreamEvent::Message(text) => {
                        app.append_to_tandem_stream(&stream.stream_id, &text);
                    }
                    StreamEvent::Result(resp) => {
                        stream.result_event = Some(resp);
                    }
                    _ => {}
                }
            } else {
                app.append_to_tandem_stream(&stream.stream_id, &line);
            }
        }

        match stream.child.wait().await {
            Ok(status) if status.success() => {
                // Content was already streamed progressively, parse and clean markers
                let content = app.get_assistant_content_by_stream_id(&stream.stream_id);
                let parsed = super::parser::parse_response(&content);
                // Update message with cleaned content (item 4: marker cleanup)
                app.update_assistant_by_stream_id(&stream.stream_id, &parsed.content);
                combined_content.push_str(&parsed.content);
            }
            Ok(status) => {
                let stderr_content = stream.stderr_buffer.lock().unwrap();
                let error_msg = if stderr_content.is_empty() {
                    format!("\n\n[Failed with status: {}]", status)
                } else {
                    format!("\n\n[Failed with status: {}]\n{}", status, stderr_content)
                };
                app.append_to_tandem_stream(&stream.stream_id, &error_msg);
            }
            Err(e) => {
                app.append_to_tandem_stream(&stream.stream_id, &format!("\n\n[Error: {}]", e));
            }
        }
        terminal.draw(|f| app.render(f))?;
    }

    // TODO(debt): Each stream.result_event contains real metrics, but we use estimated record_turn.
    // To improve: aggregate result_events (sum tokens_out/cost, use first tokens_in) and call record_turn_exact.
    // Challenge: streams may have mix of json/text modes, some may fail without result_event.
    let duration = start_time.elapsed();
    runner.record_turn(input, &combined_content, duration);
    let metrics = runner.session_metrics();
    app.set_session_metrics(
        metrics.total_tokens_in,
        metrics.total_tokens_out,
        metrics.total_cost_estimate,
        metrics.turn_count,
        duration,
    );
    app.set_loading(false);

    if let Err(e) = save_current_session(session_id, project_dir, provider_name, model_name, runner)
    {
        tracing::warn!("Failed to save session: {:?}", e);
    }
    Ok(())
}

/// Resolve an agent file path by name from the current project config.
fn resolve_project_agent(name: &str) -> Option<std::path::PathBuf> {
    let (root, config) = crate::core::project::find_project_config()?;
    for agent_ref in &config.agents {
        let agent_name = match agent_ref {
            crate::core::project::AgentRef::Named { name: n } => n.clone(),
            crate::core::project::AgentRef::Path { path } => path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            _ => continue,
        };
        if agent_name == name {
            return crate::core::project::resolve_agent(agent_ref, &root).ok();
        }
    }
    None
}

/// Resolved step data: command, args, combined prompt, display label.
struct ResolvedStep {
    cmd: String,
    args: Vec<String>,
    display_label: String,
}

/// Execute pipeline from a list of PipelineStep (supports both `provider:` and `agent:`).
#[allow(clippy::too_many_arguments)]
async fn execute_pipeline_steps(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ShellApp,
    runner: &mut ShellRunner,
    input: &str,
    steps: &[super::config::PipelineStep],
    session_id: &str,
    project_dir: &str,
    provider_name: &str,
    model_name: &str,
) -> Result<()> {
    if steps.is_empty() {
        app.add_system_message("Pipeline has no steps configured.");
        return Ok(());
    }

    let start_time = std::time::Instant::now();
    app.set_loading(true);

    let mut current_input = input.to_string();
    let total_steps = steps.len();

    for (i, step) in steps.iter().enumerate() {
        let is_last = i == total_steps - 1;

        // Each step may have multiple provider/agent entries — take the first for now
        let Some(entry) = step.providers.first() else {
            app.add_system_message(&format!("Step '{}' has no providers, skipping.", step.name));
            continue;
        };

        // Resolve entry to cmd + args + optional agent system prompt
        let (cmd, args, agent_system_prompt, display_label) = if let Some(agent_name) = &entry.agent
        {
            // Agent mode: load the agent from project config
            match resolve_project_agent(agent_name) {
                Some(path) => match crate::parser::parse_agent_file(&path) {
                    Ok(agent) => {
                        let cmd = agent.metadata.provider.clone();
                        let args = super::detect::args_for_provider(&cmd);
                        let label = format!("{} [{}]", agent_name, cmd);
                        (cmd, args, Some(agent.system_prompt.clone()), label)
                    }
                    Err(e) => {
                        app.add_system_message(&format!(
                            "Failed to parse agent '{agent_name}': {e}"
                        ));
                        continue;
                    }
                },
                None => {
                    app.add_system_message(&format!(
                        "Agent '{agent_name}' not found in project config"
                    ));
                    continue;
                }
            }
        } else if !entry.provider.is_empty() {
            // Provider mode: raw CLI invocation
            let cmd = entry.provider.clone();
            let args = super::detect::args_for_provider(&cmd);
            (cmd, args, None, entry.provider.clone())
        } else {
            app.add_system_message(&format!(
                "Step '{}' has neither agent nor provider set, skipping.",
                step.name
            ));
            continue;
        };

        let resolved = ResolvedStep {
            cmd: cmd.clone(),
            args: args.clone(),
            display_label: display_label.clone(),
        };

        let stage_label = if i == 0 { "starting" } else { "chained" };
        app.add_system_message(&format!(
            "⚙ Pipeline step {}/{}: {} — {} ({})",
            i + 1,
            total_steps,
            step.name,
            display_label,
            stage_label
        ));
        terminal.draw(|f| app.render(f))?;

        // Build the prompt: (agent system prompt)? + (step prompt)? + current input
        let mut full_prompt = String::new();
        if let Some(sp) = &agent_system_prompt {
            full_prompt.push_str(sp);
            full_prompt.push_str("\n\n");
        }
        if let Some(step_prompt) = &step.prompt {
            full_prompt.push_str(step_prompt);
            full_prompt.push_str("\n\n");
        }
        if i == 0 {
            full_prompt.push_str(&current_input);
        } else {
            full_prompt.push_str(&format!(
                "Previous step output:\n---\n{}\n---\n\nOriginal request: {}",
                current_input, input
            ));
        }

        // Start streaming for this step
        app.start_streaming_response();

        // Spawn process with streaming stdout
        let mut child = match tokio::process::Command::new(&resolved.cmd)
            .args(&resolved.args)
            .arg(&full_prompt)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                app.update_last_assistant(&format!("Failed to spawn: {}", e));
                app.set_loading(false);
                return Ok(());
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        // Spawn reader task for stdout
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let _ = stream_tx.send(line.clone());
                    }
                    Err(_) => break,
                }
            }
        });

        // Buffer to capture stderr for error reporting
        let stderr_buffer = Arc::new(Mutex::new(String::new()));
        let stderr_buf = stderr_buffer.clone();

        // Spawn reader task for stderr (to prevent buffer deadlock)
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut stderr = stderr;
            let mut buf = vec![0u8; 4096];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(s) = String::from_utf8(buf[..n].to_vec()) {
                            stderr_buf.lock().unwrap().push_str(&s);
                        }
                    }
                }
            }
        });

        let is_json_mode = super::json_runner::supports_json(&resolved.cmd);
        // TODO(debt): result_event is captured but not used for record_turn_exact.
        // To use it: accumulate metrics from all steps and call record_turn_exact at L1369.
        // Challenge: need to aggregate tokens/cost across steps.
        let mut _result_event: Option<super::json_runner::CliResponse> = None;

        // Stream loop
        loop {
            app.tick_spinner();
            app.workroom.tick();
            terminal.draw(|f| app.render(f))?;

            if event::poll(Duration::from_millis(30))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && is_cancel_key(&key)
            {
                let _ = child.kill().await;
                app.append_to_streaming("\n\n[Pipeline cancelled]");
                app.set_loading(false);
                return Ok(());
            }

            // Drain stream and append to UI progressively
            let mut got_data = false;
            while let Ok(line) = stream_rx.try_recv() {
                super::session::log_stream_event(session_id, line.trim());

                if is_json_mode {
                    use super::json_runner::{StreamEvent, parse_stream_event};
                    match parse_stream_event(&resolved.cmd, &line) {
                        StreamEvent::Init { model, agents } => {
                            if let Some(m) = model {
                                app.set_model_name(m);
                            }
                            // Set agents from init (filtered, not all set to Working)
                            app.workroom.set_agents_from_init(&agents);
                            app.workroom.set_visible(true);
                        }
                        StreamEvent::Delta(text) => {
                            app.workroom.detect_mentions(&text);
                            app.append_to_streaming(&text);
                            got_data = true;
                        }
                        StreamEvent::Message(text) => {
                            app.workroom.detect_mentions(&text);
                            app.append_to_streaming(&text);
                            got_data = true;
                        }
                        StreamEvent::Result(resp) => {
                            // Store result event for metrics, do NOT append content
                            _result_event = Some(resp);
                        }
                        StreamEvent::Error(msg) => {
                            app.append_to_streaming(&format!("\n\nError: {}", msg));
                            got_data = true;
                        }
                        StreamEvent::Ignored => {}
                    }
                } else {
                    // Text mode fallback
                    app.workroom.parse_streaming_line(&line);
                    app.append_to_streaming(&line);
                    got_data = true;
                }
            }

            if got_data {
                terminal.draw(|f| app.render(f))?;
            }

            // Check if child finished
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Drain remaining stream events
                    while let Ok(line) = stream_rx.try_recv() {
                        if is_json_mode {
                            use super::json_runner::{StreamEvent, parse_stream_event};
                            match parse_stream_event(&resolved.cmd, &line) {
                                StreamEvent::Delta(text) | StreamEvent::Message(text) => {
                                    app.append_to_streaming(&text);
                                }
                                StreamEvent::Result(resp) => {
                                    _result_event = Some(resp);
                                }
                                _ => {}
                            }
                        } else {
                            app.append_to_streaming(&line);
                        }
                    }

                    if status.success() {
                        // Get the streamed content
                        let content = app.get_last_assistant_content();
                        let parsed = super::parser::parse_response(&content);

                        // Update message with label and parsed content
                        let label = if is_last {
                            format!("{} (final)", resolved.display_label)
                        } else {
                            format!("{} (step {})", resolved.display_label, i + 1)
                        };

                        app.update_last_assistant_with_label(&label, &parsed.content);
                        current_input = parsed.content;
                    } else {
                        // Process failed
                        let stderr_content = stderr_buffer.lock().unwrap();
                        let error_msg = if stderr_content.is_empty() {
                            format!(
                                "\n\n[Pipeline step '{}' failed with status: {}]",
                                step.name, status
                            )
                        } else {
                            format!(
                                "\n\n[Pipeline step '{}' failed with status: {}]\n{}",
                                step.name, status, stderr_content
                            )
                        };
                        app.append_to_streaming(&error_msg);
                        app.set_loading(false);
                        return Ok(());
                    }
                    break;
                }
                Ok(None) => {
                    // Still running, continue loop
                }
                Err(e) => {
                    app.update_last_assistant(&format!("Error checking process: {}", e));
                    app.set_loading(false);
                    return Ok(());
                }
            }

            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        terminal.draw(|f| app.render(f))?;
    }

    let duration = start_time.elapsed();
    runner.record_turn(input, &current_input, duration);
    let metrics = runner.session_metrics();
    app.set_session_metrics(
        metrics.total_tokens_in,
        metrics.total_tokens_out,
        metrics.total_cost_estimate,
        metrics.turn_count,
        duration,
    );
    app.set_loading(false);

    if let Err(e) = save_current_session(session_id, project_dir, provider_name, model_name, runner)
    {
        tracing::warn!("Failed to save session: {:?}", e);
    }
    Ok(())
}

/// Execute a turn in PTY mode — interactive CLI with native agent delegation.
#[allow(clippy::too_many_arguments)]
async fn execute_pty_turn(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ShellApp,
    runner: &mut ShellRunner,
    input: &str,
    session_id: &str,
    project_dir: &str,
    provider_name: &str,
    model_name: &str,
) -> Result<()> {
    use super::pty_runner::{PtyConfig, PtySession, detect_agent_activity, filter_startup_noise};
    use std::time::Instant;

    let cmd = runner.command().to_string();
    let config = PtyConfig {
        command: cmd.clone(),
        width: terminal.size()?.width,
        height: 40,
    };

    app.set_loading(true);
    app.start_streaming_response();
    let start_time = Instant::now();

    let mut pty = match PtySession::spawn(&config) {
        Ok(pty) => pty,
        Err(e) => {
            app.update_last_assistant(&format!("Error spawning PTY for {}: {}", cmd, e));
            app.set_loading(false);
            return Ok(());
        }
    };

    // Wait for startup noise
    let _startup = pty.drain_until_silence(Duration::from_secs(3));

    // Send the user's message
    if let Err(e) = pty.send(input) {
        app.update_last_assistant(&format!("Error sending to PTY: {}", e));
        app.set_loading(false);
        return Ok(());
    }

    // Stream output
    let silence_timeout = Duration::from_secs(5);
    let mut last_data_time = Instant::now();
    let mut full_response = String::new();

    loop {
        app.tick_spinner();
        app.workroom.tick();
        terminal.draw(|f| app.render(f))?;

        // Check for cancel
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && is_cancel_key(&key)
        {
            pty.kill();
            app.append_to_streaming("\n\n[Cancelled]");
            app.set_loading(false);
            break;
        }

        let (new_text, done) = pty.drain();

        if !new_text.is_empty() {
            last_data_time = Instant::now();
            let clean = filter_startup_noise(&new_text);
            if !clean.is_empty() {
                for agent in &detect_agent_activity(&clean) {
                    app.workroom.on_delegate(agent);
                }
                app.append_to_streaming(&clean);
                full_response.push_str(&clean);
                terminal.draw(|f| app.render(f))?;
            }
        }

        if done || !pty.is_running() {
            let (remaining, _) = pty.drain();
            if !remaining.is_empty() {
                let clean = filter_startup_noise(&remaining);
                app.append_to_streaming(&clean);
                full_response.push_str(&clean);
            }
            break;
        }

        if last_data_time.elapsed() > silence_timeout && !full_response.is_empty() {
            break;
        }
    }

    let duration = start_time.elapsed();
    let parsed = super::parser::parse_response(&full_response);
    app.update_last_assistant(&parsed.content);
    runner.record_turn(input, &parsed.content, duration);

    let metrics = runner.session_metrics();
    app.set_session_metrics(
        metrics.total_tokens_in,
        metrics.total_tokens_out,
        metrics.total_cost_estimate,
        metrics.turn_count,
        duration,
    );

    app.workroom.on_complete();
    app.set_loading(false);
    if let Err(e) = save_current_session(session_id, project_dir, provider_name, model_name, runner)
    {
        tracing::warn!("Failed to save session: {:?}", e);
    }
    Ok(())
}
