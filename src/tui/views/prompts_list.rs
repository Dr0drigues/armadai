use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.prompts.is_empty() {
        let msg = Paragraph::new("No prompts found. Add .md files to ~/.config/armadai/prompts/")
            .block(Block::default().borders(Borders::ALL).title(" Prompts "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec!["", "NAME", "DESCRIPTION", "APPLIES TO", "SOURCE"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows: Vec<Row> = app
        .prompts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let marker = if i == app.selected_prompt { ">" } else { " " };
            let style = if i == app.selected_prompt {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                marker.to_string(),
                p.name.clone(),
                p.description.clone().unwrap_or_default(),
                p.apply_to.join(", "),
                p.source
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
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
            Constraint::Length(20),
            Constraint::Length(25),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Prompts — {} loaded ", app.prompts.len())),
    );

    frame.render_widget(table, area);
}
