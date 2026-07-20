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
    // Quit and tab-jump are available from every tab (P1-4 / P2-5): document
    // them once here instead of repeating the pair in each arm below.
    const QUIT: (&str, &str) = ("q / ^C", "Quit");
    const JUMP: (&str, &str) = ("1-8", "Jump tab");

    let shortcuts = match app.current_tab {
        Tab::Dashboard => vec![
            ("j/k", "Navigate"),
            ("Enter", "View detail"),
            ("/", "Search"),
            ("s", "Sort"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT,
        ],
        Tab::AgentDetail | Tab::PromptDetail | Tab::SkillDetail => vec![
            ("j/k", "Scroll"),
            ("Esc", "Back to list"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            QUIT,
        ],
        Tab::ModelDetail => vec![
            ("Esc", "Back to list"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            QUIT,
        ],
        Tab::StarterDetail => vec![
            ("Esc", "Back to list"),
            ("i", "Init project"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            QUIT,
        ],
        Tab::Prompts | Tab::Skills => vec![
            ("j/k", "Navigate"),
            ("Enter", "View detail"),
            ("/", "Search"),
            ("s", "Sort"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT,
        ],
        Tab::Starters => vec![
            ("j/k", "Navigate"),
            ("Enter", "View detail"),
            ("/", "Search"),
            ("s", "Sort"),
            ("i", "Init project"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT,
        ],
        Tab::History => vec![
            ("j/k", "Navigate"),
            ("/", "Search"),
            ("s", "Sort"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT,
        ],
        Tab::Costs => vec![
            ("j/k", "Navigate"),
            ("/", "Search"),
            ("s", "Sort"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT,
        ],
        Tab::Models => vec![
            ("j/k", "Navigate"),
            ("Enter", "View detail"),
            ("/", "Search"),
            ("s", "Sort"),
            ("R", "Sync models.dev"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT,
        ],
        #[cfg(feature = "storage")]
        Tab::Orchestration => vec![
            ("j/k", "Navigate"),
            ("Enter", "View detail"),
            ("/", "Search"),
            ("s", "Sort"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT,
        ],
        #[cfg(feature = "storage")]
        Tab::OrchestrationDetail => vec![
            ("j/k", "Scroll"),
            ("Esc", "Back to list"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            QUIT,
        ],
        #[cfg(not(feature = "storage"))]
        Tab::Orchestration | Tab::OrchestrationDetail => {
            vec![("Tab", "Next tab"), JUMP, (":", "Commands"), QUIT]
        }
    };

    let mut spans: Vec<Span> = shortcuts
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

    // Append status message if present
    if let Some(ref msg) = app.status_msg {
        spans.push(Span::styled(
            format!("  {msg}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::ITALIC),
        ));
    }

    let bar = Paragraph::new(Line::from(spans));
    frame.render_widget(bar, area);
}
