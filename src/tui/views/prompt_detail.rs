use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::theme;
use crate::tui::app::App;
use crate::tui::wrap::wrapped_line_count;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let prompt = match app.selected_prompt() {
        Some(p) => p,
        None => {
            app.set_detail_scroll_max(0);
            let msg = Paragraph::new("No prompt selected. Go to Prompts tab and select one.")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Prompt Detail ")
                        .style(theme::border_style()),
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
            Constraint::Min(0),    // Body (scrollable — j/k)
        ])
        .split(area);

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {} ", prompt.name), theme::heading()),
        Span::styled(
            format!("  ({})", prompt.source.display()),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(theme::border_style()),
    );
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
                .title_style(Style::default().add_modifier(Modifier::BOLD))
                .style(theme::border_style()),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(meta_widget, chunks[1]);

    // Body section
    let body_text = if prompt.body.is_empty() {
        "(no body)".to_string()
    } else {
        prompt.body.clone()
    };

    let body_area = chunks[2];
    let inner_width = body_area.width.saturating_sub(2);
    let inner_height = body_area.height.saturating_sub(2);
    let total_lines = wrapped_line_count(&body_text, inner_width);
    let overflow = total_lines > inner_height as usize;
    app.set_detail_scroll_max(total_lines.saturating_sub(inner_height as usize) as u16);
    let scroll = app.detail_scroll;

    let mut body_block = Block::default()
        .borders(Borders::ALL)
        .title(" Content ")
        .title_style(Style::default().add_modifier(Modifier::BOLD))
        .style(theme::border_style());
    if overflow {
        body_block = body_block.title(
            Line::from(format!(" {}/{} ", scroll + 1, total_lines))
                .right_aligned()
                .style(theme::muted()),
        );
    }
    let body_widget = Paragraph::new(body_text)
        .block(body_block)
        // Was `fg(Color::White)` — white-on-white on a light terminal.
        .style(Style::default())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(body_widget, body_area);
}
