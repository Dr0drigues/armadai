use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::theme;
use crate::tui::app::App;
use crate::tui::filter;
use crate::tui::widgets::search_bar;

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
                theme::selection()
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
        search_bar(frame, &app.search_query, area);
    }
}
