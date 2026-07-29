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
use crate::theme;
use armadai_core::events::RunEvent;
use armadai_core::orchestration::OrchestrationPattern;

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

/// Which layout the workroom renders. Compact is the idle/narrow fallback;
/// the three rich modes only appear when focused and wide enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Compact,
    Hierarchical,
    Blackboard,
    Ring,
}

/// Minimum inner width (columns, borders excluded) to render a rich layout.
const RICH_WIDTH_MIN: u16 = 44;

/// The workroom tracks all agents and their states
pub struct Workroom {
    agents: Vec<TrackedAgent>,
    visible: bool,
    pinned: bool,
    /// Buffer for streamed text, used to extract markers that may span chunks.
    marker_buf: String,
    /// Name of the agent currently holding the token (for END → Done).
    current_agent: Option<String>,
    /// Index of the selected agent row (drill-down focus mode).
    selected: usize,
    /// Whether the workroom currently has keyboard focus (drill-down mode).
    focused: bool,
    /// Active orchestration pattern (drives the focused layout).
    pattern: OrchestrationPattern,
    /// Whether the orchestration has finished and the live TUI is holding the
    /// final frame for the user to dismiss (Dimitri's visual-validation
    /// request: fast providers used to make the workroom flash and vanish).
    completed: bool,
    /// The run's id, captured from `RunEvent::RunStart` (OH1 Lot 6). Rendered
    /// on the completion screen so the user can copy it for a later
    /// `--resume`/`--replay` — the live TUI is the only path where it was
    /// previously surfaced nowhere at all (the alternate screen hides
    /// anything printed to stdout while it's active).
    run_id: Option<String>,
    /// Set from `RunEvent::Error` — the run ended in failure (error/timeout).
    /// Drives the completion footer to show a failure state instead of a
    /// misleading "✓ run complete" (#271).
    run_error: Option<String>,
}

