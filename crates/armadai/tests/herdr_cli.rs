//! `armadai herdr watch` as the real binary, with a fake `hc` and a fake `herdr-app` first
//! in the child's `PATH`. Each fake logs `$0` and its arguments NUL-separated.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

struct Env {
    home: tempfile::TempDir,
    bins: tempfile::TempDir,
    proj: tempfile::TempDir,
}

impl Env {
    fn new(project_yaml: &str) -> Self {
        let e = Env {
            home: tempfile::tempdir().unwrap(),
            bins: tempfile::tempdir().unwrap(),
            // a space in the root: a shell would split it
            proj: tempfile::Builder::new()
                .prefix("my proj")
                .tempdir()
                .unwrap(),
        };
        std::fs::create_dir_all(e.proj.path().join(".armadai")).unwrap();
        std::fs::write(e.proj.path().join(".armadai/project.yaml"), project_yaml).unwrap();
        e
    }
    fn log(&self) -> PathBuf {
        self.bins.path().join("log")
    }
    /// A fake `name` running `body` after logging its argument vector.
    fn fake(&self, name: &str, body: &str) {
        let p = self.bins.path().join(name);
        let script = format!(
            "#!/bin/sh\nprintf '%s\\0' \"$0\" \"$@\" >> '{}'\nprintf '\\n' >> '{}'\n{body}\n",
            self.log().display(),
            self.log().display()
        );
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    /// Each call as a vector of whole arguments (`$0` first).
    fn calls(&self) -> Vec<Vec<String>> {
        let raw = std::fs::read_to_string(self.log()).unwrap_or_default();
        raw.split("\0\n")
            .filter(|c| !c.is_empty())
            .map(|c| c.split('\0').map(str::to_string).collect())
            .collect()
    }
    fn run(
        &self,
        path: &std::ffi::OsStr,
        terminal: Option<&str>,
        dir: &Path,
    ) -> std::process::Output {
        self.run_sub("watch", path, terminal, dir)
    }
    fn run_sub(
        &self,
        sub: &str,
        path: &std::ffi::OsStr,
        terminal: Option<&str>,
        dir: &Path,
    ) -> std::process::Output {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(dir)
            .args(["herdr", sub])
            .env("HOME", self.home.path())
            .env("ARMADAI_CONFIG_DIR", self.home.path().join("cfg"))
            .env("CLAUDE_CONFIG_DIR", self.home.path().join("claude"))
            .env("PATH", path)
            .env_remove("HC_TERMINAL_ID");
        if let Some(t) = terminal {
            cmd.env("HC_TERMINAL_ID", t);
        }
        cmd.output().unwrap()
    }
    fn path(&self) -> std::ffi::OsString {
        // the system directories only after ours: `sh` and `printf` come from there
        std::env::join_paths([self.bins.path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap()
    }
    fn watch(&self) -> std::process::Output {
        self.run(&self.path(), Some("t2"), self.proj.path())
    }
}

const ON: &str = "schema_version: 1\nherdr: true\n";

fn stderr(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn watch_opens_one_pane() {
    let e = Env::new(ON);
    e.fake(
        "herdr-app",
        "echo '{ \"pane\": \"p1\", \"terminal\": \"t3\" }'",
    );
    e.fake("hc", "");
    let out = e.watch();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "opened flow status --watch in a new Herdr pane\n"
    );
    let app = e.bins.path().join("herdr-app").display().to_string();
    let argv = r#"["armadai","flow","status","--watch"]"#;
    assert_eq!(
        e.calls(),
        vec![vec![
            app,
            "do".into(),
            "pane.split".into(),
            "--direction".into(),
            "right".into(),
            "--argv".into(),
            argv.into()
        ]]
    );
}

#[test]
fn watch_refuses_outside_a_project() {
    let e = Env::new(ON);
    e.fake("herdr-app", "");
    e.fake("hc", "");
    let empty = tempfile::tempdir().unwrap();
    std::fs::create_dir(empty.path().join(".git")).unwrap();
    let out = e.run(&e.path(), Some("t2"), empty.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).starts_with("Error: no ArmadAI project found"),
        "{}",
        stderr(&out)
    );
    assert!(e.calls().is_empty());
}

#[test]
fn watch_refuses_without_herdr() {
    for yaml in ["schema_version: 1\n", "schema_version: 1\nherdr: false\n"] {
        let e = Env::new(yaml);
        e.fake("herdr-app", "");
        e.fake("hc", "");
        let out = e.watch();
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(
            stderr(&out),
            "Error: herdr is not enabled for this project: add herdr: true to .armadai/project.yaml\n"
        );
        assert!(e.calls().is_empty());
    }
}

#[test]
fn watch_refuses_outside_herdr() {
    for terminal in [None, Some("")] {
        let e = Env::new(ON);
        e.fake("herdr-app", "");
        e.fake("hc", "");
        let out = e.run(&e.path(), terminal, e.proj.path());
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(
            stderr(&out),
            "Error: not in a Herdr terminal (HC_TERMINAL_ID is not set): run this command in a terminal of Herdr.app\n"
        );
        assert!(e.calls().is_empty());
    }
}

#[test]
fn watch_refuses_without_binaries() {
    let tail = "not found in PATH: Herdr.app installs hc and herdr-app in ~/.local/bin, put that folder in PATH\n";
    // neither: herdr-app is named first
    let e = Env::new(ON);
    let out = e.watch();
    assert_eq!(stderr(&out), format!("Error: herdr-app {tail}"));
    // herdr-app alone present: hc is named
    e.fake("herdr-app", "");
    let out = e.watch();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), format!("Error: hc {tail}"));
    assert!(e.calls().is_empty());
}

