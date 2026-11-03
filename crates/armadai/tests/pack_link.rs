//! `armadai link` and the policy gate on the effective project (pack reference, variables
//! resolved): what lands in the outputs, what stays in `.armadai/`, what the lock decides.
//! The snapshots are written by hand where `pack sync` would put them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use serde_json::{Value, json};

const URL: &str = "https://example.com/packs.git";
const C1: &str = "1111111111111111111111111111111111111111";
const C2: &str = "2222222222222222222222222222222222222222";
const REF: &str = "schema_version: 1\npack: { name: demo, source: 'https://example.com/packs.git', version: latest }\n";
const M6: &str = "set by none of ARMADAI_VAR_DATA, .armadai/vars.local.yaml, .armadai/vars.yaml: replaced by an empty value";

fn agent(body: &str) -> String {
    format!("---\ndescription: d\n---\n{body}\n")
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for d in ["project/.git", "config", "home"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let env = Env { dir };
        for (p, c) in files {
            put(&env.root(), p, c);
        }
        env
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// `demo` at `commit` in this machine's cache.
    fn snapshot(&self, commit: &str, files: &[(&str, &str)]) {
        let key = armadai_core::registries::cache_key(URL);
        let dir = self
            .dir
            .path()
            .join("config/registry/packs")
            .join(key)
            .join("demo")
            .join(commit);
        for (p, c) in files {
            put(&dir, p, c);
        }
    }

    fn drop_snapshot(&self, commit: &str) {
        let key = armadai_core::registries::cache_key(URL);
        let dir = self.dir.path().join("config/registry/packs").join(key);
        std::fs::remove_dir_all(dir.join("demo").join(commit)).unwrap();
    }

    fn lock(&self, version: &str, commit: &str) {
        put(
            &self.root(),
            ".armadai/pack.lock",
            &format!(
                "format: 1\npack: demo\nsource: '{URL}'\nresolved:\n  version: {version}\n  commit: '{commit}'\n"
            ),
        );
    }

    fn run(&self, args: &[&str], envs: &[(&str, &str)], stdin: &str) -> Output {
        Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("HOME", self.dir.path().join("home"))
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env_remove("ARMADAI_VAR_DATA")
            .envs(envs.iter().copied())
            .args(args)
            .write_stdin(stdin.to_string())
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str], envs: &[(&str, &str)]) -> String {
        let out = self.run(args, envs, "");
        let (o, e) = (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        );
        assert!(out.status.success(), "{args:?} failed:\n{o}\n{e}");
        o
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root().join(rel)).unwrap()
    }

    fn outputs(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut t: BTreeMap<PathBuf, Vec<u8>> = tree(&self.root().join(".claude"))
            .into_iter()
            .map(|(p, b)| (Path::new(".claude").join(p), b))
            .collect();
        t.extend(
            tree(&self.root())
                .into_iter()
                .filter(|(p, _)| p == Path::new("CLAUDE.md")),
        );
        t
    }
}

