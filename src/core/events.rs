use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Structured run events emitted in headless/JSON mode. Short keys for token economy.
#[allow(dead_code)]
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
}

/// Sink for run events. `NullSink` is a zero-cost no-op; `JsonlSink` writes JSONL to a writer.
#[allow(dead_code)]
pub trait EventSink: Send + Sync {
    fn emit(&self, ev: &RunEvent);
}

#[allow(dead_code)]
pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _ev: &RunEvent) {}
}

#[allow(dead_code)]
pub struct JsonlSink {
    pub out: Mutex<Box<dyn std::io::Write + Send>>,
}

impl JsonlSink {
    #[allow(dead_code)]
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
#[allow(dead_code)]
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
