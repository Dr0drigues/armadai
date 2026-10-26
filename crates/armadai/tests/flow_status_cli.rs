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
        self.raw_log(session, &event_lines(events), mtime);
    }

    /// Write a session log made of the given JSON lines, last modified at `mtime`.
    fn raw_log(&self, session: &str, text: &str, mtime: u64) {
        let dir = self.root().join(".armadai/state/claude");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{session}.jsonl"));
        std::fs::write(&path, text).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime))
            .unwrap();
    }

    /// The two flows of issue #527 (`evolution` and `bugfix`, sharing their
    /// entry step, `u` holding `ui` in one and `fix-ui` in the other).
    fn issue_project(&self) {
        let a = self.root().join(".armadai");
        for agent in ["an", "u", "t", "r", "v"] {
            std::fs::write(
                a.join(format!("agents/{agent}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        std::fs::write(a.join("flows/evolution.yaml"), EVOLUTION).unwrap();
        std::fs::write(a.join("flows/bugfix.yaml"), BUGFIX).unwrap();
    }

    /// Run `armadai __record-flow-event` the way the hook does:
    /// `(exit code, stdout, stderr)`.
    fn hook(&self, payload: &str) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .write_stdin(payload)
            .arg("__record-flow-event")
            .timeout(std::time::Duration::from_secs(20))
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
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

const EVOLUTION: &str = "steps:\n  analyse: { agent: an }\n  ui: { agent: u }\n  test: { agent: t }\n  review: { agent: r }\n  validate: { agent: v }\nlinks:\n  - { from: analyse, to: ui }\n  - { from: ui, to: test }\n  - { from: test, to: review }\n  - { from: review, to: validate }\n";
const BUGFIX: &str = "steps:\n  analyse: { agent: an }\n  test: { agent: t }\n  fix-ui: { agent: u }\n  review: { agent: r }\n  validate: { agent: v }\nlinks:\n  - { from: analyse, to: test }\n  - { from: test, to: fix-ui }\n  - { from: fix-ui, to: review }\n  - { from: review, to: validate }\n";

/// The journal lines of `events`, one JSON object each.
fn event_lines(events: &[Ev]) -> String {
    (events.iter())
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
        .collect()
}

/// Scenario 1 of issue #527: `t` starts, `u` starts before `t` stops.
fn issue_s1() -> Vec<Ev> {
    vec![
        (T0, true, "i1", "an", None),
        (T0 + 10, false, "i1", "an", None),
        (T0 + 20, true, "i3", "t", None),
        (T0 + 30, true, "i2", "u", None),
        (T0 + 40, false, "i3", "t", None),
        (T0 + 50, false, "i2", "u", None),
        (T0 + 60, true, "i4", "r", None),
        (T0 + 70, false, "i4", "r", None),
        (T0 + 80, true, "i5", "v", None),
        (T0 + 90, false, "i5", "v", None),
    ]
}

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

const LOOPING: &str = "session s1 (UTC)\n\
\n\
deliver  in progress\n\
\x20 ✓ design    done\n\
\x20 ✓ build     done\n\
\x20 ● guard     running\n\
\x20 ○ validate  pending\n\
\x20 build → guard : 2\n\
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
            "session s1 (UTC)\n\nship  in progress\n  ● release  running\n"
        )
    );
    let (_, out, _) = env.run(&env.root(), &["flow", "status", "deliver"]);
    assert_eq!(out, LOOPING);
}

#[test]
fn list_without_any_session_says_so() {
    let env = Env::new();
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "--list"]);
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
fn outside_any_project_it_says_so() {
    let env = Env::new();
    let (code, out, err) = env.run(env.dir.path(), &["flow", "status"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(1),
            "",
            "Error: no ArmadAI project found (.armadai/project.yaml); run `armadai init --project`, or `armadai migrate` for a legacy project\n"
        )
    );
}

#[test]
fn an_unknown_flow_in_a_project_without_flow_says_so() {
    let env = Env::new();
    std::fs::remove_file(env.root().join(".armadai/flows/deliver.yaml")).unwrap();
    std::fs::remove_file(env.root().join(".armadai/flows/ship.yaml")).unwrap();
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "x"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(1),
            "",
            "Error: unknown flow `x`: the project has no flow\n"
        )
    );
}

#[test]
fn a_session_id_that_could_leave_the_logs_is_refused() {
    let env = Env::new();
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "--session", "../x"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(1),
            "",
            "Error: `../x` is not a session id; see `armadai flow status --list`\n"
        )
    );
}

#[test]
fn list_takes_no_flow_nor_session() {
    let env = Env::new();
    looping(&env, "s1", T0);
    for args in [
        vec!["flow", "status", "deliver", "--list"],
        vec!["flow", "status", "--list", "--session", "s1"],
    ] {
        let (code, out, _) = env.run(&env.root(), &args);
        assert_eq!((code, out.as_str()), (Some(2), ""), "{args:?}");
    }
}

/// `link` says when it patched `.armadai/.gitignore`, and only then.
#[test]
fn link_reports_the_gitignore_it_patched() {
    let env = Env::new();
    let ignore = env
        .root()
        .canonicalize()
        .unwrap()
        .join(".armadai/.gitignore");
    std::fs::write(&ignore, "mine\n").unwrap();
    let line = format!("  patched {}\n", ignore.display());
    let (code, out, _) = env.run(&env.root(), &["link", "--target", "claude"]);
    assert_eq!(code, Some(0));
    assert_eq!(out.matches(&line).count(), 1, "{out}");
    assert_eq!(std::fs::read_to_string(&ignore).unwrap(), "mine\nstate/\n");
    let (_, out, _) = env.run(&env.root(), &["link", "--target", "claude"]);
    assert!(!out.contains(&line), "{out}");
}

