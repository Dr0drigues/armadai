//! Black-box regressions for the `link`/`list` refusal policy (task 7b
//! review, "Missing coverage"): `link`'s refuse-to-write and `list`'s
//! warn-and-continue had zero automated coverage — a refuse-unconditionally
//! implementation would have passed every existing test. Spawns the real
//! binary (like `audit_usage.rs`) since the policy is wired in
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

    /// Regression guard for Finding 1's own regression: a pre-existing
    /// `.md` failure keeps its EXACT old behaviour through `link` — warn,
    /// link what did load, exit 0. A refuse-to-write policy applied
    /// unconditionally on any warning would fail this test.
    #[test]
    fn link_survives_a_pre_existing_md_failure() {
        let dir = project_with_a_pre_existing_md_failure();
        let root = dir.path().join("project");

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .args(["link", "--target", "claude", "--dry-run"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "a pre-existing .md failure must not refuse the link: stdout={stdout} stderr={stderr}"
        );
        assert!(
            stderr.contains("broken"),
            "must still warn about it: {stderr}"
        );
        assert!(
            stdout.contains("good"),
            "the healthy agent must still be projected: {stdout}"
        );
    }

    /// The actual Finding 1 fix: a declaration this chantier's format
    /// dropped DOES refuse the write.
    #[test]
    fn link_refuses_on_a_dropped_declaration() {
        let dir = project_with_a_dropped_declaration();
        let root = dir.path().join("project");

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .args(["link", "--target", "claude", "--dry-run"]);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !output.status.success(),
            "a dropped declaration must refuse the link: stdout={stdout} stderr={stderr}"
        );
        // Full message, not just a prefix: round 2's edit introduced 14
        // literal stray spaces mid-string ("link              a smaller")
        // via a rustfmt-collapsed backslash-continuation, and a
        // prefix-only `contains("refusing to link")` check was blind to
        // it -- the whole gate passed with the damaged string still
        // reaching a user's terminal. Asserting the exact text is what
        // makes a reflow accident fail the suite instead of shipping.
        assert!(
            stderr.contains(
                "one or more agents could not be loaded (see warning(s) above) — refusing to \
                 link a smaller fleet than declared. Fix the issue(s), or rerun once resolved."
            ),
            "must say why it refused, verbatim: {stderr}"
        );
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

    /// A project with only declared agents — no `agents:` entries at all,
    /// relying entirely on `.armadai/agents.yaml` — the layout the format
    /// exists to enable.
    fn project_all_declared() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai/prompts")).unwrap();
        std::fs::write(root.join("armadai.yaml"), "link:\n  target: claude\n").unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  \
             - name: zzz-declared-one\n    prompt: [base]\n  \
             - name: zzz-declared-two\n    prompt: [base]\n",
        )
        .unwrap();
        std::fs::write(root.join(".armadai/prompts/base.md"), "You are {{name}}.\n").unwrap();
        dir
    }

    /// I2: `unlink.rs` was the fourth copy of the `config.agents.is_empty()`
    /// gate that `link`/`list`/`run` all widened to also check
    /// `.armadai/agents.yaml` — and it was the one copy that got missed.
    /// Before the fix, `unlink --target claude` on this exact project
    /// answered the false "No agents declared in project config." right
    /// after `link` had written its files, and removed nothing.
    #[test]
    fn unlink_removes_what_link_generated_for_a_declarations_only_project() {
        let dir = project_all_declared();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let mut link_cmd = Command::cargo_bin("armadai").unwrap();
        link_cmd
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .args(["link", "--target", "claude"]);
        let link_output = link_cmd.output().unwrap();
        assert!(
            link_output.status.success(),
            "link must succeed for an all-declared project: stdout={} stderr={}",
            String::from_utf8_lossy(&link_output.stdout),
            String::from_utf8_lossy(&link_output.stderr)
        );

        let generated = root.join(".claude/agents/zzz-declared-one.md");
        assert!(
            generated.is_file(),
            "link must have written the declared agent's projection: {}",
            generated.display()
        );

        let mut unlink_cmd = Command::cargo_bin("armadai").unwrap();
        unlink_cmd
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .args(["unlink", "--target", "claude"]);
        let unlink_output = unlink_cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&unlink_output.stdout);
        let stderr = String::from_utf8_lossy(&unlink_output.stderr);

        assert!(
            unlink_output.status.success(),
            "unlink must not refuse an all-declared project: stdout={stdout} stderr={stderr}"
        );
        assert!(
            !stderr.contains("No agents declared in project config"),
            "must not report the false message link's own output already disproves: {stderr}"
        );
        assert!(
            !generated.exists(),
            "unlink must have removed what link generated: {}",
            generated.display()
        );
    }
}