#[test]
fn watch_does_not_look_in_local_bin() {
    // fakes in $HOME/.local/bin, which is not in PATH: they must not be called
    let e = Env::new(ON);
    let dir = e.home.path().join(".local/bin");
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["hc", "herdr-app"] {
        let p = dir.join(name);
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nprintf '%s\\0' \"$0\" \"$@\" >> '{}'\n",
                e.log().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    let out = e.run(&path, Some("t2"), e.proj.path());
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "Error: herdr-app not found in PATH: Herdr.app installs hc and herdr-app in ~/.local/bin, put that folder in PATH\n"
    );
    assert!(e.calls().is_empty());
}

#[test]
fn watch_reports_a_failed_call() {
    // (a) the error on stdout as a string, exit 1
    let e = Env::new(ON);
    e.fake(
        "herdr-app",
        "echo '{ \"error\": \"no pane \\\"p0\\\"\" }'\nexit 1",
    );
    e.fake("hc", "");
    let out = e.watch();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "Error: herdr-app do pane.split failed: no pane \"p0\"\n"
    );
    // (b) a closed window: stderr only, exit 2
    let e = Env::new(ON);
    let msg = "herdr-app: no Herdr window at /x/herdr-app.sock: Connection refused (os error 61)";
    e.fake("herdr-app", &format!("echo '{msg}' >&2\nexit 2"));
    e.fake("hc", "");
    let out = e.watch();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        format!("Error: herdr-app do pane.split failed: {msg}\n")
    );
    // (c) killed by a signal, nothing written
    let e = Env::new(ON);
    e.fake("herdr-app", "kill -9 $$");
    e.fake("hc", "");
    let out = e.watch();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "Error: herdr-app do pane.split failed: killed by signal 9\n"
    );
}

/// Empty `PATH` (no fake, no real `hc`) and no `HC_TERMINAL_ID` in the child.
fn bare(yaml: &str) -> std::process::Output {
    let e = Env::new(yaml);
    let empty = tempfile::tempdir().unwrap();
    e.run(empty.path().as_os_str(), None, e.proj.path())
}

#[test]
fn herdr_guard_checks_the_setting_before_the_terminal() {
    let out = bare("schema_version: 1\nherdr: false\n");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "Error: herdr is not enabled for this project: add herdr: true to .armadai/project.yaml\n"
    );
}

#[test]
fn herdr_guard_checks_the_terminal_before_the_binaries() {
    let out = bare(ON);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "Error: not in a Herdr terminal (HC_TERMINAL_ID is not set): run this command in a terminal of Herdr.app\n"
    );
}

#[test]
fn herdr_help_lists_both_commands() {
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .args(["herdr", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for line in [
        "  watch      Open armadai flow status --watch in a new pane to the right",
        "  open-team  Open a space with the coordinator, flow status and the sessions declared under herdr",
    ] {
        assert!(text.lines().any(|l| l == line), "{text}");
    }
    let w = Command::cargo_bin("armadai")
        .unwrap()
        .args(["herdr", "watch", "--help"])
        .output()
        .unwrap();
    let w = String::from_utf8_lossy(&w.stdout);
    assert!(
        w.starts_with("Open armadai flow status --watch in a new pane to the right\n\nUsage:"),
        "{w}"
    );
    assert!(
        text.starts_with("Open ArmadAI panes in Herdr.app (run it in a terminal of the app)\n"),
        "{text}"
    );
}

// ---- open-team ----

const TEAM: &str = "schema_version: 1\nname: demo\ntargets: [claude]\nherdr:\n  sessions:\n    \
    - {name: chantier, kind: claude}\n    - {name: relecteur, kind: codex}\n";

const FAKE_APP: &str = r#"case "$1" in
ls) echo '{"spaces":[{"id":"sp01","name":"demo","folder":"/x","active":true,"tabs":[{"id":"tb01","name":"1","active":true,"terminals":[{"terminal":"t8","pane":"p08","status":null,"app":null,"name":null,"sub_agents":[]},{"terminal":"t9","pane":"p09","status":null,"app":null,"name":null,"sub_agents":[]}]}]}]}';;
do) case "$2" in
  space.new) echo '{ "space": "sp01", "tab": "tb01", "terminal": "t9" }';;
  pane.split) echo '{ "pane": "p10", "terminal": "t10" }';;
  tab.rename) echo '{ "ok": true }';;
  esac;;
