use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::theme;
use crate::tui::app::{App, Tab};

/// Render the keyboard shortcuts bar at the bottom of the screen.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    // Top-level "press Esc again to quit" is armed (see
    // `App::esc_armed` / `handle_top_level_esc` in `tui/mod.rs`): replace
    // the whole bar with a prominent warning, mirroring the shell TUI's
    // `render_hint_bar` (`src/shell/tui.rs`) — including its wording.
    if app.esc_armed {
        let bar = Paragraph::new("Press Esc again to quit").style(theme::warning());
        frame.render_widget(bar, area);
        return;
    }

    // Tab-jump is available from every tab (P2-5): document it once here
    // instead of repeating it in each arm below. Quit has two flavors: at
    // the top level `q`/Ctrl+C quit instantly and Esc arms a confirming
    // second press; in a detail view Esc instead goes back (documented
    // separately per arm), so only `q`/Ctrl+C quit there.
    const QUIT_TOP_LEVEL: (&str, &str) = ("q / Esc×2 / ^C", "Quit");
    const QUIT_DETAIL: (&str, &str) = ("q / ^C", "Quit");
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
            QUIT_TOP_LEVEL,
        ],
        Tab::AgentDetail | Tab::PromptDetail | Tab::SkillDetail => vec![
            ("j/k", "Scroll"),
            ("PgUp/PgDn", "Page"),
            ("Esc", "Back to list"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            QUIT_DETAIL,
        ],
        Tab::ModelDetail => vec![
            ("Esc", "Back to list"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            QUIT_DETAIL,
        ],
        Tab::StarterDetail => vec![
            ("Esc", "Back to list"),
            ("i", "Init project"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            QUIT_DETAIL,
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
            QUIT_TOP_LEVEL,
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
            QUIT_TOP_LEVEL,
        ],
        Tab::History => vec![
            ("j/k", "Navigate"),
            ("/", "Search"),
            ("s", "Sort"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT_TOP_LEVEL,
        ],
        Tab::Costs => vec![
            ("j/k", "Navigate"),
            ("/", "Search"),
            ("s", "Sort"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            ("r", "Refresh"),
            QUIT_TOP_LEVEL,
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
            QUIT_TOP_LEVEL,
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
            QUIT_TOP_LEVEL,
        ],
        #[cfg(feature = "storage")]
        Tab::OrchestrationDetail => vec![
            ("j/k", "Scroll"),
            ("PgUp/PgDn", "Page"),
            ("Esc", "Back to list"),
            ("Tab", "Next tab"),
            JUMP,
            (":", "Commands"),
            QUIT_DETAIL,
        ],
        #[cfg(not(feature = "storage"))]
        Tab::Orchestration | Tab::OrchestrationDetail => {
            vec![("Tab", "Next tab"), JUMP, (":", "Commands"), QUIT_TOP_LEVEL]
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    /// Render the shortcuts bar into a small offscreen buffer and return its
    /// content as a single string, for substring assertions. Wide enough
    /// that the longest tab's full shortcut list isn't clipped (the bar
    /// itself doesn't wrap, matching how it actually renders).
    fn rendered_text(app: &App) -> String {
        let backend = TestBackend::new(200, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, app, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn armed_esc_replaces_the_bar_with_the_quit_warning() {
        let mut app = App::new();
        app.esc_armed = true;
        let text = rendered_text(&app);
        assert!(
            text.contains("Press Esc again to quit"),
            "armed bar should show the quit warning, got: {text:?}"
        );
    }

    #[test]
    fn unarmed_bar_shows_the_normal_shortcuts_not_the_warning() {
        let app = App::new();
        let text = rendered_text(&app);
        assert!(!text.contains("Press Esc again to quit"));
        assert!(text.contains("Quit"), "normal bar should list Quit");
    }
}
