use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let prompt = match app.selected_prompt() {
        Some(p) => p,
        None => {
            let msg = Paragraph::new("No prompt selected. Go to Prompts tab and select one.")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Prompt Detail "),
                );
            frame.render_widget(msg, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(5), // Metadata
            Constraint::Min(6),    // Body
        ])
        .split(area);

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", prompt.name),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ({})", prompt.source.display()),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    // Metadata section
    let mut meta_lines = vec![];

    if let Some(ref desc) = prompt.description {
        meta_lines.push(Line::from(vec![
            Span::styled(
                "Description: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(desc.as_str()),
        ]));
    }

    if !prompt.apply_to.is_empty() {
        meta_lines.push(Line::from(vec![
            Span::styled(
                "Applies to:  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                prompt.apply_to.join(", "),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }

    if meta_lines.is_empty() {
        meta_lines.push(Line::from(Span::styled(
            "(no metadata)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let meta_widget = Paragraph::new(meta_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Metadata ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(meta_widget, chunks[1]);

    // Body section
    let body_text = if prompt.body.is_empty() {
        "(no body)".to_string()
    } else {
        prompt.body.clone()
    };
    let body_widget = Paragraph::new(body_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Content ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });
    frame.render_widget(body_widget, chunks[2]);
}
