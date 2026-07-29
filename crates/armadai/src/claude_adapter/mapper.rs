use std::collections::HashMap;

use armadai_core::events::RunEvent;

use crate::claude_adapter::transcript::{Block, RelevantEntry};

const ROOT: &str = "claude";
const PROV: &str = "claude";
const MAX_CONTENT: usize = 2000;

/// Reconstructs agent-level `RunEvent`s from a stream of `RelevantEntry`.
pub struct Mapper {
    session_id: String,
    started: bool,
    model: String,
    tin: u32,
    tout: u32,
    last_text: String,
    spawns: HashMap<String, String>, // tool_use_id -> subagent_type
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
                        Block::Text(_) | Block::Other => {}
                        Block::AgentSpawn {
                            tool_use_id,
                            subagent_type,
                        } => {
                            self.spawns.insert(tool_use_id, subagent_type.clone());
                            self.agents_seen.insert(subagent_type.clone());
                            out.push(RunEvent::Delegate {
                                from: ROOT.to_string(),
                                to: subagent_type.clone(),
                            });
                            out.push(RunEvent::AgentStart {
                                agent: subagent_type,
                                prov: PROV.to_string(),
                                model: self.model.clone(),
                            });
                        }
                    }
                }
            }
            RelevantEntry::ToolResult { tool_use_id, text } => {
                if let Some(agent) = self.spawns.remove(&tool_use_id) {
                    let mut content = text;
                    content.truncate(MAX_CONTENT);
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
        out
    }

    /// Signal end-of-stream (EOF/replay or terminal stop); returns closing events.
    pub fn finish(&mut self) -> Vec<RunEvent> {
        if self.finished || !self.started {
            return Vec::new();
        }
        self.finished = true;
        let mut content = self.last_text.clone();
        content.truncate(MAX_CONTENT);
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
        evs.extend(m.push(assistant(
            vec![Block::AgentSpawn {
                tool_use_id: "tu1".into(),
                subagent_type: "core".into(),
            }],
            2,
            1,
        )));
        evs.extend(m.push(RelevantEntry::ToolResult {
            tool_use_id: "tu1".into(),
            text: "sub done".into(),
        }));
        evs.extend(m.push(assistant(vec![Block::Text("final".into())], 1, 1)));
        evs.extend(m.finish());
        assert!(evs.iter().any(
            |e| matches!(e, RunEvent::Delegate { from, to } if from == "claude" && to == "core")
        ));
        assert!(
            evs.iter()
                .any(|e| matches!(e, RunEvent::AgentStart { agent, .. } if agent == "core"))
        );
        assert!(evs.iter().any(
            |e| matches!(e, RunEvent::AgentEnd { agent, content, .. } if agent == "core" && content == "sub done")
        ));
        // agents count in Result = claude + core = 2
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

    #[test]
    fn unknown_tool_result_id_is_ignored() {
        let mut m = Mapper::new("s3");
        let _ = m.push(assistant(vec![Block::Text("x".into())], 1, 1));
        let evs = m.push(RelevantEntry::ToolResult {
            tool_use_id: "nope".into(),
            text: "y".into(),
        });
        assert!(evs.is_empty(), "no AgentEnd for an unknown tool_use_id");
    }
}
