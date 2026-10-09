//! Golden snapshots of what `armadai link` writes for the four targets kept by
//! the v1 refocus (claude, codex, copilot, opencode).
//!
//! Step 0 removes ~47k lines around `link` without touching it. This test is
//! the proof: the generated tree must stay byte-identical. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p armadai --test link_golden` ONLY when a PR
//! deliberately changes link output, and say so in the PR body.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::{Path, PathBuf};

    /// (target, directory the linker writes into)
    const TARGETS: &[(&str, &str)] = &[
        ("claude", ".claude"),
        ("codex", ".codex"),
        ("copilot", ".github"),
        ("opencode", ".opencode"),
    ];

    /// Selects the coordinator. Adjust here if `link --help` names it differently.
    const COORDINATOR_ARGS: &[&str] = &["--coordinator", "lead"];

    const LEAD: &str = "# lead\n\n## Metadata\n\n- provider: anthropic\n- model: claude-sonnet-4-5-20250929\n- temperature: 0.4\n- tags: [coordination]\n\n## System Prompt\n\nLEAD_SYSTEM_MARKER — you coordinate the team.\n\n## Instructions\n\nLEAD_INSTRUCTIONS_MARKER — delegate, then synthesize.\n\n## Output Format\n\nLEAD_OUTPUT_MARKER — one paragraph.\n";

    const DEV: &str = "# dev\n\n## Metadata\n\n- provider: anthropic\n- model: claude-sonnet-4-5-20250929\n- temperature: 0.9\n- tags: [dev]\n\n## System Prompt\n\nDEV_SYSTEM_MARKER — you write the code.\n\n### Style\n\nDEV_SUBSECTION_MARKER — small functions.\n\n## Instructions\n\nDEV_INSTRUCTIONS_MARKER — tests first.\n";

    fn isolated_config(dir: &Path) -> PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        config
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(
            root.join(".armadai/config.yaml"),
            "agents:\n  - path: agents/lead.md\n  - path: agents/dev.md\n",
        )
        .unwrap();
        std::fs::write(root.join("agents/lead.md"), LEAD).unwrap();
        std::fs::write(root.join("agents/dev.md"), DEV).unwrap();
        dir
    }

    /// Every file under `dir`, sorted by relative path, rendered as
    /// `=== <path> ===\n<content>\n`. The absolute project root is replaced by
    /// `<ROOT>` so the snapshot does not depend on the temp directory.
    fn render_tree(root: &Path, dir: &Path) -> String {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(dir, &mut files);
        files.sort();
        let root_str = root.display().to_string();
        let mut rendered = String::new();
        for file in files {
            let rel = file.strip_prefix(root).unwrap().display().to_string();
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("reading {}: {e}", file.display()))
                .replace(&root_str, "<ROOT>");
            rendered.push_str(&format!("=== {rel} ===\n{content}\n"));
        }
        rendered
    }

    fn golden_path(target: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/link")
            .join(format!("{target}.txt"))
    }

    #[test]
    fn link_output_is_byte_identical_to_the_golden_snapshots() {
        for (target, out_dir) in TARGETS {
            let dir = project();
            let root = dir.path().join("project");

            let mut cmd = Command::cargo_bin("armadai").unwrap();
            cmd.current_dir(&root)
                .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
                .args(["link", "--target", target, "--force"])
                .args(COORDINATOR_ARGS);
            let output = cmd.output().unwrap();
            assert!(
                output.status.success(),
                "link --target {target} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );

            let produced = root.join(out_dir);
            assert!(
                produced.is_dir(),
                "target {target}: {} was not created",
                produced.display()
            );
            let rendered = render_tree(&root, &produced);
            // Guards against a vacuous snapshot: both markers must be present.
            assert!(
                rendered.contains("DEV_SYSTEM_MARKER"),
                "target {target}: the dev agent did not reach the output:\n{rendered}"
            );
            assert!(
                rendered.contains("LEAD_SYSTEM_MARKER"),
                "target {target}: the coordinator did not reach the output:\n{rendered}"
            );

            let golden = golden_path(target);
            if std::env::var_os("UPDATE_GOLDEN").is_some() {
                std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
                std::fs::write(&golden, &rendered).unwrap();
                continue;
            }
            let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
                panic!(
                    "missing snapshot {} — generate it with UPDATE_GOLDEN=1",
                    golden.display()
                )
            });
            assert_eq!(
                rendered, expected,
                "target {target}: link output changed. If this is deliberate, \
                 regenerate with UPDATE_GOLDEN=1 and justify it in the PR."
            );
        }
    }
}
