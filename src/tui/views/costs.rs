use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Text,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::theme;
use crate::tui::app::App;
use crate::tui::filter;
use crate::tui::format::format_cost;
use crate::tui::widgets::search_bar;

/// Right-align a cell's content (numeric columns), matching the convention
/// that numbers line up on their least-significant digit.
fn right(text: impl Into<Text<'static>>) -> Cell<'static> {
    Cell::from(text.into().right_aligned())
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if app.costs.is_empty() {
        let msg = Paragraph::new("No cost data. Run agents to start tracking costs.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Costs ")
                .style(theme::border_style()),
        );
        frame.render_widget(msg, area);
        return;
    }

    // Apply filtering and sorting (default: cost descending — see
    // `filter::apply_filter_and_sort_costs`).
    let display_indices =
        filter::apply_filter_and_sort_costs(&app.costs, &app.search_query, app.sort_mode);

    if display_indices.is_empty() {
        let msg = Paragraph::new("No cost entries match your search.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Costs ")
                .style(theme::border_style()),
        );
        frame.render_widget(msg, area);
        return;
    }

    // Column labels: right-aligned over the numeric columns (matching the
    // values underneath) and muted, per the design-system convention that
    // headers/labels use `theme::muted()`.
    let header_style = theme::muted().add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("AGENT"),
        right("RUNS"),
        right("COST (USD)"),
        right("TOKENS IN"),
        right("TOKENS OUT"),
    ])
    .style(header_style)
    .bottom_margin(1);

    // Runs/tokens are secondary to the agent name and its cost (the point
    // of this view), so they're muted to let AGENT/COST stand out — even
    // on a selected row, where the row-level selection style still wins
    // for foreground boldness but the muted fg keeps them de-emphasized.
    let secondary_style = theme::muted();

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
                Cell::from(marker),
                Cell::from(c.agent.clone()),
                right(c.total_runs.to_string()).style(secondary_style),
                right(format_cost(c.total_cost)),
                right(c.total_tokens_in.to_string()).style(secondary_style),
                right(c.total_tokens_out.to_string()).style(secondary_style),
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
            Cell::from(""),
            Cell::from("TOTAL"),
            Cell::from(""),
            right(format_cost(total_cost)),
            Cell::from(""),
            Cell::from(""),
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
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Costs — {} loaded, {} shown{} ",
                app.costs.len(),
                display_indices.len(),
                app.sort_indicator()
            ))
            .style(theme::border_style()),
    );

    frame.render_widget(table, area);

    // Render search bar if in search mode
    if app.search_mode {
        search_bar(frame, &app.search_query, area);
    }
}
