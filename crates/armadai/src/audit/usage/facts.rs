use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

/// The native CLI's main thread, i.e. the root of every observed delegation
/// tree. Claude Code's own turns are not a declared agent, so the tree needs a
/// stable name for them.
pub const ROOT_AGENT: &str = "claude";

/// What one agent was observed doing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentUsage {
    pub invocations: u32,
    /// Assistant turns the agent actually took, read from its own transcript
    /// under `<session>/subagents/`. `invocations` counts how often it was
    /// asked; this counts how much work it did — they differ by an order of
    /// magnitude in practice.
    pub turns: u32,
    /// Model name -> number of delegations seen on that model.
    pub models: BTreeMap<String, u32>,
}

/// Deterministic aggregate of everything the scan observed. Serialisable by
/// construction: no paths, no handles, only counted facts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageFacts {
    pub sessions: u32,
    /// Oldest and newest timestamps encountered — a constat, not a filter.
    /// ISO-8601 UTC strings compare correctly lexicographically.
    pub window: Option<(String, String)>,
    pub agents: BTreeMap<String, AgentUsage>,
    /// Skill -> attributed turns (`attributionSkill`), the reliable metric.
    pub skills: BTreeMap<String, u32>,
    pub tools: BTreeMap<String, u32>,
    pub root_agent: String,
    /// Delegation edges: parent -> children.
    pub edges: BTreeMap<String, BTreeSet<String>>,
    /// Largest parallel fan-out seen in a single assistant message.
    pub max_fanout: u32,

    /// Deepest `spawnDepth` stated by a sub-agent's metadata. Unlike
    /// `depth()`, which infers a chain, this is read directly from what Claude
    /// Code recorded — no inference, no ambiguity.
    pub observed_depth: u32,
}