fn put(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Every file under `dir` with its bytes, keyed relative to `dir`.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd {
            let path = e.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

const VARS_PROJECT: &[(&str, &str)] = &[
    (
        ".armadai/project.yaml",
        "schema_version: 1\ntargets: [claude]\nnative:\n  claude:\n    permissions:\n      additionalDirectories: [\"{{var.data}}\"]\n",
    ),
    (
        ".armadai/agents/a.md",
        "---\ndescription: d\n---\nData: {{var.data}}\n",
    ),
    (
        ".armadai/commands/c.md",
        "---\ndescription: d\n---\nCommand: {{var.data}}\n",
    ),
    (
        ".armadai/skills/s/SKILL.md",
        "---\nname: s\ndescription: d\n---\nSkill: {{var.data}}\n",
    ),
];

#[test]
fn link_writes_resolved_values_only_in_outputs() {
    let mut files = VARS_PROJECT.to_vec();
    files.push((".armadai/vars.yaml", "data: /x/file\n"));
    let env = Env::new(&files);
    let before = tree(&env.root().join(".armadai"));
    env.ok(&["link"], &[("ARMADAI_VAR_DATA", "/x/env")]);

    let settings: Value = serde_json::from_str(&env.read(".claude/settings.json")).unwrap();
    assert_eq!(
        settings["permissions"]["additionalDirectories"],
        json!(["/x/env"])
    );
    let line = |rel: &str, want: &str| {
        assert!(
            env.read(rel).lines().any(|l| l == want),
            "{rel}: {}",
            env.read(rel)
        );
    };
    line(".claude/agents/a.md", "Data: /x/env");
    line(".claude/commands/c.md", "Command: /x/env");
    line(".claude/skills/s/SKILL.md", "Skill: /x/env");

    // the whole of `.armadai/` before and after: link adds only its manifest and
    // .gitignore, and changes nothing it read
    let mut after = tree(&env.root().join(".armadai"));
    let written: Vec<PathBuf> = ["link-manifest.yaml", ".gitignore"]
        .iter()
        .map(|f| {
            after
                .remove(Path::new(f))
                .map(|_| PathBuf::from(f))
                .unwrap()
        })
        .collect();
    assert_eq!(written.len(), 2);
    assert_eq!(after, before);
    // Dimitri, 2026-11-03: the local manifest keeps a resolved value in clear in a
    // managed array (`permissions.additionalDirectories`): accepted and documented.
    // Every other file of `.armadai/` stays free of it.
    for (p, bytes) in tree(&env.root().join(".armadai")) {
        if p != Path::new("link-manifest.yaml") {
            assert!(
                !String::from_utf8_lossy(&bytes).contains("/x/env"),
                "{} holds a resolved value",
                p.display()
            );
        }
    }
}

#[test]
fn a_missing_variable_warns_and_renders_empty() {
    let env = Env::new(VARS_PROJECT);
    let text = env.ok(&["link"], &[]);
    let line = format!("claude: var data: approximated ({M6})");
    assert!(text.lines().any(|l| l == line), "{text}");

    let env = Env::new(VARS_PROJECT);
    let json_out = env.ok(&["link", "--json"], &[]);
    let report: Vec<Value> = serde_json::from_str(&json_out).unwrap();
    let got: Vec<&Value> = report
        .iter()
        .filter(|l| l["element"] == "var data")
        .collect();
    assert_eq!(
        got,
        [
            &json!({"target": "claude", "element": "var data", "status": "approximated", "reason": M6})
        ]
    );

    let settings: Value = serde_json::from_str(&env.read(".claude/settings.json")).unwrap();
    assert_eq!(
        settings["permissions"]["additionalDirectories"],
        json!([""])
    );
    for (p, bytes) in env.outputs() {
        assert!(
            !String::from_utf8_lossy(&bytes).contains("{{var."),
            "{} keeps a marker",
            p.display()
        );
    }
}

fn pack_env(files: &[(&str, &str)]) -> Env {
    let mut all = vec![(".armadai/project.yaml", REF)];
    all.extend_from_slice(files);
    Env::new(&all)
}

const PACK_MANIFEST: &str = "schema_version: 1\ntargets: [claude]\n";

#[test]
fn link_reads_the_lock_not_the_range() {
    let env = pack_env(&[]);
    env.snapshot(
        C1,
        &[
            ("project.yaml", PACK_MANIFEST),
            ("agents/one.md", &agent("One")),
        ],
    );
    env.lock("1.0.0", C1);
    let text = env.ok(&["link"], &[]);
    assert!(
        text.lines()
            .any(|l| l == "claude: pack demo: compiled (1.0.0 (111111111111), from the cache)"),
        "{text}"
    );
    let first = env.outputs();
    assert!(first.contains_key(Path::new(".claude/agents/one.md")));

    // a newer version in the cache, range `latest`: the lock still decides
    env.snapshot(
        C2,
        &[
            ("project.yaml", PACK_MANIFEST),
            ("agents/two.md", &agent("Two")),
        ],
    );
    env.ok(&["link"], &[]);
    assert_eq!(env.outputs(), first);
}

#[test]
fn link_without_the_snapshot_names_pack_sync() {
    let env = pack_env(&[]);
    env.snapshot(
        C1,
        &[
            ("project.yaml", PACK_MANIFEST),
            ("agents/one.md", &agent("One")),
        ],
    );
    env.lock("1.0.0", C1);
    env.ok(&["link"], &[]);
    let linked = env.outputs();
    assert!(!linked.is_empty());
    env.drop_snapshot(C1);
    let out = env.run(&["link"], &[], "");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.lines().any(|l| l
            == "Error: pack demo 1.0.0 (111111111111) is not in the local cache: run `armadai pack sync`"),
        "{err}"
    );
    assert_eq!(env.outputs(), linked);
}

#[test]
fn two_machines_compile_the_same_outputs() {
    let machine = || {
        let env = pack_env(&[]);
        env.snapshot(
            C1,
            &[
                ("project.yaml", PACK_MANIFEST),
                ("agents/one.md", &agent("One")),
                (
                    "skills/s/SKILL.md",
                    "---\nname: s\ndescription: d\n---\nS\n",
                ),
            ],
        );
        env.lock("1.0.0", C1);
        env
    };
    let (a, b) = (machine(), machine());
    assert_ne!(a.root(), b.root());
    let ra = a.ok(&["link", "--json"], &[]);
    let rb = b.ok(&["link", "--json"], &[]);
    assert_eq!(ra, rb);
    assert!(!a.outputs().is_empty());
    assert_eq!(a.outputs(), b.outputs());
}

#[test]
fn a_dropped_output_is_removed() {
    let env = pack_env(&[]);
    env.snapshot(
        C1,
        &[
            ("project.yaml", PACK_MANIFEST),
            ("agents/old.md", &agent("Old")),
            ("agents/keep.md", &agent("Keep")),
        ],
    );
    env.snapshot(
        C2,
        &[
            ("project.yaml", PACK_MANIFEST),
            ("agents/keep.md", &agent("Keep")),
        ],
    );
    env.lock("1.0.0", C1);
    env.ok(&["link"], &[]);
    assert!(env.root().join(".claude/agents/old.md").is_file());

    env.lock("1.1.0", C2);
    let text = env.ok(&["link"], &[]);
    assert!(!env.root().join(".claude/agents/old.md").exists());
    assert!(
        text.lines()
            .any(|l| l
                == "claude: stale .claude/agents/old.md: compiled (removed: no longer produced)"),
        "{text}"
    );

    // edited by hand: kept, and said
    env.lock("1.0.0", C1);
    env.ok(&["link"], &[]);
    let old = env.root().join(".claude/agents/old.md");
    std::fs::write(&old, "mine\n").unwrap();
    env.lock("1.1.0", C2);
    let text = env.ok(&["link"], &[]);
    assert_eq!(std::fs::read_to_string(&old).unwrap(), "mine\n");
    assert!(
        text.lines().any(|l| l
            == "claude: stale .claude/agents/old.md: approximated (stale file left in place: edited since link wrote it)"),
        "{text}"
    );
}

#[test]
fn the_gate_reads_the_pack_flows() {
    let env = pack_env(&[]);
    let mut files = vec![(
        "project.yaml",
        "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n",
    )];
    let bodies: Vec<(String, String)> = ["lead", "dev", "qa", "doc"]
        .iter()
        .map(|a| (format!("agents/{a}.md"), agent(a)))
        .collect();
    files.extend(bodies.iter().map(|(p, c)| (p.as_str(), c.as_str())));
    files.push((
        "flows/ship.yaml",
        "steps:\n  build: { agent: dev }\n  test: { agent: qa }\nlinks:\n  - { from: build, to: test }\n",
    ));
    env.snapshot(C1, &files);
    env.lock("1.0.0", C1);
    let payload = |target: &str| {
        json!({
            "cwd": env.root().to_string_lossy(),
            "tool_name": "Agent",
            "agent_type": "",
            "tool_input": {"subagent_type": target},
        })
        .to_string()
    };
    let gate = |target: &str| {
        let out = env.run(&["__claude-policy-gate"], &[], &payload(target));
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(gate("dev"), "");
    let denied: Value = serde_json::from_str(&gate("doc")).unwrap();
    assert_eq!(
        denied,
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "the declared flows allow only [dev, qa] from the main thread; hand the work to one of those, or add `doc` to policy.free_agents in .armadai/project.yaml",
        }})
    );
}

