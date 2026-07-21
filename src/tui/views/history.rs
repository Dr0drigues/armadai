use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::theme;
use crate::tui::app::App;
use crate::tui::filter;
use crate::tui::format::format_cost;
use crate::tui::widgets::search_bar;

/// Display value for the History table's PROJECT column: the basename of the
/// project root path (the full path is long; the last component is enough
/// to tell projects apart), or `—` when the run has no associated project
/// (ad-hoc runs outside any `armadai.yaml`, or pre-migration rows).
fn project_display(project: Option<&str>) -> String {
    project
        .and_then(|p| std::path::Path::new(p).file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "—".to_string())
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.history.is_empty() {
        let msg =
            Paragraph::new("No execution history. Run an agent first: armadai run <agent> <input>")
                .block(Block::default().borders(Borders::ALL).title(" History "));
        frame.render_widget(msg, area);
        return;
    }

    // Apply filtering and sorting
    let display_indices =
        filter::apply_filter_and_sort_history(&app.history, &app.search_query, app.sort_mode);

    if display_indices.is_empty() {
        let msg = Paragraph::new("No history entries match your search.")
            .block(Block::default().borders(Borders::ALL).title(" History "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        "", "AGENT", "PROJECT", "PROVIDER", "MODEL", "IN", "OUT", "COST", "MS", "STATUS",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

    let rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_i, &history_i)| {
            let marker = if display_i == app.selected_history {
                ">"
            } else {
                " "
            };
            let r = &app.history[history_i];
            let model_short = if r.model.len() > 18 {
                format!("{}...", &r.model[..17])
            } else {
                r.model.clone()
            };
            let project_short = project_display(r.project.as_deref());
            let style = if display_i == app.selected_history {
                theme::selection()
            } else {
                Style::default()
            };
            Row::new(vec![
                marker.to_string(),
                r.agent.clone(),
                project_short,
                r.provider.clone(),
                model_short,
                r.tokens_in.to_string(),
                r.tokens_out.to_string(),
                format_cost(r.cost),
                r.duration_ms.to_string(),
                r.status.clone(),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(16),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(format!(
        " History — {} runs, {} shown{} ",
        app.history.len(),
        display_indices.len(),
        app.sort_indicator()
    )));

    frame.render_widget(table, area);

    // Render search bar if in search mode
    if app.search_mode {
        search_bar(frame, &app.search_query, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_display_basename_of_a_path() {
        assert_eq!(
            project_display(Some("/home/user/projects/my-app")),
            "my-app"
        );
    }

    #[test]
    fn project_display_none_shows_placeholder() {
        assert_eq!(project_display(None), "—");
    }

    #[test]
    fn project_display_trailing_slash_still_resolves_basename() {
        // Path::file_name ignores a trailing separator.
        assert_eq!(
            project_display(Some("/home/user/projects/my-app/")),
            "my-app"
        );
    }
}
