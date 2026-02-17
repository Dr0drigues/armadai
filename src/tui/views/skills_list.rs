use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.skills.is_empty() {
        let msg =
            Paragraph::new("No skills found. Install built-in skills with: armadai init --skills")
                .block(Block::default().borders(Borders::ALL).title(" Skills "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec!["NAME", "DESCRIPTION", "VERSION", "TOOLS", "SOURCE"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows: Vec<Row> = app
        .skills
        .iter()
        .map(|s| {
            Row::new(vec![
                s.name.clone(),
                s.description.clone().unwrap_or_default(),
                s.version.clone().unwrap_or_default(),
                s.tools.join(", "),
                s.source
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(15),
            Constraint::Min(25),
            Constraint::Length(10),
            Constraint::Length(20),
            Constraint::Length(25),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Skills — {} loaded ", app.skills.len())),
    );

    frame.render_widget(table, area);
}
