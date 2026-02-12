use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.history.is_empty() {
        let msg =
            Paragraph::new("No execution history. Run an agent first: armadai run <agent> <input>")
                .block(Block::default().borders(Borders::ALL).title(" History "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        "", "AGENT", "PROVIDER", "MODEL", "IN", "OUT", "COST", "MS", "STATUS",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .history
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let marker = if i == app.selected_history { ">" } else { " " };
            let model_short = if r.model.len() > 18 {
                format!("{}...", &r.model[..17])
            } else {
                r.model.clone()
            };
            let style = if i == app.selected_history {
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
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" History — {} runs ", app.history.len())),
    );

    frame.render_widget(table, area);
}
