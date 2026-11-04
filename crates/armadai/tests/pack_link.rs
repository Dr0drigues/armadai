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
            .env_remove("ARMADAI_VAR_DATA_DIR")
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
        let line = format!("{target}: var data: approximated ({M6})");
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

// Pack variables in the keys of `native` and in the `vars:` of template instances.

const KEYS_MANIFEST: &str = "schema_version: 1\ntargets: [claude, opencode]\nnative:\n  claude:\n    env:\n      \"{{var.data-dir}}/x\": \"1\"\n  opencode:\n    permission:\n      external_directory:\n        \"{{var.data-dir}}/**\": allow\n";
const E2: &str = "Error: project.yaml: under native, the key \"{{var.data-dir}}/x\" uses variable data-dir, which is empty or set by none of ARMADAI_VAR_DATA_DIR, .armadai/vars.local.yaml, .armadai/vars.yaml: a key never takes an empty value";

fn keys_env(extra: &[(&str, &str)]) -> Env {
    let mut files = vec![(".armadai/project.yaml", KEYS_MANIFEST)];
    files.extend_from_slice(extra);
    Env::new(&files)
}

fn whole(env: &Env) -> BTreeMap<PathBuf, Vec<u8>> {
    tree(&env.root())
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn opencode_key(env: &Env) -> Value {
    let v: Value = serde_json::from_str(&env.read("opencode.json")).unwrap();
    v["permission"]["external_directory"].clone()
}

#[test]
fn a_variable_in_a_native_key_is_rendered_by_priority() {
    // distinct values per layer: env > vars.local.yaml > vars.yaml
    let layers = [
        (&[][..], "/srv/shared"),
        (
            &[(".armadai/vars.local.yaml", "data-dir: /srv/local\n")][..],
            "/srv/local",
        ),
    ];
    for (local, want) in layers {
        let mut files = vec![(".armadai/vars.yaml", "data-dir: /srv/shared\n")];
        files.extend_from_slice(local);
        let env = keys_env(&files);
        env.ok(&["link"], &[]);
        assert_eq!(opencode_key(&env), json!({ format!("{want}/**"): "allow" }));
        let settings: Value = serde_json::from_str(&env.read(".claude/settings.json")).unwrap();
        assert_eq!(settings["env"], json!({ format!("{want}/x"): "1" }));
        for f in ["opencode.json", ".claude/settings.json"] {
            assert!(!env.read(f).contains("{{"), "{f}");
        }
    }
    let env = keys_env(&[
        (".armadai/vars.yaml", "data-dir: /srv/shared\n"),
        (".armadai/vars.local.yaml", "data-dir: /srv/local\n"),
    ]);
    env.ok(&["link"], &[("ARMADAI_VAR_DATA_DIR", "/srv/env")]);
    assert_eq!(opencode_key(&env), json!({"/srv/env/**": "allow"}));
    let settings: Value = serde_json::from_str(&env.read(".claude/settings.json")).unwrap();
    assert_eq!(settings["env"], json!({"/srv/env/x": "1"}));
}

#[test]
fn two_native_keys_rendering_the_same_text_stop_link_before_any_write() {
    let manifest = |extra: &str| {
        format!(
            "schema_version: 1\ntargets: [opencode]\nnative:\n  opencode:\n    permission:\n      external_directory:\n        \"{{{{var.a}}}}/**\": allow\n{extra}"
        )
    };
    let env = Env::new(&[
        (".armadai/project.yaml", &manifest("")),
        (".armadai/vars.yaml", "a: /srv/data\n"),
    ]);
    env.ok(&["link"], &[]);
    assert_eq!(opencode_key(&env), json!({"/srv/data/**": "allow"}));

    put(
        &env.root(),
        ".armadai/project.yaml",
        &manifest("        \"/srv/data/**\": deny\n"),
    );
    let before = whole(&env);
    let out = env.run(&["link"], &[], "");
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.lines().any(|l| l
            == "Error: project.yaml: under native, the keys \"{{var.a}}/**\" and \"/srv/data/**\" both render as \"/srv/data/**\""),
        "{err}"
    );
    // opencode.json and the manifest are byte-identical: nothing was written
    let mut after = whole(&env);
    let project = PathBuf::from(".armadai/project.yaml");
    assert_ne!(after.remove(&project), None);
    let mut before = before;
    before.remove(&project);
    assert_eq!(after, before);
}

