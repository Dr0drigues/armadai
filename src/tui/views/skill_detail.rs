use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let skill = match app.selected_skill() {
        Some(s) => s,
        None => {
            let msg = Paragraph::new("No skill selected. Go to Skills tab and select one.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Skill Detail "),
            );
            frame.render_widget(msg, area);
            return;
        }
    };

    let has_files =
        !skill.scripts.is_empty() || !skill.references.is_empty() || !skill.assets.is_empty();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_files {
            vec![
                Constraint::Length(3), // Title
                Constraint::Length(6), // Metadata
                Constraint::Min(6),    // Body
                Constraint::Length(5), // Files
            ]
        } else {
            vec![
                Constraint::Length(3), // Title
                Constraint::Length(6), // Metadata
                Constraint::Min(6),    // Body
            ]
        })
        .split(area);

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", skill.name),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ({})", skill.source.display()),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    // Metadata section
    let mut meta_lines = vec![];

    if let Some(ref desc) = skill.description {
        meta_lines.push(Line::from(vec![
            Span::styled(
                "Description: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(desc.as_str()),
        ]));
    }

    if let Some(ref ver) = skill.version {
        meta_lines.push(Line::from(vec![
            Span::styled(
                "Version:     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(ver.as_str()),
        ]));
    }

    if !skill.tools.is_empty() {
        meta_lines.push(Line::from(vec![
            Span::styled(
                "Tools:       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(skill.tools.join(", "), Style::default().fg(Color::Green)),
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
    let body_text = if skill.body.is_empty() {
        "(no body)".to_string()
    } else {
        skill.body.clone()
    };
    let body_widget = Paragraph::new(body_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SKILL.md ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });
    frame.render_widget(body_widget, chunks[2]);

    // Files section (only if there are files)
    if has_files {
        let file_name = |p: &std::path::Path| -> String {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        let mut file_parts: Vec<String> = Vec::new();
        if !skill.scripts.is_empty() {
            let names: Vec<String> = skill.scripts.iter().map(|p| file_name(p)).collect();
            file_parts.push(format!("scripts: {}", names.join(", ")));
        }
        if !skill.references.is_empty() {
            let names: Vec<String> = skill.references.iter().map(|p| file_name(p)).collect();
            file_parts.push(format!("references: {}", names.join(", ")));
        }
        if !skill.assets.is_empty() {
            let names: Vec<String> = skill.assets.iter().map(|p| file_name(p)).collect();
            file_parts.push(format!("assets: {}", names.join(", ")));
        }

        let files_widget = Paragraph::new(file_parts.join("\n"))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Files ")
                    .title_style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: false });
        frame.render_widget(files_widget, chunks[3]);
    }
}
