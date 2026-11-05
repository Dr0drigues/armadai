//! `ui.subagent_status_line` end to end: the `armadai-ui` plugin and the settings keys
//! `link` writes, what it leaves of the user's own keys, the destination guard, the
//! other targets, and what `unlink` takes back.

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

const USER: &str = r#"{"zeta":1,"permissions":{"allow":["Bash(git *)"]},"alpha":{"unknown":true}}"#;

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

const PLUGIN: &str = ".armadai/plugins/armadai-ui";

const WRITTEN: &str = "claude: ui.subagent_status_line: compiled (plugin armadai-ui in \
.armadai/plugins/armadai-ui, enabled in .claude/settings.json; Claude Code loads it once \
you trust the folder)";

const MARKETPLACE: &str = r#"{
  "name": "armadai-ui",
  "owner": {
    "name": "ArmadAI"
  },
  "plugins": [
    {
      "name": "armadai-ui",
      "source": ".",
      "description": "Flow progress from ArmadAI, in Claude Code"
    }
  ]
}
"#;
const PLUGIN_JSON: &str = r#"{
  "name": "armadai-ui",
  "description": "Flow progress from ArmadAI, in Claude Code"
}
"#;
const PLUGIN_SETTINGS: &str = r#"{
  "subagentStatusLine": {
    "type": "command",
    "command": "armadai __claude-subagent-statusline"
  }
}
"#;

impl Env {
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root().join(rel)).unwrap()
    }
}

fn lines(report: &str) -> Vec<&str> {
    report
        .lines()
        .filter(|l| l.contains("ui.subagent_status_line"))
        .collect()
}

fn on() -> &'static str {
    "schema_version: 1\ncoordinator: lead\nui:\n  subagent_status_line: true\n"
}

#[test]
fn the_key_writes_the_plugin_and_the_two_settings_entries_and_unlink_takes_them_back() {
    let env = Env::new(on(), &[FLOW]);
    env.put_settings(
        r#"{"zeta":1,"enabledPlugins":{"mine@other":true},"extraKnownMarketplaces":{"other":{"source":{"source":"github","repo":"a/b"}}}}"#,
    );
    let before = env.settings();
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(lines(&report), [WRITTEN]);
    assert_eq!(
        env.read(&format!("{PLUGIN}/.claude-plugin/marketplace.json")),
        MARKETPLACE
    );
    assert_eq!(
        env.read(&format!("{PLUGIN}/.claude-plugin/plugin.json")),
        PLUGIN_JSON
    );
    assert_eq!(
        env.read(&format!("{PLUGIN}/settings.json")),
        PLUGIN_SETTINGS
    );
    let s = env.settings();
    assert_eq!(
        s["enabledPlugins"],
        json!({"mine@other": true, "armadai-ui@armadai-ui": true})
    );
    assert_eq!(
        s["extraKnownMarketplaces"],
        json!({
            "other": {"source": {"source": "github", "repo": "a/b"}},
            "armadai-ui": {"source": {"source": "directory", "path": "./.armadai/plugins/armadai-ui"}}
        })
    );

    env.ok(&["unlink", "--target", "claude"]);
    assert_eq!(env.settings(), before);
    assert!(!env.root().join(".armadai/plugins").exists());
}

#[test]
fn a_project_linked_before_the_key_existed_links_and_unlinks_with_it() {
    let env = Env::new("schema_version: 1\ncoordinator: lead\n", &[FLOW]);
    env.put_settings(USER);
    let before = env.settings();
    env.ok(&["link", "--target", "claude"]);
    std::fs::write(env.root().join(".armadai/project.yaml"), on()).unwrap();
    env.ok(&["link", "--target", "claude"]);
    assert!(env.root().join(PLUGIN).is_dir());
    // the key goes away again: files, folders and entries with it
    std::fs::write(
        env.root().join(".armadai/project.yaml"),
        "schema_version: 1\ncoordinator: lead\n",
    )
    .unwrap();
    env.ok(&["link", "--target", "claude"]);
    assert!(!env.root().join(".armadai/plugins").exists());
    let s = env.settings();
    assert!(s.get("enabledPlugins").is_none() && s.get("extraKnownMarketplaces").is_none());
    env.ok(&["unlink", "--target", "claude"]);
    assert_eq!(env.settings(), before);
}

#[test]
fn a_value_of_the_user_on_a_plugin_key_stops_the_plugin_and_says_which_key() {
    let env = Env::new(on(), &[FLOW]);
    env.put_settings(r#"{"enabledPlugins":{"armadai-ui@armadai-ui":false}}"#);
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(
        lines(&report),
        ["claude: ui.subagent_status_line: not-compiled (key \
          enabledPlugins.armadai-ui@armadai-ui of .claude/settings.json already has another \
          value; link leaves it)"]
    );
    assert!(!env.root().join(".armadai/plugins").exists());
    let s = env.settings();
    assert_eq!(s["enabledPlugins"], json!({"armadai-ui@armadai-ui": false}));
    assert!(s.get("extraKnownMarketplaces").is_none());
}

#[test]
fn native_keys_of_the_plugin_collide_with_the_key() {
    for key in ["enabledPlugins", "extraKnownMarketplaces"] {
        let env = Env::new(
            &format!("{}native:\n  claude:\n    {key}: {{}}\n", on()),
            &[],
        );
        let out = env.run(&["link", "--target", "claude"]);
        assert!(!out.status.success(), "{key}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            format!(
                "Error: project.yaml: native.claude key `{key}` collides with ui.subagent_status_line\n"
            )
        );
        assert!(!env.root().join(".claude").exists(), "{key}");
    }
}

#[test]
fn other_targets_say_claude_code_only_and_only_with_the_key() {
    for target in ["opencode", "codex", "copilot"] {
        let env = Env::new(on(), &[]);
        let line = format!("{target}: ui.subagent_status_line: not-compiled (Claude Code only)");
        assert_eq!(lines(&env.ok(&["link", "--target", target])), [line]);
        assert!(!env.root().join(".armadai/plugins").exists(), "{target}");
        let env = Env::new("schema_version: 1\ncoordinator: lead\n", &[]);
        assert_eq!(
            lines(&env.ok(&["link", "--target", target])),
            Vec::<&str>::new(),
            "{target}"
        );
    }
}

#[test]
fn without_a_manifest_unlink_leaves_the_plugin_and_says_so() {
    let env = Env::new(on(), &[FLOW]);
    env.ok(&["link", "--target", "claude"]);
    std::fs::remove_file(env.root().join(".armadai/link-manifest.yaml")).unwrap();
    let out = env.run(&["unlink", "--target", "claude"]);
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.lines().any(|l| l
            == "  .armadai/plugins/armadai-ui/ not removed: no manifest records what link wrote there"),
        "{err}"
    );
    assert_eq!(
        env.read(&format!("{PLUGIN}/settings.json")),
        PLUGIN_SETTINGS
    );
}

#[cfg(unix)]
#[test]
fn link_refuses_a_plugin_folder_that_is_a_symbolic_link() {
    let env = Env::new(on(), &[FLOW]);
    let outside = env.0.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(env.root().join(".armadai")).unwrap();
    std::os::unix::fs::symlink(&outside, env.root().join(".armadai/plugins")).unwrap();
    let out = env.run(&["link", "--target", "claude"]);
    assert!(!out.status.success());
    assert!(
        std::fs::read_dir(&outside).unwrap().next().is_none(),
        "nothing may be written through the link"
    );
}
