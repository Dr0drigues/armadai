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
    pinned: bool,
}

impl Workroom {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            visible: false,
            pinned: false,
        }
    }

    /// Initialize from orchestration config (coordinator + teams)
    pub fn init_from_config(&mut self, config_yaml: &str) {
        self.agents.clear();

        // Parse coordinator (take first occurrence only)
        for line in config_yaml.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("coordinator:")
                && !self.agents.iter().any(|a| a.role == AgentRole::Coordinator)
            {
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
                if !name.is_empty() && !self.agents.iter().any(|a| a.name == name) {
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

            if in_agents && trimmed.starts_with("- ") && !trimmed.contains(':') {
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
            if in_agents
                && !trimmed.is_empty()
                && !trimmed.starts_with('-')
                && !trimmed.starts_with(' ')
            {
                in_agents = false;
            }
        }

        // Auto-show workroom if agents were found in an orchestrated project
        if self.agents.len() > 1 {
            self.visible = true;
            self.pinned = true;
        }
    }

    /// Set agents from the stream-json init event.
    /// Filters out Claude Code internal agents and deduplicates.
    pub fn set_agents_from_init(&mut self, agent_names: &[String]) {
        const INTERNAL_AGENTS: &[&str] = &[
            "general-purpose",
            "statusline-setup",
            "Explore",
            "Plan",
            "claude-code-guide",
        ];

        for name in agent_names {
            if INTERNAL_AGENTS.contains(&name.as_str()) {
                continue;
            }
            // Skip if already present — case-insensitive match
            if self
                .agents
                .iter()
                .any(|a| a.name.to_lowercase() == name.to_lowercase())
            {
                continue;
            }
            self.agents.push(TrackedAgent {
                name: name.clone(),
                state: AgentState::Idle,
                role: AgentRole::Agent,
                started_at: None,
                finished_at: None,
                spinner_frame: 0,
            });
        }
    }

    /// Notify that a delegation to an agent was detected (from text analysis).
    /// Only sets the specific mentioned agent to Working.
    pub fn on_delegate(&mut self, agent_name: &str) {
        // Set coordinator to delegating
        if let Some(coord) = self
            .agents
            .iter_mut()
            .find(|a| a.role == AgentRole::Coordinator)
            && coord.state == AgentState::Idle
        {
            coord.state = AgentState::Delegating;
            coord.started_at = Some(Instant::now());
        }

        // Set ONLY the target agent to working (not all agents)
        if let Some(agent) = self.agents.iter_mut().find(|a| a.name == agent_name)
            && agent.state == AgentState::Idle
        {
            agent.state = AgentState::Working;
            agent.started_at = Some(Instant::now());
        }
        // Don't add unknown agents dynamically — too noisy

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

    /// Reset all agents to idle for next turn.
    /// Keeps visibility if pinned.
    pub fn reset(&mut self) {
        for agent in &mut self.agents {
            agent.state = AgentState::Idle;
            agent.started_at = None;
            agent.finished_at = None;
        }
        // Don't hide if pinned — user wants to see it permanently
        if !self.pinned {
            self.visible = false;
        }
    }

    /// Toggle pinned visibility (always visible even between turns).
    pub fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
        if self.pinned {
            self.visible = true;
        }
    }

    /// Whether the workroom is pinned (always visible).
    pub fn is_pinned(&self) -> bool {
        self.pinned
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

    /// Set visibility directly
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Detect agent mentions in streamed text and set them to Working.
    /// Matches: exact name, name with spaces, partial keywords.
    pub fn detect_mentions(&mut self, text: &str) {
        let text_lower = text.to_lowercase();

        // First pass: also detect coordinator delegating
        let is_delegation = text_lower.contains("déléguer")
            || text_lower.contains("delegat")
            || text_lower.contains("spécialiste")
            || text_lower.contains("specialist");

        if is_delegation
            && let Some(coord) = self
                .agents
                .iter_mut()
                .find(|a| a.role == AgentRole::Coordinator)
            && coord.state == AgentState::Idle
        {
            coord.state = AgentState::Delegating;
            coord.started_at = Some(Instant::now());
        }

        for agent in &mut self.agents {
            if agent.state != AgentState::Idle {
                continue;
            }
            let name_lower = agent.name.to_lowercase();
            // Match: "shell-scripting-expert"
            if text_lower.contains(&name_lower) {
                agent.state = AgentState::Working;
                agent.started_at = Some(Instant::now());
                continue;
            }
            // Match: "shell scripting expert"
            let name_spaces = name_lower.replace('-', " ");
            if text_lower.contains(&name_spaces) {
                agent.state = AgentState::Working;
                agent.started_at = Some(Instant::now());
                continue;
            }
            // Match: key parts — e.g., "shell scripting" from "shell-scripting-expert"
            let parts: Vec<&str> = name_lower.split('-').collect();
            if parts.len() >= 2 {
                let key = format!("{} {}", parts[0], parts[1]);
                if text_lower.contains(&key) {
                    agent.state = AgentState::Working;
                    agent.started_at = Some(Instant::now());
                }
            }
        }
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
                AgentRole::Coordinator => Color::Rgb(231, 76, 60), // red
                AgentRole::Lead => Color::Rgb(243, 156, 18),       // orange
                AgentRole::Agent => Color::Rgb(88, 166, 255),      // blue
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

    fn setup_workroom() -> Workroom {
        let config = "orchestration:\n  coordinator: coordinator\n  teams:\n    - agents:\n        - agent-a\n        - agent-b\n";
        let mut wr = Workroom::new();
        wr.init_from_config(config);
        wr
    }

    #[test]
    fn test_parse_delegate_marker() {
        let mut wr = setup_workroom();
        wr.parse_streaming_line("Some text <!--ARMADAI_DELEGATE:agent-a--> more text");
        assert!(wr.is_visible());
        let agent = wr.agents.iter().find(|a| a.name == "agent-a").unwrap();
        assert_eq!(agent.state, AgentState::Working);
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
        let mut wr = setup_workroom();
        wr.on_delegate("agent-a");
        let agent = wr.agents.iter().find(|a| a.name == "agent-a").unwrap();
        assert_eq!(agent.state, AgentState::Working);
        wr.on_complete();
        let agent = wr.agents.iter().find(|a| a.name == "agent-a").unwrap();
        assert_eq!(agent.state, AgentState::Done);
    }

    #[test]
    fn test_reset() {
        let mut wr = Workroom::new();
        // Manually add an agent (without init_from_config which auto-pins)
        wr.agents.push(TrackedAgent {
            name: "agent-a".to_string(),
            state: AgentState::Idle,
            role: AgentRole::Agent,
            started_at: None,
            finished_at: None,
            spinner_frame: 0,
        });
        wr.on_delegate("agent-a");
        wr.on_complete();
        wr.reset();
        let agent = wr.agents.iter().find(|a| a.name == "agent-a").unwrap();
        assert_eq!(agent.state, AgentState::Idle);
        assert!(!wr.is_visible()); // not pinned, so hidden
    }

    #[test]
    fn test_detect_mentions() {
        let mut wr = setup_workroom();
        wr.detect_mentions("I'll delegate to agent-a for this task");
        let agent = wr.agents.iter().find(|a| a.name == "agent-a").unwrap();
        assert_eq!(agent.state, AgentState::Working);
        // agent-b should still be idle
        let agent_b = wr.agents.iter().find(|a| a.name == "agent-b").unwrap();
        assert_eq!(agent_b.state, AgentState::Idle);
    }

    #[test]
    fn test_set_agents_from_init_filters_internals() {
        let mut wr = Workroom::new();
        let agents = vec![
            "shell-expert".to_string(),
            "general-purpose".to_string(), // internal — should be filtered
            "Explore".to_string(),         // internal
            "container-expert".to_string(),
        ];
        wr.set_agents_from_init(&agents);
        assert_eq!(wr.agents.len(), 2);
        assert!(wr.agents.iter().any(|a| a.name == "shell-expert"));
        assert!(wr.agents.iter().any(|a| a.name == "container-expert"));
    }

    #[test]
    fn test_pinned_workroom_stays_visible() {
        let mut wr = setup_workroom();
        // setup_workroom auto-pins, verify
        assert!(wr.is_pinned());
        wr.on_delegate("agent-a");
        wr.on_complete();
        wr.reset();
        // Should still be visible because pinned
        assert!(wr.is_visible());
    }
}
