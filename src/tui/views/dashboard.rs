use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};

use crate::tui::app::{App, Tab};

pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(frame.area());

    // Tab bar
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|t| Line::from(Span::raw(t.title())))
        .collect();

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" swarm-festai "))
        .select(app.tab_index)
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, chunks[0]);

    // Content area
    let content = match app.current_tab {
        Tab::Dashboard => Paragraph::new("Agent fleet dashboard — press 'q' to quit, Tab to switch")
            .block(Block::default().borders(Borders::ALL).title(" Dashboard ")),
        Tab::Execution => Paragraph::new("Execution view — streaming output will appear here")
            .block(Block::default().borders(Borders::ALL).title(" Execution ")),
        Tab::History => Paragraph::new("Execution history")
            .block(Block::default().borders(Borders::ALL).title(" History ")),
        Tab::Costs => Paragraph::new("Cost tracking per agent")
            .block(Block::default().borders(Borders::ALL).title(" Costs ")),
    };
    frame.render_widget(content, chunks[1]);
}
