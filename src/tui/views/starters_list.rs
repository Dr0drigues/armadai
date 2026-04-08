use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::tui::app::App;
use crate::tui::filter;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.starters.is_empty() {
        let msg = Paragraph::new("No starter packs found.")
            .block(Block::default().borders(Borders::ALL).title(" Starters "));
        frame.render_widget(msg, area);
        return;
    }

    // Apply filtering and sorting
    let display_indices =
        filter::apply_filter_and_sort_starters(&app.starters, &app.search_query, app.sort_mode);

    if display_indices.is_empty() {
        let msg = Paragraph::new("No starters match your search.")
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

    let rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_i, &starter_i)| {
            let marker = if display_i == app.selected_starter {
                ">"
            } else {
                " "
            };
            let p = &app.starters[starter_i];
            let style = if display_i == app.selected_starter {
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
    .block(Block::default().borders(Borders::ALL).title(format!(
        " Starters — {} packs, {} shown{} ",
        app.starters.len(),
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
