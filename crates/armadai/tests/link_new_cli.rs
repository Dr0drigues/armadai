//! `armadai link` / `unlink` on the canonical model, end to end (step 2, Task 6):
//! refusal of a legacy project, `--dry-run` and `--json` purity, `native` files,
//! files shared with the user (`.claude/settings.json`, the root `CLAUDE.md`),
//! stale files, the no-manifest fallback.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

const PROJECT: &[(&str, &str)] = &[
    (
        ".armadai/project.yaml",
        "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n",
    ),
    (
        ".armadai/agents/lead.md",
        "---\ndescription: Leads\n---\nYou lead.\n",
    ),
    (
        ".armadai/agents/dev.md",
        "---\ndescription: Writes code\n---\nYou write code.\n",
    ),
    (
        ".armadai/skills/notes/SKILL.md",
        "---\nname: notes\ndescription: Notes\n---\nTake notes.\n",
    ),
    (
        ".armadai/flows/ship.yaml",
        "steps:\n  plan: { agent: lead }\n  build: { agent: dev }\nlinks:\n  - { from: plan, to: build }\n",
    ),
    (
        ".armadai/automations/fmt.yaml",
        "event: after-tool\nrun: cargo fmt\n",
    ),
];

/// A user's own Claude settings (hooks, permissions, an unknown key) and CLAUDE.md.
const USER_SETTINGS: &str = r#"{"zeta": 1, "permissions": {"allow": ["Bash(ls)"]}, "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "./mine.sh"}]}]}}"#;
const USER_CLAUDE_MD: &str = "# My notes\n\nKeep me.\n";

fn write(root: &Path, files: &[(&str, &str)]) {
    for (p, c) in files {
        let path = root.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, c).unwrap();
    }
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("project")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        write(&dir.path().join("project"), files);
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .args(args)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn ok(&self, args: &[&str]) -> String {
        let (ok, stdout, stderr) = self.run(args);
        assert!(ok, "{args:?} failed:\n{stdout}\n{stderr}");
        stdout
    }
}

