//! Black-box regression for the observed-usage audit pass: the compiled binary
//! must discover a transcript directory, aggregate it, and report U01/U02.
//!
//! Spawns the real binary (like `hook_stdout.rs`) because the pass is wired in
//! `cli::audit::execute` — the discovery + scan + rules + rendering chain is
//! only exercised end to end through `main()`.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// A project declaring one agent that never ran, plus a transcript in which
    /// Claude Code's built-in `general-purpose` did all the work.
    fn scenario() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        let agents = project.join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("ghost.md"),
            "---\nname: ghost\ndescription: never invoked\n---\nBody",
        )
        .unwrap();

        // The transcript lives in a directory whose name does NOT follow the
        // slug rule, so this also covers the cwd-based fallback from Task 3.
        let projects = dir.path().join("claude-projects");
        let session_dir = projects.join("unexpected-name");
        std::fs::create_dir_all(&session_dir).unwrap();
        let cwd = project.to_string_lossy().to_string();
        let lines = [
            format!(
                r#"{{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","cwd":"{cwd}","message":{{"model":"claude-opus-5","content":[{{"type":"tool_use","id":"t1","name":"Agent","input":{{"subagent_type":"general-purpose","description":"do work"}}}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","timestamp":"2026-08-02T00:00:00Z","isSidechain":false,"uuid":"u2","cwd":"{cwd}","attributionSkill":"armadai","message":{{"model":"claude-opus-5","content":[{{"type":"tool_use","id":"t2","name":"Bash","input":{{}}}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
            ),
        ];
        std::fs::write(session_dir.join("s1.jsonl"), lines.join("\n") + "\n").unwrap();

        (dir, project, projects)
    }

    #[test]
    fn audit_reports_observed_usage_and_usage_findings() {
        let (_dir, project, projects) = scenario();

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("audit")
            .arg(&project)
            .env("ARMADAI_CLAUDE_PROJECTS_DIR", &projects)
            .env("NO_COLOR", "1");
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("U01") && stdout.contains("ghost"),
            "a declared-but-unused agent must be flagged:\n{stdout}"
        );
        assert!(
            stdout.contains("U02") && stdout.contains("general-purpose"),
            "the built-in worker must be reported as undeclared:\n{stdout}"
        );
        assert!(
            stdout.contains("2026-08-01T00:00:00Z"),
            "the observed window must be stated:\n{stdout}"
        );
    }

    #[test]
    fn audit_without_any_transcript_still_succeeds_and_claims_nothing() {
        let (dir, project, _projects) = scenario();
        let empty = dir.path().join("no-transcripts-here");
        std::fs::create_dir_all(&empty).unwrap();

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("audit")
            .arg(&project)
            .env("ARMADAI_CLAUDE_PROJECTS_DIR", &empty)
            .env("NO_COLOR", "1");
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success(), "audit must not fail: {stdout}");
        assert!(
            !stdout.contains("U01") && !stdout.contains("U02"),
            "with nothing observed, no usage claim may be made:\n{stdout}"
        );
    }
}
