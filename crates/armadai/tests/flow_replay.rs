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
    replay_log(
        case,
        &std::fs::read_to_string(fixtures().join(case).join("log.jsonl")).unwrap(),
    )
}

/// The same, for another log of the case's project.
fn replay_log(case: &str, log: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    copy_dir(&fixtures().join(case).join("project"), &root);
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    let logs = root.join(".armadai/state/claude");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("s1.jsonl"), log).unwrap();
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

/// Without its marks, a guardian verdict of a task lands in the run of the
/// other flow that shares the agent; the marks put it back. This is what the
/// markers are for, and what the case must be able to show.
#[test]
fn the_misplaced_case_misplaces_its_verdicts_without_marks() {
    let case = "public-guard-misplaced";
    let log = std::fs::read_to_string(fixtures().join(case).join("log.jsonl")).unwrap();
    let plain: String = (log.lines())
        .filter(|l| !l.contains("\"event\":\"mark\""))
        .map(|l| format!("{l}\n"))
        .collect();
    let steps = |doc: &Value, flow: &str| -> Vec<(String, String)> {
        let run = doc["runs"].as_array().unwrap().iter();
        let run = run
            .filter(|r| r["flow"] == flow)
            .next_back()
            .unwrap()
            .clone();
        let steps = run["steps"].as_array().unwrap().iter();
        steps
            .map(|s| {
                let state = s["state"].as_str().unwrap_or("-");
                (s["step"].as_str().unwrap().into(), state.into())
            })
            .collect()
    };
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    let unmarked = replay_log(case, &plain);
    assert_eq!(
        steps(&unmarked, "lot"),
        vec![pair("spec", "done"), pair("review", "rejected")]
    );
    assert_eq!(
        steps(&unmarked, "task"),
        vec![pair("build-cli", "done"), pair("guard", "-")]
    );
    let marked = replay(case);
    assert_eq!(
        steps(&marked, "lot"),
        vec![pair("spec", "done"), pair("review", "-")]
    );
    assert_eq!(
        steps(&marked, "task"),
        vec![pair("build-cli", "done"), pair("guard", "approved")]
    );
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
const REFS: &[&str] = &["feature/menu", "feature/review", "feature/other"];
/// The keys a journal line may carry.
const KEYS: &[&str] = &[
    "ts",
    "event",
    "agent_id",
    "agent_type",
    "state",
    "flow",
    "step",
    "lot",
    "ref",
    "command",
];

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

/// `(file under root, what)` of everything the fixtures carry that is not
/// generic, sorted.
fn outside_the_lists(root: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut all = Vec::new();
    files(root, &mut all);
    for path in all {
        let name = path.strip_prefix(root).unwrap().display().to_string();
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
            "agents" if !AGENTS.contains(&stem.as_str()) => {
                found.push((name.clone(), format!("agent file {stem}")));
            }
            "flows" if !FLOWS.contains(&stem.as_str()) => {
                found.push((name.clone(), format!("flow file {stem}")));
            }
            _ => {}
        }
        if path.file_name().is_some_and(|n| n == "log.jsonl") {
            for line in text.lines() {
                let v: Value = serde_json::from_str(line).unwrap();
                for key in v.as_object().unwrap().keys() {
                    if !KEYS.contains(&key.as_str()) {
                        found.push((name.clone(), format!("key {key}")));
                    }
                }
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
    found.sort();
    found
}

#[test]
fn replay_fixtures_hold_nothing_private() {
    assert_eq!(outside_the_lists(&fixtures()), vec![]);
}

/// Each check bites: one planted value per check, all found, nothing else.
#[test]
fn the_allowlist_finds_each_planted_private_value() {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixtures(), dir.path());
    assert_eq!(outside_the_lists(dir.path()), vec![]);
    let case = dir.path().join("ref-line");
    let planted = [
        r#"{"ts":1,"event":"stop","agent_id":"zz9","agent_type":"acme-agent"}"#,
        r#"{"ts":2,"event":"mark","agent_type":"dev","flow":"acme-flow","step":"design"}"#,
        r#"{"ts":3,"event":"mark","agent_type":"dev","flow":"deliver","step":"acme-step"}"#,
        r#"{"ts":4,"event":"mark","agent_type":"dev","flow":"deliver","step":"build","lot":"acme-lot"}"#,
        r#"{"ts":5,"event":"stop","agent_id":"a1","agent_type":"dev","ref":"acme/ref"}"#,
        r#"{"ts":6,"event":"stop","agent_id":"a1","secret":"x"}"#,
    ];
    let log = case.join("log.jsonl");
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, text + &planted.join("\n") + "\n").unwrap();
    std::fs::write(case.join("note.txt"), "see /home/x").unwrap();
    std::fs::write(
        dir.path().join("unmarked-unchanged/note.txt"),
        "see /Users/x",
    )
    .unwrap();
    std::fs::write(case.join("project/.armadai/agents/acme.md"), "x").unwrap();
    std::fs::write(case.join("project/.armadai/flows/acme.yaml"), "x").unwrap();
    let l = "ref-line/log.jsonl".to_string();
    let pair = |f: &str, w: &str| (f.to_string(), w.to_string());
    let mut want = vec![
        pair(&l, "agent_id=zz9"),
        pair(&l, "agent_type=acme-agent"),
        pair(&l, "flow=acme-flow"),
        pair(&l, "step=acme-step"),
        pair(&l, "lot=acme-lot"),
        pair(&l, "ref=acme/ref"),
        pair(&l, "key secret"),
        pair("ref-line/note.txt", "/home/"),
        pair("unmarked-unchanged/note.txt", "/Users/"),
        pair(
            "ref-line/project/.armadai/agents/acme.md",
            "agent file acme",
        ),
        pair(
            "ref-line/project/.armadai/flows/acme.yaml",
            "flow file acme",
        ),
    ];
    want.sort();
    assert_eq!(outside_the_lists(dir.path()), want);
}
