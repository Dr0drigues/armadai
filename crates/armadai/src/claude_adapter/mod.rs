pub mod mapper;
pub mod policy_gate;
pub mod session_index;
pub mod transcript;

use std::io::{BufRead, Read};
use std::sync::Arc;

use armadai_core::events::{EventSink, RunEvent};

use mapper::Mapper;
use session_index::SessionRef;

/// Poll interval between reads of the tailed transcript in follow mode.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// Abandonment safety net, NOT the normal completion path. In follow mode we
/// finalize a turn on its terminal `stop_reason` (see [`is_terminal_stop`]);
/// while the last assistant message is still `tool_use` (or has no
/// stop_reason yet) we keep polling no matter how long the tool/subagent
/// takes — Bash/WebFetch/subagents routinely run for minutes with no
/// transcript growth, and an idle-timer finalize there would fake a `Result`
/// and stop following mid-run. This large fallback only guards against a
/// truly-abandoned, never-completed session so `drive_session` can't hang
/// forever; it should essentially never fire on a healthy transcript.
const IDLE_ABANDON_POLLS: u32 = 900; // ~3 min of no growth at POLL_INTERVAL=200ms

/// A turn is genuinely complete only when the last top-level assistant
/// message carries a TERMINAL `stop_reason`. `"tool_use"` means Claude is
/// waiting on tool/subagent results (keep following); `None` means "still
/// going" (absent/null stop_reason). Anything else terminal → the turn ended.
fn is_terminal_stop(sr: &Option<String>) -> bool {
    matches!(
        sr.as_deref(),
        Some("end_turn") | Some("stop_sequence") | Some("max_tokens")
    )
}

/// Read `session`'s transcript and emit reconstructed `RunEvent`s to `sink`.
/// `follow=false` → replay to EOF then `finish()`. `follow=true` → after EOF,
/// keep polling appended bytes, finalizing (`finish()`) once the last
/// assistant message reports a terminal `stop_reason` (see
/// [`is_terminal_stop`]); [`IDLE_ABANDON_POLLS`] is only an abandonment
/// safety net for a session that never completes.
pub async fn drive_session(
    session: SessionRef,
    sink: Arc<dyn EventSink>,
    follow: bool,
) -> anyhow::Result<()> {
    drive_session_tuned(session, sink, follow, POLL_INTERVAL, IDLE_ABANDON_POLLS).await
}

/// Same as [`drive_session`] but with the poll interval and the abandonment
/// fallback threshold as parameters, so tests can drive both the
/// terminal-`stop_reason` finalization path and the keep-polling path without
/// waiting on real-world timings.
async fn drive_session_tuned(
    session: SessionRef,
    sink: Arc<dyn EventSink>,
    follow: bool,
    poll_interval: std::time::Duration,
    idle_abandon_polls: u32,
) -> anyhow::Result<()> {
    let mut mapper = Mapper::new(&session.session_id);
    let mut offset: u64 = 0;
    let mut idle_polls: u32 = 0;
    // Terminal signal from the LAST top-level assistant message seen so far.
    let mut last_stop_reason: Option<String> = None;
    loop {
        let file = match std::fs::File::open(&session.transcript_path) {
            Ok(f) => f,
            Err(e) => {
                sink.emit(&RunEvent::Error {
                    code: "transcript_unreadable".into(),
                    msg: format!("{}: {e}", session.transcript_path.display()),
                });
                return Ok(());
            }
        };
        use std::io::Seek;
        let mut reader = std::io::BufReader::new(file);
        reader.seek(std::io::SeekFrom::Start(offset))?;
        let mut consumed = 0u64;
        let mut lines_read = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break; // EOF
            }
            // Only advance past complete (newline-terminated) lines, so a
            // partially-written trailing line is re-read next poll.
            if !line.ends_with('\n') {
                break;
            }
            consumed += n as u64;
            lines_read += 1;
            if let Some(entry) = transcript::parse_line(&line) {
                // Capture the turn-completion signal from the latest assistant
                // message before handing the entry to the mapper (which moves
                // it and does not need stop_reason).
                if let transcript::RelevantEntry::Assistant { stop_reason, .. } = &entry {
                    last_stop_reason = stop_reason.clone();
                }
                for ev in mapper.push(entry) {
                    sink.emit(&ev);
                }
            }
        }
        offset += consumed;
        if !follow {
            for ev in mapper.finish() {
                sink.emit(&ev);
            }
            return Ok(());
        }
        if lines_read > 0 {
            // The transcript grew this poll — reset the abandonment counter.
            idle_polls = 0;
        } else {
            // EOF: no new complete line this poll. Finalize on a genuine
            // terminal signal; keep polling while `tool_use`/`None` means the
            // turn is still in progress (a slow Bash/WebFetch/subagent).
            if is_terminal_stop(&last_stop_reason) {
                for ev in mapper.finish() {
                    sink.emit(&ev);
                }
                return Ok(());
            }
            idle_polls += 1;
            if idle_polls >= idle_abandon_polls {
                // Abandonment safety net (see IDLE_ABANDON_POLLS): the session
                // never reached a terminal stop_reason — finalize anyway so we
                // don't poll a dead transcript forever.
                for ev in mapper.finish() {
                    sink.emit(&ev);
                }
                return Ok(());
            }
        }
        tokio::time::sleep(poll_interval).await;
    }
}

