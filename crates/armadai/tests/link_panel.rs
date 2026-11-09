//! `ui.panel` end to end: the mod `link` adds to the `armadai-ui` plugin (its JavaScript,
//! its `panel.json` and its `hooks.json`), the report line, the other targets, and what
//! `unlink` takes back. The JavaScript itself is tested by `claude plugin test`
//! (`src/compile/ui/armadai-ui-panel.test.ts`), which CI cannot run.

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

/// The file `link` must write as the mod, byte for byte.
const MOD_JS: &str = include_str!("../src/compile/ui/armadai-ui-panel.js");

const PANEL_LINE: &str = "claude: ui.panel: approximated (mod in .armadai/plugins/armadai-ui: \
runs JavaScript and the armadai command with your permissions; needs Claude Code 2.1.318 or \
later, off under --safe-mode, disableAllHooks or claude -p)";

const HOOKS_JSON: &str = "{\n  \"modules\": [\n    \"./register.js\"\n  ]\n}\n";

impl Env {
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root().join(rel)).unwrap()
    }

    /// Every file under the plugin folder, relative to it, sorted.
    fn plugin_files(&self) -> Vec<String> {
        fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else {
                    out.push(p.strip_prefix(base).unwrap().display().to_string());
                }
            }
        }
        let base = self.root().join(PLUGIN);
        let mut out = Vec::new();
        walk(&base, &base, &mut out);
        out.sort();
        out
    }
}

fn lines<'a>(report: &'a str, key: &str) -> Vec<&'a str> {
    report.lines().filter(|l| l.contains(key)).collect()
}

fn project(ui: &str) -> String {
    format!("schema_version: 1\ncoordinator: lead\nui:\n{ui}")
}

const THREE: [&str; 3] = [
    ".claude-plugin/marketplace.json",
    ".claude-plugin/plugin.json",
    "settings.json",
];

#[test]
fn without_the_key_no_file_of_the_mod_is_written() {
    // the plugin exists (the other key), the mod does not; and with neither key, no plugin
    let env = Env::new(&project("  subagent_status_line: true\n"), &[FLOW]);
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(env.plugin_files(), THREE);
    assert_eq!(lines(&report, "ui.panel"), Vec::<&str>::new());
    let env = Env::new(&project("  panel: false\n"), &[FLOW]);
    env.ok(&["link", "--target", "claude"]);
    assert!(!env.root().join(".armadai/plugins").exists());
}

#[test]
fn the_key_writes_the_mod_as_is_with_its_data_and_one_approximated_line() {
    let env = Env::new(&project("  panel: true\n"), &[FLOW]);
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(lines(&report, "ui.panel"), [PANEL_LINE]);
    // the panel alone: no settings.json of the plugin (that is the other key's)
    assert_eq!(
        env.plugin_files(),
        [
            ".claude-plugin/marketplace.json",
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
            "hooks/register.js",
            "panel.json",
        ]
    );
    assert_eq!(env.read(&format!("{PLUGIN}/hooks/register.js")), MOD_JS);
    assert_eq!(env.read(&format!("{PLUGIN}/hooks/hooks.json")), HOOKS_JSON);
    assert_eq!(
        env.read(&format!("{PLUGIN}/panel.json")),
        "{\n  \"documents\": [\n    \"docs\"\n  ]\n}\n"
    );
    // the plugin is enabled in the project settings
    assert_eq!(
        env.settings()["enabledPlugins"],
        json!({"armadai-ui@armadai-ui": true})
    );
}

#[test]
fn ui_documents_go_to_panel_json_in_order() {
    let env = Env::new(
        &project("  panel: true\n  documents: [notes/specs, plans]\n"),
        &[FLOW],
    );
    env.ok(&["link", "--target", "claude"]);
    assert_eq!(
        env.read(&format!("{PLUGIN}/panel.json")),
        "{\n  \"documents\": [\n    \"notes/specs\",\n    \"plans\"\n  ]\n}\n"
    );
    let env = Env::new(&project("  panel: true\n  documents: []\n"), &[FLOW]);
    env.ok(&["link", "--target", "claude"]);
    assert_eq!(
        env.read(&format!("{PLUGIN}/panel.json")),
        "{\n  \"documents\": []\n}\n"
    );
}

#[test]
fn both_keys_share_the_plugin_each_with_its_own_line() {
    let env = Env::new(
        &project("  subagent_status_line: true\n  panel: true\n"),
        &[FLOW],
    );
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(lines(&report, "ui.panel"), [PANEL_LINE]);
    assert_eq!(
        lines(&report, "ui.subagent_status_line"),
        [
            "claude: ui.subagent_status_line: compiled (plugin armadai-ui in \
.armadai/plugins/armadai-ui, enabled in .claude/settings.json; Claude Code loads it once \
you trust the folder)"
        ]
    );
    assert_eq!(env.plugin_files().len(), 6);
}

