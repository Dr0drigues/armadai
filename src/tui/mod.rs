mod app;
pub mod filter;
pub mod format;
pub mod views;
pub mod widgets;
mod wrap;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;

use app::PaletteAction;

pub async fn run(ascii: bool) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize theme once (color tier detection + ASCII preference)
    crate::theme::init(ascii);

    let mut app = app::App::new();
    app.load_agents();
    app.load_prompts();
    app.load_skills();
    app.load_starters();
    app.load_models();
    load_storage_data(&mut app);

    loop {
        terminal.draw(|frame| {
            views::dashboard::render(frame, &mut app);
            // Overlay command palette if visible
            views::palette::render(frame, &app);
        })?;

        if crossterm::event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Ctrl+C quits unconditionally (aligns with the shell TUI's quit
            // key), regardless of palette/search mode or the current tab.
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                break;
            }

            // Disarm "press Esc again to quit" (see `handle_top_level_esc`
            // below) on any key that isn't Esc — the double-Esc confirmation
            // must be consecutive, mirroring `src/shell/tui.rs`'s
            // `esc_armed` semantics.
            disarm_esc_on_other_key(&mut app, key.code);

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
                                    app.load_prompts();
                                    app.load_skills();
                                    app.load_starters();
                                    app.load_models();
                                    load_storage_data(&mut app);
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

            // Search mode
            if app.search_mode {
                match key.code {
                    KeyCode::Esc => {
                        app.clear_search();
                    }
                    KeyCode::Enter => {
                        app.search_mode = false;
                    }
                    KeyCode::Backspace => {
                        app.search_query.pop();
                    }
                    KeyCode::Char(c) => {
                        app.search_query.push(c);
                    }
                    _ => {}
                }
                continue;
            }

            // Normal mode

            // Detail view: Esc goes back to parent list
            if key.code == KeyCode::Esc {
                match app.current_tab {
                    app::Tab::AgentDetail => {
                        app.switch_tab(app::Tab::Dashboard);
                        continue;
                    }
                    app::Tab::PromptDetail => {
                        app.switch_tab(app::Tab::Prompts);
                        continue;
                    }
                    app::Tab::SkillDetail => {
                        app.switch_tab(app::Tab::Skills);
                        continue;
                    }
                    app::Tab::StarterDetail => {
                        app.switch_tab(app::Tab::Starters);
                        continue;
                    }
                    app::Tab::ModelDetail => {
                        app.switch_tab(app::Tab::Models);
                        continue;
                    }
                    #[cfg(feature = "storage")]
                    app::Tab::OrchestrationDetail => {
                        app.switch_tab(app::Tab::Orchestration);
                        continue;
                    }
                    // Top-level list view: Esc no longer quits instantly.
                    // First press arms a "press Esc again to quit" safety
                    // net (shown in the shortcuts bar); a second,
                    // consecutive Esc confirms. Mirrors the shell TUI's
                    // `esc_armed` (`src/shell/tui.rs`).
                    _ => {
                        if handle_top_level_esc(&mut app) {
                            break;
                        }
                        continue;
                    }
                }
            }

            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char(':') | KeyCode::Char('p')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    app.palette.open();
                }
                KeyCode::Char(':') => app.palette.open(),
                KeyCode::Char('/') => {
                    // Enter search mode on list tabs
                    let is_searchable = matches!(
                        app.current_tab,
                        app::Tab::Dashboard
                            | app::Tab::Prompts
                            | app::Tab::Skills
                            | app::Tab::Starters
                            | app::Tab::History
                            | app::Tab::Costs
                            | app::Tab::Models
                    ) || {
                        #[cfg(feature = "storage")]
                        {
                            matches!(app.current_tab, app::Tab::Orchestration)
                        }
                        #[cfg(not(feature = "storage"))]
                        {
                            false
                        }
                    };
                    if is_searchable {
                        app.search_mode = true;
                        app.search_query.clear();
                    }
                }
                KeyCode::Char('s') => {
                    // Cycle sort mode on list tabs
                    let is_sortable = matches!(
                        app.current_tab,
                        app::Tab::Dashboard
                            | app::Tab::Prompts
                            | app::Tab::Skills
                            | app::Tab::Starters
                            | app::Tab::History
                            | app::Tab::Costs
                            | app::Tab::Models
                    ) || {
                        #[cfg(feature = "storage")]
                        {
                            matches!(app.current_tab, app::Tab::Orchestration)
                        }
                        #[cfg(not(feature = "storage"))]
                        {
                            false
                        }
                    };
                    if is_sortable {
                        app.cycle_sort_mode();
                    }
                }
                KeyCode::Tab => app.next_tab(),
                KeyCode::BackTab => app.prev_tab(),
                KeyCode::Char('j') | KeyCode::Down => {
                    // In a scrollable detail view, j/Down scrolls the body
                    // instead of moving a (non-existent) list selection.
                    if is_scrollable_detail(app.current_tab) {
                        app.scroll_detail_down();
                    } else {
                        app.select_next();
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if is_scrollable_detail(app.current_tab) {
                        app.scroll_detail_up();
                    } else {
                        app.select_prev();
                    }
                }
                KeyCode::PageDown if is_scrollable_detail(app.current_tab) => {
                    app.scroll_detail_page_down();
                }
                KeyCode::PageUp if is_scrollable_detail(app.current_tab) => {
                    app.scroll_detail_page_up();
                }
                KeyCode::Char('R')
                    if matches!(app.current_tab, app::Tab::Models | app::Tab::ModelDetail) =>
                {
                    app.status_msg = Some("Syncing models from models.dev…".to_string());
                    // Force redraw to show status message
                    terminal.draw(|frame| {
                        views::dashboard::render(frame, &mut app);
                        views::palette::render(frame, &app);
                    })?;
                    match sync_models_online().await {
                        Ok(count) => {
                            app.load_models();
                            app.status_msg =
                                Some(format!("Synced {count} providers from models.dev"));
                        }
                        Err(e) => {
                            app.status_msg = Some(format!("Sync failed: {e}"));
                        }
                    }
                }
                KeyCode::Char('r') => {
                    app.load_agents();
                    app.load_prompts();
                    app.load_skills();
                    app.load_starters();
                    app.load_models();
                    load_storage_data(&mut app);
                    app.status_msg = Some("Refreshed".to_string());
                }
                KeyCode::Enter => match app.current_tab {
                    app::Tab::Dashboard if app.selected_agent().is_some() => {
                        app.switch_tab(app::Tab::AgentDetail);
                    }
                    app::Tab::Prompts if app.selected_prompt().is_some() => {
                        app.switch_tab(app::Tab::PromptDetail);
                    }
                    app::Tab::Skills if app.selected_skill().is_some() => {
                        app.switch_tab(app::Tab::SkillDetail);
                    }
                    app::Tab::Starters if app.selected_starter().is_some() => {
                        app.switch_tab(app::Tab::StarterDetail);
                    }
                    app::Tab::Models if app.selected_model_entry().is_some() => {
                        app.switch_tab(app::Tab::ModelDetail);
                    }
                    #[cfg(feature = "storage")]
                    app::Tab::Orchestration if app.selected_orchestration_entry().is_some() => {
                        app.switch_tab(app::Tab::OrchestrationDetail);
                    }
                    _ => {}
                },
                KeyCode::Char('i')
                    if matches!(
                        app.current_tab,
                        app::Tab::Starters | app::Tab::StarterDetail
                    ) =>
                {
                    if let Some(pack) = app.selected_starter().cloned() {
                        let pack_name = pack.name.clone();
                        let yaml = crate::cli::init::generate_project_yaml(&pack, &pack_name);
                        let dotarmadai = std::path::Path::new(".armadai");
                        let config_path = dotarmadai.join("config.yaml");
                        let legacy_path = std::path::Path::new("armadai.yaml");
                        if config_path.exists() {
                            app.status_msg =
                                Some(".armadai/config.yaml already exists".to_string());
                        } else if legacy_path.exists() {
                            app.status_msg = Some(
                                "armadai.yaml already exists (migrate to .armadai/)".to_string(),
                            );
                        } else {
                            // Create .armadai/ directory structure
                            let dirs_ok = ["agents", "prompts", "skills", "starters"]
                                .iter()
                                .all(|sub| std::fs::create_dir_all(dotarmadai.join(sub)).is_ok());
                            if !dirs_ok {
                                app.status_msg =
                                    Some("Failed to create .armadai/ directories".to_string());
                            } else {
                                match std::fs::write(&config_path, yaml) {
                                    Ok(()) => {
                                        app.status_msg = Some(format!(
                                            "Created .armadai/config.yaml (pack: {pack_name})"
                                        ));
                                    }
                                    Err(e) => {
                                        app.status_msg = Some(format!(
                                            "Failed to write .armadai/config.yaml: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                KeyCode::Char('1') => app.switch_tab(app::Tab::Dashboard),
                KeyCode::Char('2') => app.switch_tab(app::Tab::Prompts),
                KeyCode::Char('3') => app.switch_tab(app::Tab::Skills),
                KeyCode::Char('4') => app.switch_tab(app::Tab::Starters),
                KeyCode::Char('5') => app.switch_tab(app::Tab::History),
                KeyCode::Char('6') => app.switch_tab(app::Tab::Costs),
                KeyCode::Char('7') => app.switch_tab(app::Tab::Models),
                #[cfg(feature = "storage")]
                KeyCode::Char('8') => app.switch_tab(app::Tab::Orchestration),
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

/// Detail tabs whose body panel honors `App::detail_scroll` (see
/// `agent_detail`, `prompt_detail`, `skill_detail`, `orchestration::render_detail`).
/// While one of these is active, j/k and the arrow keys scroll the body
/// instead of moving a list selection (there is no list to navigate).
fn is_scrollable_detail(tab: app::Tab) -> bool {
    // `Tab::OrchestrationDetail` exists regardless of the `storage` feature
    // (only the underlying data is gated), so no extra `#[cfg]` is needed
    // here — without `storage` the tab is simply never reached.
    matches!(
        tab,
        app::Tab::AgentDetail
            | app::Tab::PromptDetail
            | app::Tab::SkillDetail
            | app::Tab::OrchestrationDetail
    )
}

/// Disarm the top-level "press Esc again to quit" safety net (see
/// `handle_top_level_esc`) on any key that isn't Esc. The double-Esc
/// confirmation must be consecutive — mirrors `src/shell/tui.rs`'s
/// `esc_armed` semantics.
fn disarm_esc_on_other_key(app: &mut app::App, code: KeyCode) {
    if code != KeyCode::Esc {
        app.esc_armed = false;
    }
}

/// Handle Esc pressed on a top-level list view (no popup/search/detail
/// active). First press arms `esc_armed` (surfaced in the shortcuts bar,
/// see `views::shortcuts::render`) instead of quitting outright; a second,
/// consecutive Esc confirms. Returns `true` if the caller should quit.
fn handle_top_level_esc(app: &mut app::App) -> bool {
    if app.esc_armed {
        true
    } else {
        app.esc_armed = true;
        false
    }
}

#[cfg(feature = "storage")]
fn load_storage_data(app: &mut app::App) {
    use crate::storage::{init_db, queries};

    let db = match init_db() {
        Ok(db) => db,
        Err(_) => return,
    };

    // Load history
    if let Ok(records) = queries::get_history(&db, None, 100) {
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
                project: r.project,
            })
            .collect();
    }

    // Load costs
    if let Ok(summaries) = queries::get_costs_summary(&db, None) {
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

    // Load orchestration runs
    app.load_orchestration_runs();
}

#[cfg(not(feature = "storage"))]
fn load_storage_data(_app: &mut app::App) {
    // No storage feature — data views will be empty
}

/// Force-refresh model registry from models.dev.
#[cfg(feature = "providers-api")]
async fn sync_models_online() -> anyhow::Result<usize> {
    crate::model_registry::fetch::refresh_registry().await
}

#[cfg(not(feature = "providers-api"))]
async fn sync_models_online() -> anyhow::Result<usize> {
    anyhow::bail!("Model sync requires providers-api feature")
}

#[cfg(test)]
mod esc_quit_tests {
    use super::*;

    #[test]
    fn first_top_level_esc_arms_without_quitting() {
        let mut app = app::App::new();
        assert!(!handle_top_level_esc(&mut app));
        assert!(app.esc_armed);
    }

    #[test]
    fn second_consecutive_top_level_esc_quits() {
        let mut app = app::App::new();
        handle_top_level_esc(&mut app);
        assert!(handle_top_level_esc(&mut app));
    }

    #[test]
    fn any_other_key_disarms_the_pending_quit() {
        let mut app = app::App::new();
        app.esc_armed = true;
        disarm_esc_on_other_key(&mut app, KeyCode::Char('q'));
        assert!(!app.esc_armed);
    }

    #[test]
    fn esc_itself_does_not_disarm() {
        let mut app = app::App::new();
        app.esc_armed = true;
        disarm_esc_on_other_key(&mut app, KeyCode::Esc);
        assert!(app.esc_armed);
    }
}
