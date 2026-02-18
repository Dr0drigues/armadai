use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.starters.is_empty() {
        let msg = Paragraph::new("No starter packs found.")
            .block(Block::default().borders(Borders::ALL).title(" Starters "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        "",
        "NAME",
        "DESCRIPTION",
        "AGENTS",
        "PROMPTS",
        "SKILLS",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .starters
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let marker = if i == app.selected_starter { ">" } else { " " };
            let style = if i == app.selected_starter {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                marker.to_string(),
                p.name.clone(),
                p.description.clone(),
                p.agents.len().to_string(),
                p.prompts.len().to_string(),
                p.skills.len().to_string(),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(15),
            Constraint::Min(25),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Starters — {} packs ", app.starters.len())),
    );

    frame.render_widget(table, area);
}
