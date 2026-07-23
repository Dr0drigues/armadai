use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::theme;
use crate::tui::app::App;
use crate::tui::filter;
use crate::tui::format::format_cost;
use crate::tui::widgets::search_bar;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.costs.is_empty() {
        let msg = Paragraph::new("No cost data. Run agents to start tracking costs.")
            .block(Block::default().borders(Borders::ALL).title(" Costs "));
        frame.render_widget(msg, area);
        return;
    }

    // Apply filtering and sorting (default: cost descending — see
    // `filter::apply_filter_and_sort_costs`).
    let display_indices =
        filter::apply_filter_and_sort_costs(&app.costs, &app.search_query, app.sort_mode);

    if display_indices.is_empty() {
        let msg = Paragraph::new("No cost entries match your search.")
            .block(Block::default().borders(Borders::ALL).title(" Costs "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        "",
        "AGENT",
        "RUNS",
        "COST (USD)",
        "TOKENS IN",
        "TOKENS OUT",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

    let mut rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_i, &cost_i)| {
            let marker = if display_i == app.selected_cost {
                ">"
            } else {
                " "
            };
            let c = &app.costs[cost_i];
            let style = if display_i == app.selected_cost {
                theme::selection()
            } else {
                Style::default()
            };
            Row::new(vec![
                marker.to_string(),
                c.agent.clone(),
                c.total_runs.to_string(),
                format_cost(c.total_cost),
                c.total_tokens_in.to_string(),
                c.total_tokens_out.to_string(),
            ])
            .style(style)
        })
        .collect();

    // Grand total row over the *full* cost set (not just the filtered/shown
    // rows), visually set apart with a blank separator margin + bold text so
    // it never looks like just another (unselectable) agent row.
    let total_cost: f64 = app.costs.iter().map(|c| c.total_cost).sum();
    rows.push(
        Row::new(vec![
            String::new(),
            "TOTAL".to_string(),
            String::new(),
            format_cost(total_cost),
            String::new(),
            String::new(),
        ])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .top_margin(1),
    );

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(15),
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(format!(
        " Costs — {} loaded, {} shown{} ",
        app.costs.len(),
        display_indices.len(),
        app.sort_indicator()
    )));

    frame.render_widget(table, area);

    // Render search bar if in search mode
    if app.search_mode {
        search_bar(frame, app, &app.search_query, area);
    }
}
