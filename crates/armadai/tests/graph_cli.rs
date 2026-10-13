//! `armadai graph`: the Mermaid of one flow or of every flow, on stdout only.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

const PROJECT: &[(&str, &str)] = &[
    (".armadai/project.yaml", "schema_version: 1\n"),
    (".armadai/agents/dev.md", "---\ndescription: d\n---\nB\n"),
    (".armadai/agents/rev.md", "---\ndescription: r\n---\nB\n"),
    (".armadai/flows/ship.yaml", "sequence: [dev, rev]\n"),
    (
        ".armadai/flows/review.yaml",
        "steps:\n  write: { agent: dev }\n  check: { agent: rev, states: [approved, rejected], on_failure: write }\nlinks:\n  - { from: write, to: check }\n  - { from: check, to: write, when: rejected }\n",
    ),
];

const SHIP: &str = "%% flow: ship
flowchart TD
  n_dev[\"dev\"]
  n_rev[\"rev\"]
  n_dev --> n_rev
";

const REVIEW: &str = "%% flow: review
flowchart TD
  n_check[\"check (rev)\"]
  n_write[\"write (dev)\"]
  n_write --> n_check
  n_check -->|\"rejected\"| n_write
  n_check -.->|\"failed\"| n_write
";

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let path = dir.path().join("project").join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        std::fs::create_dir_all(dir.path().join("project/.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// `(exit code, stdout, stderr)`
    fn run(&self, cwd: &Path, args: &[&str]) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(cwd)
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

#[test]
fn one_flow_prints_only_its_mermaid() {
    let env = Env::new(PROJECT);
    let (code, out, _) = env.run(&env.root(), &["graph", "review"]);
    assert_eq!(code, Some(0));
    assert_eq!(out, REVIEW);
    let (_, out, _) = env.run(&env.root(), &["graph", "ship", "--format", "mermaid"]);
    assert_eq!(out, SHIP, "a sequence is drawn expanded");
}

#[test]
fn every_flow_is_one_block_each_in_flow_order() {
    let env = Env::new(PROJECT);
    let sub = env.root().join("src");
    std::fs::create_dir_all(&sub).unwrap();
    let (code, out, _) = env.run(&sub, &["graph"]);
    assert_eq!(code, Some(0));
    assert_eq!(out, format!("{REVIEW}\n{SHIP}"));
}

#[test]
fn an_unknown_flow_lists_the_flows() {
    let env = Env::new(PROJECT);
    let (code, out, err) = env.run(&env.root(), &["graph", "shipp"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(
        err.contains("unknown flow `shipp`") && err.contains("review, ship"),
        "{err}"
    );
}

#[test]
fn a_project_without_flows_says_so() {
    let env = Env::new(&PROJECT[..3]);
    let (code, out, err) = env.run(&env.root(), &["graph"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("no flow"), "{err}");
}

#[test]
fn outside_a_project_is_a_clear_error() {
    let env = Env::new(&[]);
    let (code, out, err) = env.run(&env.root(), &["graph"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("armadai init --project"), "{err}");
}

#[test]
fn a_legacy_project_points_to_migrate() {
    let env = Env::new(&[(".armadai/config.yaml", "agents:\n  - name: dev\n")]);
    let (code, out, err) = env.run(&env.root(), &["graph"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("run `armadai migrate`"), "{err}");
}

#[test]
fn an_invalid_project_prints_its_diagnostics_and_fails() {
    let mut files = PROJECT.to_vec();
    files.push((".armadai/flows/bad.yaml", "steps:\n  x: { agent: ghost }\n"));
    let env = Env::new(&files);
    let (code, out, err) = env.run(&env.root(), &["graph", "ship"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("unknown agent `ghost`"), "{err}");
}
