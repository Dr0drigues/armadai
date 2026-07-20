use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Structured run events emitted in headless/JSON mode. Short keys for token economy.
#[derive(Debug, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RunEvent {
    RunStart {
        v: u32,
        agents: Vec<String>,
        prov: String,
        model: String,
        in_chars: usize,
    },
    AgentStart {
        agent: String,
        prov: String,
        model: String,
    },
    AgentEnd {
        agent: String,
        tin: u32,
        tout: u32,
        cost: f64,
        content: String,
    },
    Warning {
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
    Result {
        content: String,
        tin: u32,
        tout: u32,
        cost: f64,
        agents: usize,
    },
    Error {
        code: String,
        msg: String,
    },
    Route {
        agent: String,
        tier: String,
        reason: String,
    },
    Delegate {
        from: String,
        to: String,
    },
    Vote {
        agent: String,
        conf: f32,
    },
    Board {
        agent: String,
        kind: String,
    },
    NestedStart {
        team_lead: String,
        pattern: String,
    },
    NestedEnd {
        team_lead: String,
    },
    AgentSelect {
        selected: Vec<String>,
        reason: String,
    },
}

/// Sink for run events. `NullSink` is a zero-cost no-op; `JsonlSink` writes JSONL to a writer.
pub trait EventSink: Send + Sync {
    fn emit(&self, ev: &RunEvent);
}

pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _ev: &RunEvent) {}
}

pub struct JsonlSink {
    pub out: Mutex<Box<dyn std::io::Write + Send>>,
}

impl JsonlSink {
    pub fn stdout() -> Self {
        JsonlSink {
            out: Mutex::new(Box::new(std::io::stdout())),
        }
    }
}

impl EventSink for JsonlSink {
    fn emit(&self, ev: &RunEvent) {
        if let Ok(line) = serde_json::to_string(ev) {
            let mut w = self.out.lock().unwrap();
            let _ = writeln!(w, "{line}");
            let _ = w.flush();
        }
    }
}

/// Build the sink for a run: JSONL to stdout when `json`, otherwise a no-op.
pub fn make_sink(json: bool) -> Arc<dyn EventSink> {
    if json {
        Arc::new(JsonlSink::stdout())
    } else {
        Arc::new(NullSink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_start_serializes_with_short_keys() {
        let ev = RunEvent::RunStart {
            v: 1,
            agents: vec!["dev-lead".into()],
            prov: "claude".into(),
            model: "claude-x".into(),
            in_chars: 412,
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"run_start","v":1,"agents":["dev-lead"],"prov":"claude","model":"claude-x","in_chars":412}"#
        );
    }

    #[test]
    fn agent_end_serializes_with_short_keys() {
        let ev = RunEvent::AgentEnd {
            agent: "a".into(),
            tin: 10,
            tout: 20,
            cost: 0.001,
            content: "hi".into(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"agent_end","agent":"a","tin":10,"tout":20,"cost":0.001,"content":"hi"}"#
        );
    }

    #[test]
    fn route_serializes_with_short_keys() {
        let ev = RunEvent::Route {
            agent: "dev-lead".into(),
            tier: "Max".into(),
            reason: "Tag".into(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"route","agent":"dev-lead","tier":"Max","reason":"Tag"}"#
        );
    }

    #[test]
    fn jsonl_sink_writes_one_line_per_event() {
        use std::sync::{Arc, Mutex};
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = JsonlSink {
            out: Mutex::new(Box::new(SharedBuf(buf.clone()))),
        };
        sink.emit(&RunEvent::Error {
            code: "x".into(),
            msg: "y".into(),
        });
        sink.emit(&RunEvent::Error {
            code: "z".into(),
            msg: "w".into(),
        });
        let s = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert!(
            s.lines()
                .all(|l| serde_json::from_str::<serde_json::Value>(l).is_ok())
        );
    }

    #[test]
    fn result_event_present_and_last() {
        // JSONL contract: every emitted line parses as JSON, and the `result`
        // event is always the terminal line of a run (headless consumers can
        // stop reading once they see `t == "result"`).
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = JsonlSink {
            out: Mutex::new(Box::new(SharedBuf(buf.clone()))),
        };
        sink.emit(&RunEvent::RunStart {
            v: 1,
            agents: vec!["a".into()],
            prov: "p".into(),
            model: "m".into(),
            in_chars: 3,
        });
        sink.emit(&RunEvent::AgentStart {
            agent: "a".into(),
            prov: "p".into(),
            model: "m".into(),
        });
        sink.emit(&RunEvent::AgentEnd {
            agent: "a".into(),
            tin: 1,
            tout: 2,
            cost: 0.0,
            content: "x".into(),
        });
        sink.emit(&RunEvent::Result {
            content: "x".into(),
            tin: 1,
            tout: 2,
            cost: 0.0,
            agents: 1,
        });

        let s = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(
            lines
                .iter()
                .all(|l| serde_json::from_str::<serde_json::Value>(l).is_ok())
        );
        let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(last["t"], "result");
    }

    #[test]
    fn delegate_serializes_with_short_keys() {
        let ev = RunEvent::Delegate {
            from: "dev-lead".into(),
            to: "core-specialist".into(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"delegate","from":"dev-lead","to":"core-specialist"}"#
        );
    }

    #[test]
    fn vote_serializes_with_short_keys() {
        let ev = RunEvent::Vote {
            agent: "reviewer".into(),
            conf: 0.95,
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(s, r#"{"t":"vote","agent":"reviewer","conf":0.95}"#);
    }

    #[test]
    fn board_serializes_with_short_keys() {
        let ev = RunEvent::Board {
            agent: "qa-specialist".into(),
            kind: "passed".into(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"board","agent":"qa-specialist","kind":"passed"}"#
        );
    }

    #[test]
    fn nested_start_serializes_with_short_keys() {
        let ev = RunEvent::NestedStart {
            team_lead: "research-lead".to_string(),
            pattern: "blackboard".to_string(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"nested_start","team_lead":"research-lead","pattern":"blackboard"}"#
        );
    }

    #[test]
    fn nested_end_serializes_with_short_keys() {
        let ev = RunEvent::NestedEnd {
            team_lead: "research-lead".to_string(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(s, r#"{"t":"nested_end","team_lead":"research-lead"}"#);
    }

    #[test]
    fn agent_select_serializes_with_short_keys() {
        let ev = RunEvent::AgentSelect {
            selected: vec!["rust-security".to_string(), "qa-specialist".to_string()],
            reason: "route 'security-audit' → 2 agents".to_string(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"agent_select","selected":["rust-security","qa-specialist"],"reason":"route 'security-audit' → 2 agents"}"#
        );
    }

    // Test helper: a Write that appends to a shared buffer.
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for SharedBuf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
