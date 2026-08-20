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
    /// The policy gate shares `__claude-register-session`'s stdout contract:
    /// Claude Code parses the hook's stdout, so it must carry the decision JSON
    /// and nothing else. A stray `println!` or a tracing line on stdout would
    /// corrupt the verdict — and a warning about an unparsable config is
    /// exactly the kind of thing that ends up there by accident.
    fn strict_project(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join(".armadai")).unwrap();
        std::fs::write(
            dir.join(".armadai/config.yaml"),
            "orchestration:\n  policy: strict\n  coordinator: dev-lead\n  \
             teams:\n    - agents: [qa-specialist]\n",
        )
        .unwrap();
    }

    fn gate_payload(dir: &std::path::Path, target: &str) -> String {
        format!(
            r#"{{"cwd":"{}","tool_name":"Agent","agent_type":"","tool_input":{{"subagent_type":"{target}"}}}}"#,
            dir.to_string_lossy()
        )
    }

    #[test]
    fn policy_gate_writes_only_the_decision_json_on_stdout() {
        let dir = tempfile::tempdir().unwrap();
        strict_project(dir.path());

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("__claude-policy-gate")
            .env("RUST_LOG", "armadai=debug,armadai_core=debug")
            .write_stdin(gate_payload(dir.path(), "qa-specialist"));
        let out = cmd.output().unwrap();

        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "the hook must never fail: {stdout}");
        // Exactly one line, and it must parse as the decision object.
        assert_eq!(
            stdout.trim().lines().count(),
            1,
            "stdout must carry one JSON document and nothing else:\n{stdout}"
        );
        let v: serde_json::Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    #[test]
    fn policy_gate_stdout_stays_empty_when_it_has_no_opinion() {
        let dir = tempfile::tempdir().unwrap();
        // No config at all: nothing to enforce, so nothing to say.
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("__claude-policy-gate")
            .env("RUST_LOG", "armadai=debug")
            .write_stdin(gate_payload(dir.path(), "qa-specialist"));
        let out = cmd.output().unwrap();
        assert!(out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stdout).trim().is_empty(),
            "silence is how the gate says \"no opinion\"; anything on stdout \
             would be read as a verdict"
        );
    }

    #[test]
    fn an_unparsable_config_warns_on_stderr_and_leaves_stdout_clean() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        // `Off` capitalised: a realistic typo in `policy:`.
        std::fs::write(
            dir.path().join(".armadai/config.yaml"),
            "orchestration:\n  policy: Off\n  coordinator: dev-lead\n",
        )
        .unwrap();

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("__claude-policy-gate")
            .env("RUST_LOG", "armadai_core=warn")
            .write_stdin(gate_payload(dir.path(), "qa-specialist"));
        let out = cmd.output().unwrap();

        assert!(
            String::from_utf8_lossy(&out.stdout).trim().is_empty(),
            "a config we could not read is not a verdict"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("unparsable"),
            "the typo must be reported somewhere, or the gate disappears in \
             silence:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
