use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let (provider, entry) = match app.selected_model_entry() {
        Some(e) => e,
        None => {
            let msg = Paragraph::new("No model selected. Go to Models tab and select one.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Model Detail "),
            );
            frame.render_widget(msg, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(9), // Details
            Constraint::Min(3),    // Display label
        ])
        .split(area);

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", entry.id),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ({})", provider),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    // Details section
    let name = entry.name.as_deref().unwrap_or("(unnamed)");
    let context = entry
        .limit
        .as_ref()
        .and_then(|l| l.context)
        .map(|c| format!("{}", c))
        .unwrap_or_else(|| "(unknown)".to_string());
    let max_output = entry
        .limit
        .as_ref()
        .and_then(|l| l.output)
        .map(|o| format!("{}", o))
        .unwrap_or_else(|| "(unknown)".to_string());
    let cost_in = entry
        .cost
        .as_ref()
        .and_then(|c| c.input)
        .map(|v| format!("${:.2}", v))
        .unwrap_or_else(|| "(unknown)".to_string());
    let cost_out = entry
        .cost
        .as_ref()
        .and_then(|c| c.output)
        .map(|v| format!("${:.2}", v))
        .unwrap_or_else(|| "(unknown)".to_string());

    let bold = Style::default().add_modifier(Modifier::BOLD);
    let detail_lines = vec![
        Line::from(vec![
            Span::styled("Provider:       ", bold),
            Span::raw(provider),
        ]),
        Line::from(vec![
            Span::styled("Name:           ", bold),
            Span::raw(name),
        ]),
        Line::from(vec![
            Span::styled("ID:             ", bold),
            Span::raw(&entry.id),
        ]),
        Line::from(vec![
            Span::styled("Context Window: ", bold),
            Span::raw(&context),
        ]),
        Line::from(vec![
            Span::styled("Max Output:     ", bold),
            Span::raw(&max_output),
        ]),
        Line::from(vec![
            Span::styled("Cost (input):   ", bold),
            Span::styled(&cost_in, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Cost (output):  ", bold),
            Span::styled(&cost_out, Style::default().fg(Color::Yellow)),
        ]),
    ];

    let detail_widget = Paragraph::new(detail_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Details ")
                .title_style(bold),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(detail_widget, chunks[1]);

    // Display label section
    let label = entry.display_label();
    let label_widget = Paragraph::new(label)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Display Label ")
                .title_style(bold),
        )
        .style(Style::default().fg(Color::Green))
        .wrap(Wrap { trim: false });
    frame.render_widget(label_widget, chunks[2]);
}
