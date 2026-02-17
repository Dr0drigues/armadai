use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::app::{App, Tab};

/// Render the keyboard shortcuts bar at the bottom of the screen.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let shortcuts = match app.current_tab {
        Tab::Dashboard => vec![
            ("j/k", "Navigate"),
            ("Enter", "View detail"),
            ("Tab", "Next tab"),
            (":", "Commands"),
            ("r", "Refresh"),
            ("q", "Quit"),
        ],
        Tab::AgentDetail => vec![
            ("Esc", "Back to list"),
            ("j/k", "Scroll"),
            ("Tab", "Next tab"),
            (":", "Commands"),
            ("q", "Quit"),
        ],
        Tab::History => vec![
            ("j/k", "Navigate"),
            ("Tab", "Next tab"),
            (":", "Commands"),
            ("r", "Refresh"),
            ("q", "Quit"),
        ],
        Tab::Costs => vec![
            ("Tab", "Next tab"),
            (":", "Commands"),
            ("r", "Refresh"),
            ("q", "Quit"),
        ],
        Tab::Prompts => vec![
            ("Tab", "Next tab"),
            (":", "Commands"),
            ("r", "Refresh"),
            ("q", "Quit"),
        ],
        Tab::Skills => vec![
            ("Tab", "Next tab"),
            (":", "Commands"),
            ("r", "Refresh"),
            ("q", "Quit"),
        ],
    };

    let spans: Vec<Span> = shortcuts
        .into_iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(
                    format!(" {key} "),
                    Style::default()
                        .bg(Color::DarkGray)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {desc}  "), Style::default().fg(Color::Gray)),
            ]
        })
        .collect();

    let bar = Paragraph::new(Line::from(spans));
    frame.render_widget(bar, area);
}