const M6_OPENCODE: &str = "set by none of ARMADAI_VAR_DATA, .armadai/vars.local.yaml, .armadai/vars.yaml: replaced by an empty value";

#[test]
fn a_missing_variable_is_reported_for_each_target() {
    let mut files = VARS_PROJECT.to_vec();
    files[0] = (
        ".armadai/project.yaml",
        "schema_version: 1\ntargets: [claude, opencode]\n",
    );
    let env = Env::new(&files);
    let text = env.ok(&["link"], &[]);
    for target in ["claude", "opencode"] {
        let line = format!("{target}: var data: approximated ({M6_OPENCODE})");
        assert_eq!(
            text.lines().filter(|l| *l == line).count(),
            1,
            "{target}: {text}"
        );
    }
}

#[test]
fn the_gate_refuses_when_the_pack_view_cannot_be_opened() {
    let env = pack_env(&[]);
    env.snapshot(
        C1,
        &[
            (
                "project.yaml",
                "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n",
            ),
            ("agents/lead.md", &agent("lead")),
            ("agents/dev.md", &agent("dev")),
            (
                "flows/ship.yaml",
                "steps:\n  build: { agent: dev }\nlinks: []\n",
            ),
        ],
    );
    env.lock("1.0.0", C1);
    env.ok(&["link"], &[]);
    env.drop_snapshot(C1);
    let payload = json!({
        "cwd": env.root().to_string_lossy(),
        "tool_name": "Agent",
        "agent_type": "",
        "tool_input": {"subagent_type": "dev"},
    })
    .to_string();
    let out = env.run(&["__claude-policy-gate"], &[], &payload);
    assert!(out.status.success());
    let got: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        got,
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "the project's pack cannot be read, so the delegation policy cannot be checked: pack demo 1.0.0 (111111111111) is not in the local cache: run `armadai pack sync`",
        }})
    );
}
