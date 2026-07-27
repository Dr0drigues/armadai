use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::theme;
use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let pack = match app.selected_starter() {
        Some(p) => p,
        None => {
            let msg = Paragraph::new("No starter selected. Go to Starters tab and select one.")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Starter Detail ")
                        .style(theme::border_style()),
                );
            frame.render_widget(msg, area);
            return;
        }
    };

    let mut constraints = vec![
        Constraint::Length(3), // Title
    ];
    if !pack.agents.is_empty() {
        constraints.push(Constraint::Length(
            (pack.agents.len() as u16).saturating_add(3),
        ));
    }
    if !pack.prompts.is_empty() {
        constraints.push(Constraint::Length(
            (pack.prompts.len() as u16).saturating_add(3),
        ));
    }
    if !pack.skills.is_empty() {
        constraints.push(Constraint::Length(
            (pack.skills.len() as u16).saturating_add(3),
        ));
    }
    // Fill remaining space
    constraints.push(Constraint::Min(0));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut chunk_idx = 0;

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {} ", pack.name), theme::heading()),
        Span::styled(
            format!("  — {}", pack.description),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(theme::border_style()),
    );
    frame.render_widget(title, chunks[chunk_idx]);
    chunk_idx += 1;

    // Agents section
    if !pack.agents.is_empty() {
        let lines: Vec<Line> = pack
            .agents
            .iter()
            .map(|a| {
                // Was `fg(Color::White)` — white-on-white on a light terminal.
                Line::from(Span::styled(format!("  • {a}"), Style::default()))
            })
            .collect();
        let widget = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Agents ({}) ", pack.agents.len()))
                    .title_style(Style::default().add_modifier(Modifier::BOLD))
                    .style(theme::border_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(widget, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    // Prompts section
    if !pack.prompts.is_empty() {
        let lines: Vec<Line> = pack
            .prompts
            .iter()
            .map(|p| {
                Line::from(Span::styled(
                    format!("  • {p}"),
                    Style::default().fg(Color::Yellow),
                ))
            })
            .collect();
        let widget = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Prompts ({}) ", pack.prompts.len()))
                    .title_style(Style::default().add_modifier(Modifier::BOLD))
                    .style(theme::border_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(widget, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    // Skills section
    if !pack.skills.is_empty() {
        let lines: Vec<Line> = pack
            .skills
            .iter()
            .map(|s| {
                Line::from(Span::styled(
                    format!("  • {s}"),
                    Style::default().fg(Color::Green),
                ))
            })
            .collect();
        let widget = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Skills ({}) ", pack.skills.len()))
                    .title_style(Style::default().add_modifier(Modifier::BOLD))
                    .style(theme::border_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(widget, chunks[chunk_idx]);
    }
}