impl Workroom {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            visible: false,
            pinned: false,
            marker_buf: String::new(),
            current_agent: None,
            selected: 0,
            focused: false,
            pattern: OrchestrationPattern::Hierarchical,
            completed: false,
            run_id: None,
            run_error: None,
        }
    }

    /// The captured run id (if a `RunStart` has been observed yet), for a
    /// caller to print after the TUI exits (see `run_view::run_orchestration_tui`).
    pub fn run_id(&self) -> Option<&str> {
        self.run_id.as_deref()
    }

    /// Set whether the workroom has keyboard focus (drill-down mode).
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Override the active orchestration pattern (drives the focused layout).
    /// Used to apply an explicit `--orchestrate <pattern>` flag, which takes
    /// precedence over whatever `init_from_config` inferred from the project
    /// config's `pattern:` key (or its `Hierarchical` default).
    pub fn set_pattern(&mut self, pattern: OrchestrationPattern) {
        self.pattern = pattern;
    }

    /// Whether the workroom currently has keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Number of tracked agents — used by the fullscreen run view to size the
    /// centered panel (rich layouts need roughly two lines per agent, plus
    /// arrows/footer).
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Mark the orchestration as finished so the renderer holds the final
    /// frame and shows the "press q/Esc to exit" hint instead of vanishing.
    pub fn set_completed(&mut self, completed: bool) {
        self.completed = completed;
    }

    /// Decide the layout from pattern, focus, and available inner width.
    pub(crate) fn layout_mode(&self, inner_width: u16) -> LayoutMode {
        if !self.focused || inner_width < RICH_WIDTH_MIN {
            return LayoutMode::Compact;
        }
        // `OrchestrationPattern` has more variants than the three the workroom
        // renders (Direct/Auto also exist); everything that isn't Blackboard
        // or Ring falls back to the hierarchical tree.
        match self.pattern {
            OrchestrationPattern::Blackboard => LayoutMode::Blackboard,
            OrchestrationPattern::Ring => LayoutMode::Ring,
            _ => LayoutMode::Hierarchical,
        }
    }

    /// Move the selection to the next agent (wraps around). No-op if empty.
    pub fn select_next(&mut self) {
        if self.agents.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.agents.len();
    }

    /// Move the selection to the previous agent (wraps around). No-op if empty.
    pub fn select_prev(&mut self) {
        if self.agents.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.agents.len() - 1
        } else {
            self.selected - 1
        };
    }

    /// Build a markdown detail view for the currently selected agent
    /// (name, role, state, elapsed time, last action, and a transition timeline).
    pub fn selected_detail_markdown(&self) -> Option<String> {
        let agent = self.agents.get(self.selected)?;

        let role = match agent.role {
            AgentRole::Coordinator => "coordinator",
            AgentRole::Lead => "lead",
            AgentRole::Agent => "agent",
        };
        let state = match agent.state {
            AgentState::Working => "working",
            AgentState::Delegating => "delegating",
            AgentState::Done => "done",
            AgentState::Idle => "idle",
        };

        let elapsed = match (agent.started_at, agent.finished_at) {
            (Some(start), Some(finish)) => {
                format!(
                    "{:.1}s",
                    finish.saturating_duration_since(start).as_secs_f64()
                )
            }
            (Some(start), None) => format!("{:.1}s", start.elapsed().as_secs_f64()),
            (None, _) => "—".to_string(),
        };

        let last_action = agent.last_action.as_deref().unwrap_or("—");

        let mut md = format!(
            "# {name}  ({role})\n**State:** {state} · **Elapsed:** {elapsed}\n**Last action:** {last_action}\n\n## Timeline\n",
            name = agent.name,
        );

        if agent.transitions.is_empty() {
            md.push_str("- (no transitions recorded yet)\n");
        } else {
            let first = agent.transitions[0].1;
            for (state, at) in &agent.transitions {
                let offset = at.saturating_duration_since(first).as_secs_f64();
                let label = match state {
                    AgentState::Working => "working",
                    AgentState::Delegating => "delegating",
                    AgentState::Done => "done",
                    AgentState::Idle => "idle",
                };
                md.push_str(&format!("- {label}      +{offset:.1}s\n"));
            }
        }

        Some(md)
    }

    /// Initialize from orchestration config (coordinator + teams)
    pub fn init_from_config(&mut self, config_yaml: &str) {
        self.agents.clear();
        self.pattern = parse_pattern(config_yaml);

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
        // Claude Code built-in agents AND the underlying CLI/provider names
        // (which leak into the init event's agent list) — none of these are
        // real fleet members and must not appear in the Workroom.
        const INTERNAL_AGENTS: &[&str] = &[
            "general-purpose",
            "statusline-setup",
            "Explore",
            "Plan",
            "claude-code-guide",
            // Provider CLI names that surface as a pseudo-agent in the stream.
            "claude",
            "gemini",
            "copilot",
            "cursor",
            "aider",
            "codex",
            "windsurf",
            "cline",
            "opencode",
        ];

        for name in agent_names {
            // Case-insensitive so "Claude"/"claude" are both filtered.
            let name_lc = name.to_lowercase();
            if INTERNAL_AGENTS.iter().any(|i| i.to_lowercase() == name_lc) {
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

    /// Apply one core `RunEvent` to the workroom state (provider-agnostic
    /// projection). `now` is injected so timing is deterministic in tests;
    /// the live renderer passes `Instant::now()`.
    pub fn on_run_event_at(&mut self, ev: &RunEvent, now: Instant) {
        match ev {
            RunEvent::RunStart { agents, run_id, .. } => {
                self.run_id = Some(run_id.clone());
                for name in agents {
                    if !self.agents.iter().any(|a| a.name == *name) {
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
                self.visible = true;
            }
            RunEvent::AgentStart { agent, .. } => {
                self.ensure_agent(agent);
                self.mark_working(agent, now);
                // The ES ring/blackboard engines emit agent_start/agent_end/vote
                // but never `Delegate`, so `current_agent` (which drives the
                // ring layout's token-holder highlight via `token_holder_index`)
                // would otherwise never update in the live path. The agent that
                // just started is the one holding the token.
                self.current_agent = Some(agent.clone());
            }
            RunEvent::AgentEnd { agent, content, .. } => {
                self.ensure_agent(agent);
                self.mark_done(agent, now);
                let first = content.lines().next().unwrap_or("").trim();
                if !first.is_empty() {
                    self.set_action(agent, first.to_string());
                }
            }
            RunEvent::Delegate { from, to } => {
                self.ensure_agent(to);
                self.transition(from, AgentState::Delegating, now);
                self.current_agent = Some(to.clone());
            }
            RunEvent::NestedStart { team_lead, .. } => {
                self.transition(team_lead, AgentState::Delegating, now)
            }
            RunEvent::NestedEnd { team_lead } => self.transition(team_lead, AgentState::Done, now),
            RunEvent::AgentSelect { selected, .. } => {
                for a in selected {
                    self.mark_working(a, now);
                }
            }
            RunEvent::Vote { agent, conf } => self.set_action(agent, format!("vote {conf:.2}")),
            RunEvent::Board { agent, kind } => self.set_action(agent, format!("board {kind}")),
            RunEvent::Route { agent, tier, .. } => self.set_action(agent, format!("→ {tier}")),
            RunEvent::Result { .. } => self.on_complete(),
            RunEvent::Error { msg, .. } => {
                // Record the run-level failure so the completion footer shows
                // "✗ run failed" instead of a misleading "✓ run complete"
                // (#271). Keep the first error if several arrive.
                if self.run_error.is_none() {
                    self.run_error = Some(msg.clone());
                }
                if let Some(cur) = self.current_agent.clone() {
                    self.set_action(&cur, format!("error: {msg}"));
                }
            }
            RunEvent::Warning { .. } => {}
        }
    }

    /// Ensure an agent node exists (role Agent for dynamically-appearing agents,
    /// e.g. subagents delegated during an `armadai watch` run where no config
    /// pre-seeded the roster). No-op if the agent already exists (its role is
    /// preserved — the config path is unaffected).
    fn ensure_agent(&mut self, name: &str) {
        if !self.agents.iter().any(|a| a.name == name) {
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

    fn mark_working(&mut self, name: &str, now: Instant) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == name) {
            a.state = AgentState::Working;
            a.started_at.get_or_insert(now);
            a.transitions.push((AgentState::Working, now));
        }
    }

    fn mark_done(&mut self, name: &str, now: Instant) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == name) {
            a.state = AgentState::Done;
            a.finished_at = Some(now);
            a.transitions.push((AgentState::Done, now));
        }
    }

    fn transition(&mut self, name: &str, state: AgentState, now: Instant) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == name) {
            if state == AgentState::Delegating && a.started_at.is_none() {
                a.started_at = Some(now);
            }
            a.transitions.push((state.clone(), now));
            a.state = state;
        }
    }

    fn set_action(&mut self, name: &str, action: String) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == name) {
            a.last_action = Some(action);
        }
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

    /// Index of the agent currently holding the ring token, if any.
    fn token_holder_index(&self) -> Option<usize> {
        let current = self.current_agent.as_deref()?;
        self.agents.iter().position(|a| a.name == current)
    }

    /// The focused ring layout: sequential agents with flow arrows; the
    /// token holder is highlighted (bold brass) with a "holds token" suffix.
    fn ring_lines(&self) -> Vec<Line<'_>> {
        let mut lines: Vec<Line> = Vec::new();
        let g = theme::glyphs();
        let holder = self.token_holder_index();
        let last = self.agents.len().saturating_sub(1);
        for (idx, agent) in self.agents.iter().enumerate() {
            let (icon, state_str, style) = self.state_display(agent);
            let is_holder = holder == Some(idx);
            let is_selected = self.focused && idx == self.selected;
            let name_style = if is_holder && is_selected {
                // Both the token holder and the keyboard selection: keep the
                // brass accent but still show the selection (reversed), so the
                // token holder stays focusable/visible when navigated to.
                theme::selection().add_modifier(Modifier::REVERSED)
            } else if is_holder {
                theme::selection()
            } else if is_selected {
                self.role_style(agent).add_modifier(Modifier::REVERSED)
            } else {
                self.role_style(agent)
            };
            let marker = if is_holder {
                format!("{} ", g.pointer)
            } else {
                "  ".to_string()
            };
            let mut spans = vec![
                Span::raw(marker),
                Span::styled(&agent.name, name_style),
                Span::styled(format!(" {icon} "), style),
                Span::styled(state_str, style),
            ];
            if is_holder {
                spans.push(Span::styled(
                    format!("   {} holds token", g.arrow_back),
                    theme::selection(),
                ));
            }
            lines.push(Line::from(spans));
            if idx != last {
                lines.push(Line::from(Span::styled(
                    format!("  {}", g.arrow_down),
                    theme::muted(),
                )));
            }
        }
        if self.agents.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                theme::muted(),
            )));
        } else {
            // Loop-back arrow closing the ring.
            lines.push(Line::from(Span::styled(
                format!("  {} (loops to top)", g.arrow_up),
                theme::muted(),
            )));
        }
        self.push_footer(&mut lines);
        lines
    }

    /// Render the workroom panel
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let inner_width = area.width.saturating_sub(2); // exclude borders
        let lines = match self.layout_mode(inner_width) {
            LayoutMode::Compact => self.compact_lines(),
            LayoutMode::Hierarchical => self.hierarchical_lines(),
            LayoutMode::Blackboard => self.blackboard_lines(),
            LayoutMode::Ring => self.ring_lines(),
        };

        let panel = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border_style())
                    .title(format!(" Workroom · {} ", self.pattern))
                    .title_style(theme::heading()),
            )
            // Base style for the whole panel area so uncoloured content
            // (e.g. plain `role_agent` names) inherits a neutral foreground
            // instead of the terminal default fg (which reads blue on light
            // terminals). Coloured roles/states override this.
            .style(theme::border_style());
        frame.render_widget(panel, area);
    }

    /// The idle/narrow layout: role-indented flat list (historical rendering).
    fn compact_lines(&self) -> Vec<Line<'_>> {
        let mut lines: Vec<Line> = Vec::new();
        let g = theme::glyphs();
        for (idx, agent) in self.agents.iter().enumerate() {
            let (icon, state_str, style) = self.state_display(agent);
            let role_style = self.role_style(agent);
            let indent = match agent.role {
                AgentRole::Coordinator => "",
                AgentRole::Lead => "  ",
                AgentRole::Agent => "    ",
            };
            let is_selected = self.focused && idx == self.selected;
            let name_style = if is_selected {
                role_style.add_modifier(Modifier::REVERSED)
            } else {
                role_style
            };
            let marker = if is_selected {
                format!("{} ", g.pointer)
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::raw(indent),
                Span::raw(marker),
                Span::styled(format!("{icon} "), style),
                Span::styled(&agent.name, name_style),
                Span::styled(format!("  {state_str}"), style),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                theme::muted(),
            )));
        }
        self.push_footer(&mut lines);
        lines
    }

    /// Shared state icon/label/style for an agent (used by every layout).
    fn state_display(&self, agent: &TrackedAgent) -> (String, String, Style) {
        match agent.state {
            AgentState::Working => {
                let spinner = SPINNER[agent.spinner_frame];
                let elapsed = agent
                    .started_at
                    .map(|s| format!(" {:.0}s", s.elapsed().as_secs_f64()))
                    .unwrap_or_default();
                (
                    spinner.to_string(),
                    format!("working{elapsed}"),
                    theme::working(),
                )
            }
            AgentState::Delegating => {
                let spinner = SPINNER[agent.spinner_frame];
                (
                    spinner.to_string(),
                    "delegating".to_string(),
                    theme::delegating(),
                )
            }
            AgentState::Done => ("✓".to_string(), "done".to_string(), theme::done()),
            AgentState::Idle => ("○".to_string(), "idle".to_string(), theme::muted()),
        }
    }

    /// Role-based name style.
    fn role_style(&self, agent: &TrackedAgent) -> Style {
        match agent.role {
            AgentRole::Coordinator => theme::role_coordinator(),
            AgentRole::Lead => theme::role_lead(),
            AgentRole::Agent => theme::role_agent(),
        }
    }

    /// Append the blank line + shortcuts hint footer shared by all layouts.
    ///
    /// While a run is in progress, `q`/Ctrl+C abort it regardless of focus
    /// (see `shell::run_view::run_loop`) — Ctrl+W only ever *toggles focus*,
    /// it never exits, so the RUNNING-state hint must say so (#274: the
    /// previous "Ctrl+W exit" label was false and the real abort keys went
    /// unadvertised once focused).
    fn push_footer(&self, lines: &mut Vec<Line>) {
        lines.push(Line::from(""));
        if self.focused {
            lines.push(Line::from(Span::styled(
                "q / Ctrl+C abort · Ctrl+W focus · j/k select",
                theme::muted(),
            )));
            lines.push(Line::from(Span::styled("Enter detail", theme::muted())));
        } else {
            lines.push(Line::from(Span::styled(
                "q / Ctrl+C abort · Ctrl+W focus",
                theme::muted(),
            )));
        }
        if self.completed {
            if let Some(err) = &self.run_error {
                let cross = theme::glyphs().cross;
                lines.push(Line::from(Span::styled(
                    format!("{cross} run failed: {err} · press q or Esc to exit"),
                    theme::error(),
                )));
            } else {
                let check = theme::glyphs().check;
                lines.push(Line::from(Span::styled(
                    format!("{check} run complete · press q or Esc to exit"),
                    theme::muted(),
                )));
            }
            // Last chance to copy the run_id before the alternate screen
            // clears on exit (OH1 Lot 6): shown in full, on its own line, so
            // it survives even at narrow widths without truncation logic.
            if let Some(id) = &self.run_id {
                lines.push(Line::from(Span::styled(
                    format!("run {id}"),
                    theme::muted(),
                )));
            }
        }
    }

    /// Rank for tree nesting: Coordinator=0, Lead=1, Agent=2.
    fn role_rank(role: &AgentRole) -> u8 {
        match role {
            AgentRole::Coordinator => 0,
            AgentRole::Lead => 1,
            AgentRole::Agent => 2,
        }
    }

    /// Box-drawing connector prefix for the agent at `i` in the tree layout.
    /// Coordinators have no connector; a node is "last" when the next node
    /// climbs back to a shallower level (or the list ends).
    fn tree_prefix(&self, i: usize) -> String {
        let agent = &self.agents[i];
        if agent.role == AgentRole::Coordinator {
            return String::new();
        }
        let g = theme::glyphs();
        let rank = Self::role_rank(&agent.role);
        let is_last =
            i + 1 >= self.agents.len() || Self::role_rank(&self.agents[i + 1].role) < rank;
        let connector = if is_last { g.tree_last } else { g.tree_branch };
        let indent = if agent.role == AgentRole::Agent {
            "  "
        } else {
            ""
        };
        format!("{indent}{connector} ")
    }

    /// The focused hierarchical (pyramid) layout with box-drawing connectors.
    fn hierarchical_lines(&self) -> Vec<Line<'_>> {
        let mut lines: Vec<Line> = Vec::new();
        for (idx, agent) in self.agents.iter().enumerate() {
            let (icon, state_str, style) = self.state_display(agent);
            let role_style = self.role_style(agent);
            let is_selected = self.focused && idx == self.selected;
            let name_style = if is_selected {
                role_style.add_modifier(Modifier::REVERSED)
            } else {
                role_style
            };
            lines.push(Line::from(vec![
                Span::styled(self.tree_prefix(idx), theme::muted()),
                Span::styled(format!("{icon} "), style),
                Span::styled(&agent.name, name_style),
                Span::styled(format!("  {state_str}"), style),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                theme::muted(),
            )));
        }
        self.push_footer(&mut lines);
        lines
    }

    /// The focused blackboard layout: a shared-board header, then a flat
    /// list of agents (no hierarchy — all react to shared state).
    fn blackboard_lines(&self) -> Vec<Line<'_>> {
        let mut lines: Vec<Line> = Vec::new();
        let g = theme::glyphs();
        lines.push(Line::from(Span::styled(
            format!("{} shared board · {} agents", g.board, self.agents.len()),
            theme::heading(),
        )));
        for (idx, agent) in self.agents.iter().enumerate() {
            let (icon, state_str, style) = self.state_display(agent);
            let role_style = self.role_style(agent);
            let is_selected = self.focused && idx == self.selected;
            let name_style = if is_selected {
                role_style.add_modifier(Modifier::REVERSED)
            } else {
                role_style
            };
            let suffix = if agent.state == AgentState::Idle {
                "  idle (waiting on board)".to_string()
            } else {
                format!("  {state_str}")
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{icon} "), style),
                Span::styled(&agent.name, name_style),
                Span::styled(suffix, style),
            ]));
        }
        if self.agents.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                theme::muted(),
            )));
        }
        self.push_footer(&mut lines);
        lines
    }
}

