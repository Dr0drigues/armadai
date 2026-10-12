//! `armadai validate` on the new `.armadai/project.yaml` format.

use assert_cmd::Command;
use tempfile::TempDir;

fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let path = dir.path().join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, c).unwrap();
    }
    dir
}

fn validate(dir: &TempDir, extra: &[&str]) -> assert_cmd::assert::Assert {
    let cfg = tempfile::tempdir().unwrap();
    Command::cargo_bin("armadai")
        .unwrap()
        .env("ARMADAI_CONFIG_DIR", cfg.path())
        .env("NO_COLOR", "1")
        .arg("validate")
        .arg(dir.path())
        .args(extra)
        .assert()
}

const M: (&str, &str) = (".armadai/project.yaml", "schema_version: 1\n");
const DEV: (&str, &str) = (
    ".armadai/agents/dev.md",
    "---\ndescription: dev\n---\nBody\n",
);
const BAD_FLOW: (&str, &str) = (".armadai/flows/f.yaml", "steps:\n  a: { agent: ghost }\n");

#[test]
fn valid_new_project_exits_0() {
    let dir = project(&[M, DEV, (".armadai/flows/f.yaml", "sequence: [dev]\n")]);
    let out = validate(&dir, &[]).success().get_output().stdout.clone();
    assert!(String::from_utf8_lossy(&out).contains("0 error(s), 0 warning(s)"));
}

#[test]
fn invalid_new_project_exits_1_and_prints_position() {
    let dir = project(&[M, BAD_FLOW]);
    let out = validate(&dir, &[])
        .failure()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8_lossy(&out);
    assert!(
        out.contains(".armadai/flows/f.yaml:2:15: error[unknown-agent]"),
        "{out}"
    );
    assert!(out.contains("1 error(s), 0 warning(s)"), "{out}");
}

#[test]
fn warnings_alone_exit_0() {
    let dir = project(&[
        M,
        DEV,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: dev }\n  b: { agent: dev }\nlinks:\n  - { from: a, to: b }\n  - { from: b, to: a }\n",
        ),
    ]);
    let out = validate(&dir, &[]).success().get_output().stdout.clone();
    let out = String::from_utf8_lossy(&out);
    assert!(out.contains("warning[no-entry-step]"), "{out}");
    assert!(out.contains("0 error(s), 1 warning(s)"), "{out}");
}

#[test]
fn json_output_is_pure_json() {
    let dir = project(&[M, BAD_FLOW]);
    let out = validate(&dir, &["--json"])
        .failure()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let v: Vec<serde_json::Value> = serde_json::from_slice(&out).unwrap();
    assert_eq!(v.len(), 1);
    for k in [
        "severity", "code", "file", "path", "line", "column", "message", "hint",
    ] {
        assert!(v[0].get(k).is_some(), "{k} in {}", v[0]);
    }
    assert_eq!(v[0]["code"], "unknown-agent");
    assert_eq!(v[0]["severity"], "error");
    assert_eq!(v[0]["line"], 2);
    assert_eq!(v[0]["column"], 15);
}

#[test]
fn json_of_a_valid_project_is_an_empty_array() {
    let dir = project(&[M, DEV]);
    let out = validate(&dir, &["--json"])
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(out).unwrap().trim(), "[]");
}

const INVITE: &str = "This project uses the legacy format. Run `armadai migrate` to convert it.";

#[test]
fn legacy_project_output_is_unchanged_plus_invitation() {
    let dir = project(&[("armadai.yaml", "agents: []\n")]);
    let out = validate(&dir, &[]).get_output().stdout.clone();
    let out = String::from_utf8_lossy(&out);
    assert!(out.contains("Validating project config at:"), "{out}");
    assert!(out.trim_end().ends_with(INVITE), "{out}");
}

#[test]
fn new_format_never_shows_the_invitation() {
    let dir = project(&[M]);
    let out = validate(&dir, &[]).success().get_output().stdout.clone();
    assert!(!String::from_utf8_lossy(&out).contains("legacy format"));
}

#[test]
fn json_on_a_legacy_project_warns_on_stderr_only() {
    let dir = project(&[("armadai.yaml", "agents: []\n")]);
    let a = validate(&dir, &["--json"]);
    let out = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    let err = String::from_utf8_lossy(&a.get_output().stderr).into_owned();
    assert!(
        err.contains("--json only applies to new-format projects"),
        "{err}"
    );
    assert!(!out.contains("--json"), "{out}");
    assert!(out.trim_end().ends_with(INVITE), "{out}");
}
