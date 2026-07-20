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

use super::SPINNER_FRAMES as SPINNER;

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
    /// Short excerpt of the agent's latest action/status (for drill-down).
    pub last_action: Option<String>,
    /// State-transition history with timestamps (for drill-down).
    pub transitions: Vec<(AgentState, std::time::Instant)>,
}

/// Role in the orchestration hierarchy
#[derive(Debug, Clone, PartialEq)]
pub enum AgentRole {
    Coordinator,
    Lead,
    Agent,
}

/// The workroom tracks all agents and their states
pub struct Workroom {
    agents: Vec<TrackedAgent>,
    visible: bool,
    pinned: bool,
    /// Buffer for streamed text, used to extract markers that may span chunks.
    marker_buf: String,
    /// Name of the agent currently holding the token (for END → Done).
    current_agent: Option<String>,
}

impl Workroom {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            visible: false,
            pinned: false,
            marker_buf: String::new(),
            current_agent: None,
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
                        last_action: None,
                        transitions: Vec::new(),
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
                        last_action: None,
                        transitions: Vec::new(),
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
                        last_action: None,
                        transitions: Vec::new(),
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
                last_action: None,
                transitions: Vec::new(),
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

    /// Push a state transition for an agent, updating timestamps + history.
    fn set_state(&mut self, name: &str, state: AgentState) {
        if let Some(agent) = self.agents.iter_mut().find(|a| a.name == name) {
            match state {
                AgentState::Working | AgentState::Delegating => {
                    if agent.started_at.is_none() {
                        agent.started_at = Some(Instant::now());
                    }
                }
                AgentState::Done => {
                    agent.finished_at = Some(Instant::now());
                }
                AgentState::Idle => {}
            }
            agent.transitions.push((state.clone(), Instant::now()));
            agent.state = state;
        }
    }

    fn coordinator_name(&self) -> Option<String> {
        self.agents
            .iter()
            .find(|a| a.role == AgentRole::Coordinator)
            .map(|a| a.name.clone())
    }

    /// Apply streamed text to the workroom FSM by extracting ArmadAI protocol
    /// markers. Buffers input so a marker split across chunks is handled.
    pub fn apply_stream_text(&mut self, chunk: &str) {
        self.marker_buf.push_str(chunk);

        // Extract complete markers `<!--ARMADAI_...-->` in order.
        loop {
            let Some(start) = self.marker_buf.find("<!--ARMADAI_") else {
                // No marker opener: keep only a possible partial opener tail.
                keep_tail(&mut self.marker_buf, "<!--ARMADAI_");
                break;
            };
            let Some(rel_end) = self.marker_buf[start..].find("-->") else {
                // Opener present but not yet closed: retain from `start`.
                self.marker_buf = self.marker_buf[start..].to_string();
                break;
            };
            let end = start + rel_end;
            let inner = self.marker_buf[start + 4..end].to_string(); // strip "<!--"
            self.apply_marker(inner.trim());
            self.marker_buf = self.marker_buf[end + 3..].to_string(); // strip "-->"
        }

        // Safety cap: an unterminated `<!--ARMADAI_` opener must not let the
        // buffer grow without bound within a turn.
        const MAX_MARKER_BUF: usize = 8192;
        if self.marker_buf.len() > MAX_MARKER_BUF {
            // Keep only the tail (where a real closing `-->` would still land).
            let cut = self.marker_buf.len() - MAX_MARKER_BUF;
            // Respect char boundaries.
            let mut cut = cut;
            while cut < self.marker_buf.len() && !self.marker_buf.is_char_boundary(cut) {
                cut += 1;
            }
            self.marker_buf = self.marker_buf[cut..].to_string();
        }
    }

