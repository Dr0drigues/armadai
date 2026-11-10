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
            .env_remove("RUST_LOG")
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
    assert_eq!(
        std::fs::read_to_string(&ignore).unwrap(),
        "mine\nstate/\nvars.local.yaml\n"
    );
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

#[test]
fn json_prints_the_session_as_the_schema_says() {
    let env = Env::new();
    looping(&env, "s1", T0 + 100);
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "--json"]);
    assert_eq!((code, err.as_str()), (Some(0), ""));
    assert!(out.ends_with("}\n"), "one document and a newline: {out:?}");
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    let step = |s: &str, agent: &str, mark: &str, sign: &str, state: Option<&str>| serde_json::json!({"step": s, "agent": agent, "mark": mark, "sign": sign, "state": state, "note": null, "ref": null});
    let tr = |at: &str, from: &str, to: &str, state: &str| serde_json::json!({"at": at, "from": from, "to": to, "state": state});
    assert_eq!(
        got,
        serde_json::json!({
            "schema": "armadai.flow-status/1",
            "session": {
                "id": "s1",
                "started_at": "2026-10-22T09:15:10Z",
                "last_event_at": "2026-10-22T09:15:50Z"
            },
            "runs": [{
                "flow": "deliver",
                "lot": null,
                "status": "in-progress",
                "steps": [
                    step("design", "architect", "done", "✓", Some("done")),
                    step("build", "dev", "done", "✓", Some("done")),
                    step("guard", "qa", "running", "●", None),
                    step("validate", "ops", "pending", "○", None),
                ],
                "loops": [{"from": "build", "to": "guard", "times": 2}],
                "transitions": [
                    tr("2026-10-22T09:15:20Z", "design", "build", "done"),
                    tr("2026-10-22T09:15:30Z", "build", "guard", "done"),
                    tr("2026-10-22T09:15:40Z", "guard", "build", "rejected"),
                    tr("2026-10-22T09:15:50Z", "build", "guard", "done"),
                ]
            }]
        })
    );
}

#[test]
fn json_without_any_log_or_with_an_unknown_session_still_succeeds() {
    let env = Env::new();
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "--json"]);
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (code, err.as_str(), got),
        (
            Some(0),
            "",
            serde_json::json!({"schema": "armadai.flow-status/1", "session": null, "runs": []})
        )
    );
    let (code, out, err) = env.run(
        &env.root(),
        &["flow", "status", "--json", "--session", "inconnue"],
    );
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (code, err.as_str(), got),
        (
            Some(0),
            "",
            serde_json::json!({
                "schema": "armadai.flow-status/1",
                "session": {"id": "inconnue", "started_at": null, "last_event_at": null},
                "runs": []
            })
        )
    );
}

#[test]
fn json_does_not_combine_with_watch_or_list() {
    let env = Env::new();
    for other in ["--watch", "--list"] {
        let (code, out, err) = env.run(&env.root(), &["flow", "status", "--json", other]);
        assert_eq!(
            (code, out.as_str(), err.contains("cannot be used with")),
            (Some(2), "", true),
            "--json {other}: {err}"
        );
    }
}

