mod app;
pub mod views;
pub mod widgets;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;

use app::PaletteAction;

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

    loop {
        terminal.draw(|frame| {
            views::dashboard::render(frame, &app);
            // Overlay command palette if visible
            views::palette::render(frame, &app);
        })?;

        if crossterm::event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Command palette mode
            if app.palette.visible {
                match key.code {
                    KeyCode::Esc => app.palette.close(),
                    KeyCode::Enter => {
                        if let Some(action) = app.palette.execute() {
                            app.palette.close();
                            match action {
                                PaletteAction::SwitchTab(tab) => app.switch_tab(tab),
                                PaletteAction::Refresh => {
                                    app.load_agents();
                                    load_storage_data(&mut app).await;
                                }
                                PaletteAction::Quit => break,
                                PaletteAction::NewAgent => {
                                    app.status_msg =
                                        Some("Run 'armadai new <name>' from terminal".to_string());
                                }
                            }
                        }
                    }
                    KeyCode::Up => app.palette.select_prev(),
                    KeyCode::Down => app.palette.select_next(),
                    KeyCode::Backspace => {
                        app.palette.input.pop();
                        app.palette.update_filter();
                    }
                    KeyCode::Char(c) => {
                        app.palette.input.push(c);
                        app.palette.update_filter();
                    }
                    _ => {}
                }
                continue;
            }

            // Normal mode
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char(':') | KeyCode::Char('p')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    app.palette.open();
                }
                KeyCode::Char(':') => app.palette.open(),
                KeyCode::Tab => app.next_tab(),
                KeyCode::BackTab => app.prev_tab(),
                KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
                KeyCode::Char('r') => {
                    app.load_agents();
                    load_storage_data(&mut app).await;
                    app.status_msg = Some("Refreshed".to_string());
                }
                KeyCode::Enter => {
                    if app.current_tab == app::Tab::Dashboard && app.selected_agent().is_some() {
                        app.switch_tab(app::Tab::AgentDetail);
                    }
                }
                KeyCode::Char('1') => app.switch_tab(app::Tab::Dashboard),
                KeyCode::Char('2') => app.switch_tab(app::Tab::AgentDetail),
                KeyCode::Char('3') => app.switch_tab(app::Tab::History),
                KeyCode::Char('4') => app.switch_tab(app::Tab::Costs),
                _ => {}
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
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
