//! `ui.status_line` end to end: what `link` writes into `.claude/settings.json`, what it
//! leaves of the user's own bar, the collision with `native.claude`, the other targets,
//! and what `unlink` takes back.

use std::path::PathBuf;

use assert_cmd::Command;
use serde_json::{Value, json};

const AGENT: (&str, &str) = (
    ".armadai/agents/lead.md",
    "---\ndescription: Leads\n---\nYou lead.\n",
);
const FLOW: (&str, &str) = (
    ".armadai/flows/ship.yaml",
    "steps:\n  plan: { agent: lead }\nlinks: []\n",
);

const BAR_LINE: &str = "claude: ui.status_line: compiled (statusLine in .claude/settings.json, \
running armadai __claude-statusline; it hides the statusLine of your user settings)";
const KEPT_LINE: &str = "claude: ui.status_line: not-compiled (.claude/settings.json already has \
a statusLine; link leaves it (add armadai flow status --line to your own bar instead))";

const USER: &str = r#"{"zeta":1,"permissions":{"allow":["Bash(git *)"]},"alpha":{"unknown":true}}"#;
const USER_BAR: &str =
    r#"{"zeta":1,"statusLine":{"type":"command","command":"~/mine.sh","padding":2}}"#;

struct Env(tempfile::TempDir);

impl Env {
    fn new(project_yaml: &str, extra: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for d in ["project", "config", "home"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let env = Env(dir);
        let yaml = (".armadai/project.yaml", project_yaml);
        for (p, c) in [yaml, AGENT].iter().chain(extra) {
            let path = env.root().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        env
    }

    fn root(&self) -> PathBuf {
        self.0.path().join("project")
    }

    fn settings_path(&self) -> PathBuf {
        self.root().join(".claude/settings.json")
    }

    fn put_settings(&self, text: &str) {
        std::fs::create_dir_all(self.root().join(".claude")).unwrap();
        std::fs::write(self.settings_path(), text).unwrap();
    }

    fn settings(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.settings_path()).unwrap()).unwrap()
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("HOME", self.0.path().join("home"))
            .env("ARMADAI_CONFIG_DIR", self.0.path().join("config"))
            .args(args)
            .output()
            .unwrap()
    }

    /// stdout of a command that must succeed.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

fn has_line(report: &str, line: &str) -> bool {
    report.lines().any(|l| l == line)
}

fn ui_lines(report: &str) -> Vec<&str> {
    report
        .lines()
        .filter(|l| l.contains("ui.status_line"))
        .collect()
}

fn on() -> &'static str {
    "schema_version: 1\ncoordinator: lead\nui:\n  status_line: true\n"
}

#[test]
fn nothing_is_written_without_the_opt_in() {
    for yaml in [
        "schema_version: 1\ncoordinator: lead\n",
        "schema_version: 1\ncoordinator: lead\nui: {}\n",
        "schema_version: 1\ncoordinator: lead\nui:\n  status_line: false\n",
    ] {
        let env = Env::new(yaml, &[FLOW]);
        let report = env.ok(&["link", "--target", "claude"]);
        let keys: Vec<String> = env
            .settings()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, ["hooks"], "{yaml}");
        assert!(!env.root().join(".armadai/plugins").exists(), "{yaml}");
        assert_eq!(ui_lines(&report), Vec::<&str>::new(), "{yaml}");
    }
}

#[test]
fn the_key_writes_the_status_line_and_unlink_takes_it_back() {
    let env = Env::new(on(), &[FLOW]);
    env.put_settings(USER);
    let before = env.settings();
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(
        env.settings()["statusLine"],
        json!({"type":"command","command":"armadai __claude-statusline","refreshInterval":5})
    );
    assert_eq!(ui_lines(&report), [BAR_LINE]);
    env.ok(&["unlink", "--target", "claude"]);
    assert_eq!(env.settings(), before);
}