#[test]
fn unlink_takes_the_mod_back_with_the_plugin_and_the_settings_entries() {
    let env = Env::new(&project("  panel: true\n"), &[FLOW]);
    env.put_settings(r#"{"zeta":1}"#);
    let before = env.settings();
    env.ok(&["link", "--target", "claude"]);
    env.ok(&["unlink", "--target", "claude"]);
    assert!(!env.root().join(".armadai/plugins").exists());
    assert_eq!(env.settings(), before);
}

#[test]
fn turning_the_key_off_removes_the_mod_and_keeps_the_rest_of_the_plugin() {
    let env = Env::new(
        &project("  subagent_status_line: true\n  panel: true\n"),
        &[FLOW],
    );
    env.ok(&["link", "--target", "claude"]);
    std::fs::write(
        env.root().join(".armadai/project.yaml"),
        project("  subagent_status_line: true\n"),
    )
    .unwrap();
    env.ok(&["link", "--target", "claude"]);
    assert_eq!(env.plugin_files(), THREE);
}

#[test]
fn a_clone_does_not_own_the_mod_and_unlink_leaves_it() {
    let env = Env::new(&project("  panel: true\n"), &[FLOW]);
    env.ok(&["link", "--target", "claude"]);
    std::fs::remove_file(env.root().join(".armadai/link-manifest.yaml")).unwrap();
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(
        lines(&report, "ui.panel"),
        [
            "claude: ui.panel: approximated (existing identical plugin kept, not owned: unlink leaves it)"
        ]
    );
    env.ok(&["unlink", "--target", "claude"]);
    assert_eq!(env.read(&format!("{PLUGIN}/hooks/register.js")), MOD_JS);
}

#[test]
fn a_mod_file_that_differs_is_left_and_named_in_the_line_of_the_panel() {
    let env = Env::new(&project("  panel: true\n"), &[FLOW]);
    env.ok(&["link", "--target", "claude"]);
    std::fs::remove_file(env.root().join(".armadai/link-manifest.yaml")).unwrap();
    std::fs::write(
        env.root().join(PLUGIN).join("hooks/register.js"),
        "// mine\n",
    )
    .unwrap();
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(
        lines(&report, "ui.panel"),
        [format!(
            "claude: ui.panel: not-compiled (existing plugin differs from what link writes at \
             {PLUGIN}/hooks/register.js, not owned: link leaves it; remove it to let link write it)"
        )]
    );
    assert_eq!(
        env.read(&format!("{PLUGIN}/hooks/register.js")),
        "// mine\n"
    );
}

#[test]
fn the_panel_key_collides_with_the_native_plugin_keys() {
    let env = Env::new(
        &format!(
            "{}native:\n  claude:\n    enabledPlugins: {{}}\n",
            project("  panel: true\n")
        ),
        &[],
    );
    let out = env.run(&["link", "--target", "claude"]);
    assert!(!out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Error: project.yaml: native.claude key `enabledPlugins` collides with ui.panel\n"
    );
}

#[test]
fn other_targets_say_claude_code_only_and_write_no_mod() {
    for target in ["opencode", "codex", "copilot"] {
        let env = Env::new(&project("  panel: true\n"), &[]);
        let line = format!("{target}: ui.panel: not-compiled (Claude Code only)");
        assert_eq!(
            lines(&env.ok(&["link", "--target", target]), "ui.panel"),
            [line]
        );
        assert!(!env.root().join(".armadai/plugins").exists(), "{target}");
    }
}

#[test]
fn a_mod_file_edited_after_the_link_is_kept_and_reported_under_the_panel_key() {
    let env = Env::new(
        &project("  subagent_status_line: true\n  panel: true\n"),
        &[FLOW],
    );
    env.ok(&["link", "--target", "claude"]);
    std::fs::write(
        env.root().join(PLUGIN).join("hooks/register.js"),
        "// mine\n",
    )
    .unwrap();
    std::fs::write(env.root().join(PLUGIN).join("settings.json"), "{}\n").unwrap();
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(
        lines(&report, "ui."),
        [
            format!(
                "claude: ui.panel: not-compiled (edited since the last link (use --force): \
                 {PLUGIN}/hooks/register.js)"
            ),
            format!(
                "claude: ui.subagent_status_line: not-compiled (edited since the last link \
                 (use --force): {PLUGIN}/settings.json)"
            ),
        ]
    );
    assert_eq!(
        env.read(&format!("{PLUGIN}/hooks/register.js")),
        "// mine\n"
    );
}

const SIX: [&str; 6] = [
    ".claude-plugin/marketplace.json",
    ".claude-plugin/plugin.json",
    "hooks/hooks.json",
    "hooks/register.js",
    "panel.json",
    "settings.json",
];

fn rewrite(env: &Env, ui: &str) {
    std::fs::write(env.root().join(".armadai/project.yaml"), project(ui)).unwrap();
}

#[test]
fn adding_the_panel_to_a_plugin_already_linked_writes_the_mod_and_keeps_the_rest() {
    let env = Env::new(&project("  subagent_status_line: true\n"), &[FLOW]);
    env.ok(&["link", "--target", "claude"]);
    let settings = env.settings();
    rewrite(&env, "  subagent_status_line: true\n  panel: true\n");
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(lines(&report, "ui.panel"), [PANEL_LINE]);
    assert_eq!(env.plugin_files(), SIX);
    assert_eq!(env.read(&format!("{PLUGIN}/hooks/register.js")), MOD_JS);
    assert_eq!(env.settings(), settings);
    // and unlink takes the whole plugin back, the mod included
    env.ok(&["unlink", "--target", "claude"]);
    assert!(!env.root().join(".armadai/plugins").exists());
}

#[test]
fn removing_the_other_key_keeps_the_mod_and_the_settings_entries() {
    let env = Env::new(
        &project("  subagent_status_line: true\n  panel: true\n"),
        &[FLOW],
    );
    env.ok(&["link", "--target", "claude"]);
    let settings = env.settings();
    rewrite(&env, "  panel: true\n");
    let report = env.ok(&["link", "--target", "claude"]);
    assert_eq!(lines(&report, "ui.panel"), [PANEL_LINE]);
    assert_eq!(
        lines(&report, "ui.subagent_status_line"),
        Vec::<&str>::new()
    );
    // only the plugin's own settings.json (the other key's) goes
    assert_eq!(
        env.plugin_files(),
        SIX.iter()
            .filter(|f| **f != "settings.json")
            .copied()
            .collect::<Vec<_>>()
    );
    assert_eq!(env.settings(), settings);
}
