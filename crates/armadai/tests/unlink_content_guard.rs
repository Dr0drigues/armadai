//! Black-box regressions for issue #338's mitigation half: `unlink` deleted
//! files `link` never wrote — including user content unrelated to armadai.
//! Spawns the real binary (like `link_list_gate.rs`) since the guard lives
//! end to end in `cli::unlink::execute`.
//!
//! Each test below reproduces one of the four cases confirmed by the
//! project's spike (see issue #338 body + its approved-design comment) and
//! is written to FAIL against the pre-fix `unlink` (unconditional
//! `remove_file` on every path `linker.generate()` still names today, plus
//! an unbounded `remove_empty_ancestors` sweep). After the content-match
//! guard + source-scoped skill sweep + bounded cascade land, all three pass.
//!
//! Issue #338's second half (the link manifest, `linker::manifest`) landed
//! after these were written. `link` now always writes a manifest, so every
//! test below actually runs through the **manifest** path, not this file's
//! own fallback — `link_manifest.rs` is the dedicated manifest-path suite.
//! What stays true here regardless: the guard this file is named for is
//! still live code (`cli::unlink::unlink_via_fallback`), reached whenever
//! there is no usable manifest. The last test below is this file's own
//! proof of that — the one case here that forces the fallback on purpose.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// Isolated `ARMADAI_CONFIG_DIR` per project — see `link_list_gate.rs`
    /// for why this matters (a real global agent library on this machine
    /// would otherwise leak into the shadowing check and the project
    /// registry write would hit the developer's real `~/.config/armadai/`).
    fn isolated_config(dir: &std::path::Path) -> std::path::PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        config
    }

    fn run_armadai(
        root: &std::path::Path,
        config: &std::path::Path,
        args: &[&str],
    ) -> (bool, String, String) {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(root)
            .env("ARMADAI_CONFIG_DIR", config)
            .args(args);
        let output = cmd.output().unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// A project with a coordinator ("coord") plus one regular member
    /// ("member"), targeting `claude` — the shape needed to make `link`
    /// generate `.claude/CLAUDE.md` (only emitted when a coordinator is
    /// configured, per `linker::claude::generate_claude_md`) alongside a
    /// plain per-agent file (`.claude/agents/member.md`).
    fn project_with_coordinator() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let agents = root.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: coord\n  - name: member\nlink:\n  target: claude\n  coordinator: coord\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("coord.md"),
            "# coord\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou coordinate.\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("member.md"),
            "# member\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou work.\n",
        )
        .unwrap();
        dir
    }

    /// Case 1 (issue #338): a hand-written `.claude/CLAUDE.md` predates
    /// `link` (which skips it, having no `--force`) and must survive
    /// `unlink` untouched — while the file `link` actually wrote
    /// (`.claude/agents/member.md`) must still be reclaimed. Both
    /// assertions live in one test on purpose: a fix that simply stops
    /// deleting everything would pass the first half and fail the second;
    /// today's code fails the first half by deleting real user content.
    #[test]
    fn unlink_keeps_a_hand_written_file_but_removes_what_it_generated() {
        let dir = project_with_coordinator();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        // Pre-existing, hand-written — NOT what `generate_claude_md` would
        // produce for this roster.
        let claude_md_dir = root.join(".claude");
        std::fs::create_dir_all(&claude_md_dir).unwrap();
        let hand_written = "# My own notes\n\nDo not touch this file, it is mine.\n";
        std::fs::write(claude_md_dir.join("CLAUDE.md"), hand_written).unwrap();

        let (link_ok, link_stdout, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(
            link_ok,
            "link must succeed even though CLAUDE.md pre-exists: stdout={link_stdout} stderr={link_stderr}"
        );
        assert!(
            link_stderr.contains("CLAUDE.md"),
            "link must have skipped (not overwritten) the pre-existing file: {link_stderr}"
        );
        // Sanity: link left the hand-written content untouched.
        assert_eq!(
            std::fs::read_to_string(claude_md_dir.join("CLAUDE.md")).unwrap(),
            hand_written
        );
        let member_file = root.join(".claude/agents/member.md");
        assert!(
            member_file.is_file(),
            "link must have generated the member's file: {}",
            member_file.display()
        );

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            unlink_ok,
            "unlink must succeed: stdout={unlink_stdout} stderr={unlink_stderr}"
        );

        // The core assertion: the hand-written file survives, byte for byte.
        let claude_md_path = claude_md_dir.join("CLAUDE.md");
        assert!(
            claude_md_path.exists(),
            "a hand-written CLAUDE.md must never be deleted by unlink"
        );
        assert_eq!(
            std::fs::read_to_string(&claude_md_path).unwrap(),
            hand_written,
            "the surviving file's content must be exactly what the user wrote"
        );
        // The user must be told why it was kept.
        assert!(
            unlink_stdout.contains("CLAUDE.md") || unlink_stderr.contains("CLAUDE.md"),
            "unlink must report the kept file to the user: stdout={unlink_stdout} stderr={unlink_stderr}"
        );

        // The control: a genuinely generated, untouched file is still
        // reclaimed — proves the guard doesn't degrade into "keep
        // everything".
        assert!(
            !member_file.exists(),
            "a generated, unmodified file must still be deleted by unlink"
        );
    }

    /// A project declaring one skill (`.armadai/skills/notes/`) alongside a
    /// single file-backed agent, targeting `claude`.
    fn project_with_a_skill() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let agents = root.join("agents");
        let skill_dir = root.join(".armadai/skills/notes");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: solo\nskills:\n  - name: notes\nlink:\n  target: claude\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("solo.md"),
            "# solo\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou work alone.\n",
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: notes\ndescription: take notes\n---\n\nTake notes.\n",
        )
        .unwrap();
        dir
    }

    /// Case 3 (issue #338), the worst measured outcome: a file the user
    /// drops into a linked skill's destination directory after `link` ran
    /// — with no relation to armadai whatsoever — must survive `unlink`.
    /// The skill's own copied file must still be reclaimed (control, same
    /// reasoning as the CLAUDE.md test above).
    #[test]
    fn unlink_keeps_user_files_inside_a_linked_skill_directory() {
        let dir = project_with_a_skill();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, link_stdout, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(
            link_ok,
            "link must succeed: stdout={link_stdout} stderr={link_stderr}"
        );

        let skill_dest = root.join(".claude/skills/notes");
        let copied_skill_md = skill_dest.join("SKILL.md");
        assert!(
            copied_skill_md.is_file(),
            "link must have copied the skill: {}",
            copied_skill_md.display()
        );

        // The user drops an unrelated file into the linked skill directory
        // AFTER linking — e.g. their own scratch notes sitting right next
        // to the copied SKILL.md.
        let user_file = skill_dest.join("my-own-scratchpad.txt");
        let user_content = "totally unrelated to armadai\n";
        std::fs::write(&user_file, user_content).unwrap();

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            unlink_ok,
            "unlink must succeed: stdout={unlink_stdout} stderr={unlink_stderr}"
        );

        assert!(
            user_file.exists(),
            "a user file with no source-side counterpart must survive unlink"
        );
        assert_eq!(
            std::fs::read_to_string(&user_file).unwrap(),
            user_content,
            "the surviving file's content must be untouched"
        );

        // Control: the skill file `link` actually copied, unmodified since,
        // must still be reclaimed.
        assert!(
            !copied_skill_md.exists(),
            "the skill file link copied verbatim must still be deleted by unlink"
        );
    }

    /// A minimal project: one file-backed agent, no coordinator, targeting
    /// `claude` — `link` writes exactly one file, `.claude/agents/solo.md`.
    fn project_minimal() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let agents = root.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: solo\nlink:\n  target: claude\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("solo.md"),
            "# solo\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou work alone.\n",
        )
        .unwrap();
        dir
    }

    /// Case 1's second half (issue #338): once every generated file under
    /// `.claude/` is legitimately reclaimed, the now-empty ancestor
    /// directories get cleaned up — but the cascade must stop at the
    /// target's own root directory. `.claude/` itself must never be
    /// removed, even though nothing armadai-related is left inside it.
    #[test]
    fn unlink_never_removes_the_target_root_directory() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let claude_dir = root.join(".claude");
        assert!(
            claude_dir.is_dir(),
            "link must have created .claude/: {}",
            claude_dir.display()
        );
        let agent_file = claude_dir.join("agents/solo.md");
        assert!(
            agent_file.is_file(),
            "link must have written the agent file"
        );

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !agent_file.exists(),
            "the generated agent file must be reclaimed"
        );
        assert!(
            !claude_dir.join("agents").exists(),
            "the now-empty agents/ subdirectory must be cleaned up"
        );
        assert!(
            claude_dir.is_dir(),
            "the target root directory .claude/ must never be removed by unlink, \
             even when it ends up empty"
        );
    }

    /// A project using `.armadai/config.yaml` (the current format) with a
    /// declared agent, a prompt fragment it composes from, and the
    /// declarations file itself — three siblings under `.armadai/` besides
    /// the config file, none of which `--with-config` should ever touch.
    fn project_declared_with_extra_dotarmadai_files() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai/prompts")).unwrap();
        std::fs::write(
            root.join(".armadai/config.yaml"),
            "link:\n  target: claude\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  - name: solo\n    prompt: [base]\n",
        )
        .unwrap();
        std::fs::write(root.join(".armadai/prompts/base.md"), "You are {{name}}.\n").unwrap();
        dir
    }

    /// F5: `--with-config` is the one deletion path left intentionally
    /// unguarded — there is nothing generated to diff the project's own
    /// config file against, so the flag itself is the confirmation. That
    /// makes it the one place a careless edit (e.g. widening `with_config`
    /// into removing the whole `.armadai/` directory, or all three
    /// candidate config paths instead of only the active one) would cause
    /// real damage with no content guard to catch it. Pin the exact blast
    /// radius: the active config file, and nothing else under `.armadai/`.
    #[test]
    fn unlink_with_config_removes_exactly_the_active_config_file() {
        let dir = project_declared_with_extra_dotarmadai_files();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let config_file = root.join(".armadai/config.yaml");
        let agents_decl = root.join(".armadai/agents.yaml");
        let prompt_fragment = root.join(".armadai/prompts/base.md");
        assert!(config_file.is_file(), "sanity: config file must exist");
        assert!(agents_decl.is_file(), "sanity: declarations must exist");
        assert!(
            prompt_fragment.is_file(),
            "sanity: prompt fragment must exist"
        );

        let (unlink_ok, _, unlink_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--with-config"],
        );
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !config_file.exists(),
            "--with-config must remove the active project config file"
        );
        assert!(
            agents_decl.exists(),
            "--with-config must not remove sibling .armadai/ files — its blast \
             radius must stay pinned to exactly the config file, since nothing \
             guards it"
        );
        assert!(
            prompt_fragment.exists(),
            "--with-config must not remove sibling .armadai/ files"
        );
    }

    /// This file's own proof that the #342 guard it is named for is still
    /// reachable: delete the manifest `link` just wrote before running
    /// `unlink`, forcing the fallback this file originally guarded end to
    /// end — the same hand-written-file-survives / generated-file-removed
    /// pair as the first test above, but via `unlink_via_fallback` instead
    /// of `unlink_from_manifest`.
    ///
    /// Mutation this catches: if the fallback branch were ever deleted or
    /// short-circuited (e.g. "no manifest" silently treated as "nothing to
    /// remove"), a project with no manifest — a fresh clone, or one linked
    /// before #338's second half — would either refuse to clean up
    /// anything or, worse, go back to deleting unconditionally.
    #[test]
    fn unlink_falls_back_to_the_342_guard_when_the_manifest_is_absent() {
        let dir = project_with_coordinator();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let claude_md_dir = root.join(".claude");
        std::fs::create_dir_all(&claude_md_dir).unwrap();
        let hand_written = "# My own notes\n\nDo not touch this file, it is mine.\n";
        std::fs::write(claude_md_dir.join("CLAUDE.md"), hand_written).unwrap();

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let member_file = root.join(".claude/agents/member.md");
        assert!(member_file.is_file());

        // Force the fallback: no manifest, exactly like a fresh clone or a
        // deleted `.armadai/`.
        std::fs::remove_file(root.join(".armadai/link-manifest.yaml"))
            .expect("link must have written a manifest to delete");

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            unlink_stderr.contains("falling back"),
            "unlink must announce the degraded mode: stderr={unlink_stderr}"
        );
        let claude_md_path = claude_md_dir.join("CLAUDE.md");
        assert!(
            claude_md_path.exists(),
            "the fallback's own content-match guard must still keep the hand-written file"
        );
        assert_eq!(
            std::fs::read_to_string(&claude_md_path).unwrap(),
            hand_written
        );
        assert!(
            !member_file.exists(),
            "the fallback must still reclaim a genuinely generated, unmodified file"
        );
    }

    /// Every file (not directory) under `dir`, recursively, sorted — empty
    /// when `dir` itself does not exist. Lets a round-trip test assert on
    /// what actually survives rather than on one path it remembered to
    /// name.
    fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files_under(&path));
            } else {
                found.push(path);
            }
        }
        found.sort();
        found
    }

    /// Same shape as `project_with_coordinator`, but the coordinator is
    /// referenced by its **slug** (`dev-lead`) while the agent's own name
    /// — its H1 title — is written in plain words (`Dev Lead`). Naming
    /// agents in title case and referencing the coordinator in kebab-case
    /// is exactly the combination `slugify` exists to reconcile.
    fn project_with_slug_referenced_coordinator() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let agents = root.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            // `member` FIRST, deliberately: with the coordinator at index 0 a
            // predicate hardcoded to `true` still selects it, so every assertion
            // below stays green and the match itself is never observed
            // (measured, #370 review). Listing it second makes a wrong match
            // land another agent's prompt in the root context file.
            "agents:\n  - name: member\n  - name: dev-lead\nlink:\n  target: claude\n  coordinator: dev-lead\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("dev-lead.md"),
            "# Dev Lead\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou coordinate.\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("member.md"),
            "# member\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou work.\n",
        )
        .unwrap();
        dir
    }

    /// Issue #341: `link` matched the configured coordinator against an
    /// agent's name **or** its slug, the fallback path of `unlink` against
    /// its name alone. With `coordinator: dev-lead` and an agent named
    /// `Dev Lead`, `link` recognised the coordinator and wrote
    /// `.claude/CLAUDE.md`; `unlink` did not, so that file was never even a
    /// candidate for removal — it stayed on disk, silently, with no message
    /// naming it.
    ///
    /// Deliberately forces the #342 fallback (the manifest is deleted
    /// before `unlink` runs) because that is where the divergence lives:
    /// through the manifest, `link`'s own attribution is what `unlink`
    /// reads back, so the two cannot disagree. Without this the test would
    /// take the manifest path and prove nothing.
    #[test]
    fn unlink_fallback_matches_the_coordinator_by_slug_like_link_does() {
        let dir = project_with_slug_referenced_coordinator();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let claude_md = root.join(".claude/CLAUDE.md");
        let member_file = root.join(".claude/agents/member.md");
        assert!(
            claude_md.is_file(),
            "sanity: link resolves `coordinator: dev-lead` to the agent named \
             `Dev Lead` via its slug, so it must have written the root context file"
        );
        // Existence alone proves less than it reads: this fixture lists the
        // coordinator first, so `position()` returns 0 for ANY predicate — a
        // `name_matches_reference` hardcoded to `true` left the assertion above
        // green (measured, #370 review). Asserting *whose* prompt landed there
        // is what makes the match itself observable.
        assert!(
            std::fs::read_to_string(&claude_md)
                .unwrap()
                .contains("You coordinate."),
            "the root context file must carry the *coordinator's* prompt, not another agent's"
        );
        assert!(
            member_file.is_file(),
            "sanity: link must have generated the non-coordinator member's file"
        );
        assert!(
            !root.join(".claude/agents/dev-lead.md").exists(),
            "sanity: the coordinator is excluded from the sub-agent files"
        );

        // Force the fallback: no manifest, exactly like a fresh clone, a
        // project linked before the manifest landed, or a deleted
        // `.armadai/`.
        std::fs::remove_file(root.join(".armadai/link-manifest.yaml"))
            .expect("link must have written a manifest to delete");

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");
        assert!(
            unlink_stderr.contains("falling back"),
            "this test is only meaningful on the fallback path: stderr={unlink_stderr}"
        );

        assert!(
            !claude_md.exists(),
            "the coordinator's own file must be reclaimed: `unlink` has to resolve \
             `coordinator: dev-lead` to `Dev Lead` exactly as `link` did, or the root \
             context file survives as a silent orphan: stdout={unlink_stdout}"
        );
        assert!(
            !member_file.exists(),
            "control: the plain member file must still be reclaimed: stdout={unlink_stdout}"
        );
        assert_eq!(
            files_under(&root.join(".claude")),
            Vec::<std::path::PathBuf>::new(),
            "no file may survive the round trip anywhere under the output \
             directory (the now-empty `.claude/` itself is deliberately left \
             standing — `remove_empty_ancestors` is bounded by the target root): \
             stdout={unlink_stdout}"
        );
    }
}
