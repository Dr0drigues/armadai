//! Agent Workroom — visual feedback of active agents during orchestration.
//!
//! Displays a side panel showing which agents are working, waiting, or done.
//! Parses DELEGATE markers from the streaming response to track agent activity.

#![cfg(feature = "tui")]

use ratatui::{
    prelude::*,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::time::Instant;

/// Agent activity state
#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    /// Agent is actively generating a response
    Working,
    /// Agent is waiting for sub-agents
    Delegating,
    /// Agent has completed its work
    Done,
    /// Agent is idle (not yet involved)
    Idle,
}

/// A tracked agent in the workroom
#[derive(Debug, Clone)]
pub struct TrackedAgent {
    pub name: String,
    pub state: AgentState,
    pub role: AgentRole,
    pub started_at: Option<Instant>,
    pub finished_at: Option<Instant>,
    /// Spinner frame for animation
    pub spinner_frame: usize,
}

/// Role in the orchestration hierarchy
#[derive(Debug, Clone, PartialEq)]
pub enum AgentRole {
    Coordinator,
    Lead,
    Agent,
}

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The workroom tracks all agents and their states
pub struct Workroom {
    agents: Vec<TrackedAgent>,
    visible: bool,
}

impl Workroom {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            visible: false,
        }
    }

    /// Initialize from orchestration config (coordinator + teams)
    pub fn init_from_config(&mut self, config_yaml: &str) {
        self.agents.clear();

        // Parse coordinator
        for line in config_yaml.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("coordinator:") {
                let name = trimmed
                    .strip_prefix("coordinator:")
                    .unwrap()
                    .trim()
                    .trim_matches('"');
                if !name.is_empty() {
                    self.agents.push(TrackedAgent {
                        name: name.to_string(),
                        state: AgentState::Idle,
                        role: AgentRole::Coordinator,
                        started_at: None,
                        finished_at: None,
                        spinner_frame: 0,
                    });
                }
            }
        }

        // Parse agents from teams (simplified — looks for "- agent-name" patterns)
        let mut in_agents = false;
        let mut _current_is_lead = false;
        for line in config_yaml.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("- lead:") {
                let name = trimmed
                    .strip_prefix("- lead:")
                    .unwrap()
                    .trim()
                    .trim_matches('"');
                if !name.is_empty()
                    && !self.agents.iter().any(|a| a.name == name)
                {
                    self.agents.push(TrackedAgent {
                        name: name.to_string(),
                        state: AgentState::Idle,
                        role: AgentRole::Lead,
                        started_at: None,
                        finished_at: None,
                        spinner_frame: 0,
                    });
                }
                _current_is_lead = true;
                in_agents = false;
                continue;
            }

            if trimmed == "agents:" || trimmed.starts_with("- agents:") {
                in_agents = true;
                _current_is_lead = false;
                continue;
            }

            if in_agents
                && trimmed.starts_with("- ")
                && !trimmed.contains(':')
            {
                let name = trimmed.strip_prefix("- ").unwrap().trim().trim_matches('"');
                if !name.is_empty() && !self.agents.iter().any(|a| a.name == name) {
                    self.agents.push(TrackedAgent {
                        name: name.to_string(),
                        state: AgentState::Idle,
                        role: AgentRole::Agent,
                        started_at: None,
                        finished_at: None,
                        spinner_frame: 0,
                    });
                }
            }

            // Exit agents list on non-indented, non-dash line
            if in_agents && !trimmed.is_empty() && !trimmed.starts_with('-') && !trimmed.starts_with(' ') {
                in_agents = false;
            }
        }
    }

    /// Notify that a delegation to an agent was detected
    pub fn on_delegate(&mut self, agent_name: &str) {
        // Set coordinator to delegating
        if let Some(coord) = self.agents.iter_mut().find(|a| a.role == AgentRole::Coordinator)
            && coord.state == AgentState::Idle
        {
            coord.state = AgentState::Delegating;
            coord.started_at = Some(Instant::now());
        }

        // Set target agent to working
        if let Some(agent) = self.agents.iter_mut().find(|a| a.name == agent_name) {
            agent.state = AgentState::Working;
            agent.started_at = Some(Instant::now());
        } else {
            // Unknown agent — add dynamically
            self.agents.push(TrackedAgent {
                name: agent_name.to_string(),
                state: AgentState::Working,
                role: AgentRole::Agent,
                started_at: Some(Instant::now()),
                finished_at: None,
                spinner_frame: 0,
            });
        }

        self.visible = true;
    }

    /// Notify that response streaming is complete
    pub fn on_complete(&mut self) {
        for agent in &mut self.agents {
            if agent.state == AgentState::Working || agent.state == AgentState::Delegating {
                agent.state = AgentState::Done;
                agent.finished_at = Some(Instant::now());
            }
        }
    }

    /// Reset all agents to idle for next turn
    pub fn reset(&mut self) {
        for agent in &mut self.agents {
            agent.state = AgentState::Idle;
            agent.started_at = None;
            agent.finished_at = None;
        }
        self.visible = false;
    }

    /// Advance spinner animations
    pub fn tick(&mut self) {
        for agent in &mut self.agents {
            if agent.state == AgentState::Working || agent.state == AgentState::Delegating {
                agent.spinner_frame = (agent.spinner_frame + 1) % SPINNER.len();
            }
        }
    }

    /// Whether the workroom panel should be shown
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Parse a streaming line for delegate markers
    pub fn parse_streaming_line(&mut self, line: &str) {
        if let Some(start) = line.find("<!--ARMADAI_DELEGATE:")
            && let Some(end) = line[start..].find("-->")
        {
            let marker = &line[start + 21..start + end];
            self.on_delegate(marker.trim());
        }
    }

    /// Render the workroom panel
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        for agent in &self.agents {
            let (icon, state_str, style) = match agent.state {
                AgentState::Working => {
                    let spinner = SPINNER[agent.spinner_frame];
                    let elapsed = agent
                        .started_at
                        .map(|s| format!(" {:.0}s", s.elapsed().as_secs_f64()))
                        .unwrap_or_default();
                    (
                        spinner,
                        format!("working{elapsed}"),
                        Style::default().fg(Color::Green),
                    )
                }
                AgentState::Delegating => {
                    let spinner = SPINNER[agent.spinner_frame];
                    (
                        spinner,
                        "delegating".to_string(),
                        Style::default().fg(Color::Yellow),
                    )
                }
                AgentState::Done => (
                    "✓",
                    "done".to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
                AgentState::Idle => (
                    "○",
                    "idle".to_string(),
                    Style::default().fg(Color::Rgb(60, 60, 60)),
                ),
            };

            let role_color = match agent.role {
                AgentRole::Coordinator => Color::Rgb(231, 76, 60),   // red
                AgentRole::Lead => Color::Rgb(243, 156, 18),         // orange
                AgentRole::Agent => Color::Rgb(88, 166, 255),        // blue
            };

            let indent = match agent.role {
                AgentRole::Coordinator => "",
                AgentRole::Lead => "  ",
                AgentRole::Agent => "    ",
            };

            lines.push(Line::from(vec![
                Span::raw(indent),
                Span::styled(format!("{icon} "), style),
                Span::styled(&agent.name, Style::default().fg(role_color).bold()),
                Span::styled(format!("  {state_str}"), style),
            ]));
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let panel = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(48, 54, 61)))
                .title(" Workroom ")
                .title_style(Style::default().fg(Color::Cyan).bold()),
        );

        frame.render_widget(panel, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_delegate_marker() {
        let mut wr = Workroom::new();
        wr.parse_streaming_line("Some text <!--ARMADAI_DELEGATE:shell-expert--> more text");
        assert!(wr.is_visible());
        assert_eq!(wr.agents.len(), 1);
        assert_eq!(wr.agents[0].name, "shell-expert");
        assert_eq!(wr.agents[0].state, AgentState::Working);
    }

    #[test]
    fn test_init_from_config() {
        let config = r#"
orchestration:
  coordinator: devbox-coordinator
  teams:
    - agents:
        - shell-expert
        - container-expert
    - lead: test-lead
      agents:
        - vm-linux
"#;
        let mut wr = Workroom::new();
        wr.init_from_config(config);
        assert_eq!(wr.agents.len(), 5);
        assert_eq!(wr.agents[0].name, "devbox-coordinator");
        assert_eq!(wr.agents[0].role, AgentRole::Coordinator);
    }

    #[test]
    fn test_on_complete_resets_working() {
        let mut wr = Workroom::new();
        wr.on_delegate("agent-a");
        assert_eq!(wr.agents[0].state, AgentState::Working);
        wr.on_complete();
        assert_eq!(wr.agents[0].state, AgentState::Done);
    }

    #[test]
    fn test_reset() {
        let mut wr = Workroom::new();
        wr.on_delegate("agent-a");
        wr.on_complete();
        wr.reset();
        assert_eq!(wr.agents[0].state, AgentState::Idle);
        assert!(!wr.is_visible());
    }
}
