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
use armadai_core::skill::read_text_file;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let skill = match app.selected_skill() {
        Some(s) => s,
        None => {
            app.set_detail_scroll_max(0);
            let msg = Paragraph::new("No skill selected. Go to Skills tab and select one.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Skill Detail ")
                    .style(theme::border_style()),
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

    // Scripts/Assets summary line, extracted as owned data now (rather than
    // read from `skill` further down, after the scrollable body section)
    // so `skill`'s borrow of `app` ends here — freeing `app` for the
    // mutable scroll-bound call the body section needs below.
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
    let has_other_files = !file_parts.is_empty();

    // Build layout constraints dynamically
    let mut constraints = vec![
        Constraint::Length(3), // Title
        Constraint::Length(6), // Metadata
        Constraint::Min(0),    // Body (scrollable — j/k)
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
        Span::styled(format!(" {} ", skill.name), theme::heading()),
        Span::styled(
            format!("  ({})", skill.source.display()),
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
                .title_style(Style::default().add_modifier(Modifier::BOLD))
                .style(theme::border_style()),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(meta_widget, chunks[1]);

    // Body section
    let body_text = if skill.body.is_empty() {
        "(no body)".to_string()
    } else {
        skill.body.clone()
    };

    let body_area = chunks[2];
    let inner_width = body_area.width.saturating_sub(2);
    let inner_height = body_area.height.saturating_sub(2);
    let total_lines = wrapped_line_count(&body_text, inner_width);
    let overflow = total_lines > inner_height as usize;
    app.set_detail_scroll_max(
        total_lines
            .saturating_sub(inner_height as usize)
            .min(u16::MAX as usize) as u16,
    );
    let scroll = app.detail_scroll;

    let mut body_block = Block::default()
        .borders(Borders::ALL)
        .title(" SKILL.md ")
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
                    )
                    .style(theme::border_style()),
            )
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: false });
        frame.render_widget(ref_widget, chunks[3 + i]);
    }

    // Scripts/Assets summary (compact)
    if has_other_files {
        let files_widget = Paragraph::new(file_parts.join("  |  "))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Other Files ")
                    .title_style(Style::default().add_modifier(Modifier::BOLD))
                    .style(theme::border_style()),
            )
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(files_widget, chunks[3 + ref_contents.len()]);
    }
}
