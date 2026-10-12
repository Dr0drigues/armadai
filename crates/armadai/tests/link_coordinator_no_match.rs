//! Issue #371: a `link.coordinator` that matches no agent is a no-op that
//! is **silent on both sides** — `link` simply writes no root instructions
//! file, `unlink` looks for none, and `validate` never checked the key at
//! all. Nothing is left on disk (the defect is symmetric, unlike #341), so
//! the only observable is the message that was never printed.
//!
//! And its converse, measured on this branch's own first draft: the report
//! must be a statement about the **configuration**, not a side effect of
//! whichever roster a command happens to hold. Computed from the resolver,
//! it fired on `link --agents Worker` for a `coordinator: dev-lead` that
//! resolves perfectly, on the very project where `validate` reported
//! nothing.
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

    /// The same message as `validate` renders it: its own
    /// `WARN  <file>:<key>: ` header and its own two-space continuation.
    ///
    /// The location is asserted here rather than through a bare
    /// `contains("link.coordinator")`, which the *body* satisfies on its
    /// own — the body opens with those very words. Measured: removing
    /// `LINK_COORDINATOR_KEY` from R7's reported location left all 28 test
    /// targets green, so the `armadai.yaml:link.coordinator:` prefix the
    /// PR puts in its shop window was pinned by nothing.
    const VALIDATE_WARNING: &str = concat!(
        "WARN  armadai.yaml:link.coordinator: link.coordinator 'dev-led' matches no agent — ",
        "no root instructions file (.claude/CLAUDE.md and its equivalents) is written or ",
        "removed for it.\n",
        "  It is matched against an agent's H1 title, or that title's slug — not the ",
        "`agents:` key, which is a separate namespace.\n",
        "  Titles in this roster: Worker, Dev Lead.",
    );

    fn assert_says_it_all(stream: &str, expected: &str, who: &str) {
        assert!(
            stream.contains(expected),
            "{who} must report the unmatched coordinator, verbatim.\n\
             expected:\n{expected}\n\nin:\n{stream}"
        );
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
    /// pre-fix output — and both assertions fail. Removing only
    /// `LINK_COORDINATOR_KEY` from the reported location fails the
    /// verbatim assertion alone.
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
        assert_says_it_all(&stdout, VALIDATE_WARNING, "validate");
    }

    /// R7's own guard: when part of the roster failed to load, the titles
    /// `validate` can see are not the titles `link` would see, so "matches
    /// nothing" would be a guess. The check stands down rather than
    /// reporting a coordinator that may well resolve on a machine where
    /// the missing agent is present.
    ///
    /// It stands down **out loud**. The guard used to be silent, justified
    /// in a comment by "those failures are already reported on their own
    /// terms elsewhere" — measured false inside `validate`, which has no
    /// rule that reports an unresolvable agent ref at all: on this exact
    /// fixture `validate` answered `0 error(s), 0 warning(s)` /
    /// "Validation passed" while `link` printed two warnings. A green
    /// report over two real defects is worse than either of them.
    ///
    /// Here `ghost` names an agent no file backs, so `load_all_agents`
    /// warns and the roster is short one entry — and the coordinator
    /// reference is `ghost` itself, i.e. the case where the incomplete
    /// roster is exactly what would make the check wrong.
    ///
    /// Mutation this catches: dropping the `load_warnings.is_empty()`
    /// guard makes `validate` report `ghost` as matching no agent, failing
    /// the first two assertions; restoring the silent stand-down (pushing
    /// no issue in the `else` arm) fails all three.
    #[test]
    fn validate_stands_down_out_loud_when_the_roster_did_not_fully_load() {
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

        // Not a bare `!contains("matches no agent")`: the stand-down text
        // quotes that very phrase to explain what it is declining to say.
        // What must be absent is the *claim*, which always names the key
        // and the value.
        assert!(
            !stdout.contains("link.coordinator 'ghost' matches no agent"),
            "an incomplete roster makes the coordinator check unanswerable, so it must not \
             answer it: {stdout}"
        );
        assert!(
            stdout.contains("0 error(s), 1 warning(s)"),
            "standing down is itself reported, as exactly one warning: {stdout}"
        );
        assert!(
            stdout.contains(concat!(
                "WARN  armadai.yaml:link.coordinator: link.coordinator 'ghost' was not ",
                "checked: part of the roster failed to load",
            )),
            "the user must be told the check abstained, and on which key: {stdout}"
        );
        assert!(
            stdout.contains("- Agent 'ghost' not found in"),
            "and told what stopped it — `validate` has no other rule that would say so: \
             {stdout}"
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

    /// A project that declares no agent at all and still configures a
    /// coordinator: the configuration cannot be satisfied by anything, and
    /// `validate` is the only surface that can say so — `link` refuses
    /// such a project outright ("No agents declared in project config")
    /// before ever looking at the key.
    ///
    /// This is the `(none)` arm of the report, which R7's earlier
    /// `!roster.is_empty()` guard made unreachable from every caller.
    ///
    /// Mutation this catches: reinstating that guard returns
    /// `0 error(s), 0 warning(s)` and both assertions fail.
    #[test]
    fn validate_reports_a_coordinator_on_a_project_that_declares_no_agent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents: []\nlink:\n  target: claude\n  coordinator: dev-lead\n",
        )
        .unwrap();

        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .arg("validate")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("0 error(s), 1 warning(s)"),
            "a coordinator no agent can satisfy is still a broken configuration: {stdout}"
        );
        assert!(
            stdout.contains("Titles in this roster: (none)."),
            "and the roster it was matched against is empty, which the report must say: \
             {stdout}"
        );
    }
}
