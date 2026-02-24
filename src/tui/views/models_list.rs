use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.models_flat.is_empty() {
        let msg = Paragraph::new("No model data cached. Run `armadai new -i` to populate.")
            .block(Block::default().borders(Borders::ALL).title(" Models "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        "", "PROVIDER", "MODEL ID", "NAME", "CONTEXT", "COST IN", "COST OUT",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .models_flat
        .iter()
        .enumerate()
        .map(|(i, (provider, entry))| {
            let marker = if i == app.selected_model { ">" } else { " " };
            let name = entry.name.as_deref().unwrap_or("-").to_string();
            let context = entry
                .limit
                .as_ref()
                .and_then(|l| l.context)
                .map(|c| format!("{}K", c / 1000))
                .unwrap_or_else(|| "-".to_string());
            let cost_in = entry
                .cost
                .as_ref()
                .and_then(|c| c.input)
                .map(|v| format!("${:.2}", v))
                .unwrap_or_else(|| "-".to_string());
            let cost_out = entry
                .cost
                .as_ref()
                .and_then(|c| c.output)
                .map(|v| format!("${:.2}", v))
                .unwrap_or_else(|| "-".to_string());
            let style = if i == app.selected_model {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                marker.to_string(),
                provider.clone(),
                entry.id.clone(),
                name,
                context,
                cost_in,
                cost_out,
            ])
            .style(style)
        })
        .collect();

    // Count unique providers
    let mut providers: Vec<&str> = app.models_flat.iter().map(|(p, _)| p.as_str()).collect();
    providers.dedup();
    let provider_count = providers.len();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(12),
            Constraint::Min(20),
            Constraint::Min(15),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(format!(
        " Models — {} models, {} providers ",
        app.models_flat.len(),
        provider_count
    )));

    frame.render_widget(table, area);
}
