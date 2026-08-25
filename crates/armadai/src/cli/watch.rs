use crate::claude_adapter::{drive_session, session_index};

/// Minimal synthetic project config so the Workroom seeds the transcript's
/// root agent ("claude") as the Coordinator — the delegated subagents (added
/// dynamically as role Agent, see Workroom::ensure_agent) then indent beneath
/// it in the hierarchical tree. A watched Claude Code session has no
/// armadai.yaml, so we synthesize the minimum init_from_config needs.
const WATCH_ROOT_CONFIG: &str = "coordinator: claude\n";

/// `armadai watch` — attach the Workroom to a Claude Code session (from the
/// index the plugin populates) and stream reconstructed RunEvents.
///
/// `_last` (`--last`) is accepted for CLI compatibility/documentation but no
/// longer changes resolution: an explicit `--session <id>` always wins (it
/// used to be silently overridden by `--last`, see M1) and, when no
/// `--session` is given, the most-recent session is picked regardless —
/// which was already the default with no flags at all.
pub async fn execute(_last: bool, session: Option<String>, json: bool) -> anyhow::Result<()> {
    let sessions = session_index::load()?;
    if sessions.is_empty() {
        anyhow::bail!(
            "no Claude Code sessions registered — install the armadai-workroom plugin \
             (see crates/armadai/assets/claude-plugin) and start a Claude Code session"
        );
    }
    // Explicit `--session <id>` always wins over `--last`. Only fall back to
    // "most recent" when no session id was given at all.
    let picked = if session.is_some() {
        session_index::resolve(&sessions, false, session.as_deref())
    } else {
        session_index::resolve(&sessions, true, None)
    }
    .ok_or_else(|| anyhow::anyhow!("no matching session (use --last or --session <id>)"))?;

    if json {
        // Headless: replay to JSONL on stdout (no TUI).
        let sink = armadai_core::events::make_sink(true);
        return drive_session(picked, sink, false).await;
    }

    // Live Workroom TUI, fed by the transcript adapter. `follow=true` tails.
    let (_run_id, _content) = crate::shell::run_view::run_orchestration_tui(
        move |sink| async move { drive_session(picked, sink, true).await },
        Some(WATCH_ROOT_CONFIG.to_string()),
        None,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_agent_is_seeded_as_coordinator() {
        use crate::shell::workroom::{AgentRole, Workroom};
        let mut wr = Workroom::new();
        wr.init_from_config(WATCH_ROOT_CONFIG);
        let claude = wr
            .agents_for_test()
            .iter()
            .find(|a| a.name == "claude")
            .expect("synthetic config seeds the root agent");
        assert_eq!(claude.role, AgentRole::Coordinator);
    }

    /// Holds `env_lock()` for the duration of `ARMADAI_SESSION_INDEX`
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
            let lock = armadai_core::test_support::env_lock();
            // SAFETY: modifies the global environment; serialised via `env_lock()`.
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

    /// M3: `errors_when_no_session_found` above uses an EMPTY index, so it only
    /// exercises the early `sessions.is_empty()` bail — `session_index::resolve`'s
    /// no-match-id branch is never actually hit. Use a NON-EMPTY index with a
    /// `--session` id that doesn't match any registered session.
    #[tokio::test]
    async fn errors_when_session_id_not_in_nonempty_index() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("idx.jsonl");
        let _env = SessionIndexEnvGuard::set(&idx);
        crate::claude_adapter::session_index::append(
            &crate::claude_adapter::session_index::SessionRef {
                session_id: "a".into(),
                transcript_path: dir.path().join("a.jsonl"),
                cwd: "/c".into(),
                started_at: "t".into(),
            },
        )
        .unwrap();
        let r = execute(false, Some("zzz".into()), true).await;
        assert!(
            r.is_err(),
            "a non-empty index with an unmatched --session id must still error"
        );
    }

    /// M1: `--last --session X` used to silently ignore `X` and fall back to the
    /// most-recently-registered session (`resolve`'s `last` branch short-circuits
    /// before ever checking `session_id`). An explicit `--session` must win, to the
    /// point of erroring when it doesn't match — even with `--last` also set.
    #[tokio::test]
    async fn explicit_session_wins_over_last_and_errors_when_it_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("idx.jsonl");
        let _env = SessionIndexEnvGuard::set(&idx);
        // Register two sessions; "b" is the most recent (what --last would pick).
        crate::claude_adapter::session_index::append(
            &crate::claude_adapter::session_index::SessionRef {
                session_id: "a".into(),
                transcript_path: dir.path().join("a.jsonl"),
                cwd: "/c".into(),
                started_at: "t1".into(),
            },
        )
        .unwrap();
        crate::claude_adapter::session_index::append(
            &crate::claude_adapter::session_index::SessionRef {
                session_id: "b".into(),
                transcript_path: dir.path().join("b.jsonl"),
                cwd: "/c".into(),
                started_at: "t2".into(),
            },
        )
        .unwrap();
        // Before the fix: last=true short-circuits resolve() and silently returns
        // "b", so this would be Ok. After the fix: the explicit (unmatched)
        // --session must win and this must error.
        let r = execute(true, Some("does-not-exist".into()), true).await;
        assert!(
            r.is_err(),
            "--session must take precedence over --last, even to the point of \
             erroring when it doesn't match"
        );
    }
}
