//! `armadai __record-flow-event`, run as the real binary the way a Claude Code
//! hook runs it: JSON on stdin, exit code 0 always, nothing on stdout.

use std::path::Path;

use assert_cmd::Command;

fn project(dir: &Path) {
    std::fs::create_dir_all(dir.join(".armadai")).unwrap();
    std::fs::write(dir.join(".armadai/project.yaml"), "schema_version: 1\n").unwrap();
}

/// The doc's `SubagentStop` example, with a prompt-like text in the reply.
fn stop(session: &str, cwd: &Path) -> String {
    serde_json::json!({
        "session_id": session,
        "transcript_path": "/x.jsonl",
        "cwd": cwd.to_string_lossy(),
        "permission_mode": "default",
        "hook_event_name": "SubagentStop",
        "stop_hook_active": false,
        "agent_id": "def456",
        "agent_type": "Explore",
        "agent_transcript_path": "/y.jsonl",
        "last_assistant_message": "SECRET reply text.\nSTATE: approved",
        "background_tasks": [],
        "session_crons": [],
    })
    .to_string()
}

fn record(input: &str, home: &Path) -> std::process::Output {
    record_in(input, home, home)
}

/// Run from `dir`: a payload without `cwd` must not fall back to the process's.
fn record_in(input: &str, home: &Path, dir: &Path) -> std::process::Output {
    Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(dir)
        .arg("__record-flow-event")
        .env("HOME", home)
        .env("ARMADAI_CONFIG_DIR", home.join("cfg"))
        .env("RUST_LOG", "armadai=debug")
        .write_stdin(input.to_string())
        .output()
        .unwrap()
}

fn assert_silent_success(out: &std::process::Output) {
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

const LINE: &str =
    r#"{"event":"stop","agent_id":"def456","agent_type":"Explore","state":"approved"}"#;

/// The log lines with their clock-dependent `ts` replaced by `0`; the rest of
/// each raw line is compared exactly.
fn without_ts(log: &str) -> Vec<String> {
    log.lines()
        .map(|l| {
            let ts = serde_json::from_str::<serde_json::Value>(l).unwrap()["ts"]
                .as_u64()
                .unwrap();
            assert!(ts > 1_700_000_000, "{l}");
            l.replacen(&format!(r#"{{"ts":{ts},"#), "{", 1)
        })
        .collect()
}

#[test]
fn a_hook_from_a_subdirectory_logs_into_the_project() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    project(dir.path());
    let sub = dir.path().join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();

    let out = record(&stop("s1", &sub), home.path());

    assert_silent_success(&out);
    let log = std::fs::read_to_string(dir.path().join(".armadai/state/claude/s1.jsonl")).unwrap();
    // Exact line: neither the reply text nor anything else leaks in.
    assert_eq!(without_ts(&log), vec![LINE.to_string()]);
    assert!(!sub.join(".armadai").exists());
}

#[test]
fn a_worktree_session_logs_into_the_main_checkout() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let main = root.path().join("repo");
    std::fs::create_dir_all(main.join(".git")).unwrap();
    project(&main);
    let wt = root.path().join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(
        wt.join(".git"),
        format!("gitdir: {}/.git/worktrees/wt\n", main.to_string_lossy()),
    )
    .unwrap();

    assert_silent_success(&record(&stop("s1", &wt), home.path()));

    let log = std::fs::read_to_string(main.join(".armadai/state/claude/s1.jsonl")).unwrap();
    assert_eq!(without_ts(&log), vec![LINE.to_string()]);
}

#[test]
fn two_sessions_log_into_two_files_by_appending() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());

    for s in ["s1", "s2", "s1"] {
        assert_silent_success(&record(&stop(s, dir.path()), home.path()));
    }

    let state = dir.path().join(".armadai/state/claude");
    let read = |s: &str| without_ts(&std::fs::read_to_string(state.join(s)).unwrap());
    assert_eq!(read("s1.jsonl"), vec![LINE.to_string(), LINE.to_string()]);
    assert_eq!(read("s2.jsonl"), vec![LINE.to_string()]);
    let mut names: Vec<_> = std::fs::read_dir(&state)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["s1.jsonl", "s2.jsonl"]);
}

#[test]
fn bad_input_or_no_project_records_nothing_and_succeeds() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(outside.path().join(".git")).unwrap();

    for input in [
        "{not json".to_string(),
        stop("../evil", dir.path()),
        stop("s1", outside.path()),
    ] {
        assert_silent_success(&record(&input, home.path()));
    }

    assert!(!dir.path().join(".armadai/state").exists());
    assert!(!outside.path().join(".armadai").exists());
    assert!(!dir.path().join(".armadai/evil.jsonl").exists());
}

