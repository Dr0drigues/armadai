//! Replays anonymised session logs through `armadai flow status --json` and
//! compares the whole document with the one stored beside the log. The crate
//! is binary-only, so the replay goes through the binary.
//!
//! One case per directory of `tests/fixtures/flow-replay/`: `project/.armadai/`
//! (the project), `log.jsonl` (the session log) and `expected.json`.
//! `unmarked-unchanged` holds the output of the binary before the marker work,
//! which has no `lot` and `ref` keys: the current output may differ from it by
//! those two keys at `null`, nothing else.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flow-replay")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// `flow status --json` of the case's log (session `s1`), parsed.
fn replay(case: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    copy_dir(&fixtures().join(case).join("project"), &root);
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    let logs = root.join(".armadai/state/claude");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::copy(
        fixtures().join(case).join("log.jsonl"),
        logs.join("s1.jsonl"),
    )
    .unwrap();
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(&root)
        .env("ARMADAI_CONFIG_DIR", dir.path().join("config"))
        .env("HOME", dir.path())
        .env("NO_COLOR", "1")
        .args(["flow", "status", "--session", "s1", "--json"])
        .timeout(std::time::Duration::from_secs(20))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn expected(case: &str) -> Value {
    let text = std::fs::read_to_string(fixtures().join(case).join("expected.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn cases() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn every_marked_case_replays_to_its_expected_document() {
    let marked: Vec<String> = cases()
        .into_iter()
        .filter(|c| c != "unmarked-unchanged")
        .collect();
    assert_eq!(marked.len(), 6, "{marked:?}");
    for case in &marked {
        assert_eq!(replay(case), expected(case), "case {case}");
    }
}

/// Without a marker the output is the one before the lot, plus `lot` and `ref`
/// at `null` (nothing else moved).
#[test]
fn an_unmarked_log_replays_as_before_the_lot() {
    let mut got = replay("unmarked-unchanged");
    for run in got["runs"].as_array_mut().unwrap() {
        assert_eq!(
            run.as_object_mut().unwrap().remove("lot"),
            Some(Value::Null)
        );
        for step in run["steps"].as_array_mut().unwrap() {
            assert_eq!(
                step.as_object_mut().unwrap().remove("ref"),
                Some(Value::Null)
            );
        }
    }
    assert_eq!(got, expected("unmarked-unchanged"));
}

const AGENTS: &[&str] = &[
    "lead",
    "analyst",
    "framer",
    "architect",
    "dev",
    "qa",
    "ops",
    "reviewer",
    "validator",
    "guardian",
    "cli-specialist",
    "schema-specialist",
    "adapter-specialist",
];
const FLOWS: &[&str] = &["deliver", "hotfix", "lot", "task"];
const STEPS: &[&str] = &[
    "design",
    "build",
    "guard",
    "validate",
    "patch",
    "verify",
    "spec",
    "frame",
    "plan",
    "tasks",
    "review",
    "build-cli",
    "build-schema",
    "build-adapter",
];
const LOTS: &[&str] = &["x", "y", "menu", "12", "a"];
const REFS: &[&str] = &["feature/menu", "feature/other"];

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// `(what, value)` of every identifier the fixtures carry that is not generic.
fn outside_the_lists(root: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut all = Vec::new();
    files(root, &mut all);
    for path in all {
        let name = path.display().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        for bad in ["/Users/", "/home/"] {
            if text.contains(bad) {
                found.push((name.clone(), bad.to_string()));
            }
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let parent = path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy();
        match parent.as_ref() {
            "agents" if !AGENTS.contains(&stem.as_str()) => found.push((name.clone(), stem)),
            "flows" if !FLOWS.contains(&stem.as_str()) => found.push((name.clone(), stem)),
            _ => {}
        }
        if path.file_name().is_some_and(|n| n == "log.jsonl") {
            for line in text.lines() {
                let v: Value = serde_json::from_str(line).unwrap();
                let mut check = |key: &str, list: &dyn Fn(&str) -> bool| {
                    if let Some(s) = v.get(key).and_then(Value::as_str)
                        && !s.is_empty()
                        && !list(s)
                    {
                        found.push((name.clone(), format!("{key}={s}")));
                    }
                };
                check("agent_type", &|s| AGENTS.contains(&s));
                check("flow", &|s| FLOWS.contains(&s));
                check("step", &|s| STEPS.contains(&s));
                check("lot", &|s| LOTS.contains(&s));
                check("ref", &|s| REFS.contains(&s));
                // a1, a2, ...
                check("agent_id", &|s| {
                    s.strip_prefix('a')
                        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                });
            }
        }
    }
    found
}

#[test]
fn replay_fixtures_hold_nothing_private() {
    assert_eq!(outside_the_lists(&fixtures()), vec![]);
}

/// The check itself bites: a path of a machine or a name off the lists is found.
#[test]
fn the_allowlist_finds_a_planted_private_value() {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixtures(), dir.path());
    assert_eq!(outside_the_lists(dir.path()), vec![]);
    let log = dir.path().join("ref-line/log.jsonl");
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, text.replacen("\"dev\"", "\"acme-secret-agent\"", 1)).unwrap();
    std::fs::write(
        dir.path().join("ref-line/project/.armadai/project.yaml"),
        "x: /Users/x\n",
    )
    .unwrap();
    let found = outside_the_lists(dir.path());
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .any(|(_, v)| v == "agent_type=acme-secret-agent")
    );
    assert!(found.iter().any(|(_, v)| v == "/Users/"));
}