const HOOK_PAYLOAD: &str = r#"{"session_id":"s1","cwd":"PROJECT","hook_event_name":"UserPromptExpansion","expansion_type":"slash_command","command_name":"bugfix","command_args":"secret arg","prompt":"/bugfix secret arg"}"#;

/// The command a user types goes through the real hook, into the journal, and
/// decides the flow `flow status` shows (criteria 1, 2 and 7).
#[test]
fn a_typed_command_through_the_hook_attributes_the_run() {
    let env = Env::new();
    env.issue_project();
    let payload = HOOK_PAYLOAD.replace("PROJECT", &env.root().to_string_lossy());
    let (code, out, err) = env.hook(&payload);
    assert_eq!((code, out.as_str(), err.as_str()), (Some(0), "", ""));
    // the hook stamped the real time, later than every event below: the order
    // of the file is what counts
    let path = env.root().join(".armadai/state/claude/s1.jsonl");
    let recorded = std::fs::read_to_string(&path).unwrap();
    assert_eq!(recorded.lines().count(), 1, "{recorded}");
    let (ts, line) = (recorded.strip_prefix("{\"ts\":"))
        .and_then(|r| r.split_once(','))
        .unwrap();
    assert!(ts.bytes().all(|b| b.is_ascii_digit()), "{recorded}");
    assert_eq!(line, "\"event\":\"command\",\"command\":\"bugfix\"}\n");
    std::fs::write(&path, recorded + &event_lines(&issue_s1())).unwrap();
    let (code, out, err) = env.run(&env.root(), &["flow", "status"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(0),
            "session s1 (UTC)\n\
             \n\
             bugfix  finished\n\
             \x20 ✓ analyse   done\n\
             \x20 ✓ test      done\n\
             \x20 ✓ fix-ui    done\n\
             \x20 ✓ review    done\n\
             \x20 ✓ validate  done\n\
             \x20 09:15:20  analyse → test  (done)\n\
             \x20 09:15:40  test → fix-ui  (done)\n\
             \x20 09:16:00  fix-ui → review  (done)\n\
             \x20 09:16:20  review → validate  (done)\n",
            ""
        )
    );
    // the same events without the command line: the flow is still inferred
    env.raw_log("s2", &event_lines(&issue_s1()), T0 + 10);
    let (code, out, _) = env.run(&env.root(), &["flow", "status", "--session", "s2"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        out,
        "session s2 (UTC)\n\nbugfix  in progress\n  ✓ analyse   done\n  ✓ test      done\n  ○ fix-ui    pending\n  ○ review    pending\n  ○ validate  pending\n  09:15:20  analyse → test  (done)\n"
    );
}

/// A session made of a typed command only is not a run (`--list`).
#[test]
fn list_does_not_count_a_session_with_only_a_command() {
    let env = Env::new();
    env.issue_project(); // `bugfix` must be a flow, or the line is no boundary at all
    looping(&env, "s1", T0 + 3600);
    env.raw_log(
        "c",
        "{\"ts\":1792660500,\"event\":\"command\",\"command\":\"bugfix\"}\n",
        T0,
    );
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "--list"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(0),
            "SESSION  LAST EVENT (UTC)     RUNS\n\
             s1       2026-10-22 10:15:00  1\n\
             c        2026-10-22 09:15:00  0\n",
            ""
        )
    );
}

/// Scenario 2 of issue #527 fits both flows: without a command it is shown as
/// `evolution`, and the command `/bugfix`, through the hook, makes it `bugfix`.
#[test]
fn the_hook_command_decides_between_two_flows_the_events_fit() {
    let env = Env::new();
    env.issue_project();
    let events: Vec<Ev> = vec![
        (T0, true, "i1", "an", None),
        (T0 + 10, false, "i1", "an", None),
        (T0 + 20, true, "i2", "u", None),
        (T0 + 21, true, "i3", "t", None),
        (T0 + 30, false, "i2", "u", None),
        (T0 + 40, false, "i3", "t", None),
        (T0 + 60, true, "i4", "r", None),
        (T0 + 70, false, "i4", "r", None),
        (T0 + 80, true, "i5", "v", None),
        (T0 + 90, false, "i5", "v", None),
    ];
    env.raw_log("s2", &event_lines(&events), T0 + 10);
    let (code, out, _) = env.run(&env.root(), &["flow", "status", "--session", "s2"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        out,
        "session s2 (UTC)\n\nevolution  in progress\n  ✓ analyse   done\n  ✓ ui        done\n  ○ test      pending\n  ○ review    pending\n  ○ validate  pending\n  09:15:20  analyse → ui  (done)\n"
    );
    let payload = HOOK_PAYLOAD
        .replace("PROJECT", &env.root().to_string_lossy())
        .replace("\"s1\"", "\"s3\"");
    assert_eq!(env.hook(&payload), (Some(0), String::new(), String::new()));
    let path = env.root().join(".armadai/state/claude/s3.jsonl");
    let recorded = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, recorded + &event_lines(&events)).unwrap();
    let (code, out, _) = env.run(&env.root(), &["flow", "status", "--session", "s3"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        out,
        "session s3 (UTC)\n\nbugfix  finished\n  ✓ analyse   done\n  ✓ test      done\n  ✓ fix-ui    done\n  ✓ review    done\n  ✓ validate  done\n  09:15:21  analyse → test  (done)\n  09:15:40  test → fix-ui  (done)\n  09:16:00  fix-ui → review  (done)\n  09:16:20  review → validate  (done)\n"
    );
}