#[test]
fn a_status_line_of_the_user_is_kept_and_the_rest_is_written() {
    // no hook to write: the user's file stays byte for byte
    let env = Env::new(on(), &[]);
    env.put_settings(USER_BAR);
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(
        std::fs::read_to_string(env.settings_path()).unwrap(),
        USER_BAR
    );
    assert_eq!(ui_lines(&report), [KEPT_LINE]);

    // hooks to write: they come, the bar is still the user's
    let env = Env::new(on(), &[FLOW]);
    env.put_settings(USER_BAR);
    let report = env.ok(&["link", "--target", "claude"]);
    let s = env.settings();
    assert_eq!(
        s["statusLine"],
        serde_json::from_str::<Value>(USER_BAR).unwrap()["statusLine"]
    );
    assert!(s["hooks"]["SubagentStart"].is_array());
    assert_eq!(ui_lines(&report), [KEPT_LINE]);
    // and unlink leaves the bar that was never ours
    env.ok(&["unlink", "--target", "claude"]);
    assert_eq!(
        env.settings(),
        serde_json::from_str::<Value>(USER_BAR).unwrap()
    );
}

#[test]
fn a_second_link_keeps_its_own_status_line() {
    let env = Env::new(on(), &[FLOW]);
    let first = env.ok(&["link", "--target", "claude"]);
    let text = std::fs::read_to_string(env.settings_path()).unwrap();
    let second = env.ok(&["link", "--target", "claude"]);
    assert_eq!(std::fs::read_to_string(env.settings_path()).unwrap(), text);
    assert!(has_line(&first, BAR_LINE) && has_line(&second, BAR_LINE));
    // still owned: unlink removes it
    env.ok(&["unlink", "--target", "claude"]);
    assert!(!env.settings_path().exists());
}

#[test]
fn a_native_status_line_with_the_key_is_an_error_and_nothing_is_written() {
    let env = Env::new(
        "schema_version: 1\ncoordinator: lead\nui:\n  status_line: true\nnative:\n  claude:\n    statusLine: { type: command, command: x }\n",
        &[],
    );
    let out = env.run(&["link", "--target", "claude"]);
    assert!(!out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Error: project.yaml: native.claude key `statusLine` collides with ui.status_line\n"
    );
    assert!(!env.root().join(".claude").exists());
}

#[test]
fn other_targets_say_claude_code_only_and_only_with_the_key() {
    for target in ["opencode", "codex", "copilot"] {
        let env = Env::new(on(), &[]);
        let line = format!("{target}: ui.status_line: not-compiled (Claude Code only)");
        assert_eq!(ui_lines(&env.ok(&["link", "--target", target])), [line]);
        let env = Env::new("schema_version: 1\ncoordinator: lead\n", &[]);
        assert_eq!(
            ui_lines(&env.ok(&["link", "--target", target])),
            Vec::<&str>::new(),
            "{target}"
        );
    }
}

#[test]
fn a_user_bar_equal_to_ours_is_reported_compiled_but_not_owned() {
    let ours = r#"{"statusLine":{"type":"command","command":"armadai __claude-statusline","refreshInterval":5}}"#;
    let env = Env::new(on(), &[FLOW]);
    env.put_settings(ours);
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(ui_lines(&report), [BAR_LINE]);
    env.ok(&["unlink", "--target", "claude"]);
    // never recorded as ours: unlink leaves it
    assert_eq!(env.settings(), serde_json::from_str::<Value>(ours).unwrap());
}

#[test]
fn a_second_link_replaces_an_older_value_of_its_own_bar() {
    let env = Env::new(on(), &[FLOW]);
    env.ok(&["link", "--target", "claude"]);
    // as if an earlier version had linked with another refresh interval
    let manifest = env.root().join(".armadai/link-manifest.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    assert!(text.contains("refreshInterval: 5"));
    std::fs::write(
        &manifest,
        text.replace("refreshInterval: 5", "refreshInterval: 9"),
    )
    .unwrap();
    let old = std::fs::read_to_string(env.settings_path()).unwrap();
    std::fs::write(
        env.settings_path(),
        old.replace("\"refreshInterval\": 5", "\"refreshInterval\": 9"),
    )
    .unwrap();
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(env.settings()["statusLine"]["refreshInterval"], json!(5));
    assert_eq!(ui_lines(&report), [BAR_LINE]);
}
