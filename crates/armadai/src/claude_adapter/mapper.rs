use std::collections::HashMap;

use armadai_core::events::RunEvent;

use crate::claude_adapter::transcript::{Block, RelevantEntry, ToolResult};

const ROOT: &str = "claude";
const PROV: &str = "claude";
const MAX_CONTENT: usize = 2000;

/// Truncate `s` to at most `max_bytes` bytes, backing off to the nearest
/// preceding UTF-8 char boundary. `String::truncate` panics when the byte
/// offset lands inside a multibyte sequence (accents, emoji, CJK) — this
/// never does, at the cost of a possibly-shorter-than-`max_bytes` result.
fn truncate_chars(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Reconstructs agent-level `RunEvent`s from a stream of `RelevantEntry`.
pub struct Mapper {
    session_id: String,
    started: bool,
    model: String,
    tin: u32,
    tout: u32,
    last_text: String,
    spawns: HashMap<String, String>, // tool_use_id -> agent label (description, else subagent_type)
    agents_seen: std::collections::HashSet<String>,
    finished: bool,
}

impl Mapper {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            started: false,
            model: String::new(),
            tin: 0,
            tout: 0,
            last_text: String::new(),
            spawns: HashMap::new(),
            agents_seen: std::collections::HashSet::new(),
            finished: false,
        }
    }

    /// Feed one entry; returns the RunEvents it produces (in order).
    pub fn push(&mut self, entry: RelevantEntry) -> Vec<RunEvent> {
        let mut out = Vec::new();
        match entry {
            RelevantEntry::Assistant {
                model,
                blocks,
                usage,
                // Turn-completion signal; consumed by the follow-mode driver,
                // not the mapper.
                stop_reason: _,
            } => {
                if !self.started {
                    self.started = true;
                    self.model = model.clone();
                    self.agents_seen.insert(ROOT.to_string());
                    out.push(RunEvent::RunStart {
                        run_id: self.session_id.clone(),
                        v: 1,
                        agents: vec![ROOT.to_string()],
                        prov: PROV.to_string(),
                        model: model.clone(),
                        in_chars: 0,
                    });
                    out.push(RunEvent::AgentStart {
                        agent: ROOT.to_string(),
                        prov: PROV.to_string(),
                        model: model.clone(),
                    });
                }
                self.tin = self.tin.saturating_add(usage.input_tokens);
                self.tout = self.tout.saturating_add(usage.output_tokens);
                for b in blocks {
                    match b {
                        Block::Text(t) if !t.trim().is_empty() => self.last_text = t,
                        Block::Text(_) | Block::Tool { .. } => {}
                        Block::AgentSpawn {
                            tool_use_id,
                            subagent_type,
                            description,
                        } => {
                            // Parallel same-`subagent_type` subagents (e.g. three
                            // "Explore") share a type but carry distinct
                            // `description`s. Label by description so they stay
                            // distinct Workroom nodes and are counted correctly;
                            // fall back to `subagent_type` when absent.
                            let label = if description.trim().is_empty() {
                                subagent_type
                            } else {
                                description.trim().to_string()
                            };
                            self.spawns.insert(tool_use_id, label.clone());
                            self.agents_seen.insert(label.clone());
                            out.push(RunEvent::Delegate {
                                from: ROOT.to_string(),
                                to: label.clone(),
                            });
                            out.push(RunEvent::AgentStart {
                                agent: label,
                                prov: PROV.to_string(),
                                model: self.model.clone(),
                            });
                        }
                    }
                }
            }
            RelevantEntry::ToolResults(results) => {
                // Anthropic batches ALL tool_results for a turn into one
                // `user` message (multiple `tool_result` blocks) — e.g.
                // parallel `Agent` spawns each resolve in the same message.
                // Process every block so no `AgentEnd` is dropped.
                for ToolResult { tool_use_id, text } in results {
                    if let Some(agent) = self.spawns.remove(&tool_use_id) {
                        let content = truncate_chars(&text, MAX_CONTENT);
                        out.push(RunEvent::AgentEnd {
                            agent,
                            tin: 0,
                            tout: 0,
                            cost: 0.0,
                            content,
                        });
                    }
                }
            }
        }
        out
    }

    /// Signal end-of-stream (EOF/replay or terminal stop); returns closing events.
    pub fn finish(&mut self) -> Vec<RunEvent> {
        if self.finished || !self.started {
            return Vec::new();
        }
        self.finished = true;
        let content = truncate_chars(&self.last_text, MAX_CONTENT);
        vec![
            RunEvent::AgentEnd {
                agent: ROOT.to_string(),
                tin: self.tin,
                tout: self.tout,
                cost: 0.0,
                content: content.clone(),
            },
            RunEvent::Result {
                content,
                tin: self.tin,
                tout: self.tout,
                cost: 0.0,
                agents: self.agents_seen.len(),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_adapter::transcript::{Block, RelevantEntry, Usage};
    use armadai_core::events::RunEvent;

    fn assistant(blocks: Vec<Block>, tin: u32, tout: u32) -> RelevantEntry {
        RelevantEntry::Assistant {
            model: "m".into(),
            blocks,
            usage: Usage {
                input_tokens: tin,
                output_tokens: tout,
            },
            stop_reason: None,
        }
    }

    #[test]
    fn simple_session_emits_runstart_agentstart_result() {
        let mut m = Mapper::new("s1");
        let mut evs = m.push(assistant(vec![Block::Text("hi".into())], 10, 3));
        evs.extend(m.finish());
        assert!(
            matches!(&evs[0], RunEvent::RunStart { run_id, agents, prov, .. }
            if run_id == "s1" && agents == &vec!["claude".to_string()] && prov == "claude")
        );
        assert!(matches!(&evs[1], RunEvent::AgentStart { agent, .. } if agent == "claude"));
        // AgentEnd(claude) then Result
        assert!(matches!(evs[evs.len() - 2], RunEvent::AgentEnd { .. }));
        match evs.last().unwrap() {
            RunEvent::Result {
                content,
                tin,
                tout,
                agents,
                ..
            } => {
                assert_eq!(content, "hi");
                assert_eq!(*tin, 10);
                assert_eq!(*tout, 3);
                assert_eq!(*agents, 1);
            }
            other => panic!("expected Result, got {other:?}"),
        }
    }

    #[test]
    fn subagent_spawn_and_result_emit_delegate_start_end() {
        let mut m = Mapper::new("s2");
        let mut evs = m.push(assistant(vec![Block::Text("start".into())], 5, 1));
        // A non-empty `description` becomes the agent LABEL (not `subagent_type`).
        evs.extend(m.push(assistant(
            vec![Block::AgentSpawn {
                tool_use_id: "tu1".into(),
                subagent_type: "core".into(),
                description: "Refonte du parser".into(),
            }],
            2,
            1,
        )));
        evs.extend(m.push(RelevantEntry::ToolResults(vec![ToolResult {
            tool_use_id: "tu1".into(),
            text: "sub done".into(),
        }])));
        evs.extend(m.push(assistant(vec![Block::Text("final".into())], 1, 1)));
        evs.extend(m.finish());
        // Delegate/AgentStart/AgentEnd all carry the description, not "core".
        assert!(evs.iter().any(
            |e| matches!(e, RunEvent::Delegate { from, to } if from == "claude" && to == "Refonte du parser")
        ));
        assert!(evs.iter().any(
            |e| matches!(e, RunEvent::AgentStart { agent, .. } if agent == "Refonte du parser")
        ));
        assert!(evs.iter().any(
            |e| matches!(e, RunEvent::AgentEnd { agent, content, .. } if agent == "Refonte du parser" && content == "sub done")
        ));
        assert!(
            !evs.iter()
                .any(|e| matches!(e, RunEvent::AgentStart { agent, .. } if agent == "core")),
            "subagent_type must not be used as label when a description is present"
        );
        // agents count in Result = claude + "Refonte du parser" = 2
        match evs.last().unwrap() {
            RunEvent::Result {
                agents,
                content,
                tin,
                ..
            } => {
                assert_eq!(*agents, 2);
                assert_eq!(content, "final");
                assert_eq!(*tin, 8, "5+2+1 input tokens accumulated");
            }
            _ => panic!("expected Result"),
        }
    }

    /// C1: `String::truncate` panics if the byte offset lands inside a
    /// multibyte char. Build a 2001-byte string where byte offset 2000 (=
    /// `MAX_CONTENT`) is inside the trailing 'é' (2-byte UTF-8) — must not
    /// panic, and the returned content must be a valid char-boundary prefix.
    #[test]
    fn tool_result_truncates_multibyte_content_at_char_boundary_without_panicking() {
        let mut s = "a".repeat(1999);
        s.push('é');
        assert_eq!(s.len(), 2001, "byte 2000 must fall inside the trailing 'é'");
        assert!(!s.is_char_boundary(2000));

        let mut m = Mapper::new("s5");
        let _ = m.push(assistant(
            vec![Block::AgentSpawn {
                tool_use_id: "tu1".into(),
                subagent_type: "core".into(),
                description: String::new(),
            }],
            1,
            1,
        ));
        // Must not panic.
        let evs = m.push(RelevantEntry::ToolResults(vec![ToolResult {
            tool_use_id: "tu1".into(),
            text: s.clone(),
        }]));
        match &evs[0] {
            RunEvent::AgentEnd { content, .. } => {
                assert!(content.len() <= MAX_CONTENT);
                assert!(s.starts_with(content.as_str()), "must be a valid prefix");
            }
            other => panic!("expected AgentEnd, got {other:?}"),
        }
    }

    /// C1: same hazard via `finish()`'s `last_text` truncation.
    #[test]
    fn finish_truncates_multibyte_last_text_at_char_boundary_without_panicking() {
        let mut s = "a".repeat(1999);
        s.push('é');
        assert!(!s.is_char_boundary(2000));

        let mut m = Mapper::new("s6");
        let _ = m.push(assistant(vec![Block::Text(s.clone())], 1, 1));
        let evs = m.finish(); // must not panic
        match evs.last().unwrap() {
            RunEvent::Result { content, .. } => {
                assert!(content.len() <= MAX_CONTENT);
                assert!(s.starts_with(content.as_str()), "must be a valid prefix");
            }
            other => panic!("expected Result, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tool_result_id_is_ignored() {
        let mut m = Mapper::new("s3");
        let _ = m.push(assistant(vec![Block::Text("x".into())], 1, 1));
        let evs = m.push(RelevantEntry::ToolResults(vec![ToolResult {
            tool_use_id: "nope".into(),
            text: "y".into(),
        }]));
        assert!(evs.is_empty(), "no AgentEnd for an unknown tool_use_id");
    }

    /// I3: Anthropic batches ALL `tool_result` blocks for a turn into ONE
    /// `user` message. Two parallel `Agent` spawns resolving in the same
    /// message must BOTH get an `AgentEnd` — not just the first.
    #[test]
    fn batched_tool_results_all_produce_agent_end() {
        let mut m = Mapper::new("s4");
        let _ = m.push(assistant(vec![Block::Text("start".into())], 1, 1));
        let _ = m.push(assistant(
            vec![
                Block::AgentSpawn {
                    tool_use_id: "tu1".into(),
                    subagent_type: "core".into(),
                    description: String::new(),
                },
                Block::AgentSpawn {
                    tool_use_id: "tu2".into(),
                    subagent_type: "cli".into(),
                    description: String::new(),
                },
            ],
            1,
            1,
        ));
        // Both tool_results arrive batched in a single user message/turn.
        let evs = m.push(RelevantEntry::ToolResults(vec![
            ToolResult {
                tool_use_id: "tu1".into(),
                text: "core done".into(),
            },
            ToolResult {
                tool_use_id: "tu2".into(),
                text: "cli done".into(),
            },
        ]));
        assert!(
            evs.iter().any(
                |e| matches!(e, RunEvent::AgentEnd { agent, content, .. } if agent == "core" && content == "core done")
            ),
            "first spawn's AgentEnd must be emitted"
        );
        assert!(
            evs.iter().any(
                |e| matches!(e, RunEvent::AgentEnd { agent, content, .. } if agent == "cli" && content == "cli done")
            ),
            "second spawn's AgentEnd must NOT be dropped"
        );
    }

    /// Real Claude Code case: several parallel subagents share ONE
    /// `subagent_type` ("Explore") but carry DISTINCT `description`s. Labeling
    /// by `subagent_type` would collapse them into a single Workroom node and
    /// undercount `Result.agents`. Labeling by `description` keeps them
    /// distinct: two spawns → two AgentStart/AgentEnd + `Result.agents == 3`
    /// (claude + A + B).
    #[test]
    fn same_subagent_type_distinct_descriptions_do_not_collapse() {
        let mut m = Mapper::new("s7");
        let _ = m.push(assistant(vec![Block::Text("start".into())], 1, 1));
        let evs_spawn = m.push(assistant(
            vec![
                Block::AgentSpawn {
                    tool_use_id: "tu1".into(),
                    subagent_type: "Explore".into(),
                    description: "A".into(),
                },
                Block::AgentSpawn {
                    tool_use_id: "tu2".into(),
                    subagent_type: "Explore".into(),
                    description: "B".into(),
                },
            ],
            1,
            1,
        ));
        // Two distinct AgentStart, one per description.
        assert!(
            evs_spawn
                .iter()
                .any(|e| matches!(e, RunEvent::AgentStart { agent, .. } if agent == "A")),
            "spawn A must start a node labeled by its description"
        );
        assert!(
            evs_spawn
                .iter()
                .any(|e| matches!(e, RunEvent::AgentStart { agent, .. } if agent == "B")),
            "spawn B must start a distinct node — not collapse into A"
        );
        assert_eq!(
            evs_spawn
                .iter()
                .filter(|e| matches!(e, RunEvent::AgentStart { .. }))
                .count(),
            2,
            "two distinct subagent nodes, not one collapsed"
        );

        let evs_end = m.push(RelevantEntry::ToolResults(vec![
            ToolResult {
                tool_use_id: "tu1".into(),
                text: "A done".into(),
            },
            ToolResult {
                tool_use_id: "tu2".into(),
                text: "B done".into(),
            },
        ]));
        assert!(evs_end.iter().any(
            |e| matches!(e, RunEvent::AgentEnd { agent, content, .. } if agent == "A" && content == "A done")
        ));
        assert!(evs_end.iter().any(
            |e| matches!(e, RunEvent::AgentEnd { agent, content, .. } if agent == "B" && content == "B done")
        ));

        let evs_fin = m.finish();
        match evs_fin.last().unwrap() {
            RunEvent::Result { agents, .. } => {
                assert_eq!(*agents, 3, "claude + A + B — no collapse");
            }
            other => panic!("expected Result, got {other:?}"),
        }
    }
}
