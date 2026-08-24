//! Black-box regressions for the link manifest (issue #338's second half,
//! see `docs/superpowers/specs/2026-08-24-link-manifest-design.md`). Spawns
//! the real binary (like `unlink_content_guard.rs`) since the manifest is
//! wired end to end through `cli::link::execute` (write) and
//! `cli::unlink::execute` (consume).
//!
//! Each of the four cases below is the same one `unlink_content_guard.rs`
//! guards for the #342 fallback, but exercised **through the manifest
//! path** — `link` here always writes `.armadai/link-manifest.yaml`, and
//! `unlink` finds it and never falls back to regenerating against the
//! current config. Case 2 (the orphan) is the one the fallback structurally
//! cannot fix — its companion test below deletes the manifest and shows
//! the same setup failing, which is what proves the main test exercises
//! the manifest and not the fallback.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// Isolated `ARMADAI_CONFIG_DIR` per project — see `unlink_content_guard.rs`
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

    fn manifest_path(root: &std::path::Path) -> std::path::PathBuf {
        root.join(".armadai/link-manifest.yaml")
    }

    fn read_manifest(root: &std::path::Path) -> String {
        std::fs::read_to_string(manifest_path(root)).expect("manifest must exist and be readable")
    }

    /// A project with a coordinator ("coord") and two members
    /// ("member-a", "member-b"), targeting `claude`.
    fn project_with_coordinator_and_two_members() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let agents = root.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        write_two_member_config(&root);
        std::fs::write(
            agents.join("coord.md"),
            "# coord\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou coordinate.\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("member-a.md"),
            "# member-a\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou work A.\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("member-b.md"),
            "# member-b\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou work B.\n",
        )
        .unwrap();
        dir
    }

    fn write_two_member_config(root: &std::path::Path) {
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: coord\n  - name: member-a\n  - name: member-b\n\
             link:\n  target: claude\n  coordinator: coord\n",
        )
        .unwrap();
    }

    /// Rewrite the project config so `member-b` is no longer declared —
    /// simulating the agent having been removed from the fleet, while
    /// `coord` and `member-a` remain (so `project_declares_agents` still
    /// holds and the fallback path, when exercised, still has something to
    /// regenerate).
    fn drop_member_b(root: &std::path::Path) {
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: coord\n  - name: member-a\n\
             link:\n  target: claude\n  coordinator: coord\n",
        )
        .unwrap();
    }

    /// Case 1 (issue #338, design §6 table row 1): a hand-written
    /// `.claude/CLAUDE.md` predates `link`, which skips it and records
    /// `outcome: skipped` — the inverse of "did nothing" is "do nothing",
    /// so `unlink` must never touch it. Reused control: the genuinely
    /// generated `member-a.md`/`member-b.md` must still be reclaimed.
    ///
    /// Mutation this catches: if `link`'s write loop recorded `created`
    /// (or any outcome carrying a digest that happens to match the
    /// hand-written content) instead of `skipped` for a path it left
    /// alone, `unlink` would delete a file the user wrote by hand.
    #[test]
    fn case1_hand_written_file_recorded_as_skipped_survives_unlink() {
        let dir = project_with_coordinator_and_two_members();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let claude_md_dir = root.join(".claude");
        std::fs::create_dir_all(&claude_md_dir).unwrap();
        let hand_written = "# My own notes\n\nDo not touch this file, it is mine.\n";
        std::fs::write(claude_md_dir.join("CLAUDE.md"), hand_written).unwrap();

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");
        assert!(manifest_path(&root).is_file(), "link must write a manifest");

        let manifest = read_manifest(&root);
        assert!(
            manifest.contains("outcome: skipped"),
            "the manifest must record CLAUDE.md as skipped: {manifest}"
        );
        assert!(
            manifest.contains("outcome: created"),
            "the manifest must record the generated member files as created: {manifest}"
        );

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        let claude_md_path = claude_md_dir.join("CLAUDE.md");
        assert!(
            claude_md_path.exists(),
            "a hand-written CLAUDE.md recorded as skipped must never be deleted"
        );
        assert_eq!(
            std::fs::read_to_string(&claude_md_path).unwrap(),
            hand_written,
            "the surviving file's content must be exactly what the user wrote"
        );
        assert!(
            unlink_stdout.contains("hand-written") || unlink_stdout.contains("skipped"),
            "unlink must report why the file was kept: stdout={unlink_stdout}"
        );

        // Control: genuinely generated, untouched files are still reclaimed.
        assert!(!root.join(".claude/agents/member-a.md").exists());
        assert!(!root.join(".claude/agents/member-b.md").exists());
    }

    /// Case 2 (issue #338, design §6 table row 2 — the orphan): an agent
    /// removed from the config keeps its manifest entry, so `unlink` still
    /// removes its generated file — the case the #342 fallback cannot fix
    /// at all, because the current config no longer names that path for
    /// the linker to regenerate.
    ///
    /// Mutation this catches: if `unlink` ever fell back to regenerating
    /// against the current config instead of reading the manifest (or if
    /// the manifest entry for a dropped agent were pruned instead of kept
    /// until `unlink` consumes it), this file would never be reclaimed —
    /// exactly the pre-#338 orphan-forever bug.
    #[test]
    fn case2_orphaned_agent_file_is_removed_via_manifest() {
        let dir = project_with_coordinator_and_two_members();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");
        let member_b = root.join(".claude/agents/member-b.md");
        assert!(
            member_b.is_file(),
            "link must have generated member-b's file"
        );

        drop_member_b(&root);

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !member_b.exists(),
            "an agent's file must be reclaimed via its manifest entry even after \
             the agent is removed from the config entirely"
        );
    }

    /// Companion to the test above: the exact same setup, but with the
    /// manifest deleted before `unlink` runs, forcing the #342 fallback.
    /// The orphaned file must now survive — proving the previous test
    /// actually exercises the manifest, not a fallback that happens to
    /// produce the same outcome for a different reason.
    #[test]
    fn case2_control_orphaned_agent_file_survives_without_a_manifest() {
        let dir = project_with_coordinator_and_two_members();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");
        let member_b = root.join(".claude/agents/member-b.md");
        assert!(member_b.is_file());

        drop_member_b(&root);
        std::fs::remove_file(manifest_path(&root)).expect("manifest must exist to delete");

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            member_b.exists(),
            "without a manifest, unlink regenerates from the *current* config — which \
             no longer names member-b at all — so its file can never even be a \
             candidate; it must survive, unreclaimed. This is the documented fallback \
             limitation the manifest exists to remove."
        );
        assert!(
            unlink_stderr.contains("falling back"),
            "unlink must announce the degraded mode: stderr={unlink_stderr}"
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

    /// Case 3 (issue #338, design §6 table row 3): a file the user drops
    /// into a linked skill directory after `link` ran has no manifest
    /// entry at all, so it is never even considered — the fix for the
    /// worst measured outcome (a recursive sweep of the destination
    /// directory).
    ///
    /// Mutation this catches: if `unlink`'s manifest path reintroduced a
    /// directory sweep of the skill's *destination* (e.g. "delete
    /// everything under `.claude/skills/notes/`") instead of only acting
    /// on recorded entries, this user file would be deleted even though it
    /// was added after `link` ran and could never appear in the manifest.
    #[test]
    fn case3_user_file_inside_a_linked_skill_directory_survives_via_manifest() {
        let dir = project_with_a_skill();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let manifest = read_manifest(&root);
        assert!(
            manifest.contains("skill"),
            "the manifest must attribute the copied skill file to a skill: {manifest}"
        );

        let skill_dest = root.join(".claude/skills/notes");
        let copied_skill_md = skill_dest.join("SKILL.md");
        assert!(copied_skill_md.is_file(), "link must have copied the skill");

        let user_file = skill_dest.join("my-own-scratchpad.txt");
        let user_content = "totally unrelated to armadai\n";
        std::fs::write(&user_file, user_content).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            user_file.exists(),
            "a user file with no manifest entry must survive unlink"
        );
        assert_eq!(std::fs::read_to_string(&user_file).unwrap(), user_content);
        assert!(
            !copied_skill_md.exists(),
            "the skill file link actually copied and recorded must still be reclaimed"
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

    /// Case 4 (issue #338, design §6 table row 4 / the `opencode --model`
    /// case generalised): a file whose on-disk content no longer matches
    /// the digest `link` recorded — because the user edited it after
    /// linking — is kept, not deleted, and the reason is reported.
    ///
    /// Mutation this catches: if the digest comparison were skipped, or
    /// `digest_matches` always returned `true` for a `Created` entry
    /// regardless of the actual bytes, this test would delete a file the
    /// user has since hand-edited — silent data loss.
    #[test]
    fn case4_file_edited_since_linking_is_kept_and_reported() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        assert!(solo_file.is_file());

        let edited = format!(
            "{}\n\n## A note I added by hand after linking\n",
            std::fs::read_to_string(&solo_file).unwrap()
        );
        std::fs::write(&solo_file, &edited).unwrap();

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            solo_file.exists(),
            "a file whose digest no longer matches must be kept"
        );
        assert_eq!(
            std::fs::read_to_string(&solo_file).unwrap(),
            edited,
            "the kept file's content must be exactly what the user edited it to"
        );
        assert!(
            unlink_stdout.contains("content differs"),
            "unlink must report the digest mismatch: stdout={unlink_stdout}"
        );
    }

    /// `--dry-run` must never write a manifest (design §5) — a preview of
    /// what `link` would do must have no side effect at all.
    ///
    /// Mutation this catches: if the manifest write were hoisted above the
    /// `--dry-run` early return (or the return removed), a `--dry-run`
    /// invocation on a project that was never actually linked would leave
    /// `.armadai/link-manifest.yaml` behind.
    #[test]
    fn dry_run_writes_no_manifest() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude", "--dry-run"]);
        assert!(link_ok, "link --dry-run must succeed: stderr={link_stderr}");
        assert!(
            !manifest_path(&root).exists(),
            "--dry-run must not write a link manifest"
        );
    }

    /// The manifest is grouped by target (design §3/§8), and linking a
    /// second target must not disturb the first's entries. Then relinking
    /// the *first* target with an expanded roster must replace only that
    /// target's entries, leaving the second target's untouched.
    ///
    /// Mutation this catches: if `write_target` replaced the whole
    /// manifest instead of just the named target's slice, linking `codex`
    /// after `claude` would erase `claude`'s entries (or vice versa), and
    /// relinking `claude` again would erase `codex`'s.
    #[test]
    fn manifest_is_grouped_by_target_and_relink_replaces_only_that_target() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (claude_ok, _, claude_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(
            claude_ok,
            "link claude must succeed: stderr={claude_stderr}"
        );

        let (codex_ok, _, codex_stderr) =
            run_armadai(&root, &config, &["link", "--target", "codex"]);
        assert!(codex_ok, "link codex must succeed: stderr={codex_stderr}");

        let manifest = read_manifest(&root);
        assert!(
            manifest.contains("claude:"),
            "manifest must group claude: {manifest}"
        );
        assert!(
            manifest.contains("codex:"),
            "manifest must group codex: {manifest}"
        );
        assert!(manifest.contains(".claude/agents/solo.md"));
        // codex agent files are TOML, not markdown — see `linker::codex`.
        assert!(manifest.contains(".codex/agents/solo.toml"));

        // Expand the roster and relink claude only.
        let agents_dir = root.join("agents");
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: solo\n  - name: helper\nlink:\n  target: claude\n",
        )
        .unwrap();
        std::fs::write(
            agents_dir.join("helper.md"),
            "# helper\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou help.\n",
        )
        .unwrap();

        let (relink_ok, _, relink_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(
            relink_ok,
            "relink claude must succeed: stderr={relink_stderr}"
        );

        let manifest_after = read_manifest(&root);
        assert!(
            manifest_after.contains(".claude/agents/helper.md"),
            "claude's entries must now reflect the expanded roster: {manifest_after}"
        );
        assert!(
            manifest_after.contains(".codex/agents/solo.toml"),
            "codex's entries must survive a claude-only relink untouched: {manifest_after}"
        );
    }
}
