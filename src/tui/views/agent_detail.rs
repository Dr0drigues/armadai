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

    // Determine if orchestration section is needed
    let has_orchestration = agent.metadata.orchestration.is_some()
        || agent.metadata.triggers.is_some()
        || agent.metadata.ring_config.is_some();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_orchestration {
            vec![
                Constraint::Length(3), // Title
                Constraint::Length(8), // Metadata
                Constraint::Length(5), // Orchestration
                Constraint::Length(8), // Model Resolution
                Constraint::Min(6),    // System Prompt
                Constraint::Length(6), // Instructions
            ]
        } else {
            vec![
                Constraint::Length(3), // Title
                Constraint::Length(8), // Metadata
                Constraint::Length(8), // Model Resolution
                Constraint::Min(6),    // System Prompt
                Constraint::Length(6), // Instructions
            ]
        })
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

    if !meta.model_fallback.is_empty() {
        meta_lines.push(Line::from(vec![
            Span::styled("Fallback: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                meta.model_fallback.join(", "),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

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

    if !meta.scope.is_empty() {
        meta_lines.push(Line::from(vec![
            Span::styled("Scope:    ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(meta.scope.join(", "), Style::default().fg(Color::Cyan)),
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

    // Orchestration section (conditional)
    let orch_offset: usize = if has_orchestration {
        let mut orch_lines: Vec<Line> = Vec::new();

        if let Some(ref pattern) = agent.metadata.orchestration {
            orch_lines.push(Line::from(vec![
                Span::styled("Pattern:  ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(pattern.to_string(), Style::default().fg(Color::Magenta)),
            ]));
        }

        if let Some(ref triggers) = agent.metadata.triggers {
            let mut parts = Vec::new();
            if !triggers.requires.is_empty() {
                parts.push(format!("requires: [{}]", triggers.requires.join(", ")));
            }
            if !triggers.excludes.is_empty() {
                parts.push(format!("excludes: [{}]", triggers.excludes.join(", ")));
            }
            if triggers.min_round > 0 {
                parts.push(format!("min_round: {}", triggers.min_round));
            }
            if let Some(max) = triggers.max_round {
                parts.push(format!("max_round: {max}"));
            }
            parts.push(format!("priority: {}", triggers.priority));
            orch_lines.push(Line::from(vec![
                Span::styled("Triggers: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(parts.join(", "), Style::default().fg(Color::Yellow)),
            ]));
        }

        if let Some(ref ring) = agent.metadata.ring_config {
            let mut parts = vec![format!("role: {}", ring.role)];
            if let Some(pos) = ring.position {
                parts.push(format!("position: {pos}"));
            }
            if (ring.vote_weight - 1.0).abs() > f32::EPSILON {
                parts.push(format!("weight: {:.1}", ring.vote_weight));
            }
            orch_lines.push(Line::from(vec![
                Span::styled("Ring:     ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(parts.join(", "), Style::default().fg(Color::Blue)),
            ]));
        }

        let orch_widget = Paragraph::new(orch_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Orchestration ")
                    .title_style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(orch_widget, chunks[2]);
        1 // offset for subsequent chunk indices
    } else {
        0
    };

    // Model Resolution section
    let resolution =
        crate::linker::model_resolution::preview_model_resolution(agent.metadata.model.as_deref());
    let res_lines: Vec<Line> = resolution
        .iter()
        .map(|(target, resolved)| {
            Line::from(vec![
                Span::styled(
                    format!("{:<10}", target),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(" → ", Style::default().fg(Color::DarkGray)),
                Span::styled(resolved.as_str(), Style::default().fg(Color::Green)),
            ])
        })
        .collect();
    let res_widget = Paragraph::new(res_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Model Resolution (link targets) ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(res_widget, chunks[2 + orch_offset]);

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
    frame.render_widget(prompt_widget, chunks[3 + orch_offset]);

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
    frame.render_widget(instr_widget, chunks[4 + orch_offset]);
}
