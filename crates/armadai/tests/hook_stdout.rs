//! Stdout contract of the Claude Code `PreToolUse` policy gate: Claude Code
//! interprets hook stdout, so it must carry the decision JSON and nothing else.
//! A tracing line on stdout would corrupt the verdict; the global
//! `tracing-subscriber` fmt layer is routed to stderr in `src/main.rs`.
//!
//! These spawn the real compiled binary (via `assert_cmd`), since the bug lives in
//! the global subscriber wired up in `main()` — not reachable from a plain unit test
//! in the same process.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// The policy gate stdout contract: Claude Code parses the hook's stdout, so it must
    /// carry the decision JSON and nothing else. A stray `println!` or a tracing line on stdout would
    /// corrupt the verdict — and a warning about an unparsable config is
    /// exactly the kind of thing that ends up there by accident.
    fn strict_project(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join(".armadai/agents")).unwrap();
        std::fs::write(
            dir.join(".armadai/project.yaml"),
            "schema_version: 1\ncoordinator: dev-lead\npolicy:\n  delegation: strict\n",
        )
        .unwrap();
        for a in ["dev-lead", "qa-specialist"] {
            std::fs::write(
                dir.join(format!(".armadai/agents/{a}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
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
    fn a_legacy_project_warns_on_stderr_and_leaves_stdout_clean() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(dir.path().join(".armadai/config.yaml"), "agents: []\n").unwrap();
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .arg("__claude-policy-gate")
            .write_stdin(gate_payload(dir.path(), "qa-specialist"))
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("legacy format, run armadai migrate; gate inactive"),
            "{stderr}"
        );
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
    fn an_invalid_project_warns_on_stderr_and_leaves_stdout_clean() {
        let dir = tempfile::tempdir().unwrap();
        strict_project(dir.path());
        // `Strict` capitalised: a realistic typo in `policy.delegation`.
        std::fs::write(
            dir.path().join(".armadai/project.yaml"),
            "schema_version: 1\npolicy:\n  delegation: Strict\n",
        )
        .unwrap();

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("__claude-policy-gate")
            .write_stdin(gate_payload(dir.path(), "qa-specialist"));
        let out = cmd.output().unwrap();

        assert!(out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stdout).trim().is_empty(),
            "a project we could not read is not a verdict"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("gate is inactive"),
            "the typo must be reported somewhere, or the gate disappears in \
             silence:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
