//! Golden snapshots of `armadai link` on the six legacy fixtures once migrated
//! (spec §7 criterion 1): for Claude, the whole output (`.claude/`, the root
//! `CLAUDE.md`) and the conformance report; for Codex, Copilot and OpenCode, the
//! output and the report on one fixture. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p armadai --test link_migrated_golden` ONLY for a
//! deliberate change, re-read every changed file, and say so in the PR body.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

const CASES: [&str; 6] = [
    "armadai-dev",
    "code-analysis-rust",
    "declarative",
    "demo-rust-team",
    "fullstack",
    "rust-dev",
];

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let dest = to.join(p.file_name().unwrap());
        if p.is_dir() {
            copy_tree(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}

fn armadai(root: &Path, config: &Path, args: &[&str]) -> String {
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(root)
        .env("ARMADAI_CONFIG_DIR", config)
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{args:?} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// `=== path ===\ncontent\n` for every file under each of `dirs` (relative to root).
fn render(root: &Path, dirs: &[&str]) -> String {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    for d in dirs {
        let p = root.join(d);
        if p.is_dir() {
            walk(&p, &mut files);
        } else if p.is_file() {
            files.push(p);
        }
    }
    files.sort();
    files
        .iter()
        .map(|f| {
            format!(
                "=== {} ===\n{}\n",
                f.strip_prefix(root).unwrap().display(),
                String::from_utf8_lossy(&std::fs::read(f).unwrap())
            )
        })
        .collect()
}

fn report(stdout: &str) -> String {
    let r = stdout
        .split_once("Conformance report:\n")
        .expect("report")
        .1;
    format!("=== report ===\n{r}")
}

fn check(name: &str, rendered: String) {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/link_migrated")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &rendered).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&golden)
        .unwrap_or_else(|_| panic!("missing {}: UPDATE_GOLDEN=1", golden.display()));
    assert_eq!(rendered, expected, "{name}: link output changed");
}

fn migrated(case: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/migrate")
            .join(case),
        &root,
    );
    armadai(&root, &config, &["migrate"]);
    (dir, root, config)
}

#[test]
fn claude_output_of_every_migrated_fixture() {
    for case in CASES {
        let (_dir, root, config) = migrated(case);
        let out = armadai(&root, &config, &["link", "--target", "claude"]);
        let mut rendered = render(&root, &["CLAUDE.md", ".claude"]);
        rendered.push_str(&report(&out));
        check(&format!("{case}-claude"), rendered);
    }
}

#[test]
fn codex_copilot_and_opencode_output_of_one_migrated_fixture() {
    let (_dir, root, config) = migrated("armadai-dev");
    let out = armadai(
        &root,
        &config,
        &["link", "--target", "codex", "copilot", "opencode"],
    );
    let mut rendered = render(&root, &[".codex", ".github", ".opencode"]);
    rendered.push_str(&report(&out));
    check("armadai-dev-legacy-targets", rendered);
}