/// Strips control characters (including newlines) from `s`. `subagent_type`
/// and `attributionSkill` are read straight out of a transcript entry the
/// model produced, not out of a file a user reviewed, and every renderer
/// prints them verbatim — the terminal via `anstream`, Markdown, HTML. A name
/// carrying an ANSI/OSC escape sequence must never reach a terminal write,
/// and a name carrying a raw newline or backtick-adjacent control byte must
/// never reach a Markdown list item. Called at the `record_delegation` /
/// `record_skill_turn` entry point so every renderer benefits without having
/// to sanitize on its own.
fn sanitize_identifier(s: &str) -> std::borrow::Cow<'_, str> {
    if s.chars().any(char::is_control) {
        std::borrow::Cow::Owned(s.chars().filter(|c| !c.is_control()).collect())
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

impl UsageFacts {
    pub fn observe_timestamp(&mut self, ts: &str) {
        if ts.is_empty() {
            return;
        }
        self.window = Some(match self.window.take() {
            None => (ts.to_string(), ts.to_string()),
            Some((min, max)) => (
                if ts < min.as_str() {
                    ts.to_string()
                } else {
                    min
                },
                if ts > max.as_str() {
                    ts.to_string()
                } else {
                    max
                },
            ),
        });
    }

    pub fn record_delegation(&mut self, parent: &str, child: &str, model: &str) {
        let parent = sanitize_identifier(parent);
        let child = sanitize_identifier(child);
        let entry = self.agents.entry(child.to_string()).or_default();
        entry.invocations += 1;
        if !model.is_empty() {
            *entry.models.entry(model.to_string()).or_default() += 1;
        }
        self.edges
            .entry(parent.to_string())
            .or_default()
            .insert(child.to_string());
    }

    /// Record a sub-agent that actually ran, from its sidecar metadata.
    ///
    /// `parent` is `None` at `spawnDepth == 1`, where Claude Code omits
    /// `parentAgentId` because the parent is the main thread. The edge is
    /// therefore stated by the data, not inferred from a uuid chain.
    pub fn record_subagent(
        &mut self,
        agent_type: &str,
        parent: Option<&str>,
        depth: u32,
        turns: u32,
    ) {
        let child = sanitize_identifier(agent_type).into_owned();
        if child.is_empty() {
            return;
        }
        let entry = self.agents.entry(child.clone()).or_default();
        entry.turns += turns;
        let parent = parent
            .map(|p| sanitize_identifier(p).into_owned())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| {
                if self.root_agent.is_empty() {
                    ROOT_AGENT.to_string()
                } else {
                    self.root_agent.clone()
                }
            });
        self.edges.entry(parent).or_default().insert(child);
        self.observed_depth = self.observed_depth.max(depth);
    }

    pub fn record_skill_turn(&mut self, skill: &str) {
        let skill = sanitize_identifier(skill);
        if skill.is_empty() {
            return;
        }
        *self.skills.entry(skill.to_string()).or_default() += 1;
    }

    pub fn record_tool(&mut self, tool: &str) {
        if tool.is_empty() {
            return;
        }
        *self.tools.entry(tool.to_string()).or_default() += 1;
    }

    /// Most-used model for `agent`, ties broken by name for determinism.
    #[allow(dead_code)]
    pub fn dominant_model(&self, agent: &str) -> Option<&str> {
        let usage = self.agents.get(agent)?;
        usage
            .models
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(name, _)| name.as_str())
    }

    /// Longest delegation chain from the root, using memoisation to handle fan-in
    /// (a node reachable via multiple paths).
    ///
    /// Depth convention: 0 = no delegation, 1 = root → agents, 2 = root → lead → agents.
    ///
    /// Guarantees:
    /// - **exact** on any acyclic graph, including fan-in;
    /// - **always terminates** (ancestor-only stack prevents infinite loops);
    /// - **deterministic** (BTreeMap/BTreeSet iteration order makes traversal reproducible);
    /// - on a cyclic edge set: the result is a documented lower bound (terminates, deterministic).
    #[allow(dead_code)]
    pub fn depth(&self) -> u32 {
        fn longest(
            edges: &BTreeMap<String, BTreeSet<String>>,
            node: &str,
            stack: &mut BTreeSet<String>,
            memo: &mut BTreeMap<String, u32>,
        ) -> u32 {
            if stack.contains(node) {
                return 0; // cycle: stop, never recurse
            }
            if let Some(d) = memo.get(node) {
                return *d; // DAG: reuse memoised value
            }
            stack.insert(node.to_string());
            let d = 1 + edges
                .get(node)
                .into_iter()
                .flatten()
                .map(|child| longest(edges, child, stack, memo))
                .max()
                .unwrap_or(0);
            stack.remove(node); // ancestor-only: retract on exit
            memo.insert(node.to_string(), d);
            d
        }
        if self.edges.is_empty() {
            return 0;
        }
        let root = if self.root_agent.is_empty() {
            ROOT_AGENT
        } else {
            self.root_agent.as_str()
        };
        longest(
            &self.edges,
            root,
            &mut BTreeSet::new(),
            &mut BTreeMap::new(),
        )
        .saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.skills.is_empty() && self.tools.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_tracks_oldest_and_newest_timestamp() {
        let mut f = UsageFacts::default();
        f.observe_timestamp("2026-08-02T10:00:00Z");
        f.observe_timestamp("2026-07-01T10:00:00Z");
        f.observe_timestamp("2026-08-13T10:00:00Z");
        assert_eq!(
            f.window,
            Some((
                "2026-07-01T10:00:00Z".to_string(),
                "2026-08-13T10:00:00Z".to_string()
            ))
        );
    }

    #[test]
    fn delegation_counts_invocations_and_models() {
        let mut f = UsageFacts::default();
        f.record_delegation(ROOT_AGENT, "qa", "claude-opus-5");
        f.record_delegation(ROOT_AGENT, "qa", "claude-opus-5");
        f.record_delegation(ROOT_AGENT, "qa", "claude-sonnet-5");
        let qa = &f.agents["qa"];
        assert_eq!(qa.invocations, 3);
        assert_eq!(f.dominant_model("qa"), Some("claude-opus-5"));
    }

    #[test]
    fn depth_is_one_for_a_flat_tree_and_two_when_nested() {
        let mut flat = UsageFacts::default();
        flat.record_delegation(ROOT_AGENT, "qa", "m");
        flat.record_delegation(ROOT_AGENT, "core", "m");
        assert_eq!(flat.depth(), 1, "root -> agents is depth 1");

        let mut nested = UsageFacts::default();
        nested.record_delegation(ROOT_AGENT, "lead", "m");
        nested.record_delegation("lead", "qa", "m");
        assert_eq!(nested.depth(), 2, "root -> lead -> agent is depth 2");
    }

    #[test]
    fn depth_is_zero_without_any_delegation() {
        assert_eq!(UsageFacts::default().depth(), 0);
        assert!(UsageFacts::default().is_empty());
    }

    #[test]
    fn depth_terminates_on_a_cyclic_edge_set() {
        // Defensive: a malformed transcript must never hang the audit.
        let mut f = UsageFacts::default();
        f.record_delegation(ROOT_AGENT, "a", "m");
        f.record_delegation("a", "b", "m");
        f.record_delegation("b", "a", "m");
        assert!(f.depth() >= 2, "cycle must not loop forever");
    }

    #[test]
    fn depth_handles_fan_in_correctly() {
        // Regression: a node reachable via multiple paths should not be
        // short-circuited by one branch's prior visit. The longest path wins.
        // root → {p, z}, p → c, z → y → x → c, c → w.
        // Longest chain: root → z → y → x → c → w = 5 edges.
        let mut f = UsageFacts::default();
        f.record_delegation(ROOT_AGENT, "p", "m");
        f.record_delegation(ROOT_AGENT, "z", "m");
        f.record_delegation("p", "c", "m");
        f.record_delegation("z", "y", "m");
        f.record_delegation("y", "x", "m");
        f.record_delegation("x", "c", "m");
        f.record_delegation("c", "w", "m");
        assert_eq!(f.depth(), 5, "longest path root→z→y→x→c→w is depth 5");
    }

    /// Regression: `record_skill_turn` lacked the empty-string guard that
    /// `record_tool` and the model field already have. An
    /// `attributionSkill: ""` must not create a blank-named entry.
    #[test]
    fn record_skill_turn_ignores_the_empty_string() {
        let mut f = UsageFacts::default();
        f.record_skill_turn("");
        assert!(
            f.skills.is_empty(),
            "an empty attributionSkill must not create a blank entry: {:?}",
            f.skills
        );
    }

    /// `subagent_type` comes from a transcript entry the model produced, not
    /// from a file a user reviewed, and is printed verbatim by every
    /// renderer. An ANSI escape sequence and an embedded newline must not
    /// survive into storage.
    #[test]
    fn record_delegation_strips_control_characters_from_identifiers() {
        let mut f = UsageFacts::default();
        f.record_delegation(ROOT_AGENT, "qa\u{1b}[31m\ninjected", "m");
        assert_eq!(
            f.agents.keys().next().map(String::as_str),
            Some("qa[31minjected"),
            "the ESC (0x1b) and the newline must be stripped: {:?}",
            f.agents
        );
        assert!(
            f.edges[ROOT_AGENT].contains("qa[31minjected"),
            "the sanitized name must be the one recorded in edges too: {:?}",
            f.edges
        );
    }

    /// Same guarantee for `attributionSkill`, exercised through
    /// `record_skill_turn` with an OSC-style sequence (ESC ] ... BEL).
    #[test]
    fn record_skill_turn_strips_control_characters() {
        let mut f = UsageFacts::default();
        f.record_skill_turn("armadai\u{1b}]0;evil\u{7}");
        assert_eq!(
            f.skills.keys().next().map(String::as_str),
            Some("armadai]0;evil"),
            "the ESC (0x1b) and BEL (0x07) must be stripped: {:?}",
            f.skills
        );
    }

    #[test]
    fn a_subagent_at_depth_one_attaches_to_the_root() {
        let mut f = UsageFacts {
            root_agent: ROOT_AGENT.to_string(),
            ..Default::default()
        };
        // Claude Code omits parentAgentId at depth 1: the parent IS the root.
        f.record_subagent("dev-lead", None, 1, 42);
        assert_eq!(f.agents["dev-lead"].turns, 42);
        assert!(f.edges[ROOT_AGENT].contains("dev-lead"));
        assert_eq!(f.observed_depth, 1);
    }

    #[test]
    fn a_nested_subagent_attaches_to_its_named_parent() {
        let mut f = UsageFacts {
            root_agent: ROOT_AGENT.to_string(),
            ..Default::default()
        };
        f.record_subagent("dev-lead", None, 1, 10);
        f.record_subagent("qa-specialist", Some("dev-lead"), 2, 30);
        assert!(f.edges["dev-lead"].contains("qa-specialist"));
        assert!(!f.edges[ROOT_AGENT].contains("qa-specialist"));
        assert_eq!(f.observed_depth, 2, "depth is the max seen, not the last");
    }

    #[test]
    fn turns_accumulate_across_several_runs_of_the_same_agent() {
        let mut f = UsageFacts::default();
        f.record_subagent("qa-specialist", None, 1, 5);
        f.record_subagent("qa-specialist", None, 1, 7);
        assert_eq!(f.agents["qa-specialist"].turns, 12);
    }

    #[test]
    fn turns_are_independent_of_invocations() {
        // A sub-agent's own transcript says how much work it did; the parent's
        // transcript says how often it was asked. Neither implies the other.
        let mut f = UsageFacts::default();
        f.record_delegation(ROOT_AGENT, "qa-specialist", "m");
        f.record_subagent("qa-specialist", None, 1, 99);
        let u = &f.agents["qa-specialist"];
        assert_eq!((u.invocations, u.turns), (1, 99));
    }

    #[test]
    fn a_subagent_with_a_blank_type_is_ignored() {
        let mut f = UsageFacts::default();
        f.record_subagent("", None, 1, 5);
        assert!(f.agents.is_empty());
    }

    #[test]
    fn skill_turns_and_tools_accumulate() {
        let mut f = UsageFacts::default();
        f.record_skill_turn("armadai");
        f.record_skill_turn("armadai");
        f.record_tool("Bash");
        assert_eq!(f.skills["armadai"], 2);
        assert_eq!(f.tools["Bash"], 1);
    }
}
