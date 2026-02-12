use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::tui::app::App;

/// Render the command palette as a centered overlay.
pub fn render(frame: &mut Frame, app: &App) {
    if !app.palette.visible {
        return;
    }

    let area = centered_rect(50, 40, frame.area());

    // Clear the area behind the palette
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    // Input field
    let input = Paragraph::new(Line::from(vec![
        Span::styled(": ", Style::default().fg(Color::Cyan)),
        Span::raw(&app.palette.input),
        Span::styled("_", Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Command Palette ")
            .title_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(input, chunks[0]);

    // Command list
    let items: Vec<ListItem> = app
        .palette
        .filtered
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let style = if i == app.palette.selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let line = Line::from(vec![
                Span::styled(&cmd.name, style),
                Span::styled(
                    format!("  {}", cmd.description),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(list, chunks[1]);
}

/// Create a centered rectangle of `percent_x` width and `percent_y` height.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
