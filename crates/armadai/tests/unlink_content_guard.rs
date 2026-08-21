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
}
