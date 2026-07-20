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
    if app.prompts.is_empty() {
        let msg = Paragraph::new("No prompts found. Add .md files to ~/.config/armadai/prompts/")
            .block(Block::default().borders(Borders::ALL).title(" Prompts "));
        frame.render_widget(msg, area);
        return;
    }

    // Apply filtering and sorting
    let display_indices =
        filter::apply_filter_and_sort_prompts(&app.prompts, &app.search_query, app.sort_mode);

    if display_indices.is_empty() {
        let msg = Paragraph::new("No prompts match your search.")
            .block(Block::default().borders(Borders::ALL).title(" Prompts "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec!["", "NAME", "DESCRIPTION", "APPLIES TO", "SOURCE"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_i, &prompt_i)| {
            let marker = if display_i == app.selected_prompt {
                ">"
            } else {
                " "
            };
            let p = &app.prompts[prompt_i];
            let style = if display_i == app.selected_prompt {
                theme::selection()
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
    .block(Block::default().borders(Borders::ALL).title(format!(
        " Prompts — {} loaded, {} shown{} ",
        app.prompts.len(),
        display_indices.len(),
        app.sort_indicator()
    )));

    frame.render_widget(table, area);

    // Render search bar if in search mode
    if app.search_mode {
        search_bar(frame, &app.search_query, area);
    }
}
