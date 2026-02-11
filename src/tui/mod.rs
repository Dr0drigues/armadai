mod app;
pub mod views;
pub mod widgets;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use tokio_stream::StreamExt;

pub async fn run() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = app::App::new();
    app.load_agents();
    load_storage_data(&mut app).await;

    // Channel for streaming execution output
    let (exec_tx, mut exec_rx) = tokio::sync::mpsc::channel::<String>(256);

    loop {
        terminal.draw(|frame| {
            views::dashboard::render(frame, &app);
        })?;

        // Drain any pending streaming tokens
        while let Ok(token) = exec_rx.try_recv() {
            app.exec_output.push(token);
        }

        // Poll for keyboard events with a short timeout (for streaming responsiveness)
        if crossterm::event::poll(std::time::Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Tab => app.next_tab(),
                KeyCode::BackTab => app.prev_tab(),
                KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
                KeyCode::Char('r') => {
                    // Refresh data
                    app.load_agents();
                    load_storage_data(&mut app).await;
                }
                KeyCode::Enter => {
                    // Launch execution of selected agent
                    if !app.exec_running
                        && let Some(agent) = app.agents.get(app.selected_agent).cloned()
                    {
                        app.exec_output.clear();
                        app.exec_running = true;
                        app.current_tab = app::Tab::Execution;
                        app.tab_index = 1;

                        let tx = exec_tx.clone();
                        tokio::spawn(async move {
                            run_agent_streaming(agent, tx).await;
                        });
                    }
                }
                _ => {}
            }
        }

        // Check if execution finished
        if app.exec_running && exec_rx.is_empty() {
            // Small heuristic: if the channel is empty and the task has had time to start,
            // check again after next poll cycle. Execution complete is signaled by
            // "[DONE]" token.
            if app.exec_output.last().is_some_and(|s| s.contains("[DONE]")) {
                app.exec_running = false;
                load_storage_data(&mut app).await;
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_agent_streaming(
    agent: crate::core::agent::Agent,
    tx: tokio::sync::mpsc::Sender<String>,
) {
    use crate::providers::factory::create_provider;
    use crate::providers::traits::{ChatMessage, CompletionRequest};

    let _ = tx.send(format!("Running agent: {}\n\n", agent.name)).await;

    let provider = match create_provider(&agent) {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.send(format!("Error: {e}\n[DONE]")).await;
            return;
        }
    };

    let model = agent
        .metadata
        .model
        .clone()
        .or_else(|| agent.metadata.command.clone())
        .unwrap_or_else(|| "default".to_string());

    let request = CompletionRequest {
        model,
        system_prompt: agent.system_prompt.clone(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello! Introduce yourself briefly.".to_string(),
        }],
        temperature: agent.metadata.temperature,
        max_tokens: agent.metadata.max_tokens,
    };

    // Try streaming first, fall back to complete
    match provider.stream(request.clone()).await {
        Ok(mut stream) => {
            while let Some(result) = stream.next().await {
                match result {
                    Ok(token) => {
                        if tx.send(token).await.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(format!("\nStream error: {e}")).await;
                        break;
                    }
                }
            }
            let _ = tx.send("\n\n[DONE]".to_string()).await;
        }
        Err(_) => {
            // Fallback to complete
            match provider.complete(request).await {
                Ok(response) => {
                    let _ = tx.send(response.content).await;
                    let _ = tx
                        .send(format!(
                            "\n\n[tokens: {}/{}, cost: ${:.6}]\n[DONE]",
                            response.tokens_in, response.tokens_out, response.cost
                        ))
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(format!("Error: {e}\n[DONE]")).await;
                }
            }
        }
    }
}

#[cfg(feature = "storage")]
async fn load_storage_data(app: &mut app::App) {
    use crate::storage::{init_db, queries};

    let db = match init_db().await {
        Ok(db) => db,
        Err(_) => return,
    };

    // Load history
    if let Ok(records) = queries::get_history(&db, None, 100).await {
        app.history = records
            .into_iter()
            .map(|r| app::RunEntry {
                input_preview: r.input.chars().take(40).collect(),
                output_preview: r.output.chars().take(40).collect(),
                agent: r.agent,
                provider: r.provider,
                model: r.model,
                tokens_in: r.tokens_in,
                tokens_out: r.tokens_out,
                cost: r.cost,
                duration_ms: r.duration_ms,
                status: r.status,
            })
            .collect();
    }

    // Load costs
    if let Ok(summaries) = queries::get_costs_summary(&db, None).await {
        app.costs = summaries
            .into_iter()
            .map(|s| app::CostEntry {
                agent: s.agent,
                total_runs: s.total_runs,
                total_cost: s.total_cost,
                total_tokens_in: s.total_tokens_in,
                total_tokens_out: s.total_tokens_out,
            })
            .collect();
    }
}

#[cfg(not(feature = "storage"))]
async fn load_storage_data(_app: &mut app::App) {
    // No storage feature — data views will be empty
}
