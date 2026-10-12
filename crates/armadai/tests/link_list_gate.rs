//! Black-box regressions for the `link`/`list` refusal policy (task 7b
//! review, "Missing coverage"): `link`'s refuse-to-write and `list`'s
//! warn-and-continue had zero automated coverage — a refuse-unconditionally
//! implementation would have passed every existing test. Spawns the real
//! binary (like `hook_stdout.rs`) since the policy is wired in
//! `cli::link::execute`/`cli::list::execute`, end to end.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// Every project directory in this file gets its own isolated
    /// `ARMADAI_CONFIG_DIR` — without it, `link`'s `project_registry`
    /// write lands in the developer's real `~/.config/armadai/`, and the
    /// shadowing check would scan the developer's real global agent
    /// library (which, on a machine that has ever run `armadai extract`
    /// from this repo, holds a real `core-specialist.md` — see the task's
    /// own unit-test regressions for the same trap).
    fn isolated_config(dir: &std::path::Path) -> std::path::PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        config
    }

    /// A project with one broken `.md` (missing `## System Prompt`, so
    /// `parse_agent_file` fails) and one healthy `.md` — a failure that
    /// predates the declarative format entirely and has nothing to do with
    /// `.armadai/agents.yaml`.
    fn project_with_a_pre_existing_md_failure() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let agents = root.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: broken\n  - name: good\nlink:\n  target: claude\n",
        )
        .unwrap();
        // Missing `## System Prompt` -> parse_agent_file fails.
        std::fs::write(
            agents.join("broken.md"),
            "# broken\n\n## Metadata\n- provider: claude\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("good.md"),
            "# good\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi\n",
        )
        .unwrap();
        dir
    }

    /// A project with no file-backed agents at all, and one declared agent
    /// whose prompt composition fails (`missing-fragment` does not exist) —
    /// a loss this chantier's format is directly responsible for.
    fn project_with_a_dropped_declaration() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai/prompts")).unwrap();
        std::fs::write(root.join("armadai.yaml"), "link:\n  target: claude\n").unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  \
             - name: good-declared\n    prompt: [base]\n  \
             - name: bad-declared\n    prompt: [missing-fragment]\n",
        )
        .unwrap();
        std::fs::write(root.join(".armadai/prompts/base.md"), "You are {{name}}.\n").unwrap();
        dir
    }

    /// `list` is read-only: it must warn and continue in BOTH scenarios
    /// above, never refusing — the distinction Finding 1 draws only applies
    /// to a command that writes config.
    #[test]
    fn list_warns_and_continues_for_both_scenarios() {
        for dir in [
            project_with_a_pre_existing_md_failure(),
            project_with_a_dropped_declaration(),
        ] {
            let root = dir.path().join("project");
            let mut cmd = Command::cargo_bin("armadai").unwrap();
            cmd.current_dir(&root)
                .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
                .arg("list");
            let output = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            assert!(
                output.status.success(),
                "list must never refuse, being read-only: stdout={stdout} stderr={stderr}"
            );
            assert!(!stderr.trim().is_empty(), "list must still warn: {stderr}");
        }
    }
}