/// Detect the orchestration pattern from a project config YAML string.
/// Tolerant line scan (matches the heuristic style of `init_from_config`):
/// reads the first `pattern:` value and maps it, defaulting to Hierarchical.
fn parse_pattern(config_yaml: &str) -> OrchestrationPattern {
    for line in config_yaml.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("pattern:") {
            let value = rest.trim().trim_matches('"').to_ascii_lowercase();
            return match value.as_str() {
                "blackboard" => OrchestrationPattern::Blackboard,
                "ring" => OrchestrationPattern::Ring,
                _ => OrchestrationPattern::Hierarchical,
            };
        }
    }
    OrchestrationPattern::Hierarchical
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
    fn dynamically_delegated_agents_become_nodes() {
        use armadai_core::events::RunEvent;
        let t = std::time::Instant::now();
        let mut wr = Workroom::new();
        // watch-style: RunStart seeds only the root.
        wr.on_run_event_at(
            &RunEvent::RunStart {
                run_id: "r".into(),
                v: 1,
                agents: vec!["claude".into()],
                prov: "claude".into(),
                model: "m".into(),
                in_chars: 0,
            },
            t,
        );
        assert_eq!(wr.agents.len(), 1);
        // A delegation to an unknown agent must create its node.
        wr.on_run_event_at(
            &RunEvent::Delegate {
                from: "claude".into(),
                to: "core-specialist".into(),
            },
            t,
        );
        wr.on_run_event_at(
            &RunEvent::AgentStart {
                agent: "core-specialist".into(),
                prov: "claude".into(),
                model: "m".into(),
            },
            t,
        );
        assert!(
            wr.agents.iter().any(|a| a.name == "core-specialist"),
            "delegated subagent should appear as a node"
        );
        // AgentStart for another unknown agent also creates it.
        wr.on_run_event_at(
            &RunEvent::AgentStart {
                agent: "qa-specialist".into(),
                prov: "claude".into(),
                model: "m".into(),
            },
            t,
        );
        assert_eq!(
            wr.agents.len(),
            3,
            "claude + core-specialist + qa-specialist"
        );
        // New nodes are role Agent (indented under a coordinator in the tree).
        let core = wr
            .agents
            .iter()
            .find(|a| a.name == "core-specialist")
            .unwrap();
        assert_eq!(core.role, AgentRole::Agent);
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
    fn test_set_agents_from_init_filters_cli_provider_names() {
        let mut wr = Workroom::new();
        wr.set_agents_from_init(&[
            "core-specialist".to_string(),
            "claude".to_string(), // CLI/provider pseudo-agent — must be filtered
            "Claude".to_string(), // case-insensitive
            "gemini".to_string(),
            "general-purpose".to_string(), // built-in — filtered
            "qa-specialist".to_string(),
        ]);
        let names: Vec<String> = wr
            .agents_for_test()
            .iter()
            .map(|a| a.name.clone())
            .collect();
        assert!(names.contains(&"core-specialist".to_string()));
        assert!(names.contains(&"qa-specialist".to_string()));
        assert!(
            !names.iter().any(|n| n.eq_ignore_ascii_case("claude")),
            "claude leaked: {names:?}"
        );
        assert!(!names.contains(&"gemini".to_string()));
        assert!(!names.contains(&"general-purpose".to_string()));
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

    #[test]
    fn parse_pattern_reads_known_values() {
        assert_eq!(
            parse_pattern("orchestration:\n  pattern: blackboard\n"),
            OrchestrationPattern::Blackboard
        );
        assert_eq!(
            parse_pattern("orchestration:\n  pattern: \"ring\"\n"),
            OrchestrationPattern::Ring
        );
        assert_eq!(
            parse_pattern("orchestration:\n  pattern: Hierarchical\n"),
            OrchestrationPattern::Hierarchical
        );
    }

    #[test]
    fn parse_pattern_defaults_to_hierarchical() {
        assert_eq!(parse_pattern(""), OrchestrationPattern::Hierarchical);
        assert_eq!(
            parse_pattern("orchestration:\n  pattern: bogus\n"),
            OrchestrationPattern::Hierarchical
        );
    }

    #[test]
    fn init_from_config_sets_pattern() {
        let mut wr = Workroom::new();
        wr.init_from_config("orchestration:\n  pattern: ring\ncoordinator: dev-lead\n");
        assert_eq!(wr.pattern, OrchestrationPattern::Ring);
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

    #[test]
    fn test_select_next_prev_wrap() {
        let mut wr = wr_dev_lead_core(); // 2 agents: dev-lead, core-specialist
        assert_eq!(wr.selected, 0);
        wr.select_next();
        assert_eq!(wr.selected, 1);
        wr.select_next(); // wraps back to 0
        assert_eq!(wr.selected, 0);
        wr.select_prev(); // wraps to last
        assert_eq!(wr.selected, 1);
        wr.select_prev();
        assert_eq!(wr.selected, 0);
    }

    #[test]
    fn test_select_next_prev_empty_roster_no_panic() {
        let mut wr = Workroom::new();
        assert!(wr.agents_for_test().is_empty());
        wr.select_next();
        wr.select_prev();
        assert_eq!(wr.selected, 0);
    }

    #[test]
    fn test_focused_toggle() {
        let mut wr = Workroom::new();
        assert!(!wr.is_focused());
        wr.set_focused(true);
        assert!(wr.is_focused());
        wr.set_focused(false);
        assert!(!wr.is_focused());
    }

    #[test]
    fn test_selected_detail_markdown_none_when_empty() {
        let wr = Workroom::new();
        assert!(wr.selected_detail_markdown().is_none());
    }

    #[test]
    fn layout_mode_compact_when_unfocused() {
        let mut wr = Workroom::new();
        wr.pattern = OrchestrationPattern::Ring;
        wr.set_focused(false);
        assert_eq!(wr.layout_mode(60), LayoutMode::Compact);
    }

    #[test]
    fn layout_mode_rich_when_focused_and_wide() {
        let mut wr = Workroom::new();
        wr.pattern = OrchestrationPattern::Ring;
        wr.set_focused(true);
        assert_eq!(wr.layout_mode(60), LayoutMode::Ring);
    }

    #[test]
    fn layout_mode_degrades_when_narrow() {
        let mut wr = Workroom::new();
        wr.pattern = OrchestrationPattern::Blackboard;
        wr.set_focused(true);
        assert_eq!(wr.layout_mode(30), LayoutMode::Compact);
    }

    #[test]
    fn completion_hint_uses_muted_theme_and_glyph_check() {
        let mut wr = Workroom::new();
        wr.set_completed(true);
        let mut lines = Vec::new();
        wr.push_footer(&mut lines);
        let hint_line = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("run complete")))
            .expect("completion hint line present");
        let span = hint_line
            .spans
            .iter()
            .find(|s| s.content.contains("run complete"))
            .unwrap();
        assert_eq!(span.style, theme::muted());
        // Glyph comes from theme::glyphs() (unicode ✓ or its ASCII fallback
        // depending on init(ascii)), never a hardcoded character.
        assert!(span.content.starts_with(theme::glyphs().check));
    }

    #[test]
    fn error_run_shows_failure_footer_not_complete() {
        // #271: a run that ends on a RunEvent::Error must show "run failed",
        // never a misleading "run complete", once the live TUI marks the run
        // finished.
        let mut wr = Workroom::new();
        wr.on_run_event_at(
            &RunEvent::Error {
                code: "cli_timeout".to_string(),
                msg: "CLI command timed out after 300s".to_string(),
            },
            Instant::now(),
        );
        wr.set_completed(true);
        let mut lines = Vec::new();
        wr.push_footer(&mut lines);

        // No "run complete" line.
        assert!(
            !lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("run complete"))),
            "must not show a completion hint when the run failed"
        );
        // A "run failed: <msg>" line, styled as an error, with the cross glyph.
        let fail_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("run failed"))
            .expect("failure hint line present");
        assert!(
            fail_span
                .content
                .contains("CLI command timed out after 300s")
        );
        assert_eq!(fail_span.style, theme::error());
        assert!(fail_span.content.starts_with(theme::glyphs().cross));
    }

    #[test]
    fn no_completion_hint_when_not_completed() {
        let wr = Workroom::new();
        let mut lines = Vec::new();
        wr.push_footer(&mut lines);
        assert!(
            !lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("run complete")))
        );
    }

    #[test]
    fn run_id_captured_from_run_start_and_shown_on_completion() {
        // OH1 Lot 6 gap: an orchestrated run in the live Workroom TUI never
        // surfaced its run_id anywhere (the non-TUI path prints it to
        // stdout, but `human_output` is false for the TUI path to avoid
        // corrupting the alt-screen). The Workroom itself must capture it
        // from `RunStart` and render it on the hold-on-completion screen —
        // the user's last chance to copy it before the alt-screen clears.
        let mut wr = Workroom::new();
        let t = Instant::now();
        wr.on_run_event_at(
            &RunEvent::RunStart {
                run_id: "abc123".into(),
                v: 1,
                agents: vec!["alpha".into()],
                prov: "fake".into(),
                model: "m".into(),
                in_chars: 0,
            },
            t,
        );
        assert_eq!(wr.run_id(), Some("abc123"));

        // Not shown before completion — only on the hold-on-completion screen.
        let mut lines = Vec::new();
        wr.push_footer(&mut lines);
        assert!(
            !lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("abc123")))
        );

        wr.set_completed(true);
        let mut lines = Vec::new();
        wr.push_footer(&mut lines);
        assert!(
            lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("abc123"))),
            "completion footer should render the run_id for a later --resume/--replay"
        );
    }

    #[test]
    fn set_pattern_overrides_default() {
        let mut wr = Workroom::new();
        wr.set_focused(true);
        // Default pattern (no config parsed) is Hierarchical → Hierarchical layout.
        assert_eq!(wr.layout_mode(60), LayoutMode::Hierarchical);
        wr.set_pattern(OrchestrationPattern::Ring);
        assert_eq!(wr.layout_mode(60), LayoutMode::Ring);
    }

    #[test]
    fn set_pattern_overrides_config_after_init() {
        let mut wr = Workroom::new();
        wr.set_focused(true);
        // Config has no `pattern:` key → parse_pattern defaults to Hierarchical.
        wr.init_from_config("coordinator: lead\nagents:\n- a\n- b\n");
        assert_eq!(wr.layout_mode(60), LayoutMode::Hierarchical);
        // An explicit `--orchestrate blackboard` flag must win over that default.
        wr.set_pattern(OrchestrationPattern::Blackboard);
        assert_eq!(wr.layout_mode(60), LayoutMode::Blackboard);
    }

    #[test]
    fn tree_prefix_marks_last_sibling() {
        let mut wr = Workroom::new();
        wr.init_from_config("coordinator: lead\nagents:\n- a\n- b\n");
        // index 0 = coordinator (no connector)
        assert_eq!(wr.tree_prefix(0), "");
        // last agent uses the "last" connector, earlier ones the branch
        let last = wr.agents.len() - 1;
        assert!(wr.tree_prefix(last).contains(theme::glyphs().tree_last));
        assert!(
            wr.tree_prefix(last - 1)
                .contains(theme::glyphs().tree_branch)
        );
    }

    #[test]
    fn test_selected_detail_markdown_has_name_and_timeline() {
        let mut wr = wr_dev_lead_core();
        wr.apply_stream_text("<!--ARMADAI_DELEGATE:core-specialist-->");
        wr.apply_stream_text("<!--ARMADAI_META:status=complete-->");
        wr.apply_stream_text("<!--ARMADAI_END-->");

        // Select core-specialist explicitly (index depends on config order —
        // wr_dev_lead_core() yields [dev-lead, core-specialist]).
        wr.selected = 1;
        let md = wr.selected_detail_markdown().expect("agent selected");
        assert!(md.contains("core-specialist"));
        assert!(md.contains("(agent)"));
        assert!(md.contains("## Timeline"));
        assert!(md.contains("working"));
        assert!(md.contains("done"));
        assert!(md.contains("Last action:** complete"));
    }

    #[test]
    fn blackboard_lines_has_board_header() {
        let mut wr = Workroom::new();
        wr.init_from_config(
            "orchestration:\n  pattern: blackboard\ncoordinator: c\nagents:\n- a\n- b\n",
        );
        let lines = wr.blackboard_lines();
        // First line is the shared-board header carrying the board glyph.
        let first = &lines[0];
        let text: String = first.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains(theme::glyphs().board));
        assert!(text.contains("agents"));
    }

    #[test]
    fn token_holder_index_matches_current_agent() {
        let mut wr = Workroom::new();
        wr.init_from_config(
            "orchestration:\n  pattern: ring\ncoordinator: c\nagents:\n- alpha\n- beta\n",
        );
        // No token holder initially.
        assert_eq!(wr.token_holder_index(), None);
        // The token moves to an agent via a DELEGATE marker in the stream.
        wr.parse_streaming_line("<!--ARMADAI_DELEGATE:alpha-->");
        let idx = wr.token_holder_index().expect("alpha holds the token");
        assert_eq!(wr.agents[idx].name, "alpha");
    }

    #[test]
    fn agent_start_sets_token_holder_without_delegate() {
        // ES ring/blackboard engines never emit `Delegate` — only
        // agent_start/agent_end/vote — so the token-holder highlight must be
        // driven by `AgentStart` alone (review finding M2).
        let mut wr = Workroom::new();
        let t = Instant::now();
        wr.on_run_event_at(
            &RunEvent::RunStart {
                run_id: "r1".into(),
                v: 1,
                agents: vec!["alpha".into(), "beta".into()],
                prov: "fake".into(),
                model: "m".into(),
                in_chars: 0,
            },
            t,
        );
        assert_eq!(wr.token_holder_index(), None);

        wr.on_run_event_at(
            &RunEvent::AgentStart {
                agent: "alpha".into(),
                prov: "fake".into(),
                model: "m".into(),
            },
            t,
        );
        let idx = wr
            .token_holder_index()
            .expect("alpha holds the token after AgentStart");
        assert_eq!(wr.agents[idx].name, "alpha");
    }

    #[test]
    fn ring_lines_marks_token_holder_bold_brass() {
        let mut wr = Workroom::new();
        wr.init_from_config(
            "orchestration:\n  pattern: ring\ncoordinator: c\nagents:\n- alpha\n- beta\n",
        );
        wr.parse_streaming_line("<!--ARMADAI_DELEGATE:alpha-->");
        let lines = wr.ring_lines();
        // The span carrying the holder name uses the bold selection (brass) style.
        let holder_styled = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.content.contains("alpha") && s.style.add_modifier.contains(Modifier::BOLD));
        assert!(holder_styled);
    }

    use armadai_core::events::RunEvent;
    use std::time::Instant;

    fn rs(agents: &[&str]) -> RunEvent {
        RunEvent::RunStart {
            run_id: "r1".into(),
            v: 1,
            agents: agents.iter().map(|s| s.to_string()).collect(),
            prov: "fake".into(),
            model: "m".into(),
            in_chars: 0,
        }
    }

    #[test]
    fn on_run_event_seeds_and_transitions() {
        let mut wr = Workroom::new();
        let t = Instant::now();
        wr.on_run_event_at(&rs(&["dev-lead", "core-specialist"]), t);
        assert_eq!(wr.agents.len(), 2);
        assert!(wr.is_visible());

        wr.on_run_event_at(
            &RunEvent::Delegate {
                from: "dev-lead".into(),
                to: "core-specialist".into(),
            },
            t,
        );
        assert_eq!(
            wr.agents
                .iter()
                .find(|a| a.name == "dev-lead")
                .unwrap()
                .state,
            AgentState::Delegating
        );

        wr.on_run_event_at(
            &RunEvent::AgentStart {
                agent: "core-specialist".into(),
                prov: "fake".into(),
                model: "m".into(),
            },
            t,
        );
        assert_eq!(
            wr.agents
                .iter()
                .find(|a| a.name == "core-specialist")
                .unwrap()
                .state,
            AgentState::Working
        );

        wr.on_run_event_at(
            &RunEvent::AgentEnd {
                agent: "core-specialist".into(),
                tin: 1,
                tout: 2,
                cost: 0.0,
                content: "done reticulating\nsplines".into(),
            },
            t,
        );
        let a = wr
            .agents
            .iter()
            .find(|a| a.name == "core-specialist")
            .unwrap();
        assert_eq!(a.state, AgentState::Done);
        assert_eq!(a.last_action.as_deref(), Some("done reticulating"));
    }

    #[test]
    fn on_run_event_result_finalizes() {
        let mut wr = Workroom::new();
        let t = Instant::now();
        wr.on_run_event_at(&rs(&["a"]), t);
        wr.on_run_event_at(
            &RunEvent::AgentStart {
                agent: "a".into(),
                prov: "f".into(),
                model: "m".into(),
            },
            t,
        );
        wr.on_run_event_at(
            &RunEvent::Result {
                content: "x".into(),
                tin: 0,
                tout: 0,
                cost: 0.0,
                agents: 1,
            },
            t,
        );
        // on_complete turns any Working agent into Done.
        assert_eq!(
            wr.agents.iter().find(|a| a.name == "a").unwrap().state,
            AgentState::Done
        );
    }

    #[test]
    fn on_run_event_unknown_variants_are_noops() {
        let mut wr = Workroom::new();
        let t = Instant::now();
        wr.on_run_event_at(&rs(&["a"]), t);
        wr.on_run_event_at(
            &RunEvent::Warning {
                code: "w".into(),
                from: None,
                to: None,
            },
            t,
        );
        wr.on_run_event_at(
            &RunEvent::Route {
                agent: "a".into(),
                tier: "fast".into(),
                reason: "r".into(),
            },
            t,
        );
        assert_eq!(
            wr.agents
                .iter()
                .find(|a| a.name == "a")
                .unwrap()
                .last_action
                .as_deref(),
            Some("→ fast")
        );
    }
}
