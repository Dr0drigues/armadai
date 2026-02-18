use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::core::skill::read_text_file;
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

    // Read reference file contents
    let ref_contents: Vec<(String, String)> = skill
        .references
        .iter()
        .filter_map(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            read_text_file(p).map(|content| (name, content))
        })
        .collect();

    let has_other_files = !skill.scripts.is_empty() || !skill.assets.is_empty();

    // Build layout constraints dynamically
    let mut constraints = vec![
        Constraint::Length(3), // Title
        Constraint::Length(6), // Metadata
        Constraint::Min(6),    // Body
    ];

    // One block per reference file
    for _ in &ref_contents {
        constraints.push(Constraint::Min(4));
    }

    // Scripts/Assets summary block
    if has_other_files {
        constraints.push(Constraint::Length(3));
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
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

    // Reference file content blocks
    for (i, (name, content)) in ref_contents.iter().enumerate() {
        let ref_widget = Paragraph::new(content.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {name} "))
                    .title_style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: false });
        frame.render_widget(ref_widget, chunks[3 + i]);
    }

    // Scripts/Assets summary (compact)
    if has_other_files {
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
        if !skill.assets.is_empty() {
            let names: Vec<String> = skill.assets.iter().map(|p| file_name(p)).collect();
            file_parts.push(format!("assets: {}", names.join(", ")));
        }

        let files_widget = Paragraph::new(file_parts.join("  |  "))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Other Files ")
                    .title_style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(files_widget, chunks[3 + ref_contents.len()]);
    }
}
