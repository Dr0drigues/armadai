//! Golden snapshots of what `armadai link` writes for the four targets from a
//! canonical project (step 2): the generated tree (plus the root `CLAUDE.md` for
//! Claude) and the conformance report. Regenerate with
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

    /// A canonical project exercising every element the Claude adapter compiles.
    const FILES: &[(&str, &str)] = &[
        (
            ".armadai/project.yaml",
            "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n  free_agents: [Explore]\n",
        ),
        (
            ".armadai/agents/lead.md",
            "---\ndescription: Coordinates the team\ntier: max\n---\nLEAD_SYSTEM_MARKER — you coordinate the team.\n\n## Instructions\n\nLEAD_INSTRUCTIONS_MARKER — delegate, then synthesize.\n",
        ),
        (
            ".armadai/agents/dev.md",
            "---\ndescription: 'Writes code: \"small\" functions'\ntier: pro\ntemperature: 0.9\ntags: [dev]\nskills: [review]\ncapabilities:\n  shell: ask\n  web: deny\n---\nDEV_SYSTEM_MARKER — you write the code.\n\n### Style\n\nDEV_SUBSECTION_MARKER — small functions.\n",
        ),
        (
            ".armadai/skills/review/SKILL.md",
            "---\nname: review\ndescription: Review checklist\n---\nREVIEW_MARKER\n",
        ),
        (
            ".armadai/flows/ship.yaml",
            "steps:\n  build: { agent: dev, states: [blocked] }\n  plan: { agent: lead }\nlinks:\n  - { from: plan, to: build }\n  - { from: build, to: plan, when: blocked }\n",
        ),
        (
            ".armadai/automations/fmt.yaml",
            "event: after-tool\nrun: cargo fmt\nnative:\n  claude:\n    matcher: Edit|Write\n",
        ),
    ];

    fn isolated_config(dir: &Path) -> PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        config
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        for (p, c) in FILES {
            let path = root.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
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
                .args(["link", "--target", target]);
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
            let mut rendered = render_tree(&root, &produced);
            if *target == "claude" {
                let md = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
                rendered = format!("=== CLAUDE.md ===\n{md}\n{rendered}");
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let report = stdout
                .split_once("Conformance report:\n")
                .expect("link prints its conformance report")
                .1;
            rendered.push_str(&format!("=== report ===\n{report}"));
            // Guards against a vacuous snapshot: both markers must be present
            // (the coordinator lives in CLAUDE.md for claude, in the root file elsewhere).
            assert!(
                rendered.contains("DEV_SYSTEM_MARKER"),
                "target {target}: the dev agent did not reach the output:\n{rendered}"
            );
            assert!(
                rendered.contains("LEAD_SYSTEM_MARKER"),
                "target {target}: the coordinator did not reach the output:\n{rendered}"
            );

            // The response-marker protocol was removed: no target may write
            // an `ARMADAI_` marker into the generated files again.
            assert!(
                !rendered.contains("ARMADAI_"),
                "target {target}: the output contains an ARMADAI_ marker:\n{rendered}"
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

    /// `link` must never reach the network (decision of 2026-10-09): every
    /// proxy variable points at a live local listener, so any HTTP(S) attempt
    /// from an env-honoring client shows up as an accepted connection.
    /// Direct sockets that bypass the proxy variables are not covered.
    #[test]
    fn link_does_not_touch_the_network() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let dir = project();
        let root = dir.path().join("project");
        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .env("HTTPS_PROXY", &proxy)
            .env("HTTP_PROXY", &proxy)
            .env("ALL_PROXY", &proxy)
            .env("https_proxy", &proxy)
            .env("http_proxy", &proxy)
            .env("all_proxy", &proxy)
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .args(["link", "--target", "opencode"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "link failed: {text}");
        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "link opened a connection through the proxy"
        );
    }
}