esac"#;

const FAKE_HC: &str = r#"echo '{"address":"t11","agent":"claude","state":"live","turn":0}'"#;

impl Env {
    fn team(yaml: &str) -> Self {
        let e = Env::new(yaml);
        e.fake("herdr-app", FAKE_APP);
        e.fake("hc", FAKE_HC);
        e
    }
    fn open_team(&self) -> std::process::Output {
        self.run_sub("open-team", &self.path(), Some("t2"), self.proj.path())
    }
    fn app(&self) -> String {
        self.bins.path().join("herdr-app").display().to_string()
    }
    fn hc(&self) -> String {
        self.bins.path().join("hc").display().to_string()
    }
    fn root(&self) -> String {
        self.proj
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    }
}

fn v(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

fn stdout(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

const TEAM_SUCCESS: &str = "opened Herdr space `demo`\n  coordinator  claude\n  flow status  armadai flow status --watch\n  chantier     claude\n  relecteur    codex\n";

/// The calls of `open-team` on `TEAM`, with `chantier` renamed `renames` times in a row.
fn team_calls(e: &Env, renames: usize) -> Vec<Vec<String>> {
    let (app, hc, root) = (e.app(), e.hc(), e.root());
    let new = format!(r#"{{"name":"demo","folder":"{root}","argv":["claude"]}}"#);
    let argv = r#"["armadai","flow","status","--watch"]"#;
    let run = |name: &str, kind: &str| {
        v(&[
            &hc,
            "run",
            "--bg",
            "--name",
            name,
            "--cwd",
            &root,
            "--no-parent",
            "--label",
            "herdr.space=sp01",
            "--label",
            &format!("herdr.tab=tb01-{name}"),
            kind,
        ])
    };
    let rename = |name: &str| {
        v(&[
            &app,
            "do",
            "tab.rename",
            "--id",
            &format!("tb01-{name}"),
            "--name",
            name,
        ])
    };
    let mut calls = vec![
        v(&[&app, "do", "space.new", &new]),
        v(&[&app, "ls"]),
        v(&[
            &app,
            "do",
            "pane.split",
            "--direction",
            "down",
            "--pane",
            "p09",
            "--argv",
            argv,
        ]),
        run("chantier", "claude"),
    ];
    calls.extend(std::iter::repeat_n(rename("chantier"), renames));
    calls.extend([run("relecteur", "codex"), rename("relecteur")]);
    calls
}

#[test]
fn open_team_calls_in_order() {
    let e = Env::team(TEAM);
    let out = e.open_team();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(e.calls(), team_calls(&e, 1));
    assert_eq!(stdout(&out), TEAM_SUCCESS);
}

#[test]
fn open_team_waits_for_the_tab() {
    let e = Env::team(TEAM);
    // the tab is not registered yet for the first two `tab.rename` of `chantier`
    e.fake(
        "herdr-app",
        &FAKE_APP.replace(
            r#"tab.rename) echo '{ "ok": true }';;"#,
            &format!(
                r#"tab.rename) if [ "$4" = tb01-chantier ] && [ "$(grep -c 'tab.rename' '{log}')" -le 2 ]; then echo '{{ "error": "no tab \"tb01-chantier\"" }}'; exit 1; fi; echo '{{ "ok": true }}';;"#,
                log = e.log().display()
            ),
        ),
    );
    let out = e.open_team();
    assert!(out.status.success(), "{out:?}\n{}", stderr(&out));
    assert_eq!(stdout(&out), TEAM_SUCCESS);
    assert_eq!(e.calls(), team_calls(&e, 3));
}

#[test]
fn open_team_unexpected_ls() {
    let e = Env::team(TEAM);
    e.fake(
        "herdr-app",
        r#"case "$1" in
ls) echo '{"spaces":[{"tabs":[{"terminals":[{"terminal":"t8","pane":"p08"}]}]}]}';;
do) echo '{ "space": "sp01", "tab": "tb01", "terminal": "t9" }';;
esac"#,
    );
    let out = e.open_team();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "Error: herdr-app ls failed: unexpected output\n"
    );
    assert!(
        e.calls()
            .iter()
            .all(|c| !c.contains(&"pane.split".to_string()))
    );
}

