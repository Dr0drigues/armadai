#![cfg(feature = "tui")]

use crate::core::events::{EventSink, RunEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// An `EventSink` that forwards a clone of every `RunEvent` into a channel,
/// so a TUI render loop can drain and project them onto a `Workroom`.
pub struct WorkroomSink {
    tx: UnboundedSender<RunEvent>,
}

impl WorkroomSink {
    pub fn new() -> (Self, UnboundedReceiver<RunEvent>) {
        let (tx, rx) = unbounded_channel();
        (Self { tx }, rx)
    }
}

impl EventSink for WorkroomSink {
    fn emit(&self, ev: &RunEvent) {
        // Receiver gone (TUI exited) → drop silently; the run still completes.
        let _ = self.tx.send(ev.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::{EventSink, RunEvent};
    use crate::shell::workroom::Workroom;
    use std::time::Instant;

    #[test]
    fn sink_forwards_events_to_projection() {
        let (sink, mut rx) = WorkroomSink::new();
        sink.emit(&RunEvent::RunStart {
            v: 1,
            agents: vec!["a".into(), "b".into()],
            prov: "f".into(),
            model: "m".into(),
            in_chars: 0,
        });
        sink.emit(&RunEvent::AgentStart {
            agent: "a".into(),
            prov: "f".into(),
            model: "m".into(),
        });
        sink.emit(&RunEvent::AgentEnd {
            agent: "a".into(),
            tin: 0,
            tout: 0,
            cost: 0.0,
            content: "hi".into(),
        });
        drop(sink);

        let mut wr = Workroom::new();
        let now = Instant::now();
        while let Ok(ev) = rx.try_recv() {
            wr.on_run_event_at(&ev, now);
        }
        let agents = wr.agents_for_test();
        assert_eq!(agents.len(), 2);
        assert_eq!(
            agents.iter().find(|a| a.name == "a").unwrap().state,
            crate::shell::workroom::AgentState::Done
        );
    }
}
