use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;
use crate::tui::filter;

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
        "", "AGENT", "PROVIDER", "MODEL", "IN", "OUT", "COST", "MS", "STATUS",
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
            let style = if display_i == app.selected_history {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                marker.to_string(),
                r.agent.clone(),
                r.provider.clone(),
                model_short,
                r.tokens_in.to_string(),
                r.tokens_out.to_string(),
                format!("{:.4}", r.cost),
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
            Constraint::Min(12),
            Constraint::Length(10),
            Constraint::Length(20),
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
        render_search_bar(frame, app, area);
    }
}

fn render_search_bar(frame: &mut Frame, app: &App, list_area: Rect) {
    let search_area = ratatui::layout::Rect {
        x: list_area.x,
        y: list_area.bottom() - 1,
        width: list_area.width,
        height: 1,
    };

    let query_display = format!("/ {}\u{2588}", app.search_query);
    let search = Paragraph::new(query_display)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default());
    frame.render_widget(search, search_area);
}
