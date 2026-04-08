use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;
use crate::tui::filter;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.models_flat.is_empty() {
        let msg = Paragraph::new("No model data cached. Run `armadai new -i` to populate.")
            .block(Block::default().borders(Borders::ALL).title(" Models "));
        frame.render_widget(msg, area);
        return;
    }

    // Apply filtering and sorting
    let display_indices =
        filter::apply_filter_and_sort_models(&app.models_flat, &app.search_query, app.sort_mode);

    if display_indices.is_empty() {
        let msg = Paragraph::new("No models match your search.")
            .block(Block::default().borders(Borders::ALL).title(" Models "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        "", "PROVIDER", "MODEL ID", "NAME", "CONTEXT", "COST IN", "COST OUT",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

    let rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_i, &model_i)| {
            let marker = if display_i == app.selected_model {
                ">"
            } else {
                " "
            };
            let (provider, entry) = &app.models_flat[model_i];
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
            let style = if display_i == app.selected_model {
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

    // Count unique providers in display
    let mut providers: Vec<&str> = display_indices
        .iter()
        .map(|&i| app.models_flat[i].0.as_str())
        .collect();
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
        " Models — {} shown, {} providers{} ",
        display_indices.len(),
        provider_count,
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