#[test]
fn a_missing_cwd_session_or_directory_records_nothing_and_succeeds() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(outside.path().join(".git")).unwrap();
    let without = |key: &str| {
        let mut v: serde_json::Value = serde_json::from_str(&stop("s1", dir.path())).unwrap();
        v.as_object_mut().unwrap().remove(key);
        v.to_string()
    };

    for input in [
        without("session_id"),
        without("cwd"),
        stop("s1", &outside.path().join("gone/deeper")),
    ] {
        assert_silent_success(&record_in(&input, home.path(), dir.path()));
    }

    assert!(!dir.path().join(".armadai/state").exists());
    assert!(!outside.path().join("gone").exists());
    assert!(!outside.path().join(".armadai").exists());
}

#[cfg(unix)]
#[test]
fn a_linked_state_directory_records_nothing_and_succeeds() {
    for linked in [".armadai/state", ".armadai/state/claude"] {
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());
        let outside = tempfile::tempdir().unwrap();
        let link = dir.path().join(linked);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();

        assert_silent_success(&record(&stop("s1", dir.path()), home.path()));

        let left: Vec<_> = std::fs::read_dir(outside.path()).unwrap().collect();
        assert!(left.is_empty(), "{linked}: {left:?}");
    }
}

#[cfg(unix)]
#[test]
fn a_linked_log_file_records_nothing_and_succeeds() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("mine");
    std::fs::write(&target, "mine\n").unwrap();
    let logs = dir.path().join(".armadai/state/claude");
    std::fs::create_dir_all(&logs).unwrap();
    std::os::unix::fs::symlink(&target, logs.join("s1.jsonl")).unwrap();

    assert_silent_success(&record(&stop("s1", dir.path()), home.path()));

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine\n");
}

/// The doc's `UserPromptExpansion` example (a project command with arguments).
fn expansion(kind: &str, cwd: &Path) -> String {
    serde_json::json!({
        "session_id": "s1",
        "transcript_path": "/x.jsonl",
        "cwd": cwd.to_string_lossy(),
        "permission_mode": "default",
        "hook_event_name": "UserPromptExpansion",
        "expansion_type": kind,
        "command_name": "bugfix",
        "command_args": "secret arg",
        "command_source": "project",
        "prompt": "/bugfix secret arg",
    })
    .to_string()
}

/// A hook's stdout is added to the model's context for this event: both
/// streams must stay empty, with the tracing filter unset.
fn record_quietly(input: &str, home: &Path) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(home)
        .arg("__record-flow-event")
        .env("HOME", home)
        .env("ARMADAI_CONFIG_DIR", home.join("cfg"))
        .env_remove("RUST_LOG")
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    (out.status.code(), out.stdout, out.stderr)
}

#[test]
fn a_typed_command_is_recorded_silently() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    project(dir.path());
    let silent = (Some(0), Vec::new(), Vec::new());
    let log_path = dir.path().join(".armadai/state/claude/s1.jsonl");
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };

    // the doc's payload: the line is the name and nothing else
    let before = now();
    assert_eq!(
        record_quietly(&expansion("slash_command", dir.path()), home.path()),
        silent
    );
    let after = now();
    let log = std::fs::read_to_string(&log_path).unwrap();
    let ts = serde_json::from_str::<serde_json::Value>(log.trim_end()).unwrap()["ts"]
        .as_u64()
        .unwrap();
    assert!(
        (before..=after).contains(&ts),
        "{ts} not in {before}..={after}"
    );
    assert_eq!(
        log,
        format!("{{\"ts\":{ts},\"event\":\"command\",\"command\":\"bugfix\"}}\n")
    );
    assert!(!log.contains("secret") && !log.contains("/bugfix"));

    // an MCP prompt, invalid JSON and a payload outside any project: silent, nothing recorded
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(outside.path().join(".git")).unwrap();
    for input in [
        expansion("mcp_prompt", dir.path()),
        "{not json".to_string(),
        expansion("slash_command", outside.path()),
    ] {
        assert_eq!(record_quietly(&input, home.path()), silent, "{input}");
    }
    assert_eq!(std::fs::read_to_string(&log_path).unwrap(), log);
    assert!(!outside.path().join(".armadai").exists());
}
