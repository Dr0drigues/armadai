pub mod mapper;
pub mod session_index;
pub mod transcript;

use std::io::{BufRead, Read};
use std::sync::Arc;

use armadai_core::events::{EventSink, RunEvent};

use mapper::Mapper;
use session_index::SessionRef;

/// Read `session`'s transcript and emit reconstructed `RunEvent`s to `sink`.
/// `follow=false` → replay to EOF then `finish()`. `follow=true` → after EOF,
/// keep polling appended bytes until a terminal `Result` is produced.
pub async fn drive_session(
    session: SessionRef,
    sink: Arc<dyn EventSink>,
    follow: bool,
) -> anyhow::Result<()> {
    let mut mapper = Mapper::new(&session.session_id);
    let mut offset: u64 = 0;
    let mut done = false;
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
            if let Some(entry) = transcript::parse_line(&line) {
                for ev in mapper.push(entry) {
                    if matches!(ev, RunEvent::Result { .. }) {
                        done = true;
                    }
                    sink.emit(&ev);
                }
            }
        }
        offset += consumed;
        if done {
            return Ok(());
        }
        if !follow {
            for ev in mapper.finish() {
                sink.emit(&ev);
            }
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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
    let entry = SessionRef {
        session_id,
        transcript_path: transcript_path.into(),
        cwd: get("cwd"),
        started_at: get("timestamp"),
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
}
