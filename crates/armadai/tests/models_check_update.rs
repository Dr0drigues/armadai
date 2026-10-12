//! `armadai models check` / `update` on the canonical model: deprecated ids live in
//! an agent's `native.<target>.model` (agents otherwise carry a tier).

use std::path::{Path, PathBuf};

use assert_cmd::Command;

const DEV: &str = "---\n# kept: a comment the update must not lose\ndescription: Writes code\nnative:\n  codex:\n    model: gpt-4-turbo\n---\nYou write code.\n";
const LEAD: &str = "---\ndescription: Leads\ntier: max\n---\nYou lead.\n";

fn project(dir: &Path) -> PathBuf {
    let root = dir.join("project");
    for (p, c) in [
        (".armadai/project.yaml", "schema_version: 1\n"),
        (".armadai/agents/dev.md", DEV),
        (".armadai/agents/lead.md", LEAD),
    ] {
        let path = root.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, c).unwrap();
    }
    root
}

fn armadai(dir: &Path, cwd: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(cwd)
        .env("ARMADAI_CONFIG_DIR", dir.join("config"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn check_reports_a_deprecated_native_model_and_update_replaces_only_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path());
    let (ok, out) = armadai(dir.path(), &root, &["models", "check"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("dev") && out.contains("gpt-4-turbo") && out.contains("gpt-4o"),
        "{out}"
    );
    assert!(out.contains("1 deprecated model(s) found"), "{out}");

    let (ok, out) = armadai(dir.path(), &root, &["models", "update"]);
    assert!(ok, "{out}");
    let dev = std::fs::read_to_string(root.join(".armadai/agents/dev.md")).unwrap();
    assert_eq!(
        dev,
        DEV.replace("gpt-4-turbo", "gpt-4o"),
        "only the id changes"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".armadai/agents/lead.md")).unwrap(),
        LEAD
    );
    let (_, out) = armadai(dir.path(), &root, &["models", "check"]);
    assert!(out.contains("All models are up to date"), "{out}");
}

#[test]
fn check_all_reads_the_projects_link_registers() {
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path());
    let (ok, out) = armadai(dir.path(), &root, &["link"]);
    assert!(ok, "{out}");
    let (ok, out) = armadai(dir.path(), &root, &["models", "check", "--all"]);
    assert!(ok, "{out}");
    assert!(!out.contains("No project config found"), "{out}");
    assert!(out.contains("gpt-4-turbo"), "{out}");
    let (ok, out) = armadai(dir.path(), &root, &["models", "check", "--all", "--prune"]);
    assert!(
        ok && !out.contains("Pruned"),
        "a linked project is not stale: {out}"
    );
}

#[test]
fn a_legacy_project_is_refused_with_the_migrate_hint() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("legacy");
    std::fs::create_dir_all(root.join(".armadai")).unwrap();
    std::fs::write(root.join(".armadai/config.yaml"), "agents: []\n").unwrap();
    let (ok, out) = armadai(dir.path(), &root, &["models", "check"]);
    assert!(!ok && out.contains("run `armadai migrate`"), "{out}");
}

#[test]
fn check_all_names_a_registered_legacy_project() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("legacy");
    std::fs::create_dir_all(legacy.join(".armadai")).unwrap();
    std::fs::write(legacy.join(".armadai/config.yaml"), "agents: []\n").unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(
        dir.path().join("config/projects.json"),
        serde_json::json!({"projects": [{"path": legacy, "last_seen": "2026-10-12T00:00:00Z"}]})
            .to_string(),
    )
    .unwrap();
    let (ok, out) = armadai(dir.path(), dir.path(), &["models", "check", "--all"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("legacy format, run `armadai migrate`"),
        "{out}"
    );
}
