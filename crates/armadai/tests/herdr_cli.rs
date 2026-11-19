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
            proj: tempfile::tempdir().unwrap(),
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
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(dir)
            .args(["herdr", "watch"])
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
            "down".into(),
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
        "  watch      Open armadai flow status --watch in a new pane below",
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
        w.starts_with("Open armadai flow status --watch in a new pane below\n\nUsage:"),
        "{w}"
    );
    assert!(
        text.starts_with("Open ArmadAI panes in Herdr.app (run it in a terminal of the app)\n"),
        "{text}"
    );
}
