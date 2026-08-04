//! I2 regression: the Claude Code plugin's `SessionStart` hook contract requires
//! `armadai __claude-register-session` to print **nothing on stdout** (Claude Code
//! interprets hook stdout). `register_from_reader` (see
//! `src/claude_adapter/mod.rs`) calls `tracing::warn!` when the session index write
//! fails — if the global `tracing-subscriber` fmt layer writes to stdout (its
//! default), that warning corrupts the hook's stdout contract and would also
//! corrupt `armadai watch --json`'s RunEvent stream. The fix routes the fmt layer to
//! stderr in `src/main.rs`.
//!
//! These spawn the real compiled binary (via `assert_cmd`), since the bug lives in
//! the global subscriber wired up in `main()` — not reachable from a plain unit test
//! in the same process.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// Force `session_index::append` to fail: point `ARMADAI_SESSION_INDEX` at a
    /// path whose parent directory can never be created because a *file* (not a
    /// directory) sits in the middle of the path. This makes `register_from_reader`
    /// take its `tracing::warn!` branch — the exact path that leaked to stdout
    /// before the fix.
    fn unwritable_index_path(root: &std::path::Path) -> std::path::PathBuf {
        let blocker = root.join("blocker-file");
        std::fs::write(&blocker, b"not a directory").unwrap();
        blocker.join("subdir").join("idx.jsonl")
    }

    #[test]
    fn register_session_write_failure_prints_nothing_on_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let idx = unwritable_index_path(dir.path());

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("__claude-register-session")
            .env("ARMADAI_SESSION_INDEX", &idx)
            .env("RUST_LOG", "armadai=info")
            .write_stdin(r#"{"session_id":"z","transcript_path":"/t/z.jsonl"}"#);

        let output = cmd.output().unwrap();
        assert!(
            output.status.success(),
            "hook must always exit 0 (contract), got {:?}",
            output.status
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.is_empty(),
            "hook stdout must be EMPTY (Claude Code parses it) — got: {stdout:?}"
        );
    }

    #[test]
    fn register_session_normal_payload_prints_nothing_on_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("idx.jsonl");

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("__claude-register-session")
            .env("ARMADAI_SESSION_INDEX", &idx)
            .write_stdin(r#"{"session_id":"","transcript_path":""}"#);

        let output = cmd.output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.is_empty(), "got: {stdout:?}");
    }
}