#[test]
fn an_empty_or_unset_variable_in_a_native_key_stops_link_before_any_write() {
    // a first good link, so that "nothing written" is a comparison with real outputs
    let env = keys_env(&[(".armadai/vars.yaml", "data-dir: /srv/shared\n")]);
    env.ok(&["link"], &[]);
    let before = whole(&env);
    let cases: [(&str, Option<&str>, &str); 3] = [
        ("unset", None, ""),
        ("empty in vars.yaml", None, "data-dir: ''\n"),
        (
            "empty in the environment",
            Some(""),
            "data-dir: /srv/shared\n",
        ),
    ];
    for (name, envv, vars) in cases {
        put(&env.root(), ".armadai/vars.yaml", vars);
        let envs: Vec<(&str, &str)> = envv
            .map(|v| ("ARMADAI_VAR_DATA_DIR", v))
            .into_iter()
            .collect();
        let out = env.run(&["link"], &envs, "");
        assert_eq!(out.status.code(), Some(1), "{name}");
        let err = stderr(&out);
        assert!(err.lines().any(|l| l == E2), "{name}: {err}");
        let mut after = whole(&env);
        let mut want = before.clone();
        after.remove(Path::new(".armadai/vars.yaml"));
        want.remove(Path::new(".armadai/vars.yaml"));
        assert_eq!(after, want, "{name}");
    }

    // the same variable in a value, not a key: link succeeds with an empty value
    let env = Env::new(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\ntargets: [claude]\nnative:\n  claude:\n    permissions:\n      additionalDirectories: [\"{{var.data-dir}}\"]\n",
        ),
        (".armadai/vars.yaml", "data-dir: ''\n"),
    ]);
    env.ok(&["link"], &[]);
    let settings: Value = serde_json::from_str(&env.read(".claude/settings.json")).unwrap();
    assert_eq!(
        settings["permissions"]["additionalDirectories"],
        json!([""])
    );
    std::fs::remove_file(env.root().join(".armadai/vars.yaml")).unwrap();
    let text = env.ok(&["link"], &[]);
    let line = "claude: var data-dir: approximated (set by none of ARMADAI_VAR_DATA_DIR, .armadai/vars.local.yaml, .armadai/vars.yaml: replaced by an empty value)";
    assert!(text.lines().any(|l| l == line), "{text}");
}

#[test]
fn a_native_key_with_a_dot_after_rendering_names_the_rendered_key() {
    // R3: the refusal is the OpenCode adapter's, on the rendered key
    let env = Env::new(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\ntargets: [opencode]\nnative:\n  opencode:\n    \"{{var.k}}\": 1\n",
        ),
        (".armadai/vars.yaml", "k: a.b\n"),
    ]);
    let before = whole(&env);
    let out = env.run(&["link"], &[], "");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).lines().any(|l| l
            == "Error: native.opencode key `a.b` contains a `.`: armadai cannot own it in opencode.json"),
        "{}",
        stderr(&out)
    );
    assert_eq!(whole(&env), before);
}

const TEMPLATE: &str = "---\ndescription: d\n---\nIn {{dir}}\n";

fn instance_env(vars_block: &str, shared: &str) -> Env {
    Env::new(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\ntargets: [claude]\n",
        ),
        (".armadai/templates/t.md", TEMPLATE),
        (
            ".armadai/agents/r.md",
            &format!("---\nfrom: t\n{vars_block}---\nbody\n"),
        ),
        (".armadai/vars.yaml", shared),
    ])
}

#[test]
fn an_instance_gets_rendered_vars_and_its_source_stays_unchanged() {
    for (value, yaml, want) in [
        ("/srv/data", "data-dir: /srv/data\n", "In /srv/data"),
        ("a: b # c", "data-dir: 'a: b # c'\n", "In a: b # c"),
    ] {
        let env = instance_env("vars:\n  dir: \"{{var.data-dir}}\"\n", yaml);
        let src = env.read(".armadai/agents/r.md");
        env.ok(&["link"], &[]);
        let out = env.read(".claude/agents/r.md");
        assert!(out.lines().any(|l| l == want), "{value}: {out}");
        assert!(!out.contains("{{"), "{out}");
        assert_eq!(env.read(".armadai/agents/r.md"), src);
    }
}

#[test]
fn a_nested_or_list_value_in_instance_vars_is_refused_before_any_write() {
    for block in [
        "vars:\n  dir:\n    x: \"{{var.data-dir}}\"\n",
        "vars:\n  dir:\n    - \"{{var.data-dir}}\"\n",
    ] {
        let env = instance_env(block, "data-dir: /srv/data\n");
        let before = whole(&env);
        let out = env.run(&["link"], &[], "");
        assert_eq!(out.status.code(), Some(1), "{block}");
        let err = stderr(&out);
        assert!(
            err.contains("invalid type") && err.contains("expected a string"),
            "{block}: {err}"
        );
        assert_eq!(whole(&env), before, "{block}");
    }
}

#[test]
fn validate_keeps_pack_variables_literal_and_reports_no_error() {
    let env = Env::new(&[
        (".armadai/project.yaml", KEYS_MANIFEST),
        (".armadai/templates/t.md", TEMPLATE),
        (
            ".armadai/agents/r.md",
            "---\nfrom: t\nvars:\n  dir: \"{{var.data-dir}}\"\n---\nbody\n",
        ),
    ]);
    let out = env.run(&["validate"], &[], "");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "{text}{}", stderr(&out));
    assert_eq!(text, "0 error(s), 0 warning(s)\n");
}
