//! `armadai flow status`: reads the session logs the hooks wrote, from the
//! project root the hooks resolve (subdirectory, git worktree).

use std::path::{Path, PathBuf};

use assert_cmd::Command;

const DELIVER: &str = "steps:\n  design: { agent: architect }\n  build: { agent: dev }\n  guard: { agent: qa, states: [approved, rejected] }\n  validate: { agent: ops }\nlinks:\n  - { from: design, to: build }\n  - { from: build, to: guard }\n  - { from: guard, to: build, when: rejected }\n  - { from: guard, to: validate, when: approved }\n";

/// 2026-10-22 09:15:00 UTC
const T0: u64 = 1_792_660_500;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("project/.armadai");
        std::fs::create_dir_all(a.join("agents")).unwrap();
        std::fs::create_dir_all(a.join("flows")).unwrap();
        std::fs::write(a.join("project.yaml"), "schema_version: 1\n").unwrap();
        for agent in ["architect", "dev", "qa", "ops", "rel"] {
            std::fs::write(
                a.join(format!("agents/{agent}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        std::fs::write(a.join("flows/deliver.yaml"), DELIVER).unwrap();
        std::fs::write(
            a.join("flows/ship.yaml"),
            "steps:\n  release: { agent: rel }\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("project/.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// Write a session log, one event per `(ts, start?, agent_id, agent_type,
    /// state)`, last modified at `mtime`.
    fn log(&self, session: &str, events: &[Ev], mtime: u64) {
        let dir = self.root().join(".armadai/state/claude");
        std::fs::create_dir_all(&dir).unwrap();
        let text: String = (events.iter())
            .map(|(ts, start, id, agent, state)| {
                let line = serde_json::json!({
                    "ts": ts,
                    "event": if *start { "start" } else { "stop" },
                    "agent_id": id,
                    "agent_type": agent,
                    "state": state,
                });
                format!("{line}\n")
            })
            .collect();
        let path = dir.join(format!("{session}.jsonl"));
        std::fs::write(&path, text).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime))
            .unwrap();
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
            .timeout(std::time::Duration::from_secs(20))
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

type Ev = (u64, bool, &'static str, &'static str, Option<&'static str>);

/// design, build, guard (rejected), build, then guard running.
fn looping(env: &Env, session: &str, mtime: u64) {
    let events: Vec<Ev> = vec![
        (T0 + 10, true, "a1", "architect", None),
        (T0 + 11, false, "a1", "architect", None),
        (T0 + 20, true, "a2", "dev", None),
        (T0 + 21, false, "a2", "dev", None),
        (T0 + 30, true, "a3", "qa", None),
        (T0 + 31, false, "a3", "qa", Some("rejected")),
        (T0 + 40, true, "a4", "dev", None),
        (T0 + 41, false, "a4", "dev", None),
        (T0 + 50, true, "a5", "qa", None),
    ];
    env.log(session, &events, mtime);
}

const LOOPING: &str = "session s1\n\
\n\
deliver  in progress\n\
\x20 ✓ design    done\n\
\x20 ✓ build     done\n\
\x20 ● guard     running\n\
\x20 ○ validate  pending\n\
\x20 build ⇄ guard : 2\n\
\x20 09:15:20  design → build  (done)\n\
\x20 09:15:30  build → guard  (done)\n\
\x20 09:15:40  guard → build  (rejected)\n\
\x20 09:15:50  build → guard  (done)\n";

#[test]
fn no_session_yet_is_a_message_and_success() {
    let env = Env::new();
    let (code, out, err) = env.run(&env.root(), &["flow", "status"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(0),
            "no flow activity recorded yet (link installs the hooks; start a flow in Claude Code)\n",
            ""
        )
    );
}

#[test]
fn the_latest_session_is_shown_from_a_subdirectory() {
    let env = Env::new();
    looping(&env, "s1", T0 + 100);
    env.log("s0", &[(T0, true, "x", "architect", None)], T0);
    let sub = env.root().join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let (code, out, err) = env.run(&sub, &["flow", "status"]);
    assert_eq!((code, out.as_str(), err.as_str()), (Some(0), LOOPING, ""));
}

#[test]
fn a_worktree_reads_the_logs_of_its_main_checkout() {
    let env = Env::new();
    looping(&env, "s1", T0);
    let wt = env.dir.path().join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(
        wt.join(".git"),
        format!(
            "gitdir: {}/.git/worktrees/wt\n",
            env.root().to_string_lossy()
        ),
    )
    .unwrap();
    let (code, out, _) = env.run(&wt, &["flow", "status", "deliver", "--session", "s1"]);
    assert_eq!((code, out.as_str()), (Some(0), LOOPING));
}

#[test]
fn an_older_session_is_chosen_by_id() {
    let env = Env::new();
    looping(&env, "s1", T0);
    env.log("s2", &[(T0, true, "x", "architect", None)], T0 + 100);
    let (code, out, _) = env.run(&env.root(), &["flow", "status", "--session", "s1"]);
    assert_eq!((code, out.as_str()), (Some(0), LOOPING));
}

#[test]
fn list_shows_every_session_with_its_date_and_runs() {
    let env = Env::new();
    looping(&env, "s1", T0 + 3600);
    env.log("empty", &[(T0, true, "x", "nobody", None)], T0);
    let (code, out, _) = env.run(&env.root(), &["flow", "status", "--list"]);
    assert_eq!(
        (code, out.as_str()),
        (
            Some(0),
            "SESSION  LAST EVENT (UTC)     RUNS\n\
             s1       2026-10-22 10:15:00  1\n\
             empty    2026-10-22 09:15:00  0\n"
        )
    );
}

#[test]
fn an_unknown_flow_or_session_says_what_to_do() {
    let env = Env::new();
    looping(&env, "s1", T0);
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "delivr"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(1),
            "",
            "Error: unknown flow `delivr`; flows: deliver, ship\n"
        )
    );
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "--session", "nope"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(1),
            "",
            "Error: no session `nope` recorded; see `armadai flow status --list`\n"
        )
    );
}

#[test]
fn a_flow_id_keeps_only_its_runs() {
    let env = Env::new();
    looping(&env, "s1", T0);
    // a run of `ship` opens alongside the one of `deliver`
    let path = env.root().join(".armadai/state/claude/s1.jsonl");
    let text = std::fs::read_to_string(&path).unwrap()
        + r#"{"ts":1792660560,"event":"start","agent_id":"r1","agent_type":"rel","state":null}"#
        + "\n";
    std::fs::write(&path, text).unwrap();
    let (code, out, _) = env.run(&env.root(), &["flow", "status", "ship"]);
    assert_eq!(
        (code, out.as_str()),
        (
            Some(0),
            "session s1\n\nship  in progress\n  ● release  running\n"
        )
    );
    let (_, out, _) = env.run(&env.root(), &["flow", "status", "deliver"]);
    assert_eq!(out, LOOPING);
}