    /// Apply a single marker body (e.g. `ARMADAI_DELEGATE:core-specialist`).
    fn apply_marker(&mut self, body: &str) {
        if let Some(target) = body.strip_prefix("ARMADAI_DELEGATE:") {
            let target = target.trim().to_string();
            if target.is_empty() {
                return; // malformed marker — ignore
            }
            if let Some(coord) = self.coordinator_name() {
                self.set_state(&coord, AgentState::Delegating);
            }
            self.set_state(&target, AgentState::Working);
            self.current_agent = Some(target);
            self.visible = true;
        } else if let Some(status) = body.strip_prefix("ARMADAI_META:status=") {
            if let Some(cur) = self.current_agent.clone()
                && let Some(agent) = self.agents.iter_mut().find(|a| a.name == cur)
            {
                agent.last_action = Some(status.trim().to_string());
            }
        } else if body.trim() == "ARMADAI_END" {
            // NOTE: ArmadAI markers can be echoed in Claude Code recaps
            // (e.g. `| ... recap ... <!--ARMADAI_END-->`), so a stray END is a
            // false positive. Only act when an agent is genuinely active; the
            // authoritative end-of-turn completion is `on_complete()`.
            if let Some(cur) = self.current_agent.clone()
                && let Some(agent) = self.agents.iter().find(|a| a.name == cur)
                && matches!(agent.state, AgentState::Working | AgentState::Delegating)
            {
                self.set_state(&cur, AgentState::Done);
                // Clear the current agent so a later stray recap echo of
                // `<!--ARMADAI_END-->` finds no active agent and no-ops (closes
                // the recap-guard hole). The coordinator stays `Delegating`
                // and is finalized to `Done` by `on_complete()` at true
                // end-of-turn — so the coordinator ends the turn Done, not Idle.
                self.current_agent = None;
            }
        }
    }

