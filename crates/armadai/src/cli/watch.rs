use crate::claude_adapter::{drive_session, session_index};

/// `armadai watch` — attach the Workroom to a Claude Code session (from the
/// index the plugin populates) and stream reconstructed RunEvents.
pub async fn execute(last: bool, session: Option<String>, json: bool) -> anyhow::Result<()> {
    let sessions = session_index::load()?;
    if sessions.is_empty() {
        anyhow::bail!(
            "no Claude Code sessions registered — install the armadai-workroom plugin \
             (see crates/armadai/assets/claude-plugin) and start a Claude Code session"
        );
    }
    // Default (no --last, no --session): pick the most recent.
    let picked =
        session_index::resolve(&sessions, last || session.is_none(), session.as_deref())
            .ok_or_else(|| anyhow::anyhow!("no matching session (use --last or --session <id>)"))?;

    if json {
        // Headless: replay to JSONL on stdout (no TUI).
        let sink = armadai_core::events::make_sink(true);
        return drive_session(picked, sink, false).await;
    }

    // Live Workroom TUI, fed by the transcript adapter. `follow=true` tails.
    let (_run_id, _content) = crate::shell::run_view::run_orchestration_tui(
        move |sink| async move { drive_session(picked, sink, true).await },
        None,
        None,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds `ENV_MUTEX` for the duration of `ARMADAI_SESSION_INDEX`
    /// mutation, serialising it against other env-mutating tests across the
    /// crate. Wrapped in a struct (rather than a bare local `MutexGuard`)
    /// because these tests are `#[tokio::test]` and hold the lock across an
    /// `.await` — clippy's `await_holding_lock` flags a bare guard binding
    /// there but not one nested inside another type. Mirrors the
    /// `TempStorageGuard` pattern in `cli/run.rs` and `web/api.rs`.
    struct SessionIndexEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl SessionIndexEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let lock = armadai_core::config::ENV_MUTEX.lock().unwrap();
            // SAFETY: modifies the global environment; serialised via ENV_MUTEX.
            unsafe {
                std::env::set_var("ARMADAI_SESSION_INDEX", path);
            }
            Self { _lock: lock }
        }
    }

    impl Drop for SessionIndexEnvGuard {
        fn drop(&mut self) {
            // SAFETY: restoring env state at end of test scope.
            unsafe {
                std::env::remove_var("ARMADAI_SESSION_INDEX");
            }
        }
    }

    #[tokio::test]
    async fn json_mode_replays_without_tui() {
        let dir = tempfile::tempdir().unwrap();
        let tp = dir.path().join("t.jsonl");
        std::fs::write(
            &tp,
            concat!(
                r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
                "\n",
            ),
        )
        .unwrap();
        let idx = dir.path().join("idx.jsonl");
        let _env = SessionIndexEnvGuard::set(&idx);
        crate::claude_adapter::session_index::append(
            &crate::claude_adapter::session_index::SessionRef {
                session_id: "s".into(),
                transcript_path: tp,
                cwd: "/c".into(),
                started_at: "t".into(),
            },
        )
        .unwrap();
        // json=true → no TUI; must resolve --last and complete without error.
        let r = execute(true, None, true).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn errors_when_no_session_found() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("empty.jsonl");
        let _env = SessionIndexEnvGuard::set(&idx);
        let r = execute(false, Some("does-not-exist".into()), true).await;
        assert!(r.is_err());
    }
}
