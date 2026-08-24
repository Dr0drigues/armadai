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

    /// `sha256:<hex>` of `content` — computed independently of
    /// `linker::manifest::digest_of` (this test crate has no access to the
    /// binary's internals) so a forged manifest entry below can carry a
    /// digest that genuinely matches a victim file's content, proving the
    /// trust-root check — not a digest mismatch — is what keeps it safe.
    fn sha256_digest(content: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content);
        format!("sha256:{:x}", hasher.finalize())
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

        // Drop the skill from the config entirely. The fallback's own
        // skill sweep calls `project::resolve_all_skills(&config, &root)`
        // against the *current* config — with no `skills:` entry left,
        // it would find zero skill directories and never even look at
        // `.claude/skills/notes/`, so it could not reclaim
        // `SKILL.md` either. Only a manifest, which recorded the skill
        // file independently of the config, can still tell `unlink` to
        // remove exactly it. Without this, the fallback would happen to
        // pass this same assertion for an unrelated reason (it still
        // finds the skill via the still-declared config) and this test
        // would not be proving what its name says.
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: solo\nlink:\n  target: claude\n",
        )
        .unwrap();

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
            "the skill file link actually copied and recorded must still be reclaimed, \
             even though the skill is no longer declared in the config at all"
        );
    }

    /// Companion to the test above, mirroring `case2_control`: the exact
    /// same setup, but with the manifest deleted before `unlink` runs. With
    /// the skill also dropped from the config, the fallback has nothing to
    /// regenerate for it at all — so *neither* file in
    /// `.claude/skills/notes/` is touched, proving the previous test's
    /// reclaim of `SKILL.md` really came from the manifest.
    #[test]
    fn case3_control_skill_file_survives_without_a_manifest_once_undeclared() {
        let dir = project_with_a_skill();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let skill_dest = root.join(".claude/skills/notes");
        let copied_skill_md = skill_dest.join("SKILL.md");
        assert!(copied_skill_md.is_file());

        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: solo\nlink:\n  target: claude\n",
        )
        .unwrap();
        std::fs::remove_file(manifest_path(&root)).expect("manifest must exist to delete");

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            copied_skill_md.exists(),
            "without a manifest, and with the skill no longer declared, the fallback \
             has no way to know `.claude/skills/notes/` is its own output at all — the \
             file it legitimately copied must survive, unreclaimed"
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
        // The exact manifest-path phrase — not just "content differs",
        // which the fallback also prints (without "from what link wrote").
        // A run against the fallback (reviewer-verified by replay) passes
        // this test's other assertions identically, so the phrase is the
        // one thing here that actually pins the manifest path.
        assert!(
            unlink_stdout.contains("content differs from what link wrote"),
            "unlink must report the digest mismatch via the manifest-specific phrasing: \
             stdout={unlink_stdout}"
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

    /// Security fix 1a (post-implementation review): a manifest entry
    /// naming a path outside the target's own trusted root must never be
    /// acted on, no matter how correct its digest is — the manifest is
    /// data on disk, not something `unlink` wrote itself moments ago, and
    /// must be treated as untrusted input.
    ///
    /// Mutation this catches: if the trust-root check were removed, or
    /// weakened to a textual `starts_with(".claude")` check instead of a
    /// normalised containment check, this test would delete a file
    /// entirely outside the project — a real four-times-measured escape
    /// (a forged `../` entry, an absolute path, and — with no malice at
    /// all — a legitimate `--output ../sibling` that a naive fix could
    /// still get wrong).
    #[test]
    fn forged_manifest_entry_outside_the_trusted_root_is_refused_not_deleted() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        // A file entirely outside the project — a forged entry will try
        // to reach it via `../outside/victim.txt`.
        let outside_dir = dir.path().join("outside");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let victim = outside_dir.join("victim.txt");
        let victim_content = b"do not delete me\n";
        std::fs::write(&victim, victim_content).unwrap();

        // Overwrite the manifest with a hand-crafted one: a real target
        // root (`.claude`), and a single entry whose digest genuinely
        // matches the victim file — so if the trust-root check were
        // absent, `unlink` would happily delete it.
        let forged_digest = sha256_digest(victim_content);
        let forged_manifest = [
            "version: 1",
            "targets:",
            "  claude:",
            "    linked_at: \"2026-01-01T00:00:00Z\"",
            "    root: .claude",
            "    created_dirs: []",
            "    entries:",
            "      - path: ../outside/victim.txt",
            "        produced_by: { kind: agent, name: solo }",
            "        outcome: created",
            &format!("        digest: \"{forged_digest}\""),
            "",
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            victim.exists(),
            "a forged entry outside the trusted root must never be deleted"
        );
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            victim_content,
            "the surviving file's content must be untouched"
        );
        assert!(
            unlink_stderr.contains("outside its trusted root"),
            "unlink must report the refused entry: stderr={unlink_stderr}"
        );
    }

    /// A legitimate `--output` that climbs above the project root
    /// (`../sibling/out`) must still link and unlink cleanly — the trust
    /// boundary is the target's own declared root, not the project root,
    /// so this is accepted while an *unrelated* escape (the test above)
    /// is refused. The sibling output directory's own root survives
    /// unremoved, exactly like `.claude/` does by default — only what's
    /// created *inside* it is reclaimed.
    #[test]
    fn a_legitimate_output_dir_outside_the_project_root_still_links_and_unlinks_cleanly() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) = run_armadai(
            &root,
            &config,
            &["link", "--target", "claude", "--output", "../sibling/out"],
        );
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let sibling_file = dir.path().join("sibling/out/agents/solo.md");
        assert!(
            sibling_file.is_file(),
            "link must have written under the sibling output dir"
        );

        let (unlink_ok, _, unlink_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--output", "../sibling/out"],
        );
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !sibling_file.exists(),
            "a legitimate --output outside the project root must still be reclaimed"
        );
        assert!(
            !dir.path().join("sibling/out/agents").exists(),
            "the created subdirectory must be cleaned up"
        );
        assert!(
            dir.path().join("sibling/out").is_dir(),
            "the target's own declared root must never be removed, even outside the \
             project — matching the .claude/ invariant for any --output"
        );
    }

    /// Fix 1b resolves the false claim a review found in a comment ("the
    /// manifest path can never widen what gets deleted"): a nested
    /// `--output tools/ai-configs` was previously removed in its entirety
    /// (the fallback kept it) because the old ancestor-sweep boundary was
    /// derived from the *first path component* of an entry, not the whole
    /// declared root. With `created_dirs` recording exactly what `link`
    /// created, the target's own root is never a removal candidate at all,
    /// regardless of nesting.
    ///
    /// Mutation this catches: if the boundary were still derived from a
    /// single path component (or omitted `created_dirs` filtering) instead
    /// of the exact recorded root, this test's last two assertions would
    /// fail — `tools/ai-configs` (or `tools`) would be gone.
    #[test]
    fn a_nested_output_directory_is_cleaned_up_without_removing_its_own_root() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) = run_armadai(
            &root,
            &config,
            &["link", "--target", "claude", "--output", "tools/ai-configs"],
        );
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let nested_file = root.join("tools/ai-configs/agents/solo.md");
        assert!(nested_file.is_file());

        let (unlink_ok, _, unlink_stderr) = run_armadai(
            &root,
            &config,
            &[
                "unlink",
                "--target",
                "claude",
                "--output",
                "tools/ai-configs",
            ],
        );
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !nested_file.exists(),
            "the generated file must be reclaimed"
        );
        assert!(
            !root.join("tools/ai-configs/agents").exists(),
            "the created subdirectory must be cleaned up"
        );
        assert!(
            root.join("tools/ai-configs").is_dir(),
            "the target's own declared root must never be removed by unlink, even when \
             it ends up empty — the same invariant as `.claude/`, now for any --output"
        );
        assert!(
            root.join("tools").is_dir(),
            "an ancestor of the target root survives too, since the root itself does"
        );
    }

    /// Important fix 2 (post-review): `--agents` must narrow to the named
    /// agents' own files *and* the coordinator's — an unnamed agent's file
    /// and the coordinator's context document must both be left alone,
    /// matching what the fallback actually does (verified by the review:
    /// filtering the roster before extracting the coordinator silently
    /// drops the coordinator from consideration too).
    ///
    /// Mutation this catches: if `produced_by.kind == Coordinator` entries
    /// were still unconditionally included regardless of `--agents` (the
    /// pre-fix behaviour), `.claude/CLAUDE.md` would be deleted here even
    /// though `--agents member-a` never named the coordinator.
    #[test]
    fn unlink_agents_filter_leaves_the_coordinator_and_unmatched_agents_alone() {
        let dir = project_with_coordinator_and_two_members();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let claude_md = root.join(".claude/CLAUDE.md");
        let member_a = root.join(".claude/agents/member-a.md");
        let member_b = root.join(".claude/agents/member-b.md");
        assert!(claude_md.is_file());
        assert!(member_a.is_file());
        assert!(member_b.is_file());

        let (unlink_ok, _, unlink_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--agents", "member-a"],
        );
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !member_a.exists(),
            "the named agent's own file must still be removed"
        );
        assert!(
            member_b.exists(),
            "an unnamed agent's file must be left alone by --agents"
        );
        assert!(
            claude_md.exists(),
            "the coordinator's file must survive a --agents filter that doesn't name the \
             coordinator, matching the fallback's own behaviour"
        );
    }
}
