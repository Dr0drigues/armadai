use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let agent = match app.selected_agent() {
        Some(a) => a,
        None => {
            let msg = Paragraph::new("No agent selected. Go to Agents tab and select one.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Agent Detail "),
            );
            frame.render_widget(msg, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(8), // Metadata
            Constraint::Min(6),    // System Prompt
            Constraint::Length(6), // Instructions (if any)
        ])
        .split(area);

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", agent.name),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ({})", agent.source.display()),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    // Metadata section
    let meta = &agent.metadata;
    let mut meta_lines = vec![
        Line::from(vec![
            Span::styled("Provider: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&meta.provider),
        ]),
        Line::from(vec![
            Span::styled("Model:    ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(agent.model_display()),
        ]),
    ];

    if !meta.tags.is_empty() {
        meta_lines.push(Line::from(vec![
            Span::styled("Tags:     ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(meta.tags.join(", "), Style::default().fg(Color::Yellow)),
        ]));
    }

    if !meta.stacks.is_empty() {
        meta_lines.push(Line::from(vec![
            Span::styled("Stacks:   ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(meta.stacks.join(", "), Style::default().fg(Color::Green)),
        ]));
    }

    if let Some(ref rl) = meta.rate_limit {
        meta_lines.push(Line::from(vec![
            Span::styled("Rate:     ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(rl.as_str()),
        ]));
    }

    if let Some(timeout) = meta.timeout {
        meta_lines.push(Line::from(vec![
            Span::styled("Timeout:  ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("{timeout}s")),
        ]));
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

    // System Prompt section
    let prompt_text = if agent.system_prompt.is_empty() {
        "(no system prompt)".to_string()
    } else {
        agent.system_prompt.clone()
    };
    let prompt_widget = Paragraph::new(prompt_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" System Prompt ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });
    frame.render_widget(prompt_widget, chunks[2]);

    // Instructions section
    let instr_text = agent.instructions.as_deref().unwrap_or("(no instructions)");
    let instr_widget = Paragraph::new(instr_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Instructions ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .style(Style::default().fg(Color::Gray))
        .wrap(Wrap { trim: false });
    frame.render_widget(instr_widget, chunks[3]);
}