    /// Parse a streaming line for ArmadAI protocol markers.
    pub fn parse_streaming_line(&mut self, line: &str) {
        self.apply_stream_text(line);
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
                AgentState::Done => ("✓", "done".to_string(), Style::default().fg(Color::Green)),
                AgentState::Idle => (
                    "○",
                    "idle".to_string(),
                    Style::default().fg(Color::DarkGray),
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

/// Retain only a trailing partial-prefix of `needle` at the end of `buf`
/// (so a marker split across chunks can still be completed next call).
fn keep_tail(buf: &mut String, needle: &str) {
    let max = needle.len().min(buf.len());
    for k in (1..=max).rev() {
        if buf.is_char_boundary(buf.len() - k) && needle.starts_with(&buf[buf.len() - k..]) {
            *buf = buf[buf.len() - k..].to_string();
            return;
        }
    }
    buf.clear();
}

#[cfg(test)]
impl Workroom {
    pub fn agents_for_test(&self) -> &[TrackedAgent] {
        &self.agents
    }

    pub fn marker_buf_len_for_test(&self) -> usize {
        self.marker_buf.len()
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
            last_action: None,
            transitions: Vec::new(),
        });
        wr.on_delegate("agent-a");
        wr.on_complete();
        wr.reset();
        let agent = wr.agents.iter().find(|a| a.name == "agent-a").unwrap();
        assert_eq!(agent.state, AgentState::Idle);
        assert!(!wr.is_visible()); // not pinned, so hidden
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

    #[test]
    fn test_tracked_agent_has_enriched_fields() {
        let mut wr = Workroom::new();
        wr.init_from_config("coordinator: dev-lead\nteams:\n  - agents: [core-specialist]\n");
        // Every tracked agent starts with no last action and an empty transition log.
        for a in wr.agents_for_test() {
            assert!(a.last_action.is_none());
            assert!(a.transitions.is_empty());
        }
    }

    fn wr_dev_lead_core() -> Workroom {
        let mut wr = Workroom::new();
        // Block-style YAML (the line-based parser does NOT expand inline `[...]`).
        wr.init_from_config(
            "coordinator: dev-lead\nteams:\n  - agents:\n      - core-specialist\n",
        );
        wr
    }

    #[test]
    fn test_apply_stream_text_delegate_sets_working_and_records_transition() {
        let mut wr = wr_dev_lead_core();
        wr.apply_stream_text("<!--ARMADAI_DELEGATE:core-specialist-->");
        let a = wr
            .agents_for_test()
            .iter()
            .find(|a| a.name == "core-specialist")
            .unwrap();
        assert_eq!(a.state, AgentState::Working);
        assert!(!a.transitions.is_empty());
        let coord = wr
            .agents_for_test()
            .iter()
            .find(|a| a.name == "dev-lead")
            .unwrap();
        assert_eq!(coord.state, AgentState::Delegating);
    }

    #[test]
    fn test_apply_stream_text_end_marks_current_done() {
        let mut wr = wr_dev_lead_core();
        wr.apply_stream_text("<!--ARMADAI_DELEGATE:core-specialist-->");
        wr.apply_stream_text("<!--ARMADAI_META:status=complete-->");
        wr.apply_stream_text("<!--ARMADAI_END-->");
        let a = wr
            .agents_for_test()
            .iter()
            .find(|a| a.name == "core-specialist")
            .unwrap();
        assert_eq!(a.state, AgentState::Done);
        assert_eq!(a.last_action.as_deref(), Some("complete"));
        assert!(a.finished_at.is_some());
    }

    #[test]
    fn test_apply_stream_text_handles_marker_split_across_chunks() {
        let mut wr = wr_dev_lead_core();
        wr.apply_stream_text("<!--ARMADAI_DELE");
        wr.apply_stream_text("GATE:core-specialist-->");
        let a = wr
            .agents_for_test()
            .iter()
            .find(|a| a.name == "core-specialist")
            .unwrap();
        assert_eq!(a.state, AgentState::Working);
    }

    // Regression: ArmadAI markers can be ECHOED in Claude Code recaps
    // (e.g. `| Ceci est un recap ... <!--ARMADAI_END-->`). A stray END with no
    // active agent must be a safe no-op — never mark an idle agent Done, never panic.
    #[test]
    fn test_apply_stream_text_stray_end_in_recap_is_noop() {
        let mut wr = wr_dev_lead_core();
        wr.apply_stream_text("| Ceci est un recap de Claude Code <!--ARMADAI_END-->");
        for a in wr.agents_for_test() {
            assert_ne!(
                a.state,
                AgentState::Done,
                "stray recap END must not mark '{}' done",
                a.name
            );
        }
    }

    // The recap-guard must protect an ALREADY-DONE agent too: after an agent
    // finishes, a stray END (echoed in a recap) must not re-transition anything.
    #[test]
    fn test_stray_end_after_agent_done_is_noop() {
        let mut wr = wr_dev_lead_core();
        wr.apply_stream_text("<!--ARMADAI_DELEGATE:core-specialist-->");
        wr.apply_stream_text("<!--ARMADAI_END-->"); // core-specialist -> Done, control back to coordinator
        let before: Vec<_> = wr
            .agents_for_test()
            .iter()
            .map(|a| (a.name.clone(), a.state.clone()))
            .collect();
        wr.apply_stream_text("| recap echo <!--ARMADAI_END-->"); // stray, coordinator is Idle here
        let after: Vec<_> = wr
            .agents_for_test()
            .iter()
            .map(|a| (a.name.clone(), a.state.clone()))
            .collect();
        assert_eq!(
            before, after,
            "a stray recap END must not change any agent state"
        );
    }

    #[test]
    fn test_empty_delegate_target_is_ignored() {
        let mut wr = wr_dev_lead_core();
        wr.apply_stream_text("<!--ARMADAI_DELEGATE:-->");
        // No agent named "" — nothing works; coordinator not forced to delegate to nobody.
        assert!(
            wr.agents_for_test()
                .iter()
                .all(|a| a.state == AgentState::Idle)
        );
    }

    #[test]
    fn test_unterminated_opener_buffer_is_capped() {
        let mut wr = wr_dev_lead_core();
        // A stray opener that never closes, followed by a lot of prose.
        wr.apply_stream_text("<!--ARMADAI_");
        for _ in 0..1000 {
            wr.apply_stream_text("some long prose without any closing marker ");
        }
        // Buffer must not grow unbounded.
        assert!(
            wr.marker_buf_len_for_test() <= 8192,
            "unterminated marker buffer must be capped"
        );
    }
}
