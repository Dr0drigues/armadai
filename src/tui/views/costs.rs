use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.costs.is_empty() {
        let msg = Paragraph::new("No cost data. Run agents to start tracking costs.")
            .block(Block::default().borders(Borders::ALL).title(" Costs "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        "AGENT",
        "RUNS",
        "COST (USD)",
        "TOKENS IN",
        "TOKENS OUT",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

    let mut total_cost = 0.0;

    let rows: Vec<Row> = app
        .costs
        .iter()
        .map(|c| {
            total_cost += c.total_cost;
            Row::new(vec![
                c.agent.clone(),
                c.total_runs.to_string(),
                format!("${:.6}", c.total_cost),
                c.total_tokens_in.to_string(),
                c.total_tokens_out.to_string(),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(15),
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Costs — total: ${total_cost:.4} ")),
    );

    frame.render_widget(table, area);
}