impl Env {
    /// `armadai flow status --line ...` with `stdin` piped: `(code, stdout, stderr)`.
    fn line(&self, cwd: &Path, args: &[&str], stdin: &str) -> (Option<i32>, String, String) {
        let mut a = vec!["flow", "status", "--line"];
        a.extend_from_slice(args);
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(cwd)
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .args(&a)
            .write_stdin(stdin)
            .timeout(std::time::Duration::from_secs(20))
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Three sessions with distinct lines: `new` (newest), `mid`, `S` (oldest).
    fn three_logs(&self) {
        let events: Vec<Ev> = vec![
            (T0 + 10, true, "a1", "architect", None),
            (T0 + 11, false, "a1", "architect", None),
        ];
        looping(self, "new", T0 + 300);
        self.log("mid", &events, T0 + 200);
        self.log("S", &events[..1], T0 + 100);
    }
}

const NEW_LINE: &str = "deliver ● guard (qa) 2/4 (loops 1)\n";
const MID_LINE: &str = "deliver ○ build (dev) 1/4\n";
const S_LINE: &str = "deliver ● design (architect) 0/4\n";

#[test]
fn line_prints_the_latest_log_without_a_session_id_and_the_one_asked_for_with_it() {
    let env = Env::new();
    env.three_logs();
    let root = env.root();
    let got = |stdin: &str| env.line(&root, &[], stdin);
    assert_eq!(
        got(r#"{"terminal_width":120}"#),
        (Some(0), NEW_LINE.into(), String::new()),
        "json without session_id"
    );
    assert_eq!(
        got(""),
        (Some(0), NEW_LINE.into(), String::new()),
        "empty stdin"
    );
    assert_eq!(
        got("not json"),
        (Some(0), NEW_LINE.into(), String::new()),
        "invalid json"
    );
    assert_eq!(
        got(r#"{"session_id":"S"}"#),
        (Some(0), S_LINE.into(), String::new()),
        "session_id"
    );
    assert_eq!(
        env.line(&root, &["--session", "mid"], r#"{"session_id":"S"}"#),
        (Some(0), MID_LINE.into(), String::new()),
        "--session wins"
    );
}

#[test]
fn line_takes_the_project_of_the_json_cwd() {
    let env = Env::new();
    env.three_logs();
    let stdin = format!(r#"{{"cwd":"{}"}}"#, env.root().display());
    let (code, out, err) = env.line(env.dir.path(), &[], &stdin);
    assert_eq!((code, out.as_str(), err.as_str()), (Some(0), NEW_LINE, ""));
}

#[test]
fn line_is_silent_and_succeeds_when_it_cannot_answer() {
    let env = Env::new();
    env.three_logs();
    let root = env.root();
    // an invalid project
    let bad = env.dir.path().join("bad");
    std::fs::create_dir_all(bad.join(".armadai")).unwrap();
    std::fs::write(
        bad.join(".armadai/project.yaml"),
        "schema_version: 1\nnope: 1\n",
    )
    .unwrap();
    std::fs::create_dir_all(bad.join(".git")).unwrap();
    let cases = [
        ("outside a project", env.line(env.dir.path(), &[], "")),
        ("invalid project", env.line(&bad, &[], "")),
        ("unknown flow", env.line(&root, &["nope"], "")),
        ("unknown session", env.line(&root, &["--session", "zz"], "")),
    ];
    for (name, got) in cases {
        assert_eq!(got, (Some(0), "\n".to_string(), String::new()), "{name}");
    }
}

#[test]
fn line_does_not_wait_for_a_stdin_that_stays_open() {
    use std::process::Stdio;
    let env = Env::new();
    env.three_logs();
    for pass in 0..5 {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_armadai"))
            .args(["flow", "status", "--line"])
            .current_dir(env.root())
            .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
            .env("HOME", env.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let _open = child.stdin.take(); // the handle stays open until the end of the pass
        let start = std::time::Instant::now();
        let done = loop {
            if child.try_wait().unwrap().is_some() {
                break true;
            }
            if start.elapsed() > std::time::Duration::from_secs(5) {
                child.kill().unwrap();
                child.wait().unwrap();
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(
            done,
            "pass {pass}: --line was still waiting for stdin after 5 s"
        );
    }
}

impl Env {
    /// `armadai __claude-statusline` with `stdin` piped, `NO_COLOR` set (it must colour anyway).
    fn statusline(&self, cwd: &Path, stdin: &str) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(cwd)
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .arg("__claude-statusline")
            .write_stdin(stdin)
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

#[test]
fn statusline_prints_the_coloured_line_of_the_session_asked_for() {
    let env = Env::new();
    env.three_logs();
    let root = env.root();
    let stdin = format!(r#"{{"session_id":"S","cwd":"{}"}}"#, root.display());
    let (code, out, err) = env.statusline(env.dir.path(), &stdin);
    // the same pieces as `--line --color always` (the oldest session, not the latest)
    let (_, want, _) = env.line(&root, &["--color", "always", "--session", "S"], "");
    assert!(want.contains('\x1b'), "{want:?}");
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (Some(0), want.as_str(), "")
    );
    assert_eq!(strip_sgr(&out), S_LINE);
}

#[test]
fn statusline_prints_nothing_when_the_session_is_not_named_or_unknown() {
    let env = Env::new();
    env.three_logs();
    let root = env.root();
    let c = root.display();
    let cases = [
        ("no session_id", format!(r#"{{"cwd":"{c}"}}"#)),
        (
            "unknown session",
            format!(r#"{{"session_id":"inconnue","cwd":"{c}"}}"#),
        ),
        (
            "session leaving the directory",
            format!(r#"{{"session_id":"../x","cwd":"{c}"}}"#),
        ),
        ("invalid json", "not json".to_string()),
        ("empty stdin", String::new()),
    ];
    for (name, stdin) in cases {
        let got = env.statusline(&root, &stdin);
        assert_eq!(got, (Some(0), String::new(), String::new()), "{name}");
    }
}

#[test]
fn statusline_prints_nothing_for_a_log_without_a_run() {
    let env = Env::new();
    env.three_logs();
    env.raw_log("empty", "", T0 + 400);
    let root = env.root();
    let got = env.statusline(&root, r#"{"session_id":"empty"}"#);
    assert_eq!(got, (Some(0), String::new(), String::new()));
}

#[test]
fn statusline_does_not_wait_for_a_stdin_that_stays_open() {
    use std::process::Stdio;
    let env = Env::new();
    env.three_logs();
    for pass in 0..5 {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_armadai"))
            .arg("__claude-statusline")
            .current_dir(env.root())
            .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
            .env("HOME", env.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let _open = child.stdin.take();
        let start = std::time::Instant::now();
        let done = loop {
            if child.try_wait().unwrap().is_some() {
                break true;
            }
            if start.elapsed() > std::time::Duration::from_secs(5) {
                child.kill().unwrap();
                child.wait().unwrap();
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(
            done,
            "pass {pass}: __claude-statusline was still waiting for stdin after 5 s"
        );
    }
}

/// Remove the SGR escapes of `s`.
fn strip_sgr(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn color_always_paints_and_never_does_not_even_where_stdout_is_a_pipe() {
    let env = Env::new();
    env.three_logs();
    let root = env.root();
    let (_, always, _) = env.line(&root, &["--color", "always"], "");
    assert!(always.contains('\x1b'), "always: {always:?}");
    assert_eq!(strip_sgr(&always), NEW_LINE);
    // `never` must beat a forced colour: CLICOLOR_FORCE makes `auto` paint on a pipe
    let forced = |color: &str| {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
            .env("HOME", env.dir.path())
            .env_remove("NO_COLOR")
            .env("CLICOLOR_FORCE", "1")
            .args(["flow", "status", "--line", "--color", color])
            .write_stdin("")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(forced("auto").contains('\x1b'), "auto with CLICOLOR_FORCE");
    assert_eq!(forced("never"), NEW_LINE);
    let (_, auto, _) = env.line(&root, &[], "");
    assert_eq!(auto, NEW_LINE, "auto on a pipe with NO_COLOR");
    // the table and the text of `flow status` follow --color too
    let (_, text, _) = env.run(&root, &["flow", "status", "--color", "always"]);
    assert!(
        text.contains('\x1b'),
        "flow status --color always: {text:?}"
    );
}

#[test]
fn a_reader_that_closes_early_is_not_a_panic() {
    use std::process::Stdio;
    let env = Env::new(); // no log: --list and --json both print without a session
    for args in [&["--json"][..], &["--list"][..], &["--line"][..]] {
        for pass in 0..3 {
            let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_armadai"))
                .args(["flow", "status"])
                .args(args)
                .current_dir(env.root())
                .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
                .env("HOME", env.dir.path())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            drop(child.stdout.take()); // the reader is gone before the first write
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                (
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr).into_owned()
                ),
                (Some(0), String::new()),
                "flow status {args:?}, pass {pass}"
            );
        }
    }
}

/// Three runs: `ship` (finished), a `deliver` closed by a second `/deliver`
/// (interrupted), and that second `deliver`, with a loop, still running.
fn three_runs_log(env: &Env) {
    let cmd = |ts: u64, name: &str| {
        format!("{{\"ts\":{ts},\"event\":\"command\",\"command\":\"{name}\"}}\n")
    };
    let ev = |events: &[Ev]| event_lines(events);
    let text = cmd(T0, "ship")
        + &ev(&[
            (T0 + 1, true, "c1", "rel", None),
            (T0 + 2, false, "c1", "rel", None),
        ])
        + &cmd(T0 + 10, "deliver")
        + &ev(&[
            (T0 + 11, true, "a1", "architect", None),
            (T0 + 12, false, "a1", "architect", None),
        ])
        + &cmd(T0 + 20, "deliver")
        + &ev(&[
            (T0 + 21, true, "b1", "architect", None),
            (T0 + 22, false, "b1", "architect", None),
            (T0 + 23, true, "b2", "dev", None),
            (T0 + 24, false, "b2", "dev", None),
            (T0 + 25, true, "b3", "qa", None),
            (T0 + 26, false, "b3", "qa", Some("rejected")),
            (T0 + 27, true, "b4", "dev", None),
            (T0 + 28, false, "b4", "dev", None),
            (T0 + 40, true, "b5", "qa", None),
        ]);
    env.raw_log("s1", &text, T0 + 100);
}

#[test]
fn json_goes_through_the_same_runs_as_the_text() {
    let env = Env::new();
    three_runs_log(&env);
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "--json"]);
    assert_eq!((code, err.as_str()), (Some(0), ""));
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    let runs = got["runs"].as_array().unwrap();
    let summary: Vec<(&str, &str)> = runs
        .iter()
        .map(|r| (r["flow"].as_str().unwrap(), r["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        summary,
        [
            ("ship", "finished"),
            ("deliver", "interrupted"),
            ("deliver", "in-progress")
        ],
        "{out}"
    );
    assert_eq!(
        runs[2]["loops"],
        serde_json::json!([{"from": "build", "to": "guard", "times": 2}])
    );
    assert_eq!(runs[1]["loops"], serde_json::json!([]));
    // the same runs, as the text shows them
    let (_, text, _) = env.run(&env.root(), &["flow", "status"]);
    let heads: Vec<&str> = text
        .lines()
        .filter(|l| {
            l.ends_with("  in progress")
                || l.ends_with("  finished")
                || l.ends_with("  interrupted")
        })
        .collect();
    assert_eq!(
        heads,
        [
            "ship  finished",
            "deliver  interrupted",
            "deliver  in progress"
        ]
    );
}

/// What a status bar really runs: `--line` with stdout a pipe and no colour
/// variable at all (the host's own are removed too).
#[test]
fn line_in_auto_prints_no_escape_on_a_pipe_without_any_colour_variable() {
    let env = Env::new();
    env.three_logs();
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(env.root())
        .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
        .env("HOME", env.dir.path())
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("CLICOLOR")
        .args(["flow", "status", "--line"])
        .write_stdin("")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), NEW_LINE);
}

#[test]
fn md_prints_the_snapshot_without_colour_even_when_forced() {
    let env = Env::new();
    looping(&env, "s1", T0 + 100);
    let want = "# Flow status: session s1\n\nSnapshot at the last recorded event, 2026-10-22 09:15:50 UTC.\n\n## deliver: in progress\n\n| Step | Mark | Status |\n|---|---|---|\n| design | ✓ | done |\n| build | ✓ | done |\n| guard | ● | running |\n| validate | ○ | pending |\n\nLinks taken more than once: build → guard (2).\n\n| Time (UTC) | From | To | State |\n|---|---|---|---|\n| 2026-10-22 09:15:20 | design | build | done |\n| 09:15:30 | build | guard | done |\n| 09:15:40 | guard | build | rejected |\n| 09:15:50 | build | guard | done |\n";
    for extra in [&["--color", "always"][..], &[][..]] {
        let mut args = vec!["flow", "status", "--format", "md"];
        args.extend_from_slice(extra);
        let (code, out, err) = env.run(&env.root(), &args);
        assert_eq!((code, err.as_str(), out.as_str()), (Some(0), "", want));
    }
}

#[test]
fn md_without_a_log_says_there_is_no_activity_and_text_is_unchanged() {
    let env = Env::new();
    let (code, out, _) = env.run(&env.root(), &["flow", "status", "--format", "md"]);
    assert_eq!(
        (code, out.as_str()),
        (
            Some(0),
            "no flow activity recorded yet (link installs the hooks; start a flow in Claude Code)\n"
        )
    );
    looping(&env, "s1", T0 + 100);
    let (_, out, _) = env.run(&env.root(), &["flow", "status", "--format", "text"]);
    assert_eq!(out, LOOPING);
}

#[test]
fn md_does_not_combine_with_watch_list_json_or_line() {
    let env = Env::new();
    for other in ["--watch", "--list", "--json", "--line"] {
        let (code, out, _) = env.run(&env.root(), &["flow", "status", "--format", "md", other]);
        assert_eq!((code, out.as_str()), (Some(2), ""), "{other}");
    }
}

impl Env {
    /// `armadai __claude-subagent-statusline` with `stdin` piped, `NO_COLOR` set.
    fn subagent_statusline(&self, cwd: &Path, stdin: &str) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(cwd)
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .arg("__claude-subagent-statusline")
            .write_stdin(stdin)
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

#[test]
fn subagent_statusline_prints_one_json_line_per_row_it_overrides() {
    let env = Env::new();
    env.three_logs();
    let root = env.root();
    let stdin = format!(
        r#"{{"session_id":"S","cwd":"{}","columns":80,"tasks":[{{"id":"t1","agentType":"architect","status":"running","tokenCount":12}},{{"id":"t2","agentType":"Explore"}}]}}"#,
        root.display()
    );
    let (code, out, err) = env.subagent_statusline(env.dir.path(), &stdin);
    let (_, line, _) = env.line(&root, &["--color", "always", "--session", "S"], "");
    assert_eq!((code, err.as_str()), (Some(0), ""));
    let row: serde_json::Value = serde_json::from_str(out.trim_end_matches('\n')).unwrap();
    assert_eq!(out.matches('\n').count(), 1, "{out:?}");
    // the row of an agent is the line of the run without the agent piece
    let without_agent =
        line.trim_end_matches('\n')
            .replacen("\u{1b}[90m(architect)\u{1b}[0m ", "", 1);
    assert_ne!(without_agent, line.trim_end_matches('\n'));
    assert_eq!(
        row,
        serde_json::json!({"id": "t1", "content": without_agent})
    );
    assert_eq!(
        strip_sgr(row["content"].as_str().unwrap()),
        "deliver ● design 0/4"
    );
}

#[test]
fn subagent_statusline_prints_nothing_on_bad_input() {
    let env = Env::new();
    env.three_logs();
    let root = env.root();
    let c = root.display();
    let cases = [
        ("empty stdin", String::new()),
        (
            "invalid json",
            r#"{"session_id":"S","tasks":[{"id":"t1""#.to_string(),
        ),
        (
            "tasks not an array",
            format!(r#"{{"session_id":"S","cwd":"{c}","tasks":"x"}}"#),
        ),
        ("no tasks", format!(r#"{{"session_id":"S","cwd":"{c}"}}"#)),
        (
            "unknown session",
            format!(
                r#"{{"session_id":"nope","cwd":"{c}","tasks":[{{"id":"t1","agentType":"architect"}}]}}"#
            ),
        ),
    ];
    for (name, stdin) in cases {
        let got = env.subagent_statusline(&root, &stdin);
        assert_eq!(got, (Some(0), String::new(), String::new()), "{name}");
    }
}

#[test]
fn subagent_statusline_without_a_session_id_falls_back_to_the_latest_log() {
    let env = Env::new();
    env.three_logs();
    let root = env.root();
    let stdin = format!(
        r#"{{"cwd":"{}","tasks":[{{"id":"t1","agentType":"qa"}}]}}"#,
        root.display()
    );
    let (code, out, err) = env.subagent_statusline(env.dir.path(), &stdin);
    assert_eq!((code, err.as_str()), (Some(0), ""));
    let row: serde_json::Value = serde_json::from_str(out.trim_end_matches('\n')).unwrap();
    assert_eq!(row["id"], "t1");
    assert_eq!(
        strip_sgr(row["content"].as_str().unwrap()),
        // a row names no agent: the agent is the row's
        "deliver ● guard 2/4 (loops 1)"
    );
}

#[test]
fn subagent_statusline_does_not_wait_for_a_stdin_that_stays_open() {
    use std::process::Stdio;
    let env = Env::new();
    env.three_logs();
    for pass in 0..5 {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_armadai"))
            .arg("__claude-subagent-statusline")
            .current_dir(env.root())
            .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
            .env("HOME", env.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let _open = child.stdin.take();
        let start = std::time::Instant::now();
        let done = loop {
            if child.try_wait().unwrap().is_some() {
                break true;
            }
            if start.elapsed() > std::time::Duration::from_secs(5) {
                child.kill().unwrap();
                child.wait().unwrap();
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(done, "pass {pass}: still waiting for stdin after 5 s");
    }
}

/// `flow status --watch` with stdout in a pipe: waits for the first image
/// (`first` bytes, 20 s at most: a slow start under load, not the nominal time),
/// then `after`, sends `signal` (`None`: kills) and returns what was read and how it ended.
struct Watched {
    out: Vec<u8>,
    status: std::process::ExitStatus,
}

fn watch_in_a_pipe(
    env: &Env,
    args: &[&str],
    first: usize,
    after: std::time::Duration,
    sigint: bool,
) -> Watched {
    use std::io::Read;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_armadai"))
        .current_dir(env.root())
        .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
        .env("HOME", env.dir.path())
        .env("NO_COLOR", "1")
        .args(["flow", "status", "--watch"])
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut out = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while out.len() < first {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(chunk) => out.extend(chunk),
            Err(_) => break,
        }
    }
    std::thread::sleep(after);
    if sigint {
        let ok = std::process::Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(ok.success());
    } else {
        child.kill().unwrap();
    }
    let status = child.wait().unwrap();
    reader.join().unwrap();
    out.extend(rx.try_iter().flatten());
    Watched { out, status }
}

/// `s` without its SGR sequences (`ESC [ ... m`).
fn without_sgr(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            let mut seq = String::from("\x1b[");
            chars.next();
            for d in chars.by_ref() {
                seq.push(d);
                if d == 'm' || d.is_ascii_alphabetic() {
                    break;
                }
            }
            if !seq.ends_with('m') {
                out += &seq;
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn watch_outside_a_terminal_is_the_list_once_without_escape() {
    let env = Env::new();
    looping(&env, "s1", T0);
    // more than two steps of one second: a redraw at every step would show
    let w = watch_in_a_pipe(
        &env,
        &[],
        LOOPING.len(),
        std::time::Duration::from_millis(2500),
        false,
    );
    assert_eq!(String::from_utf8_lossy(&w.out), LOOPING);
}

#[test]
fn watch_outside_a_terminal_with_color_always_has_colour_only() {
    let env = Env::new();
    looping(&env, "s1", T0);
    let w = watch_in_a_pipe(
        &env,
        &["--color", "always"],
        LOOPING.len(),
        std::time::Duration::from_millis(2500),
        false,
    );
    let raw = String::from_utf8_lossy(&w.out).into_owned();
    assert!(
        raw.contains("\x1b[") && raw.contains('m'),
        "no colour at all: {raw:?}"
    );
    assert_eq!(without_sgr(&raw), LOOPING, "{raw:?}");
}

#[cfg(unix)]
#[test]
fn ctrl_c_outside_a_terminal_still_kills_by_sigint() {
    use std::os::unix::process::ExitStatusExt;
    let env = Env::new();
    looping(&env, "s1", T0);
    let w = watch_in_a_pipe(
        &env,
        &[],
        LOOPING.len(),
        std::time::Duration::from_millis(200),
        true,
    );
    assert_eq!(w.status.signal(), Some(2), "{:?}", w.status);
}

// Covers the path outside a terminal only; the path in a terminal is checked in a pty (real trial).
#[cfg(unix)]
#[test]
fn ctrl_c_ignored_by_the_parent_stays_ignored() {
    use std::io::Read;
    let env = Env::new();
    looping(&env, "s1", T0);
    // a signal ignored survives `exec`: the watch starts with SIGINT ignored
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "trap '' INT; exec \"$0\" flow status --watch"])
        .arg(env!("CARGO_BIN_EXE_armadai"))
        .current_dir(env.root())
        .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
        .env("HOME", env.dir.path())
        .env("NO_COLOR", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut first = vec![0u8; LOOPING.len()];
    stdout.read_exact(&mut first).unwrap(); // the first image: the watch is running
    let sent = std::process::Command::new("/bin/kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    // long enough for a process that does not ignore the signal to die of it
    std::thread::sleep(std::time::Duration::from_secs(1));
    let alive = child.try_wait().unwrap().is_none();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        alive,
        "SIGINT was ignored by the parent, the watch must ignore it too"
    );
}

// ---- flow stats ----

/// design (4m12s), build (30s), guard rejects (12s), build (45s), guard
/// approves (41s), validate (1h02m05s); each call also writes the second
/// `stop` of an invocation, with no state.
fn stats_events() -> Vec<Ev> {
    let t = T0;
    vec![
        (t, true, "a1", "architect", None),
        (t + 252, false, "a1", "architect", Some("done")),
        (t + 253, false, "a1", "architect", None),
        (t + 300, true, "a2", "dev", None),
        (t + 330, false, "a2", "dev", None),
        (t + 331, false, "a2", "dev", None),
        (t + 400, true, "a3", "qa", None),
        (t + 412, false, "a3", "qa", Some("rejected")),
        (t + 413, false, "a3", "qa", None),
        (t + 500, true, "a4", "dev", None),
        (t + 545, false, "a4", "dev", None),
        (t + 546, false, "a4", "dev", None),
        (t + 600, true, "a5", "qa", None),
        (t + 641, false, "a5", "qa", Some("approved")),
        (t + 642, false, "a5", "qa", None),
        (t + 700, true, "a6", "ops", None),
        (t + 4425, false, "a6", "ops", Some("approved")),
        (t + 4426, false, "a6", "ops", None),
        // the main thread: stopped, never started, no type
        (t + 5000, false, "m1", "", None),
    ]
}

const STATS_TEXT: &str = "\
flow stats  session s1  by agent
AGENT      CALLS  RESUMED  MEDIAN  MAX    TOTAL  STATES                  REJECTED  LOOPS
architect      1        0  4m12s   4m12s  4m12s  done 1                  -         0
dev            2        0  37s     45s    1m15s  -                       -         1
ops            1        0  1h02m   1h02m  1h02m  approved 1              -         0
qa             2        0  26s     41s    53s    approved 1, rejected 1  50%       0
durations run from a start to the first stop of the same agent: a resumed agent counts once, so they are lower bounds
states count each verdict rendered, a resumed agent can render several
";

#[test]
fn stats_by_agent_prints_the_table_and_its_note() {
    let env = Env::new();
    env.log("s1", &stats_events(), T0 + 10);
    let (code, out, err) = env.run(&env.root(), &["flow", "stats"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (Some(0), STATS_TEXT, "")
    );
    // the same as naming the session and the default grouping
    let named = env.run(
        &env.root(),
        &["flow", "stats", "--session", "s1", "--by", "agent"],
    );
    assert_eq!(named.1, STATS_TEXT);
}

#[test]
fn stats_json_is_versioned_and_keeps_every_key() {
    let env = Env::new();
    env.log("s1", &stats_events(), T0 + 10);
    let (code, out, err) = env.run(&env.root(), &["flow", "stats", "--json"]);
    assert_eq!((code, err.as_str()), (Some(0), ""));
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    let want = serde_json::json!({
        "schema": "armadai.flow-stats/1",
        "sessions": ["s1"],
        "by": "agent",
        "groups": [
            {"key": "architect", "calls": 1, "resumed": 0,
             "duration_s": {"median": 252, "max": 252, "total": 252},
             "states": {"done": 1}, "rejection_rate": null, "loops": 0},
            {"key": "dev", "calls": 2, "resumed": 0,
             "duration_s": {"median": 37, "max": 45, "total": 75},
             "states": {}, "rejection_rate": null, "loops": 1},
            {"key": "ops", "calls": 1, "resumed": 0,
             "duration_s": {"median": 3725, "max": 3725, "total": 3725},
             "states": {"approved": 1}, "rejection_rate": null, "loops": 0},
            {"key": "qa", "calls": 2, "resumed": 0,
             "duration_s": {"median": 26, "max": 41, "total": 53},
             "states": {"approved": 1, "rejected": 1}, "rejection_rate": 0.5, "loops": 0},
        ],
        "unmarked_calls": 0,
    });
    assert_eq!(got, want);
}

#[test]
fn stats_by_lot_without_a_marker_says_so_in_one_line_and_succeeds() {
    let env = Env::new();
    env.log("s1", &stats_events(), T0 + 10);
    for by in ["lot", "step"] {
        let (code, out, err) = env.run(&env.root(), &["flow", "stats", "--by", by]);
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (
                Some(0),
                "no call is marked: --by step and --by lot need [flow/step#lot] markers (see armadai flow status)\n",
                ""
            ),
            "--by {by}"
        );
    }
    let (code, out, _) = env.run(&env.root(), &["flow", "stats", "--by", "lot", "--json"]);
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(code, Some(0));
    assert_eq!(
        (
            &got["schema"],
            &got["by"],
            &got["groups"],
            &got["unmarked_calls"]
        ),
        (
            &serde_json::json!("armadai.flow-stats/1"),
            &serde_json::json!("lot"),
            &serde_json::json!([]),
            &serde_json::json!(6)
        )
    );
}

#[test]
fn stats_by_step_and_lot_group_the_marked_calls_and_name_the_others() {
    let env = Env::new();
    let mark = |ts: u64, agent: &str, step: &str, lot: Option<&str>| {
        let mut m = serde_json::json!({"ts": ts, "event": "mark", "agent_type": agent,
            "flow": "deliver", "step": step});
        if let Some(l) = lot {
            m["lot"] = l.into();
        }
        format!("{m}\n")
    };
    let mut log = String::new();
    log += &mark(T0, "dev", "build", Some("12"));
    log += &event_lines(&[
        (T0 + 1, true, "b1", "dev", None),
        (T0 + 31, false, "b1", "dev", None),
    ]);
    log += &mark(T0 + 40, "qa", "guard", Some("12"));
    log += &event_lines(&[
        (T0 + 41, true, "b2", "qa", None),
        (T0 + 53, false, "b2", "qa", Some("rejected")),
    ]);
    log += &mark(T0 + 60, "dev", "build", Some("7"));
    log += &event_lines(&[
        (T0 + 61, true, "b3", "dev", None),
        (T0 + 106, false, "b3", "dev", None),
    ]);
    // no marker for this one
    log += &event_lines(&[
        (T0 + 110, true, "b4", "ops", None),
        (T0 + 120, false, "b4", "ops", None),
    ]);
    env.raw_log("s1", &log, T0 + 10);
    let (code, out, _) = env.run(&env.root(), &["flow", "stats", "--by", "step"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        out,
        "\
flow stats  session s1  by step
STEP           CALLS  RESUMED  MEDIAN  MAX  TOTAL  STATES      REJECTED  LOOPS
deliver/build      2        0  37s     45s  1m15s  -           -         0
deliver/guard      1        0  12s     12s  12s    rejected 1  100%      0
durations run from a start to the first stop of the same agent: a resumed agent counts once, so they are lower bounds
states count each verdict rendered, a resumed agent can render several
1 call is not marked and is left out of --by step
"
    );
    let (_, out, _) = env.run(&env.root(), &["flow", "stats", "--by", "lot"]);
    assert_eq!(
        out,
        "\
flow stats  session s1  by lot
LOT  CALLS  RESUMED  MEDIAN  MAX  TOTAL  STATES      REJECTED  LOOPS
12       2        0  21s     30s  42s    rejected 1  100%      0
7        1        0  45s     45s  45s    -           -         0
durations run from a start to the first stop of the same agent: a resumed agent counts once, so they are lower bounds
states count each verdict rendered, a resumed agent can render several
1 call is not marked and is left out of --by lot
"
    );
}

#[test]
fn stats_names_several_unmarked_calls_in_the_plural() {
    let env = Env::new();
    let mark = serde_json::json!({"ts": T0, "event": "mark", "agent_type": "dev",
        "flow": "deliver", "step": "build", "lot": "12"});
    let mut log = format!("{mark}\n");
    log += &event_lines(&[
        (T0 + 1, true, "b1", "dev", None),
        (T0 + 31, false, "b1", "dev", None),
        (T0 + 110, true, "b4", "ops", None),
        (T0 + 120, false, "b4", "ops", None),
        (T0 + 130, true, "b5", "architect", None),
        (T0 + 140, false, "b5", "architect", None),
    ]);
    env.raw_log("s1", &log, T0 + 10);
    let (code, out, _) = env.run(&env.root(), &["flow", "stats", "--by", "lot"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        out.lines().last(),
        Some("2 calls are not marked and are left out of --by lot")
    );
}

#[test]
fn stats_without_a_log_is_the_existing_message_and_an_empty_document() {
    let env = Env::new();
    let (code, out, err) = env.run(&env.root(), &["flow", "stats"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(0),
            "no flow activity recorded yet (link installs the hooks; start a flow in Claude Code)\n",
            ""
        )
    );
    let (code, out, _) = env.run(&env.root(), &["flow", "stats", "--json"]);
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(code, Some(0));
    assert_eq!(
        got,
        serde_json::json!({"schema": "armadai.flow-stats/1", "sessions": [], "by": "agent",
            "groups": [], "unmarked_calls": 0})
    );
}

#[test]
fn stats_of_a_flow_with_no_call_in_the_session_says_which_flow() {
    let env = Env::new();
    env.log("s1", &stats_events(), T0 + 10);
    // `ship` holds `rel` only, which never ran
    let (code, out, err) = env.run(&env.root(), &["flow", "stats", "ship"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (Some(0), "no call of flow ship in this session\n", "")
    );
    let (_, out, _) = env.run(&env.root(), &["flow", "stats", "ship", "--json"]);
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (&got["groups"], &got["unmarked_calls"]),
        (&serde_json::json!([]), &serde_json::json!(0))
    );
    let (code, out, _) = env.run(&env.root(), &["flow", "stats", "ship", "--all"]);
    assert_eq!(
        (code, out.as_str()),
        (Some(0), "no call of flow ship in these sessions\n")
    );
    // no log at all: the general line, whatever the flow
    let none = Env::new();
    let (_, out, _) = none.run(&none.root(), &["flow", "stats", "ship"]);
    assert!(out.starts_with("no flow activity recorded yet"), "{out}");
}

#[test]
fn stats_refuses_an_unknown_flow_or_session_with_the_way_forward() {
    let env = Env::new();
    env.log("s1", &stats_events(), T0 + 10);
    let (code, _, err) = env.run(&env.root(), &["flow", "stats", "nope"]);
    assert_eq!(code, Some(1));
    assert!(
        err.contains("unknown flow `nope`; flows: deliver, ship"),
        "{err}"
    );
    let (code, _, err) = env.run(&env.root(), &["flow", "stats", "--session", "zz"]);
    assert_eq!(code, Some(1));
    assert!(
        err.contains("no session `zz` recorded; see `armadai flow status --list`"),
        "{err}"
    );
}

#[test]
fn stats_all_reads_every_session_and_a_flow_keeps_its_own_agents() {
    let env = Env::new();
    env.log("s1", &stats_events(), T0 + 10);
    env.log(
        "s2",
        &[
            (T0 + 9000, true, "z1", "dev", None),
            (T0 + 9010, false, "z1", "dev", None),
            (T0 + 9100, true, "r1", "rel", None),
            (T0 + 9107, false, "r1", "rel", None),
        ],
        T0 + 20,
    );
    let (_, out, _) = env.run(&env.root(), &["flow", "stats", "--all", "--json"]);
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(got["sessions"], serde_json::json!(["s1", "s2"]));
    let calls = |key: &str| {
        got["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["key"] == key)
            .map(|g| g["calls"].clone())
    };
    assert_eq!(
        (calls("dev"), calls("rel")),
        (Some(3.into()), Some(1.into()))
    );
    // `ship` holds `rel` only
    let (_, out, _) = env.run(&env.root(), &["flow", "stats", "ship", "--all", "--json"]);
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    let keys: Vec<&str> = got["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["rel"]);
}

// ---- optional steps ----

/// `check` is optional and never runs: the run ends on `release`.
fn optional_project(env: &Env) {
    let a = env.root().join(".armadai");
    for agent in ["sa", "sb", "sc", "sd", "se"] {
        std::fs::write(
            a.join(format!("agents/{agent}.md")),
            "---\ndescription: d\n---\nB\n",
        )
        .unwrap();
    }
    std::fs::write(
        a.join("flows/opt.yaml"),
        "steps:\n  design: { agent: sa }\n  build: { agent: sb }\n  check: { agent: sc, optional: true }\n  release: { agent: sd }\nlinks:\n  - { from: design, to: build }\n  - { from: build, to: check }\n  - { from: check, to: release }\n",
    )
    .unwrap();
    let events: Vec<Ev> = vec![
        (T0 + 10, true, "o1", "sa", None),
        (T0 + 11, false, "o1", "sa", None),
        (T0 + 20, true, "o2", "sb", None),
        (T0 + 21, false, "o2", "sb", None),
        (T0 + 30, true, "o3", "sd", None),
        (T0 + 31, false, "o3", "sd", None),
    ];
    env.log("s1", &events, T0 + 100);
}

#[test]
fn a_skipped_step_reads_in_text_markdown_and_json_and_the_run_finishes() {
    let env = Env::new();
    optional_project(&env);
    let (code, out, err) = env.run(&env.root(), &["flow", "status", "opt"]);
    assert_eq!((code, err.as_str()), (Some(0), ""));
    assert_eq!(
        out,
        "session s1 (UTC)\n\
         \n\
         opt  finished\n\
         \x20 ✓ design   done\n\
         \x20 ✓ build    done\n\
         \x20 ⤼ check    (skipped)\n\
         \x20 ✓ release  done\n\
         \x20 09:15:20  design → build  (done)\n\
         \x20 09:15:30  build → release  (done)\n"
    );
    let (_, md, _) = env.run(&env.root(), &["flow", "status", "opt", "--format", "md"]);
    assert!(
        md.contains("| design | ✓ | done |\n| build | ✓ | done |\n| check | ⤼ | (skipped) |\n| release | ✓ | done |\n"),
        "{md}"
    );
    let (_, js, _) = env.run(&env.root(), &["flow", "status", "opt", "--json"]);
    let doc: serde_json::Value = serde_json::from_str(&js).unwrap();
    let steps: Vec<(&str, &str, &str, &str)> = doc["runs"][0]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["step"].as_str().unwrap(),
                s["agent"].as_str().unwrap(),
                s["mark"].as_str().unwrap(),
                s["sign"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("design", "sa", "done", "✓"),
            ("build", "sb", "done", "✓"),
            ("check", "sc", "skipped", "⤼"),
            ("release", "sd", "done", "✓"),
        ]
    );
    assert_eq!(doc["runs"][0]["steps"][2]["note"], "skipped");
    assert_eq!(doc["runs"][0]["status"], "finished");
    // `--line` does not count it: 3 steps done of 3, and a finished run names no agent
    let (_, line, _) = env.line(&env.root(), &["opt"], "");
    assert_eq!(line, "opt finished 3/3\n");
}