#[test]
fn open_team_names_each_session_once() {
    let e = Env::team(TEAM);
    assert!(e.open_team().status.success());
    let names = |prog: &str| -> Vec<String> {
        e.calls()
            .iter()
            .filter(|c| c[0].ends_with(prog))
            .flat_map(|c| {
                c.windows(2)
                    .filter(|w| w[0] == "--name")
                    .map(|w| w[1].clone())
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    assert_eq!(names("/hc"), v(&["chantier", "relecteur"]));
    assert_eq!(names("/herdr-app"), v(&["chantier", "relecteur"]));
}

#[test]
fn open_team_uses_no_shell() {
    let e = Env::team(&TEAM.replace("name: demo", "name: my team"));
    assert!(e.open_team().status.success());
    let root = e.root();
    for c in e.calls() {
        for (i, a) in c.iter().enumerate().skip(1) {
            assert!(a != "sh" && a != "-c", "{c:?}");
            if c[i - 1] == "--cwd" {
                // the root, with its space, in one argument
                assert_eq!(a, &root);
                continue;
            }
            // a space only inside a JSON value
            assert!(
                !a.contains(' ') || a.starts_with('{') || a.starts_with('['),
                "{a}"
            );
        }
    }
    assert!(e.calls()[0][3].contains("my team"));
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut all = vec![];
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for ent in std::fs::read_dir(&d).unwrap() {
            let p = ent.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else {
                all.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    all.sort();
    all
}

#[test]
fn open_team_touches_no_file() {
    let e = Env::team(TEAM);
    let before = (snapshot(e.proj.path()), snapshot(e.home.path()));
    assert!(e.open_team().status.success());
    assert_eq!((snapshot(e.proj.path()), snapshot(e.home.path())), before);
}

#[test]
fn open_team_stops_on_a_failed_call() {
    let e = Env::team(TEAM);
    e.fake(
        "hc",
        r#"case "$*" in *relecteur*) echo '{"error":{"hint":"pick another name, or kill the one holding it","kind":"name_taken","message":"a live terminal is already named \"relecteur\""}}' >&2; exit 1;; esac"#,
    );
    let out = e.open_team();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "Error: hc run relecteur failed: a live terminal is already named \"relecteur\"\n"
    );
    let calls = e.calls();
    assert_eq!(calls.last().unwrap()[1], "run");
    assert_eq!(calls.last().unwrap()[4], "relecteur");
    assert_eq!(calls.len(), 6);
}

#[test]
fn coordinator_kind_follows_targets() {
    let kind_of = |targets: &str| {
        let e = Env::team(&format!(
            "schema_version: 1\ntargets: {targets}\nherdr: true\n"
        ));
        let out = e.open_team();
        (out, e)
    };
    let (out, e) = kind_of("[opencode, claude]");
    assert!(out.status.success(), "{out:?}");
    assert!(
        e.calls()[0][3].ends_with(r#""argv":["opencode"]}"#),
        "{:?}",
        e.calls()[0]
    );
    let (out, e) = kind_of("[]");
    assert!(out.status.success(), "{out:?}");
    assert!(e.calls()[0][3].ends_with(r#""argv":["claude"]}"#));
    // the key omitted
    let e = Env::team("schema_version: 1\nherdr: true\n");
    let out = e.open_team();
    assert!(out.status.success(), "{out:?}");
    assert!(e.calls()[0][3].ends_with(r#""argv":["claude"]}"#));
    let (out, e) = kind_of("[copilot]");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "Error: no target of this project runs in Herdr (claude, codex or opencode); targets: copilot\n"
    );
    assert!(e.calls().is_empty());
}

#[test]
fn open_team_refusals() {
    let tail = "not found in PATH: Herdr.app installs hc and herdr-app in ~/.local/bin, put that folder in PATH\n";
    let sub = |e: &Env, path: &std::ffi::OsStr, t: Option<&str>| {
        e.run_sub("open-team", path, t, e.proj.path())
    };
    let e = Env::team("schema_version: 1\n");
    let out = sub(&e, &e.path(), Some("t2"));
    assert_eq!(
        stderr(&out),
        "Error: herdr is not enabled for this project: add herdr: true to .armadai/project.yaml\n"
    );
    let e = Env::team(ON);
    let out = sub(&e, &e.path(), None);
    assert_eq!(
        stderr(&out),
        "Error: not in a Herdr terminal (HC_TERMINAL_ID is not set): run this command in a terminal of Herdr.app\n"
    );
    let e = Env::new(ON);
    assert_eq!(
        stderr(&sub(&e, &e.path(), Some("t2"))),
        format!("Error: herdr-app {tail}")
    );
    e.fake("herdr-app", "");
    assert_eq!(
        stderr(&sub(&e, &e.path(), Some("t2"))),
        format!("Error: hc {tail}")
    );
    assert!(e.calls().is_empty());
}
