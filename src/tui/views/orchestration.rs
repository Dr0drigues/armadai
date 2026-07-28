#![cfg(feature = "storage")]

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::theme;
use crate::tui::app::App;
use crate::tui::filter;
use crate::tui::widgets::search_bar;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.orchestration_runs.is_empty() {
        let msg = Paragraph::new(
            "No orchestration runs found. Run a coordinated agent with armadai run <coordinator>",
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Orchestration ")
                .style(theme::border_style()),
        );
        frame.render_widget(msg, area);
        return;
    }

    // Apply filtering and sorting
    let display_indices = filter::apply_filter_and_sort_orchestration(
        &app.orchestration_runs,
        &app.search_query,
        app.sort_mode,
    );

    if display_indices.is_empty() {
        let msg = Paragraph::new("No orchestration runs match your search.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Orchestration ")
                .style(theme::border_style()),
        );
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec!["", "RUN ID", "PATTERN", "ROUNDS", "HALT REASON"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_i, &orch_i)| {
            let marker = if display_i == app.selected_orchestration {
                ">"
            } else {
                " "
            };
            let r = &app.orchestration_runs[orch_i];
            let run_id_short = if r.run_id.len() > 20 {
                format!("{}...", &r.run_id[..17])
            } else {
                r.run_id.clone()
            };
            let halt_reason = r.halt_reason.as_deref().unwrap_or("—");
            let style = if display_i == app.selected_orchestration {
                theme::selection()
            } else {
                Style::default()
            };
            Row::new(vec![
                marker.to_string(),
                run_id_short,
                r.pattern.clone(),
                r.rounds.to_string(),
                halt_reason.to_string(),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(20),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Orchestration — {} runs, {} shown{} ",
                app.orchestration_runs.len(),
                display_indices.len(),
                app.sort_indicator()
            ))
            .style(theme::border_style()),
    );

    frame.render_widget(table, area);

    // Render search bar if in search mode
    if app.search_mode {
        search_bar(frame, &app.search_query, area);
    }
}

pub fn render_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    if let Some(entry) = app.selected_orchestration_entry() {
        use crate::db::init_db;
        use crate::storage::queries;

        let detail_content = match init_db() {
            Ok(db) => {
                match queries::get_orchestration_run(&db, &entry.run_id) {
                    Ok(Some(record)) => {
                        let mut lines = vec![
                            format!("Run ID: {}", record.run_id),
                            format!("Pattern: {}", record.pattern),
                            format!("Rounds: {}", record.rounds),
                            format!(
                                "Halt reason: {}",
                                record.halt_reason.as_deref().unwrap_or("N/A")
                            ),
                            String::new(),
                            "Config:".to_string(),
                        ];

                        // Pretty print config JSON
                        if let Ok(config) =
                            serde_json::from_str::<serde_json::Value>(&record.config_json)
                            && let Ok(pretty) = serde_json::to_string_pretty(&config)
                        {
                            lines.extend(pretty.lines().map(|s| format!("  {}", s)));
                        }

                        lines.push(String::new());
                        lines.push("Outcome:".to_string());

                        // Pretty print outcome JSON if present
                        if let Some(outcome_json) = &record.outcome_json {
                            if let Ok(outcome) =
                                serde_json::from_str::<serde_json::Value>(outcome_json)
                                && let Ok(pretty) = serde_json::to_string_pretty(&outcome)
                            {
                                lines.extend(pretty.lines().map(|s| format!("  {}", s)));
                            }
                        } else {
                            lines.push("  (No outcome yet)".to_string());
                        }

                        lines.join("\n")
                    }
                    Ok(None) => format!("Run {} not found", entry.run_id),
                    Err(e) => format!("Error loading run: {}", e),
                }
            }
            Err(e) => format!("Database error: {}", e),
        };

        // Owned now (rather than kept as a format! borrowing `entry`) so
        // `entry`'s borrow of `app` ends here, freeing `app` for the
        // mutable scroll-bound call below.
        let run_title = format!(" Run {} ", entry.run_id);

        // This view renders unwrapped (no `.wrap(...)`, to preserve the
        // pretty-printed JSON's indentation) — so, unlike the other detail
        // views, its line count is just the raw newline count, not a
        // wrapped-line estimate.
        let inner_height = area.height.saturating_sub(2);
        let total_lines = detail_content.lines().count().max(1);
        let overflow = total_lines > inner_height as usize;
        app.set_detail_scroll_max(
            total_lines
                .saturating_sub(inner_height as usize)
                .min(u16::MAX as usize) as u16,
        );
        let scroll = app.detail_scroll;

        let mut block = Block::default()
            .borders(Borders::ALL)
            .title(run_title)
            .style(theme::border_style());
        if overflow {
            block = block.title(
                Line::from(format!(" {}/{} ", scroll + 1, total_lines))
                    .right_aligned()
                    .style(theme::muted()),
            );
        }

        let paragraph = Paragraph::new(detail_content)
            .block(block)
            .scroll((scroll, 0));

        frame.render_widget(paragraph, area);
    } else {
        app.set_detail_scroll_max(0);
        let msg = Paragraph::new("No orchestration run selected").block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Run Detail ")
                .style(theme::border_style()),
        );
        frame.render_widget(msg, area);
    }
}
