use std::collections::{BTreeMap, BTreeSet};

/// The native CLI's main thread, i.e. the root of every observed delegation
/// tree. Claude Code's own turns are not a declared agent, so the tree needs a
/// stable name for them.
pub const ROOT_AGENT: &str = "claude";

/// What one agent was observed doing.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentUsage {
    pub invocations: u32,
    /// Model name -> number of delegations seen on that model.
    pub models: BTreeMap<String, u32>,
}

/// Deterministic aggregate of everything the scan observed. Serialisable by
/// construction: no paths, no handles, only counted facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    pub fn record_skill_turn(&mut self, skill: &str) {
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

    #[allow(dead_code)]
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
