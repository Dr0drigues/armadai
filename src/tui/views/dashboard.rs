use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table, Tabs},
};

use crate::theme;
use crate::tui::app::{App, Tab};
use crate::tui::filter;
use crate::tui::widgets::search_bar;

pub fn render(frame: &mut Frame, app: &mut App) {
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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" ArmadAI ")
                .style(theme::border_style()),
        )
        .select(app.tab_index)
        // Default terminal fg adapts to either theme.
        .style(Style::default())
        .highlight_style(theme::accent_style());
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
        Tab::Models => super::models_list::render(frame, app, chunks[1]),
        Tab::ModelDetail => super::model_detail::render(frame, app, chunks[1]),
        #[cfg(feature = "storage")]
        Tab::Orchestration => super::orchestration::render(frame, app, chunks[1]),
        #[cfg(feature = "storage")]
        Tab::OrchestrationDetail => super::orchestration::render_detail(frame, app, chunks[1]),
        #[cfg(not(feature = "storage"))]
        Tab::Orchestration | Tab::OrchestrationDetail => {
            let msg = Paragraph::new("Orchestration tab requires storage feature");
            frame.render_widget(
                msg.block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Orchestration ")
                        .style(theme::border_style()),
                ),
                chunks[1],
            );
        }
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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Agents ")
                .style(theme::border_style()),
        );
        frame.render_widget(msg, area);
        return;
    }

    // Apply filtering and sorting
    let display_indices =
        filter::apply_filter_and_sort_agents(&app.agents, &app.search_query, app.sort_mode);

    if display_indices.is_empty() {
        let msg = Paragraph::new("No agents match your search.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Agents ")
                .style(theme::border_style()),
        );
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec!["", "AGENT", "PROVIDER", "MODEL", "TAGS"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_i, &agent_i)| {
            let marker = if display_i == app.selected_agent {
                ">"
            } else {
                " "
            };
            let agent = &app.agents[agent_i];
            let tags = agent.metadata.tags.join(", ");
            let style = if display_i == app.selected_agent {
                theme::accent_style()
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
            .title(format!(
                " Agents — {} loaded, {} shown{} ",
                app.agents.len(),
                display_indices.len(),
                app.sort_indicator()
            ))
            .style(theme::border_style()),
    );

    frame.render_widget(table, area);

    // Render search bar if in search mode
    if app.search_mode {
        search_bar(frame, &app.search_query, area);
    }
}
