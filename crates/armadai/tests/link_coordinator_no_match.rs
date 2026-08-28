//! Issue #371: a `link.coordinator` that matches no agent is a no-op that
//! is **silent on both sides** — `link` simply writes no root instructions
//! file, `unlink` looks for none, and `validate` never checked the key at
//! all. Nothing is left on disk (the defect is symmetric, unlike #341), so
//! the only observable is the message that was never printed.
//!
//! Spawns the real binary, like `link_list_gate.rs`, because what is under
//! test is exactly what reaches a user's terminal.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// Isolated per test directory — see `link_list_gate.rs`'s own note:
    /// without it, `link`'s `project_registry` write lands in the
    /// developer's real `~/.config/armadai/` and the shadowing check
    /// scans their real global agent library.
    fn isolated_config(dir: &std::path::Path) -> std::path::PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        config
    }

    /// A two-agent project whose `link.coordinator` is spelled
    /// `coordinator`.
    ///
    /// The roster is deliberately built so that a matching reference
    /// resolves the **second** agent, not the first: a fixture with the
    /// coordinator at position 0 stays green under an always-true match
    /// predicate (the trap measured on #370), and the negative control
    /// below would then prove nothing.
    ///
    /// The two H1 titles (`Worker`, `Dev Lead`) are also deliberately
    /// different from their `agents:` keys in exactly one case: `dev-lead`
    /// is the key, `Dev Lead` the title. That is the namespace split
    /// `docs/wiki/link.md` documents, and the reason the warning must send
    /// the user to the title rather than to the key.
    fn project_with(coordinator: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            format!(
                "agents:\n  - name: worker\n  - name: dev-lead\nlink:\n  target: claude\n  \
                 coordinator: {coordinator}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("agents/worker.md"),
            "# Worker\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nDo the work.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("agents/dev-lead.md"),
            "# Dev Lead\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nLead.\n",
        )
        .unwrap();
        dir
    }

    /// The message's own words, asserted verbatim rather than by prefix —
    /// the lesson `link_list_gate.rs` records from a rustfmt-collapsed
    /// continuation that shipped 14 stray spaces mid-sentence past a
    /// prefix-only check.
    const HEADLINE: &str = "link.coordinator 'dev-led' matches no agent";
    const NAMESPACE_HINT: &str = "It is matched against an agent's H1 title, or that title's slug — not the `agents:` \
         key, which is a separate namespace.";
    const ROSTER_HINT: &str = "Titles in this roster: Worker, Dev Lead.";

    fn assert_says_it_all(stream: &str, who: &str) {
        for expected in [HEADLINE, NAMESPACE_HINT, ROSTER_HINT] {
            assert!(
                stream.contains(expected),
                "{who} must report the unmatched coordinator, verbatim.\n\
                 expected: {expected}\nin: {stream}"
            );
        }
    }

    /// `link` writes the per-agent files and announces success; the only
    /// thing that can tell the user their coordinator did not resolve is
    /// this warning. It stays a warning, not a refusal: the link itself is
    /// valid, it is the configuration that is not what the user meant.
    ///
    /// Mutation this catches: dropping the warning branch from
    /// `linker::take_coordinator` (returning `(None, None)` on no match,
    /// the pre-fix behaviour) leaves stderr silent and every assertion in
    /// `assert_says_it_all` fails.
    #[test]
    fn link_reports_a_coordinator_that_matches_no_agent() {
        let dir = project_with("dev-led");
        let root = dir.path().join("project");

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .args(["link", "--target", "claude"]);
        let output = cmd.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "an unmatched coordinator is a warning, not a refusal: {stderr}"
        );
        assert_says_it_all(&stderr, "link");
    }

    /// The other half of the pair. Warning only in `link` would recreate
    /// the very asymmetry #341/#370 closed, so `unlink` must say the same
    /// thing — on the path where it actually resolves the reference
    /// against a roster, i.e. the manifest-less fallback.
    ///
    /// `.armadai/` is removed between the two halves for exactly that
    /// reason: with the manifest present, `unlink` reclaims by record and
    /// never consults `link.coordinator` at all, so a test that skipped
    /// the removal would prove nothing (the trap recorded in
    /// `tests/unlink_content_guard.rs`).
    ///
    /// Mutation this catches: the same dropped warning branch; also,
    /// wiring the warning into `cli::link` only leaves this test red.
    #[test]
    fn unlink_reports_a_coordinator_that_matches_no_agent() {
        let dir = project_with("dev-led");
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .args(["link", "--target", "claude"])
            .output()
            .unwrap();

        // Force the fallback path — the only one that resolves the
        // configured coordinator against the roster.
        std::fs::remove_dir_all(root.join(".armadai")).unwrap();

        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .args(["unlink", "--target", "claude"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "an unmatched coordinator is a warning, not a refusal: {stderr}"
        );
        assert_says_it_all(&stderr, "unlink");
    }

    /// The error is one of configuration, not of execution, so `validate`
    /// — the command whose whole job is to find those — must report it
    /// too. As a warning: unlike `orchestration.coordinator` (checked
    /// against the `agents:` keys, a pure config lookup), this one needs
    /// the roster's H1 titles, which means loading every agent file. That
    /// makes the check dependent on what actually resolves on this
    /// machine, and failing a build on a check that can degrade is a
    /// worse trade than reporting it.
    ///
    /// Mutation this catches: removing the R7 block from
    /// `validate_project_config` returns `0 error(s), 0 warning(s)` — the
    /// pre-fix output — and both assertions fail.
    #[test]
    fn validate_reports_a_coordinator_that_matches_no_agent() {
        let dir = project_with("dev-led");
        let root = dir.path().join("project");

        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .arg("validate")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            output.status.success(),
            "an unmatched coordinator is a warning, so validate still passes: {stdout}"
        );
        assert!(
            stdout.contains("0 error(s), 1 warning(s)"),
            "validate must count it as exactly one warning: {stdout}"
        );
        assert!(
            stdout.contains("link.coordinator"),
            "validate must name the key at fault, not just the value: {stdout}"
        );
        assert_says_it_all(&stdout, "validate");
    }

    /// The negative control, without which every assertion above is also
    /// satisfied by an unconditional warning. A coordinator that *does*
    /// resolve — here by slug, `dev-lead` against the title `Dev Lead`,
    /// and at roster position 1 rather than 0 — must leave all three
    /// commands silent about it.
    ///
    /// Mutation this catches: warning unconditionally (or matching with a
    /// plain `==` on names, which `Dev Lead`/`dev-lead` fails) makes this
    /// test red while the three above stay green.
    #[test]
    fn a_coordinator_that_resolves_is_reported_by_nobody() {
        let dir = project_with("dev-lead");
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let link = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .args(["link", "--target", "claude"])
            .output()
            .unwrap();
        let link_err = String::from_utf8_lossy(&link.stderr);
        assert!(
            root.join(".claude/CLAUDE.md").is_file(),
            "control precondition: the coordinator really did resolve"
        );
        assert!(
            !link_err.contains("matches no agent"),
            "link must stay silent when the coordinator resolves: {link_err}"
        );

        std::fs::remove_dir_all(root.join(".armadai")).unwrap();

        let unlink = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .args(["unlink", "--target", "claude"])
            .output()
            .unwrap();
        let unlink_err = String::from_utf8_lossy(&unlink.stderr);
        assert!(
            !unlink_err.contains("matches no agent"),
            "unlink must stay silent when the coordinator resolves: {unlink_err}"
        );

        let validate = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .arg("validate")
            .output()
            .unwrap();
        let validate_out = String::from_utf8_lossy(&validate.stdout);
        assert!(
            validate_out.contains("0 error(s), 0 warning(s)"),
            "validate must stay silent when the coordinator resolves: {validate_out}"
        );
    }

    /// R7's own guard: when part of the roster failed to load, the
    /// titles `validate` can see are not the titles `link` would see, so
    /// "matches nothing" would be a guess. The check stands down rather
    /// than reporting a coordinator that may well resolve on a machine
    /// where the missing agent is present.
    ///
    /// Here `ghost` names an agent no file backs, so `load_all_agents`
    /// warns and the roster is short one entry — and the coordinator
    /// reference is `ghost` itself, i.e. the case where the incomplete
    /// roster is exactly what would make the check wrong.
    ///
    /// Mutation this catches: dropping the `load_warnings.is_empty()`
    /// guard makes `validate` report `ghost` as matching no agent, and
    /// this test's `0 warning(s)` assertion fails.
    #[test]
    fn validate_stands_down_when_the_roster_did_not_fully_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: worker\n  - name: ghost\nlink:\n  target: claude\n  \
             coordinator: ghost\n",
        )
        .unwrap();
        std::fs::write(
            root.join("agents/worker.md"),
            "# Worker\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nDo the work.\n",
        )
        .unwrap();
        // No agents/ghost.md — the reference does not resolve.

        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .arg("validate")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("0 error(s), 0 warning(s)"),
            "an incomplete roster makes the coordinator check unanswerable, so it must \
             not answer: {stdout}"
        );
    }

    /// A project with no `link.coordinator` at all must not be warned
    /// about either — the setting is optional, and most projects have
    /// none. Separate from the control above because it exercises a
    /// different branch: `None` rather than `Some` that resolves.
    ///
    /// Mutation this catches: warning whenever the roster produced no
    /// coordinator (rather than only when a reference was given and
    /// failed) fires here, where nothing was ever configured.
    #[test]
    fn a_project_with_no_coordinator_is_reported_by_nobody() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: worker\nlink:\n  target: claude\n",
        )
        .unwrap();
        std::fs::write(
            root.join("agents/worker.md"),
            "# Worker\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nDo the work.\n",
        )
        .unwrap();

        let config = isolated_config(dir.path());
        let link = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .args(["link", "--target", "claude"])
            .output()
            .unwrap();
        let link_err = String::from_utf8_lossy(&link.stderr);
        assert!(
            !link_err.contains("matches no agent"),
            "no coordinator configured means nothing to warn about: {link_err}"
        );

        let validate = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", &config)
            .arg("validate")
            .output()
            .unwrap();
        let validate_out = String::from_utf8_lossy(&validate.stdout);
        assert!(
            validate_out.contains("0 error(s), 0 warning(s)"),
            "no coordinator configured means nothing to warn about: {validate_out}"
        );
    }
}
