use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table, Tabs},
};

use crate::tui::app::{App, Tab};

pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tab bar
            Constraint::Min(0),    // Content
            Constraint::Length(1), // Shortcuts bar
        ])
        .split(frame.area());

    // Tab bar
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|t| Line::from(Span::raw(t.title())))
        .collect();

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" ArmadAI "))
        .select(app.tab_index)
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, chunks[0]);

    // Content area — dispatch to the right view
    match app.current_tab {
        Tab::Dashboard => render_agent_list(frame, app, chunks[1]),
        Tab::AgentDetail => super::agent_detail::render(frame, app, chunks[1]),
        Tab::Prompts => super::prompts_list::render(frame, app, chunks[1]),
        Tab::PromptDetail => super::prompt_detail::render(frame, app, chunks[1]),
        Tab::Skills => super::skills_list::render(frame, app, chunks[1]),
        Tab::SkillDetail => super::skill_detail::render(frame, app, chunks[1]),
        Tab::Starters => super::starters_list::render(frame, app, chunks[1]),
        Tab::StarterDetail => super::starter_detail::render(frame, app, chunks[1]),
        Tab::History => super::history::render(frame, app, chunks[1]),
        Tab::Costs => super::costs::render(frame, app, chunks[1]),
    };

    // Shortcuts bar
    super::shortcuts::render(frame, app, chunks[2]);
}

fn render_agent_list(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if app.agents.is_empty() {
        let msg = Paragraph::new(
            "No agents found. Create one with: armadai new my-agent\n\n\
             Press ':' to open command palette",
        )
        .block(Block::default().borders(Borders::ALL).title(" Agents "));
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec!["", "AGENT", "PROVIDER", "MODEL", "TAGS"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows: Vec<Row> = app
        .agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let marker = if i == app.selected_agent { ">" } else { " " };
            let tags = agent.metadata.tags.join(", ");
            let style = if i == app.selected_agent {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                marker.to_string(),
                agent.name.clone(),
                agent.metadata.provider.clone(),
                agent.model_display(),
                tags,
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(15),
            Constraint::Length(12),
            Constraint::Length(30),
            Constraint::Min(15),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Agents — {} loaded ", app.agents.len())),
    );

    frame.render_widget(table, area);
}