/// Every file under `root` (bytes; `settings.json` compared as JSON values),
/// skipping link's own bookkeeping under `.armadai/`.
fn tree(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            if rel == Path::new(".armadai/link-manifest.yaml")
                || rel == Path::new(".armadai/.gitignore")
            {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
                let text = if rel.ends_with("settings.json") {
                    serde_json::from_str::<serde_json::Value>(&text)
                        .unwrap()
                        .to_string()
                } else {
                    text
                };
                out.insert(rel, text);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Every path under `root`, files and directories, `.armadai/` bookkeeping included.
fn paths(root: &Path) -> Vec<PathBuf> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            out.push(path.strip_prefix(root).unwrap().to_path_buf());
            if path.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn a_legacy_project_is_refused_with_the_migrate_hint() {
    let env = Env::new(&[(".armadai/config.yaml", "agents:\n  - name: dev\n")]);
    let before = paths(&env.root());
    let (ok, _, stderr) = env.run(&["link"]);
    assert!(!ok);
    assert!(
        stderr.contains("legacy format, run `armadai migrate`"),
        "{stderr}"
    );
    assert_eq!(paths(&env.root()), before, "nothing written");
}

#[test]
fn dry_run_writes_nothing_at_all() {
    let env = Env::new(PROJECT);
    write(
        &env.root(),
        &[
            (".claude/settings.json", USER_SETTINGS),
            ("CLAUDE.md", USER_CLAUDE_MD),
        ],
    );
    let before = (paths(&env.root()), tree(&env.root()));
    let out = env.ok(&["link", "--dry-run"]);
    assert!(out.contains(".claude/agents/dev.md"), "{out}");
    assert!(out.contains("CLAUDE.md (ArmadAI block)"), "{out}");
    assert_eq!((paths(&env.root()), tree(&env.root())), before);
}

#[test]
fn json_prints_only_the_report() {
    let env = Env::new(PROJECT);
    let out = env.ok(&["link", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("stdout is one JSON document");
    let lines = v.as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l["element"] == "agent dev" && l["status"] == "compiled")
    );
    assert!(
        lines
            .iter()
            .any(|l| l["element"] == "policy delegation=strict")
    );
}

#[test]
fn link_then_unlink_gives_the_user_tree_back_and_relinking_is_idempotent() {
    let env = Env::new(PROJECT);
    write(
        &env.root(),
        &[
            (".claude/settings.json", USER_SETTINGS),
            ("CLAUDE.md", USER_CLAUDE_MD),
        ],
    );
    let before = tree(&env.root());

    env.ok(&["link"]);
    let linked = tree(&env.root());
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(env.root().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["zeta"], 1, "user keys kept");
    assert_eq!(
        settings["hooks"]["PreToolUse"].as_array().unwrap().len(),
        2,
        "user hook + gate"
    );
    let md = std::fs::read_to_string(env.root().join("CLAUDE.md")).unwrap();
    assert!(
        md.starts_with(USER_CLAUDE_MD) && md.contains("You lead."),
        "{md}"
    );
    assert!(
        std::fs::read_to_string(env.root().join(".armadai/.gitignore"))
            .unwrap()
            .contains("link-manifest.yaml")
    );

    let again = env.ok(&["link"]);
    assert_eq!(tree(&env.root()), linked, "relinking changes nothing");
    assert!(again.contains("up-to-date"), "{again}");

    env.ok(&["unlink"]);
    assert_eq!(
        tree(&env.root()),
        before,
        "unlink removes exactly what link added"
    );
}

#[test]
fn a_native_file_is_copied_to_the_project_root_and_unlinked() {
    let mut files = PROJECT.to_vec();
    files.push((".armadai/native/claude/.mcp.json", "{\"mcpServers\": {}}\n"));
    let env = Env::new(&files);
    let out = env.ok(&["link"]);
    assert_eq!(
        std::fs::read_to_string(env.root().join(".mcp.json")).unwrap(),
        "{\"mcpServers\": {}}\n"
    );
    assert!(out.contains("native .mcp.json: compiled"), "{out}");
    env.ok(&["unlink"]);
    assert!(!env.root().join(".mcp.json").exists());
}

#[test]
fn a_native_file_colliding_with_a_generated_one_is_an_error_without_writes() {
    let mut files = PROJECT.to_vec();
    files.push((".armadai/native/claude/.claude/agents/dev.md", "mine\n"));
    let env = Env::new(&files);
    let before = paths(&env.root());
    let (ok, _, stderr) = env.run(&["link"]);
    assert!(!ok);
    assert!(stderr.contains("collides"), "{stderr}");
    assert_eq!(paths(&env.root()), before);
}

#[cfg(unix)]
#[test]
fn a_native_symlink_leaving_its_folder_is_refused_without_writes() {
    let env = Env::new(PROJECT);
    let outside = env.dir.path().join("secret");
    std::fs::write(&outside, "secret").unwrap();
    std::fs::create_dir_all(env.root().join(".armadai/native/claude")).unwrap();
    std::os::unix::fs::symlink(&outside, env.root().join(".armadai/native/claude/leak")).unwrap();
    let before = paths(&env.root());
    let (ok, _, stderr) = env.run(&["link"]);
    assert!(!ok);
    assert!(stderr.contains("leak"), "{stderr}");
    assert_eq!(paths(&env.root()), before);
}

#[test]
fn an_invalid_project_is_refused_without_writes() {
    // a duplicate key: the legacy parser kept the last value; the model refuses
    let mut files = PROJECT.to_vec();
    files.push((
        ".armadai/agents/bad.md",
        "---\ndescription: a\ndescription: b\n---\nB\n",
    ));
    let env = Env::new(&files);
    let before = paths(&env.root());
    let (ok, _, stderr) = env.run(&["link"]);
    assert!(!ok);
    assert!(
        stderr.contains("bad.md"),
        "the offending file is named: {stderr}"
    );
    assert_eq!(paths(&env.root()), before);
}

#[test]
fn a_coordinator_naming_no_agent_is_refused() {
    let mut files = PROJECT.to_vec();
    files[0] = (
        ".armadai/project.yaml",
        "schema_version: 1\ncoordinator: ghost\n",
    );
    let env = Env::new(&files);
    let (ok, _, stderr) = env.run(&["link"]);
    assert!(!ok);
    assert!(stderr.contains("unknown agent `ghost`"), "{stderr}");
}

#[test]
fn output_with_several_targets_and_unknown_agents_are_refused() {
    let env = Env::new(PROJECT);
    let (ok, _, stderr) = env.run(&["link", "--target", "claude", "codex", "--output", "x"]);
    assert!(!ok && stderr.contains("--output"), "{stderr}");
    let (ok, _, stderr) = env.run(&["link", "--agents", "nobody"]);
    assert!(!ok && stderr.contains("nobody"), "{stderr}");
    for removed in ["--coordinator", "--model"] {
        let (ok, _, _) = env.run(&["link", removed, "x"]);
        assert!(!ok, "{removed} no longer exists");
    }
}

#[test]
fn agents_filter_writes_only_the_named_agents_and_reports_the_others() {
    let mut files = PROJECT.to_vec();
    files.push((
        ".armadai/agents/qa.md",
        "---\ndescription: Tests\n---\nYou test.\n",
    ));
    let env = Env::new(&files);
    let out = env.ok(&["link", "--agents", "dev"]);
    assert!(env.root().join(".claude/agents/dev.md").is_file());
    assert!(!env.root().join(".claude/agents/qa.md").exists());
    assert!(
        out.contains("agent qa: not-compiled (skipped (--agents))"),
        "{out}"
    );
    assert!(
        env.root().join("CLAUDE.md").is_file(),
        "the root file is not filtered"
    );
}

#[test]
fn an_agent_removed_from_the_project_has_its_file_removed_on_relink() {
    let env = Env::new(PROJECT);
    env.ok(&["link"]);
    let dev = env.root().join(".claude/agents/dev.md");
    assert!(dev.is_file());
    std::fs::remove_file(env.root().join(".armadai/agents/dev.md")).unwrap();
    std::fs::write(
        env.root().join(".armadai/flows/ship.yaml"),
        "steps:\n  plan: { agent: lead }\n",
    )
    .unwrap();
    let out = env.ok(&["link"]);
    assert!(!dev.exists(), "stale file removed");
    assert!(
        out.contains("stale .claude/agents/dev.md: compiled (removed"),
        "{out}"
    );
}

#[test]
fn an_edited_stale_file_is_left_in_place_and_reported() {
    let env = Env::new(PROJECT);
    env.ok(&["link"]);
    let dev = env.root().join(".claude/agents/dev.md");
    std::fs::write(&dev, "edited by hand\n").unwrap();
    std::fs::remove_file(env.root().join(".armadai/agents/dev.md")).unwrap();
    std::fs::write(
        env.root().join(".armadai/flows/ship.yaml"),
        "steps:\n  plan: { agent: lead }\n",
    )
    .unwrap();
    let out = env.ok(&["link"]);
    assert_eq!(std::fs::read_to_string(&dev).unwrap(), "edited by hand\n");
    assert!(out.contains("stale file left in place"), "{out}");
}

#[test]
fn the_former_claude_md_under_dot_claude_is_reported() {
    let mut files = PROJECT.to_vec();
    files.push((".claude/CLAUDE.md", "old coordinator file\n"));
    let env = Env::new(&files);
    let out = env.ok(&["link"]);
    assert!(
        out.contains(".claude/CLAUDE.md: approximated (stale file left in place"),
        "{out}"
    );
    assert!(env.root().join(".claude/CLAUDE.md").is_file());
}

/// Without a manifest, unlink removes only files still equal to what link would
/// write now, and never touches the files shared with the user.
#[test]
fn without_a_manifest_unlink_removes_only_unchanged_generated_files() {
    let env = Env::new(PROJECT);
    write(&env.root(), &[(".claude/settings.json", USER_SETTINGS)]);
    env.ok(&["link"]);
    std::fs::remove_file(env.root().join(".armadai/link-manifest.yaml")).unwrap();
    let skill_user_file = env.root().join(".claude/skills/notes/mine.txt");
    std::fs::write(&skill_user_file, "mine\n").unwrap();
    let edited = env.root().join(".claude/skills/notes/SKILL.md");
    std::fs::write(&edited, "edited\n").unwrap();
    let settings = std::fs::read_to_string(env.root().join(".claude/settings.json")).unwrap();

    let (ok, stdout, stderr) = env.run(&["unlink"]);
    assert!(ok, "{stderr}");
    assert!(stderr.contains("No link manifest found"), "{stderr}");
    assert!(
        stderr.contains("hooks/CLAUDE.md block not removed"),
        "{stderr}"
    );
    assert!(
        !env.root().join(".claude/agents/dev.md").exists(),
        "{stdout}"
    );
    assert!(
        edited.is_file() && skill_user_file.is_file(),
        "user content kept"
    );
    assert!(
        env.root().join(".claude").is_dir(),
        "the target root survives"
    );
    assert_eq!(
        std::fs::read_to_string(env.root().join(".claude/settings.json")).unwrap(),
        settings
    );
    assert!(env.root().join("CLAUDE.md").is_file());
}

/// A manifest is data on disk: a forged patched path outside where link patches
/// is refused, and the file it names is left untouched.
#[test]
fn a_forged_patched_path_is_refused_untouched() {
    let env = Env::new(PROJECT);
    env.ok(&["link"]);
    let victim = env.dir.path().join("victim.md");
    let text = "<!-- armadai:begin -->\nmine\n<!-- armadai:end -->\n";
    std::fs::write(&victim, text).unwrap();
    let manifest = env.root().join(".armadai/link-manifest.yaml");
    let forged = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace("path: CLAUDE.md", "path: ../victim.md");
    assert!(forged.contains("../victim.md"), "{forged}");
    std::fs::write(&manifest, forged).unwrap();

    let (ok, _, stderr) = env.run(&["unlink"]);
    assert!(!ok, "a refusal fails the run");
    assert!(stderr.contains("refused"), "{stderr}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), text);
}

/// A hook identical to one link would add is the user's: kept, reported, never
/// claimed, so unlink leaves it.
#[test]
fn an_identical_user_hook_is_reported_and_left_to_the_user() {
    let env = Env::new(PROJECT);
    let user =
        r#"{"hooks": {"PostToolUse": [{"hooks": [{"type": "command", "command": "cargo fmt"}]}]}}"#;
    write(&env.root(), &[(".claude/settings.json", user)]);
    let out = env.ok(&["link"]);
    assert!(
        out.contains(
            "hook PostToolUse cargo fmt: approximated (existing identical hook kept, not owned)"
        ),
        "{out}"
    );
    assert!(out.contains("settings.json reformatted"), "{out}");
    env.ok(&["unlink"]);
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(env.root().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(v, serde_json::from_str::<serde_json::Value>(user).unwrap());
}

/// A settings.json link created holds nothing once the project has no hook left:
/// relinking removes it rather than leaving `{}`.
#[test]
fn relinking_without_hooks_removes_the_settings_file_link_created() {
    let env = Env::new(PROJECT);
    env.ok(&["link"]);
    let settings = env.root().join(".claude/settings.json");
    assert!(settings.is_file());
    std::fs::remove_file(env.root().join(".armadai/automations/fmt.yaml")).unwrap();
    std::fs::write(
        env.root().join(".armadai/project.yaml"),
        "schema_version: 1\ncoordinator: lead\n",
    )
    .unwrap();
    env.ok(&["link"]);
    assert!(!settings.exists());
}