/// Hook entrypoint: read the SessionStart payload from stdin, append the
/// session to the index. Always returns `Ok(())` (errors warned), so the hook
/// never disturbs Claude Code.
pub fn register_from_stdin() -> anyhow::Result<()> {
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() {
        return Ok(());
    }
    let _ = register_from_reader(&buf[..]);
    Ok(())
}

fn register_from_reader(mut r: impl Read) -> anyhow::Result<()> {
    let mut buf = String::new();
    r.read_to_string(&mut buf)?;
    let v: serde_json::Value = serde_json::from_str(buf.trim())?;
    let get = |k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let session_id = get("session_id");
    let transcript_path = get("transcript_path");
    if session_id.is_empty() || transcript_path.is_empty() {
        return Ok(()); // nothing usable; do not error
    }
    // The SessionStart hook payload has no `timestamp`. The hook fires at
    // session start, so stamp `started_at` with the current time when the
    // payload omits (or leaves empty) a timestamp — otherwise it stays "".
    let started_at = {
        let ts = v
            .get("timestamp")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if ts.is_empty() {
            chrono::Utc::now().to_rfc3339()
        } else {
            ts.to_string()
        }
    };
    let entry = SessionRef {
        session_id,
        transcript_path: transcript_path.into(),
        cwd: get("cwd"),
        started_at,
    };
    if let Err(e) = session_index::append(&entry) {
        tracing::warn!("failed to register Claude Code session: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use armadai_core::events::RunEvent;
    use std::sync::{Arc, Mutex};

    struct CapSink(Arc<Mutex<Vec<RunEvent>>>);
    impl armadai_core::events::EventSink for CapSink {
        fn emit(&self, ev: &RunEvent) {
            self.0.lock().unwrap().push(ev.clone());
        }
    }

    #[tokio::test]
    async fn drive_session_replays_a_transcript_to_events() {
        let dir = tempfile::tempdir().unwrap();
        let tp = dir.path().join("t.jsonl");
        std::fs::write(
            &tp,
            concat!(
                r#"{"type":"ai-title","aiTitle":"noise"}"#,
                "\n",
                r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":4,"output_tokens":2}}}"#,
                "\n",
            ),
        )
        .unwrap();
        let session = session_index::SessionRef {
            session_id: "s".into(),
            transcript_path: tp,
            cwd: "/c".into(),
            started_at: "t".into(),
        };
        let store = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn armadai_core::events::EventSink> = Arc::new(CapSink(store.clone()));
        drive_session(session, sink, false).await.unwrap();
        let evs = store.lock().unwrap();
        assert!(matches!(&evs[0], RunEvent::RunStart { run_id, .. } if run_id == "s"));
        assert!(
            matches!(evs.last().unwrap(), RunEvent::Result { content, .. } if content == "hello")
        );
    }

    /// In follow mode, `mapper.push` never itself yields a terminal `Result`
    /// (only `finish()` does). Finalization happens on the transcript's
    /// terminal signal: the last assistant message's `stop_reason` is
    /// `"end_turn"`. Drive an already-complete transcript with `follow=true`,
    /// a tiny poll interval, and a DELIBERATELY HUGE abandonment threshold —
    /// so the fact that it still finalizes promptly proves it went through the
    /// terminal-`stop_reason` path (the first idle poll), NOT the idle
    /// fallback. A timeout guards against a regression that would hang.
    #[tokio::test]
    async fn drive_session_follow_mode_finalizes_on_terminal_stop_reason() {
        let dir = tempfile::tempdir().unwrap();
        let tp = dir.path().join("t.jsonl");
        std::fs::write(
            &tp,
            concat!(
                r#"{"type":"assistant","message":{"model":"m","stop_reason":"end_turn","content":[{"type":"text","text":"done talking"}],"usage":{"input_tokens":3,"output_tokens":2}}}"#,
                "\n",
            ),
        )
        .unwrap();
        let session = session_index::SessionRef {
            session_id: "s-follow".into(),
            transcript_path: tp,
            cwd: "/c".into(),
            started_at: "t".into(),
        };
        let store = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn armadai_core::events::EventSink> = Arc::new(CapSink(store.clone()));
        // Huge abandon threshold: if finalization depended on the idle
        // fallback the test would run ~minutes and the timeout would trip.
        // The terminal `stop_reason` must fire on the first idle poll instead.
        let res = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            drive_session_tuned(
                session,
                sink,
                /* follow = */ true,
                std::time::Duration::from_millis(1),
                /* idle_abandon_polls = */ 1_000_000,
            ),
        )
        .await;
        assert!(
            res.is_ok(),
            "must finalize via terminal stop_reason, not hang on the idle fallback"
        );
        res.unwrap().unwrap();
        let evs = store.lock().unwrap();
        assert!(
            matches!(evs.last().unwrap(), RunEvent::Result { content, .. } if content == "done talking"),
            "terminal stop_reason must finalize (emit Result): {evs:?}"
        );
    }

    /// The whole point of the P2 fix: while the last assistant message's
    /// `stop_reason` is `"tool_use"` (Claude is waiting on a tool/subagent
    /// that may run for minutes with no transcript growth), follow mode must
    /// NOT finalize on idle — it must keep polling. We prove it by setting the
    /// abandon threshold absurdly high and asserting the call is STILL running
    /// (times out) with NO `Result` emitted after a short bounded window.
    #[tokio::test]
    async fn drive_session_follow_mode_keeps_polling_while_tool_use() {
        let dir = tempfile::tempdir().unwrap();
        let tp = dir.path().join("t.jsonl");
        std::fs::write(
            &tp,
            concat!(
                r#"{"type":"assistant","message":{"model":"m","stop_reason":"tool_use","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"sleep 600"}}],"usage":{"input_tokens":3,"output_tokens":2}}}"#,
                "\n",
            ),
        )
        .unwrap();
        let session = session_index::SessionRef {
            session_id: "s-tooluse".into(),
            transcript_path: tp,
            cwd: "/c".into(),
            started_at: "t".into(),
        };
        let store = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn armadai_core::events::EventSink> = Arc::new(CapSink(store.clone()));
        // High abandon threshold so ONLY a (wrongly implemented) terminal
        // finalize could end it. The call must still be polling after the
        // window → timeout Err.
        let res = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            drive_session_tuned(
                session,
                sink,
                /* follow = */ true,
                std::time::Duration::from_millis(1),
                /* idle_abandon_polls = */ 1_000_000,
            ),
        )
        .await;
        assert!(
            res.is_err(),
            "follow mode must keep polling while stop_reason is tool_use, not finalize"
        );
        let evs = store.lock().unwrap();
        assert!(
            !evs.iter().any(|e| matches!(e, RunEvent::Result { .. })),
            "no Result may be emitted while the turn is still tool_use: {evs:?}"
        );
    }

    #[test]
    fn register_from_reader_appends_index() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("idx.jsonl");
        // SAFETY: single-threaded test; serialise env via ENV_MUTEX in real cross-test setups.
        let _g = armadai_core::config::ENV_MUTEX.lock().unwrap();
        unsafe {
            std::env::set_var("ARMADAI_SESSION_INDEX", &idx);
        }
        let payload = r#"{"session_id":"z","transcript_path":"/t/z.jsonl","cwd":"/c"}"#;
        register_from_reader(payload.as_bytes()).unwrap();
        let v = session_index::load().unwrap();
        unsafe {
            std::env::remove_var("ARMADAI_SESSION_INDEX");
        }
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].session_id, "z");
    }

    /// Fix B: the SessionStart hook payload carries NO `timestamp`, so
    /// `started_at` used to stay "". It must instead be stamped with the
    /// current time (RFC3339) at registration.
    #[test]
    fn register_from_reader_stamps_started_at_when_payload_has_no_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("idx.jsonl");
        // SAFETY: single-threaded test; serialise env via ENV_MUTEX.
        let _g = armadai_core::config::ENV_MUTEX.lock().unwrap();
        unsafe {
            std::env::set_var("ARMADAI_SESSION_INDEX", &idx);
        }
        // No `timestamp` field at all.
        let payload = r#"{"session_id":"nots","transcript_path":"/t/nots.jsonl","cwd":"/c"}"#;
        register_from_reader(payload.as_bytes()).unwrap();
        let v = session_index::load().unwrap();
        unsafe {
            std::env::remove_var("ARMADAI_SESSION_INDEX");
        }
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].session_id, "nots");
        assert!(
            !v[0].started_at.is_empty(),
            "started_at must be stamped when the payload has no timestamp"
        );
        // Must be a parseable RFC3339 timestamp.
        assert!(
            chrono::DateTime::parse_from_rfc3339(&v[0].started_at).is_ok(),
            "started_at must be valid RFC3339, got {:?}",
            v[0].started_at
        );
    }
}
