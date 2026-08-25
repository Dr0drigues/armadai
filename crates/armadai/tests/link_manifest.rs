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
        // Byte-wise, like `manifest::digest_of`: sha2 0.11's `finalize()`
        // returns a type without `LowerHex`. Kept as an independent
        // re-implementation so this stays a real cross-check of the
        // production digest rather than a call to it.
        let mut out = String::from("sha256:");
        for byte in hasher.finalize() {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    fn read_manifest(root: &std::path::Path) -> String {
        std::fs::read_to_string(manifest_path(root)).expect("manifest must exist and be readable")
    }

    /// The `outcome:` the manifest records for one entry, by path.
    ///
    /// `read_manifest(..).contains("outcome: skipped")` cannot say *which*
    /// entry it found, so a test with more than one entry can pass on the
    /// wrong one. Relies on `ManifestEntry`'s field order (path,
    /// produced_by, outcome, digest), which is what serde serialises.
    fn outcome_for(manifest: &str, entry_path: &str) -> String {
        for chunk in manifest.split("- path: ").skip(1) {
            let (first_line, rest) = chunk.split_once('\n').unwrap_or((chunk, ""));
            if first_line.trim() != entry_path {
                continue;
            }
            for line in rest.lines() {
                if let Some(value) = line.trim().strip_prefix("outcome: ") {
                    return value.trim().to_string();
                }
            }
            panic!("entry {entry_path} carries no outcome in:\n{manifest}");
        }
        panic!("no manifest entry for {entry_path} in:\n{manifest}");
    }

    /// The `created_dirs` list the manifest records for its single target
    /// (every test using this links exactly one target).
    fn created_dirs_of(manifest: &str) -> Vec<String> {
        let after = manifest
            .split_once("created_dirs:")
            .unwrap_or_else(|| panic!("manifest has no created_dirs:\n{manifest}"))
            .1;
        let (same_line, rest) = after.split_once('\n').unwrap_or((after, ""));
        if same_line.trim() == "[]" {
            return Vec::new();
        }
        let mut dirs = Vec::new();
        for line in rest.lines() {
            match line.trim().strip_prefix("- ") {
                Some(value) => dirs.push(value.trim().to_string()),
                None => break,
            }
        }
        dirs
    }

    /// The paths the manifest records as entries, in order.
    fn entry_paths_of(manifest: &str) -> Vec<String> {
        manifest
            .split("- path: ")
            .skip(1)
            .map(|chunk| {
                chunk
                    .split_once('\n')
                    .map(|(first, _)| first)
                    .unwrap_or(chunk)
                    .trim()
                    .to_string()
            })
            .collect()
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

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        // Design review R5: refusing a forged entry is a failure to
        // complete the requested work, not a partial success — the
        // trustworthy `.claude/agents/solo.md` entry still gets removed
        // below, but the process must still exit non-zero overall.
        assert!(
            !unlink_ok,
            "unlink must exit non-zero when it refused an untrusted entry:              stderr={unlink_stderr}"
        );

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
        // The *cause*, not only the refusal (issue #348): this manifest was
        // hand-crafted, so the manifest is what must be blamed. Without
        // this, mutating `diagnose_trust_failure` to always answer
        // `FilesystemDiverged` left the whole suite green — the two
        // messages the sibling fix split apart were interchangeable again.
        assert!(
            unlink_stderr.contains("corrupt or forged"),
            "a hand-crafted manifest entry is the manifest's own fault and must be \
             reported as such: stderr={unlink_stderr}"
        );
        // The refusal summary must sit on the same stream as the refusals
        // it points at: as a `println!` it told a `2>/dev/null` caller to
        // read reasons that stream had just discarded.
        assert!(
            unlink_stderr.contains("manifest item(s) were refused"),
            "the refusal summary belongs on stderr, beside the per-item reasons: \
             stderr={unlink_stderr}"
        );
        assert!(
            !unlink_stdout.contains("manifest item(s) were refused"),
            "and must not be left on stdout, pointing at reasons stdout never \
             carried: stdout={unlink_stdout}"
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

    /// Design review R1 (critical): the trust boundary must not be read
    /// from the thing it is meant to constrain. A forged `root: "/"`
    /// claims to own the entire filesystem, which would make
    /// `is_trusted` accept *any* absolute path — reproducing the
    /// original path-traversal defect entirely on a build that already
    /// has the per-entry guard. Only cross-checking the declared root
    /// against one `unlink` computes independently, from the project's
    /// own config, catches this.
    ///
    /// Mutation this catches: if `unlink` trusted `TargetManifest::root`
    /// on its own (as it did before this fix) instead of confirming it
    /// against the config-derived root, this test would delete a file
    /// entirely outside the project.
    #[test]
    fn forged_root_wide_enough_to_contain_anything_is_refused() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let outside_dir = dir.path().join("outside-root-forge");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let victim = outside_dir.join("victim.txt");
        let victim_content = b"do not delete me either\n";
        std::fs::write(&victim, victim_content).unwrap();

        let forged_digest = sha256_digest(victim_content);
        let victim_str = victim.to_str().unwrap().replace('\\', "/");
        let forged_manifest = [
            "version: 1".to_string(),
            "targets:".to_string(),
            "  claude:".to_string(),
            "    linked_at: \"2026-01-01T00:00:00Z\"".to_string(),
            "    root: \"/\"".to_string(),
            "    created_dirs: []".to_string(),
            "    entries:".to_string(),
            format!("      - path: \"{victim_str}\""),
            "        produced_by: { kind: agent, name: solo }".to_string(),
            "        outcome: created".to_string(),
            format!("        digest: \"{forged_digest}\""),
            String::new(),
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            unlink_ok,
            "falling back to the #342 guard must still succeed: stderr={unlink_stderr}"
        );

        assert!(
            victim.exists(),
            "a forged root: / must never be trusted, even for an entry it would \
             otherwise contain"
        );
        assert_eq!(std::fs::read(&victim).unwrap(), victim_content);
        assert!(
            unlink_stderr.contains("doesn't match this project's current output directory"),
            "unlink must report the root mismatch: stderr={unlink_stderr}"
        );
        // Control: the fallback still reclaims the legitimately generated file.
        assert!(!root.join(".claude/agents/solo.md").exists());
    }

    /// Design review R3 (closed by R1's construction for a wide forged
    /// root, but not for a manifest whose `root` is otherwise legitimate
    /// and simply names that same root inside `created_dirs` too): the
    /// target's own root must never be removed, independent of trusting
    /// the manifest at all — `is_trusted` alone would accept it (a path
    /// always contains itself).
    ///
    /// Mutation this catches: if the explicit `dir_path ==
    /// resolved_target_root` guard were removed and only `is_trusted`
    /// gated `created_dirs` removal, this test's `.claude` would be gone.
    #[test]
    fn forged_created_dirs_naming_the_targets_own_root_is_refused() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        let solo_digest = sha256_digest(&std::fs::read(&solo_file).unwrap());

        let forged_manifest = [
            "version: 1",
            "targets:",
            "  claude:",
            "    linked_at: \"2026-01-01T00:00:00Z\"",
            "    root: .claude",
            "    created_dirs:",
            "      - .claude/agents",
            "      - .claude",
            "    entries:",
            "      - path: .claude/agents/solo.md",
            "        produced_by: { kind: agent, name: solo }",
            "        outcome: created",
            &format!("        digest: \"{solo_digest}\""),
            "",
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            !unlink_ok,
            "unlink must exit non-zero for the refused created_dirs entry: \
             stderr={unlink_stderr}"
        );

        assert!(
            !solo_file.exists(),
            "the legitimate file entry must still be reclaimed"
        );
        assert!(
            root.join(".claude").is_dir(),
            "the target's own root must never be removed, even when a manifest's \
             created_dirs names it directly"
        );
        // The other half of the two-cause split: this manifest really does
        // name the root in its own text, so this is the case that *is* the
        // manifest's fault. Its sibling test
        // (`a_symlink_merging_a_recorded_dir_into_the_target_root_blames_the_filesystem`)
        // pins the opposite verdict for an intact manifest, and one of the
        // two fails if the cause is hardcoded either way.
        assert!(
            unlink_stderr.contains("names the target's own root"),
            "a manifest that literally records the root names it: stderr={unlink_stderr}"
        );
        assert!(
            unlink_stderr.contains("corrupt or forged"),
            "and that is the manifest's own fault: stderr={unlink_stderr}"
        );
    }

    /// Design review R2 (critical in effect): lexical normalisation alone
    /// is not a security boundary against a symlink. `.claude/agents` is
    /// replaced with a symlink to a directory entirely outside the
    /// project; a manifest entry `.claude/agents/keys.md` reads as
    /// contained under `.claude` textually, but the file it actually
    /// names lives elsewhere.
    ///
    /// Mutation this catches: if `is_trusted` resolved paths with
    /// `lexically_normalize` alone instead of `resolve_real` (which
    /// canonicalises the existing prefix), this test would delete a file
    /// outside the project through the symlink.
    #[test]
    #[cfg(unix)]
    fn a_symlinked_intermediate_directory_does_not_bypass_the_trust_boundary() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let claude_agents = root.join(".claude/agents");
        std::fs::remove_dir_all(&claude_agents).unwrap();
        let outside_dir = dir.path().join("outside-agents");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let victim = outside_dir.join("keys.md");
        let victim_content = b"super secret\n";
        std::fs::write(&victim, victim_content).unwrap();
        std::os::unix::fs::symlink(&outside_dir, &claude_agents).unwrap();

        let forged_digest = sha256_digest(victim_content);
        let forged_manifest = [
            "version: 1",
            "targets:",
            "  claude:",
            "    linked_at: \"2026-01-01T00:00:00Z\"",
            "    root: .claude",
            "    created_dirs: []",
            "    entries:",
            "      - path: .claude/agents/keys.md",
            "        produced_by: { kind: agent, name: solo }",
            "        outcome: created",
            &format!("        digest: \"{forged_digest}\""),
            "",
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            !unlink_ok,
            "unlink must exit non-zero for the refused entry: stderr={unlink_stderr}"
        );

        assert!(
            victim.exists(),
            "a symlinked intermediate directory must not let an entry escape the \
             trusted root"
        );
        assert_eq!(std::fs::read(&victim).unwrap(), victim_content);
    }

    /// Design review R6 (important): `link ; link ; unlink` must remove
    /// everything the first `link` wrote. Relinking after no config
    /// change — or after a change that doesn't affect a given file — is
    /// an everyday action, not a reason for `unlink` to start treating
    /// `link`'s own output as hand-written.
    ///
    /// Mutation this catches: if a pre-existing file whose content
    /// already matches were still recorded `skipped` (the pre-fix
    /// behaviour), `solo.md` would survive the final `unlink` here,
    /// reported as "hand-written" about a file `link` wrote twice.
    #[test]
    fn link_then_link_then_unlink_removes_everything() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link1_ok, _, link1_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link1_ok, "first link must succeed: stderr={link1_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        assert!(solo_file.is_file());

        let (link2_ok, link2_stdout, link2_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link2_ok, "second link must succeed: stderr={link2_stderr}");
        assert!(
            link2_stdout.contains("up-to-date"),
            "the second link must recognise its own unchanged output: stdout={link2_stdout}"
        );

        let manifest = read_manifest(&root);
        assert!(
            manifest.contains("outcome: created"),
            "a file link wrote before, and would write identically again, must still be \
             recorded as created, not downgraded to skipped: {manifest}"
        );

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !solo_file.exists(),
            "link; link; unlink must remove everything link wrote"
        );
    }

    /// Companion to `unlink_agents_filter_leaves_the_coordinator_and_unmatched_agents_alone`,
    /// pinning the filter from the other direction (a test gap the review
    /// found: `--agents` appears only once in this suite, and only
    /// asserted survival — a mutation that unconditionally excludes every
    /// `Coordinator`-kind entry whenever `--agents` is set would pass
    /// that test too). Naming the coordinator explicitly must still
    /// remove its document.
    #[test]
    fn unlink_agents_filter_including_the_coordinators_name_still_removes_it() {
        let dir = project_with_coordinator_and_two_members();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let claude_md = root.join(".claude/CLAUDE.md");
        assert!(claude_md.is_file());

        let (unlink_ok, _, unlink_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--agents", "coord"],
        );
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !claude_md.exists(),
            "naming the coordinator explicitly in --agents must still remove its \
             document — proves the filter checks the name, not a blanket exclusion \
             of Coordinator-kind entries"
        );
    }

    /// Second half of the same pin: a plain `unlink`, with no `--agents`
    /// filter at all, must remove the coordinator's document too.
    #[test]
    fn plain_unlink_without_a_filter_removes_the_coordinators_document_too() {
        let dir = project_with_coordinator_and_two_members();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let claude_md = root.join(".claude/CLAUDE.md");
        assert!(claude_md.is_file());

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !claude_md.exists(),
            "a plain unlink with no --agents filter must remove the coordinator's \
             document too"
        );
    }

    /// The original worst case of issue #338, generalised beyond a single
    /// colliding filename (`case1` above pins that narrower one: a
    /// hand-written `.claude/CLAUDE.md` at a path `link` would also
    /// write): a `.claude/` directory that predates `armadai link`
    /// entirely — the user's own Claude Code config, with content that
    /// has nothing to do with anything the linker would ever generate,
    /// not even a colliding filename — must survive a `link` + `unlink`
    /// round trip completely intact, directory and all. Verified correct
    /// by hand in two separate design reviews, never pinned by an
    /// automated test until issue #348.
    ///
    /// Mutation this catches: drop the `if current.exists() { break; }`
    /// early-out from `linker::manifest::create_dir_all_recording`, so it
    /// reports directories it merely *walked past* rather than only the
    /// ones it created. `created_dirs` then names the user's own
    /// pre-existing `.claude/agents/`, the manifest assertion below fails,
    /// and — because a `created_dirs` entry is all `unlink` needs to
    /// justify removing an empty directory — the directory-survival
    /// assertion fails too.
    ///
    /// Its own earlier version could not fail, and this is why: the guards
    /// it claimed to protect (`decide_created_dir`'s `IsTargetRoot`, and
    /// `write_files`' refusal to record the root) are unreachable for a
    /// `.claude/` that already existed, because
    /// `create_dir_all_recording` never reports a directory it did not
    /// create. Deleting either guard left *this scenario* green
    /// (measured) — not the suite, which has its own tests for both:
    /// mutating `write_files`' `created != target_root` filter turns 19
    /// tests red (measured with `--no-fail-fast`; without it cargo stops
    /// at the first failing target and only 3 show). The point is only
    /// that this scenario proved nothing about them. The load-bearing
    /// invariant here is one layer earlier — `link` must not *record* a
    /// pre-existing user directory as created — so that is what this now
    /// asserts, on the manifest `link` wrote and on the directory that
    /// survives because of it.
    #[test]
    fn a_preexisting_user_claude_directory_survives_link_then_unlink_intact() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        // The user's own `.claude/` predates `link` entirely: unrelated
        // content, plus — and this is the part that makes the test
        // load-bearing — an `agents/` directory they created themselves
        // and left empty, which is exactly where `link` is about to write.
        // `link` therefore creates *nothing*, and the manifest must say
        // so; a `created_dirs` that claimed this directory would hand
        // `unlink` a licence to remove it once solo.md is gone.
        let claude_dir = root.join(".claude");
        std::fs::create_dir_all(claude_dir.join("commands")).unwrap();
        std::fs::create_dir_all(claude_dir.join("agents")).unwrap();
        std::fs::write(claude_dir.join("settings.json"), "{\"theme\":\"dark\"}\n").unwrap();
        std::fs::write(
            claude_dir.join("commands/my-command.md"),
            "# My own command\n\nDo not touch.\n",
        )
        .unwrap();

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        assert!(solo_file.is_file(), "link must have generated solo's file");

        // The recorded effect, not just the final filesystem state: every
        // directory this write needed already existed, so `link` created
        // none and `created_dirs` must be empty.
        let manifest = read_manifest(&root);
        assert_eq!(
            created_dirs_of(&manifest),
            Vec::<String>::new(),
            "link must record only directories it actually created — a pre-existing \
             user directory it merely wrote into is not one of them: {manifest}"
        );
        assert_eq!(
            entry_paths_of(&manifest),
            vec![".claude/agents/solo.md".to_string()],
            "only the file link generated may be recorded as an entry — nothing the \
             user put in .claude/ themselves: {manifest}"
        );

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !solo_file.exists(),
            "the genuinely generated file must still be reclaimed"
        );
        assert!(
            claude_dir.join("agents").is_dir(),
            "the user's own pre-existing (and now empty again) .claude/agents/ must \
             survive — unlink may only remove directories link recorded creating"
        );
        assert!(
            claude_dir.is_dir(),
            "the user's own pre-existing .claude/ directory must never be removed"
        );
        assert!(
            claude_dir.join("settings.json").is_file(),
            "unrelated pre-existing content must survive untouched"
        );
        assert_eq!(
            std::fs::read_to_string(claude_dir.join("settings.json")).unwrap(),
            "{\"theme\":\"dark\"}\n"
        );
        assert!(
            claude_dir.join("commands/my-command.md").is_file(),
            "a pre-existing subdirectory with unrelated content must survive \
             untouched"
        );
    }

    // ── issue #348: residues of this chantier's manifest ───────────────

    /// 1st bullet: `--dry-run` must apply the exact same `created_dirs`
    /// guard as the real pass, refusal and exit code included — reuses
    /// R3's forged-root scenario above
    /// (`forged_created_dirs_naming_the_targets_own_root_is_refused`) but
    /// through `--dry-run` instead of a real unlink. Before this fix, the
    /// dry run's own `created_dirs` count used only `is_trusted` (which a
    /// path always satisfies against itself), so it announced both
    /// `.claude/agents` *and* `.claude` itself as eligible for cleanup
    /// and exited 0 — while the real pass refuses `.claude` and exits 1.
    ///
    /// Also pins **which stream** each half of the preview goes to (issue
    /// #348, third round): the refusal on stderr beside the reasons a
    /// real pass puts there, the counts on stdout. They had drifted —
    /// every refusal in the preview was a `println!` while every refusal
    /// in the real pass was an `eprintln!`, so a caller journalling
    /// `2>errors.log` got the reasons from the run and nothing from its
    /// preview.
    ///
    /// Mutation this catches: if `--dry-run`'s `created_dirs` handling
    /// reverted to a plain "is_trusted" count with no `decide_created_dir`
    /// guard, this test's exit-code assertion would fail, and the printed
    /// count would go back to 2 instead of 1.
    #[test]
    fn dry_run_applies_the_same_created_dirs_guard_as_the_real_pass() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        let solo_digest = sha256_digest(&std::fs::read(&solo_file).unwrap());

        let forged_manifest = [
            "version: 1",
            "targets:",
            "  claude:",
            "    linked_at: \"2026-01-01T00:00:00Z\"",
            "    root: .claude",
            "    created_dirs:",
            "      - .claude/agents",
            "      - .claude",
            "    entries:",
            "      - path: .claude/agents/solo.md",
            "        produced_by: { kind: agent, name: solo }",
            "        outcome: created",
            &format!("        digest: \"{solo_digest}\""),
            "",
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (dry_run_ok, dry_run_stdout, dry_run_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--dry-run"],
        );
        assert!(
            !dry_run_ok,
            "--dry-run must refuse and exit non-zero, exactly like the real pass \
             would: stdout={dry_run_stdout} stderr={dry_run_stderr}"
        );
        assert!(
            dry_run_stdout.contains("(1 directory eligible for cleanup)"),
            "only the legitimate .claude/agents entry is eligible — the forged \
             .claude entry (naming the target's own root) must be excluded from the \
             count, not just from the actual deletion: stdout={dry_run_stdout}"
        );
        assert!(
            dry_run_stderr.contains("would refuse")
                && dry_run_stderr.contains("names the target's own root"),
            "the preview's refusal belongs on stderr, where the real pass puts its \
             own: stderr={dry_run_stderr}"
        );
        assert!(
            !dry_run_stdout.contains("would refuse"),
            "and must not also be on stdout, or a caller reading one stream sees a \
             different run than a caller reading the other: stdout={dry_run_stdout}"
        );
        assert!(
            dry_run_stderr.contains("manifest item(s) would be refused"),
            "the refusal summary must sit beside the refusals it points at: \
             stderr={dry_run_stderr}"
        );

        // A dry run must never have any side effect either way.
        assert!(solo_file.exists());
        assert!(root.join(".claude").is_dir());
    }

    /// 2nd bullet: when a symlink appears **between** `link` and `unlink`
    /// — not a forged manifest at all, the manifest is exactly what
    /// `link` itself wrote — the refusal must say the filesystem changed,
    /// not accuse the manifest of being "corrupt or forged". The two
    /// causes call for two different next steps from the user.
    ///
    /// Mutation this catches: if the cause were collapsed back into a
    /// single message (`diagnose_trust_failure` dropped in favour of a
    /// plain `is_trusted` boolean), this test's stderr assertions would
    /// fail — the message would go back to blaming the manifest for a
    /// disk change it had nothing to do with.
    #[test]
    #[cfg(unix)]
    fn a_symlink_appearing_after_link_is_blamed_on_the_filesystem_not_the_manifest() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        // The manifest at this point is exactly what `link` wrote —
        // nothing hand-edited, nothing forged. Only the filesystem
        // changes afterwards: `.claude/agents` is replaced with a symlink
        // pointing outside the project, entirely independent of the
        // manifest's own content.
        let claude_agents = root.join(".claude/agents");
        std::fs::remove_dir_all(&claude_agents).unwrap();
        let outside_dir = dir.path().join("elsewhere");
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::os::unix::fs::symlink(&outside_dir, &claude_agents).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            !unlink_ok,
            "unlink must still refuse and exit non-zero: stderr={unlink_stderr}"
        );

        assert!(
            unlink_stderr.contains("something on the filesystem does"),
            "unlink must point at the filesystem, which is what takes the recorded \
             path outside the root: stderr={unlink_stderr}"
        );
        assert!(
            !unlink_stderr.contains("since link ran"),
            "and must not date that — the symlink did appear after `link` here, but \
             nothing in the manifest records the filesystem as it was then, so the \
             claim would be right only by luck: stderr={unlink_stderr}"
        );
        assert!(
            !unlink_stderr.contains("corrupt or forged"),
            "the manifest is intact — it must not be blamed for a filesystem \
             change it had nothing to do with: stderr={unlink_stderr}"
        );
    }

    /// 3rd bullet: a manifest entry whose path is a *broken* symlink (the
    /// link itself is present on disk, but its target is gone) must not
    /// be reported "already absent" — it is present, just unverifiable,
    /// and must be kept and reported as such.
    ///
    /// Mutation this catches: if the existence check reverted to a plain
    /// `!path.exists()` (`true` for a broken symlink too, since it
    /// follows the link to a target that's gone), this test's stdout
    /// assertions would fail — the entry would be silently miscounted
    /// absent instead of reported kept.
    ///
    /// Also pins that the kept-files footer accounts for this outcome
    /// (issue #348, third round): both footers on the manifest path
    /// carried a closed list of three reasons that omitted the very
    /// outcome this test produces, so a user reading the explanation could
    /// not find their own case in it. Reverting either footer to that list
    /// fails here.
    #[test]
    #[cfg(unix)]
    fn a_broken_symlink_at_an_entrys_path_is_kept_and_reported_not_absent() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        assert!(solo_file.is_file());

        // Replace the linked file with a dangling symlink — present on
        // disk, but its target doesn't exist.
        std::fs::remove_file(&solo_file).unwrap();
        std::os::unix::fs::symlink(root.join("does-not-exist.md"), &solo_file).unwrap();
        assert!(
            solo_file.symlink_metadata().is_ok(),
            "the symlink itself must be present on disk before unlink runs"
        );

        // The preview has to say the same thing. Emptying
        // `print_manifest_dry_run`'s own `is_symlink()` branch left the
        // whole suite green — a surviving mutant on the very fix this
        // chantier had just shipped.
        let (dry_ok, dry_stdout, dry_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--dry-run"],
        );
        assert!(dry_ok, "the preview must succeed: stderr={dry_stderr}");
        assert!(
            dry_stdout.contains("0 would be removed, 1 would be kept, 0 already absent"),
            "the preview must count the dangling link kept, exactly as the real pass \
             does below: stdout={dry_stdout}"
        );
        assert!(
            dry_stdout.contains("broken symlink"),
            "and must say why: stdout={dry_stdout}"
        );
        assert!(
            dry_stdout.contains("or a broken symlink whose content cannot be compared"),
            "and the footer explaining kept files must account for this outcome \
             instead of listing three reasons that exclude it: stdout={dry_stdout}"
        );

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            solo_file.symlink_metadata().is_ok(),
            "the dangling symlink must still be on disk — unlink must not silently \
             miscount it as an unrelated absence"
        );
        assert!(
            unlink_stdout.contains("0 deleted, 1 kept, 0 already absent"),
            "a broken symlink is present on disk and must count as kept, not \
             absent: stdout={unlink_stdout}"
        );
        assert!(
            unlink_stdout.contains("broken symlink"),
            "unlink must explain why the dangling link was kept: \
             stdout={unlink_stdout}"
        );
        assert!(
            unlink_stdout.contains("or a broken symlink whose content cannot be compared"),
            "and the footer explaining kept files must account for this outcome too: \
             stdout={unlink_stdout}"
        );
    }

    /// 4th bullet, residue of design review R3: the trust root alone
    /// bounds *where* a `created_dirs` entry may resolve, not *whether*
    /// it corresponds to anything `link` actually created. A forged entry
    /// naming a pre-existing, currently-empty directory the user made by
    /// hand — inside the target's own tree, so it passes the trust-root
    /// check trivially — must still survive, because it was never
    /// `link`'s to remove.
    ///
    /// Mutation this catches: if the plausibility check
    /// (`created_dir_is_plausible`) were removed and only the trust-root
    /// and "not the root itself" guards remained, this test's
    /// directory-survival assertion would fail — the forged entry would
    /// be silently removed for being merely empty and in-bounds.
    #[test]
    fn forged_created_dirs_naming_a_preexisting_empty_user_directory_is_refused() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        let solo_digest = sha256_digest(&std::fs::read(&solo_file).unwrap());

        // A directory the user made by hand, unrelated to anything link
        // wrote — link never created it, and no manifest entry lives
        // under it.
        let user_dir = root.join(".claude/my-own-empty-dir");
        std::fs::create_dir_all(&user_dir).unwrap();

        let forged_manifest = [
            "version: 1",
            "targets:",
            "  claude:",
            "    linked_at: \"2026-01-01T00:00:00Z\"",
            "    root: .claude",
            "    created_dirs:",
            "      - .claude/agents",
            "      - .claude/my-own-empty-dir",
            "    entries:",
            "      - path: .claude/agents/solo.md",
            "        produced_by: { kind: agent, name: solo }",
            "        outcome: created",
            &format!("        digest: \"{solo_digest}\""),
            "",
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            !unlink_ok,
            "unlink must exit non-zero for the implausible created_dirs entry: \
             stderr={unlink_stderr}"
        );

        assert!(
            !solo_file.exists(),
            "the legitimate file entry must still be reclaimed"
        );
        assert!(
            user_dir.is_dir(),
            "a pre-existing, empty user directory falsely claimed by created_dirs \
             must survive — it corresponds to no file link ever recorded creating"
        );
    }

    /// Coordination note 2: a re-`link` that leaves an already-linked file
    /// untouched (no `--force`, because the newly generated content
    /// differs from what's on disk) must not downgrade that file to
    /// "hand-written" in the manifest when the file itself is still
    /// exactly what the *first* `link` wrote — only the upstream agent
    /// source changed, not the file. Getting this wrong turns a
    /// completely untouched, still-`link`-owned file into a permanent
    /// residue `unlink` can never reclaim again short of a `--force`
    /// relink the user has no reason to know they need.
    ///
    /// Mutation this catches: if the manifest kept mislabelling this file
    /// `skipped`/no-digest on the second link (the pre-fix behaviour),
    /// the final `unlink` here would report it "kept ... hand-written"
    /// instead of deleting it, and the file-survival assertion would
    /// fail.
    #[test]
    fn relink_of_an_untouched_file_after_an_upstream_source_change_stays_reclaimable() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link1_ok, _, link1_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link1_ok, "first link must succeed: stderr={link1_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        let first_content = std::fs::read_to_string(&solo_file).unwrap();

        // The agent's own source changes upstream — the next link would
        // generate different content — but nobody touches the linked
        // file itself.
        std::fs::write(
            root.join("agents/solo.md"),
            "# solo\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nYou work \
             alone, updated.\n",
        )
        .unwrap();

        let (link2_ok, _, link2_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link2_ok, "second link must succeed: stderr={link2_stderr}");
        assert!(
            link2_stderr.contains("skip:"),
            "without --force the file must not be overwritten: stderr={link2_stderr}"
        );
        assert_eq!(
            std::fs::read_to_string(&solo_file).unwrap(),
            first_content,
            "the file itself must remain untouched without --force"
        );

        let manifest = read_manifest(&root);
        assert!(
            manifest.contains("outcome: created"),
            "a file that is still exactly what the first link wrote must stay \
             attributed to link, not be downgraded to skipped just because the \
             upstream source changed: {manifest}"
        );
        assert!(
            !manifest.contains("outcome: skipped"),
            "with a single agent and no other entries, the manifest must not \
             contain a skipped outcome at all: {manifest}"
        );

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        assert!(
            !solo_file.exists(),
            "a file that was never touched since the first link wrote it must \
             still be reclaimed by unlink, even though a later link's \
             regeneration would have produced different content"
        );
    }
    /// 2nd bullet, sibling branch: `decide_created_dir`'s `IsTargetRoot`
    /// arm is reachable **by a pure filesystem mutation with the manifest
    /// untouched** — replace a legitimately recorded `.claude/agents` with
    /// a symlink to `.claude` and the recorded directory now resolves onto
    /// the target's own root. The cause used to be hardcoded there ("the
    /// manifest may be corrupt or forged"), so this scenario produced
    /// exactly the false accusation the sibling fix removed from the
    /// entry-level guard — in the very PR that removed it.
    ///
    /// Mutation this catches: replace `CreatedDirDecision::IsTargetRoot`'s
    /// computed cause with a hardcoded
    /// `TrustFailure::ManifestEscapesRoot` (or restore the hardcoded
    /// message in either of `unlink`'s two `IsTargetRoot` arms) and both
    /// the "corrupt or forged" assertions below fail — on the real pass
    /// and on `--dry-run`, which is why both are exercised here.
    #[test]
    #[cfg(unix)]
    fn a_symlink_merging_a_recorded_dir_into_the_target_root_blames_the_filesystem() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        // Precondition: the manifest legitimately records `.claude/agents`
        // as a directory `link` created. Nothing below edits the manifest.
        let manifest = read_manifest(&root);
        assert_eq!(
            created_dirs_of(&manifest),
            vec![".claude/agents".to_string()],
            "link must have recorded the directory it created: {manifest}"
        );

        // Only the filesystem moves: `.claude/agents` becomes a symlink to
        // its own parent, so the recorded directory resolves onto the
        // target root without the manifest naming the root at all.
        let claude_agents = root.join(".claude/agents");
        std::fs::remove_dir_all(&claude_agents).unwrap();
        std::os::unix::fs::symlink(".", &claude_agents).unwrap();
        assert_eq!(
            std::fs::canonicalize(&claude_agents).unwrap(),
            std::fs::canonicalize(root.join(".claude")).unwrap(),
            "the symlink must really collapse the recorded directory onto the root"
        );

        let (dry_ok, dry_stdout, dry_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--dry-run"],
        );
        assert!(
            !dry_ok,
            "the preview must refuse too, exactly like the real pass: \
             stdout={dry_stdout} stderr={dry_stderr}"
        );
        assert!(
            dry_stderr.contains("resolves onto the target's own root"),
            "the preview must say the directory *resolves onto* the root, not that it \
             names it — on stderr, like the real pass: stderr={dry_stderr}"
        );
        assert!(
            !dry_stdout.contains("would refuse"),
            "the preview must not put the same refusal on stdout as well: \
             stdout={dry_stdout}"
        );
        assert!(
            !dry_stderr.contains("corrupt or forged"),
            "the manifest is byte-for-byte what link wrote — the preview must not \
             accuse it: stderr={dry_stderr}"
        );

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            !unlink_ok,
            "unlink must refuse and exit non-zero: stderr={unlink_stderr}"
        );
        assert!(
            unlink_stderr.contains("resolves onto the target's own root"),
            "the real pass must say the same thing its preview said: \
             stderr={unlink_stderr}"
        );
        assert!(
            unlink_stderr.contains("something on the filesystem does"),
            "the user must be pointed at the disk, which is what puts the recorded \
             directory on the root: stderr={unlink_stderr}"
        );
        // The code compares recorded text against the filesystem as it is
        // now; it holds no snapshot of link time, so it must not date the
        // change (issue #348, third round). Here the symlink really did
        // appear after `link` — the point is that `unlink` cannot know
        // that, and a message asserting it would be right by luck.
        assert!(
            !unlink_stderr.contains("since link ran"),
            "the refusal must not claim *when* the filesystem came to say this — \
             nothing in the manifest records the filesystem as it was at link time: \
             stderr={unlink_stderr}"
        );
        assert!(
            !unlink_stderr.contains("corrupt or forged"),
            "the manifest is byte-for-byte what link wrote — it must not be blamed \
             for a filesystem change it had nothing to do with: stderr={unlink_stderr}"
        );

        // The symlink itself is left alone: refusing means touching nothing.
        assert!(claude_agents.symlink_metadata().is_ok());
    }

    /// 4th bullet, the half `created_dir_is_plausible` does **not** cover:
    /// that check asks whether a recorded directory is an ancestor of some
    /// recorded `Created` entry — it never asks whether that entry itself
    /// passed the trust boundary. A manifest can therefore make an
    /// out-of-project directory "plausible" simply by forging an entry
    /// under it, which is why `decide_created_dir` must keep its own trust
    /// check rather than lean on plausibility.
    ///
    /// Mutation this catches: delete the `Untrusted` arm from
    /// `decide_created_dir` (its `diagnose_trust_failure` early return).
    /// The forged directory below then reads as plausible — the forged
    /// entry `../victim-empty/ghost.md` sits under it — and is removed for
    /// being merely empty, so the directory-survival assertion fails.
    /// Measured: with that arm deleted and no test like this one, the full
    /// suite stayed green.
    #[test]
    fn forged_created_dirs_outside_the_trusted_root_is_refused() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        let solo_digest = sha256_digest(&std::fs::read(&solo_file).unwrap());

        // An empty directory outside the project entirely — a real one a
        // user might own, which the forged manifest below claims `link`
        // created and therefore may remove.
        let victim_dir = dir.path().join("victim-empty");
        std::fs::create_dir_all(&victim_dir).unwrap();

        let forged_manifest = [
            "version: 1",
            "targets:",
            "  claude:",
            "    linked_at: \"2026-01-01T00:00:00Z\"",
            "    root: .claude",
            "    created_dirs:",
            "      - ../victim-empty",
            "    entries:",
            "      - path: .claude/agents/solo.md",
            "        produced_by: { kind: agent, name: solo }",
            "        outcome: created",
            &format!("        digest: \"{solo_digest}\""),
            // Present only to make the forged directory pass
            // `created_dir_is_plausible` — it is an ancestor of a recorded
            // `created` entry. Nothing on disk answers to this path.
            "      - path: ../victim-empty/ghost.md",
            "        produced_by: { kind: agent, name: solo }",
            "        outcome: created",
            &format!("        digest: \"{solo_digest}\""),
            "",
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            !unlink_ok,
            "unlink must exit non-zero for the refused items: stderr={unlink_stderr}"
        );

        assert!(
            victim_dir.is_dir(),
            "an empty directory outside the trusted root must survive, however \
             plausible the manifest's own entries make it look"
        );
        assert!(
            unlink_stderr.contains("recorded directory '../victim-empty'"),
            "unlink must name the refused directory, not only the refused entry: \
             stderr={unlink_stderr}"
        );
        // Control: the one legitimate entry is still reclaimed.
        assert!(!solo_file.exists());
    }

    /// The #342 fallback is the *degraded* mode — no manifest, so odd
    /// trees are likelier, not less — and it reported a dangling symlink
    /// as "already absent" long after the manifest path stopped (issue
    /// #348's 3rd bullet, whose text restricts it to neither path). The
    /// link is present on disk and its content can never be compared with
    /// what `link` would generate, so the guard's own conservative "kept"
    /// outcome applies, with an accurate reason.
    ///
    /// Mutation this catches: remove the `path.is_symlink()` branch from
    /// either of `unlink_via_fallback`'s two existence checks (the real
    /// pass's or its `--dry-run` preview's) and the corresponding count
    /// assertion below flips back to "already absent".
    #[test]
    #[cfg(unix)]
    fn the_fallback_reports_a_broken_symlink_as_kept_not_absent() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        std::fs::remove_file(&solo_file).unwrap();
        std::os::unix::fs::symlink(root.join("does-not-exist.md"), &solo_file).unwrap();

        // Drop the manifest so `unlink` takes the #342 fallback — the
        // path this test is about.
        std::fs::remove_dir_all(root.join(".armadai")).unwrap();

        let (dry_ok, dry_stdout, dry_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--dry-run"],
        );
        assert!(dry_ok, "the preview must succeed: stderr={dry_stderr}");
        assert!(
            dry_stderr.contains("No link manifest found"),
            "this test is only meaningful on the fallback path: stderr={dry_stderr}"
        );
        assert!(
            dry_stdout.contains("0 would be removed, 1 would be kept, 0 already absent"),
            "a dangling link is present on disk — the preview must count it kept, \
             not absent: stdout={dry_stdout}"
        );
        assert!(
            dry_stdout.contains("broken symlink"),
            "the preview must say why it would be kept: stdout={dry_stdout}"
        );

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");
        assert!(
            unlink_stdout.contains("0 deleted, 1 kept, 0 already absent"),
            "the real pass must count it exactly as its own preview did: \
             stdout={unlink_stdout}"
        );
        assert!(
            unlink_stdout.contains("broken symlink"),
            "unlink must explain why the dangling link was kept: \
             stdout={unlink_stdout}"
        );
        assert!(
            solo_file.symlink_metadata().is_ok(),
            "the dangling symlink must still be on disk, reported rather than \
             miscounted as an unrelated absence"
        );
    }

    /// The invariant the `previous_digests` reclaim in
    /// `linker::manifest::write_files` must never move (issue #348,
    /// coordination note 2): that reclaim exists so `link`'s *own*
    /// untouched output keeps its `created` attribution when the upstream
    /// source changes, and it must stay unable to claim a file `link`
    /// never wrote. A hand-written file therefore stays `skipped` across
    /// any number of `link` runs, however often the generated content
    /// changes underneath it.
    ///
    /// Mutation this catches: make `write_files`' final differing-content
    /// branch record `Outcome::Created` with the on-disk digest instead of
    /// `Outcome::Skipped`/`None` — i.e. let `link` claim whatever it finds
    /// in its way. Both the manifest assertions and the file-survival
    /// assertion below fail. No test pinned this before; the reclaim was
    /// added with only its own positive case covered.
    #[test]
    fn a_hand_written_file_stays_skipped_across_repeated_links() {
        let dir = project_with_coordinator_and_two_members();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let claude_dir = root.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let hand_written = "# My own notes\n\nDo not touch this file, it is mine.\n";
        std::fs::write(claude_dir.join("CLAUDE.md"), hand_written).unwrap();

        for pass in 1..=3 {
            // Change the coordinator's source between passes so the
            // content `link` would generate for CLAUDE.md differs every
            // time — the situation the reclaim is about, applied to a file
            // that is not link's to reclaim.
            std::fs::write(
                root.join("agents/coord.md"),
                format!(
                    "# coord\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\n\
                     You coordinate, revision {pass}.\n"
                ),
            )
            .unwrap();

            let (link_ok, _, link_stderr) =
                run_armadai(&root, &config, &["link", "--target", "claude"]);
            assert!(
                link_ok,
                "link pass {pass} must succeed: stderr={link_stderr}"
            );

            let manifest = read_manifest(&root);
            assert_eq!(
                outcome_for(&manifest, ".claude/CLAUDE.md"),
                "skipped",
                "pass {pass}: a file link never wrote must stay recorded as \
                 hand-written, whatever the manifest already said: {manifest}"
            );
            assert!(
                !manifest.contains(&sha256_digest(hand_written.as_bytes())),
                "pass {pass}: link must not record a digest for content it did not \
                 write — that digest is what would authorise deleting it: {manifest}"
            );
            assert_eq!(
                std::fs::read_to_string(claude_dir.join("CLAUDE.md")).unwrap(),
                hand_written,
                "pass {pass}: the file itself must be untouched"
            );
        }

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");
        assert_eq!(
            std::fs::read_to_string(claude_dir.join("CLAUDE.md")).unwrap(),
            hand_written,
            "three links later, the hand-written file must still be there, untouched"
        );
        // Control: the generated files are still reclaimed.
        assert!(!root.join(".claude/agents/member-a.md").exists());
        assert!(!root.join(".claude/agents/member-b.md").exists());
    }

    // ── issue #348, third round ────────────────────────────────────────

    /// A project whose skill has a subdirectory — the ordinary case, no
    /// forging, no symlink. `link` records the parent `.claude/skills`
    /// while writing `SKILL.md` and only then records the deeper
    /// `.claude/skills/notes/refs` while writing the file inside it, so
    /// `created_dirs` holds a parent *before* one of its own descendants:
    ///
    /// ```yaml
    /// created_dirs:
    /// - .claude/agents
    /// - .claude/skills/notes
    /// - .claude/skills
    /// - .claude/skills/notes/refs
    /// ```
    ///
    /// Walked in that order, `.claude/skills/notes` and `.claude/skills`
    /// are both still non-empty when they come up (the `refs/` below them
    /// has not been removed yet) and survive as empty residues — the very
    /// class of bug this branch is named after.
    ///
    /// Mutation this catches: drop the
    /// `linker::manifest::deepest_first` call from `unlink`'s real pass
    /// (`for dir in created_dirs` instead of `for dir in
    /// deepest_first(&created_dirs)`). Measured before this test existed:
    /// removing it from *both* call sites left the whole suite green,
    /// `deepest_first` being pinned only in isolation by its own unit
    /// test.
    #[test]
    fn a_skill_with_a_subdirectory_leaves_no_empty_directories_behind() {
        let dir = project_with_a_nested_skill();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        // Precondition — the ordering that makes this test load-bearing:
        // a parent recorded before a descendant of its own.
        let recorded = created_dirs_of(&read_manifest(&root));
        let parent = recorded
            .iter()
            .position(|d| d == ".claude/skills")
            .expect("link must record .claude/skills");
        let deeper = recorded
            .iter()
            .position(|d| d == ".claude/skills/notes/refs")
            .expect("link must record .claude/skills/notes/refs");
        assert!(
            parent < deeper,
            "this test only bites while link records a parent before a deeper \
             descendant; if that ever changes, rebuild the fixture rather than \
             deleting the test: {recorded:?}"
        );

        let (unlink_ok, unlink_stdout, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(unlink_ok, "unlink must succeed: stderr={unlink_stderr}");

        for residue in [
            ".claude/agents",
            ".claude/skills",
            ".claude/skills/notes",
            ".claude/skills/notes/refs",
        ] {
            assert!(
                !root.join(residue).exists(),
                "'{residue}' must not survive as an empty directory: \
                 stdout={unlink_stdout}"
            );
        }
        // The target's own root is never recorded and never removed.
        assert!(
            root.join(".claude").is_dir(),
            "the target root itself must stay: stdout={unlink_stdout}"
        );

        // The run summary belongs on stdout, where a caller redirecting
        // stderr away still sees it (measured good in both the manifest
        // and the fallback path — do not let a refusal-stream change take
        // it along).
        assert!(
            unlink_stdout.contains("Unlinked 'claude'"),
            "the summary belongs on stdout: stdout={unlink_stdout}"
        );
        assert!(
            !unlink_stderr.contains("Unlinked 'claude'"),
            "and only there: stderr={unlink_stderr}"
        );
    }

    /// The `--dry-run` half of the ordering above. A preview prints
    /// nothing at all for a directory it would clean up, so the only place
    /// its walk order is observable is the refusals — which is also the
    /// only thing that can pin
    /// `linker::manifest::deepest_first`'s *second* call site.
    ///
    /// Two recorded directories at different depths, deliberately listed
    /// shallowest-first, correspond to no recorded file, so both are
    /// refused and both are printed. Deepest first means the deeper one is
    /// named first, in the preview and in the real pass alike.
    ///
    /// Mutation this catches: drop the `deepest_first` call from
    /// `print_manifest_dry_run` and the preview lists them in manifest
    /// order — shallowest first — while the real pass still leads with the
    /// deeper one. Dropping it from the real pass instead fails the second
    /// half. Each call site is covered on its own.
    #[test]
    fn preview_and_real_pass_walk_recorded_directories_in_the_same_order() {
        let dir = project_minimal();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        let solo_digest = sha256_digest(&std::fs::read(&solo_file).unwrap());

        // Shallowest first, so manifest order and deepest-first order
        // genuinely disagree.
        let forged_manifest = [
            "version: 1",
            "targets:",
            "  claude:",
            "    linked_at: \"2026-01-01T00:00:00Z\"",
            "    root: .claude",
            "    created_dirs:",
            "      - .claude/notes",
            "      - .claude/notes/deep/deeper",
            "    entries:",
            "      - path: .claude/agents/solo.md",
            "        produced_by: { kind: agent, name: solo }",
            "        outcome: created",
            &format!("        digest: \"{solo_digest}\""),
            "",
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (_, _, dry_stderr) = run_armadai(
            &root,
            &config,
            &["unlink", "--target", "claude", "--dry-run"],
        );
        let dry_deep = dry_stderr
            .find(".claude/notes/deep/deeper (would refuse")
            .unwrap_or_else(|| panic!("preview must refuse the deeper dir: {dry_stderr}"));
        let dry_shallow = dry_stderr
            .find(".claude/notes (would refuse")
            .unwrap_or_else(|| panic!("preview must refuse the shallow dir: {dry_stderr}"));
        assert!(
            dry_deep < dry_shallow,
            "the preview must walk recorded directories deepest first, like the pass \
             it previews: stderr={dry_stderr}"
        );

        let (_, _, unlink_stderr) = run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        let real_deep = unlink_stderr
            .find("'.claude/notes/deep/deeper'")
            .unwrap_or_else(|| panic!("real pass must refuse the deeper dir: {unlink_stderr}"));
        let real_shallow = unlink_stderr
            .find("'.claude/notes'")
            .unwrap_or_else(|| panic!("real pass must refuse the shallow dir: {unlink_stderr}"));
        assert!(
            real_deep < real_shallow,
            "the real pass must walk recorded directories deepest first: \
             stderr={unlink_stderr}"
        );
    }

    /// A manifest naming the target's own root through a **non-canonical
    /// absolute path** — an ancestor reached through a symlink, exactly
    /// how `/tmp` reaches `/private/tmp` on macOS or a symlinked home or
    /// automount does on Linux. Nothing inside the project is symlinked
    /// and nothing on disk moved: the recorded text names the root, and
    /// the refusal must say so.
    ///
    /// Mutation this catches: compare the two paths with
    /// `linker::manifest::resolve` instead of `resolve_lexical` in
    /// `decide_created_dir` (what the previous fix wave shipped). The
    /// refusal then reads "resolves onto the target's own root … something
    /// on the filesystem does", sending the user to inspect a directory
    /// where there is nothing to find — the false accusation issue #348
    /// exists to remove, reintroduced by the fix for it.
    #[test]
    #[cfg(unix)]
    fn a_manifest_naming_the_root_by_a_non_canonical_path_blames_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        // The project sits under `real/`; `alias/` is a second spelling of
        // `real/`, above the project root — nothing within the project is
        // a symlink.
        let root = dir.path().join("real/project");
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
        std::os::unix::fs::symlink("real", dir.path().join("alias")).unwrap();
        let config = isolated_config(dir.path());

        let (link_ok, _, link_stderr) =
            run_armadai(&root, &config, &["link", "--target", "claude"]);
        assert!(link_ok, "link must succeed: stderr={link_stderr}");

        let solo_file = root.join(".claude/agents/solo.md");
        let solo_digest = sha256_digest(&std::fs::read(&solo_file).unwrap());
        let through_the_alias = dir.path().join("alias/project/.claude");
        assert_eq!(
            std::fs::canonicalize(&through_the_alias).unwrap(),
            std::fs::canonicalize(root.join(".claude")).unwrap(),
            "the alias must really be another spelling of the target root"
        );

        let forged_manifest = [
            "version: 1".to_string(),
            "targets:".to_string(),
            "  claude:".to_string(),
            "    linked_at: \"2026-01-01T00:00:00Z\"".to_string(),
            "    root: .claude".to_string(),
            "    created_dirs:".to_string(),
            format!("      - {}", through_the_alias.display()),
            "    entries:".to_string(),
            "      - path: .claude/agents/solo.md".to_string(),
            "        produced_by: { kind: agent, name: solo }".to_string(),
            "        outcome: created".to_string(),
            format!("        digest: \"{solo_digest}\""),
            String::new(),
        ]
        .join("\n");
        std::fs::write(manifest_path(&root), forged_manifest).unwrap();

        let (unlink_ok, _, unlink_stderr) =
            run_armadai(&root, &config, &["unlink", "--target", "claude"]);
        assert!(
            !unlink_ok,
            "unlink must refuse the recorded root: stderr={unlink_stderr}"
        );
        assert!(
            unlink_stderr.contains("names the target's own root"),
            "the manifest names the root — a second spelling of a directory is still \
             that directory: stderr={unlink_stderr}"
        );
        assert!(
            unlink_stderr.contains("corrupt or forged"),
            "and that is the manifest's own fault: stderr={unlink_stderr}"
        );
        assert!(
            !unlink_stderr.contains("something on the filesystem does"),
            "nothing on the filesystem moved — the user must not be sent to inspect \
             a directory where there is nothing to find: stderr={unlink_stderr}"
        );
        assert!(
            root.join(".claude").is_dir(),
            "and the target root must survive either way"
        );
    }

    /// Fixture for the ordering test above: a skill with a subdirectory,
    /// which is what makes `link` record a parent directory before a
    /// deeper descendant of it.
    fn project_with_a_nested_skill() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let agents = root.join("agents");
        let skill_dir = root.join(".armadai/skills/notes");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::create_dir_all(skill_dir.join("refs")).unwrap();
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
        std::fs::write(skill_dir.join("refs/style.md"), "reference material\n").unwrap();
        dir
    }
}
