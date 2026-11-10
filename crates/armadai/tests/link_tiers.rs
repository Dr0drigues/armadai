//! `armadai link`, end to end: what each target writes for every tier (#455), and
//! that `link` neither reads the models cache nor opens a connection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use armadai_core::model_resolution::{ModelTier, fallback_model_for_tier};
use assert_cmd::Command;
use serde_json::Value;

/// `quick`/`steady`/`deep` carry a tier, `plain` has none.
const FILES: &[(&str, &str)] = &[
    (".armadai/project.yaml", "schema_version: 1\n"),
    (
        ".armadai/agents/quick.md",
        "---\ndescription: d\ntier: fast\n---\nB\n",
    ),
    (
        ".armadai/agents/steady.md",
        "---\ndescription: d\ntier: pro\n---\nB\n",
    ),
    (
        ".armadai/agents/deep.md",
        "---\ndescription: d\ntier: max\n---\nB\n",
    ),
    (".armadai/agents/plain.md", "---\ndescription: d\n---\nB\n"),
];

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for d in ["project", "config", "home", "bin"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        for (p, c) in FILES {
            let path = dir.path().join("project").join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn command(&self) -> Command {
        let mut c = Command::cargo_bin("armadai").unwrap();
        c.current_dir(self.root())
            .env("HOME", self.dir.path().join("home"))
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"));
        c
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.command().args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    fn report(&self, target: &str) -> Vec<Value> {
        serde_json::from_str(&self.ok(&["link", "--target", target, "--json"])).unwrap()
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root().join(rel)).unwrap()
    }
}

/// Every file under `root` with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
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
    walk(root, root, &mut out);
    out
}

#[test]
fn codex_writes_the_table_model_of_each_tier() {
    let (fast, pro, max) = (
        fallback_model_for_tier("openai", ModelTier::Fast),
        fallback_model_for_tier("openai", ModelTier::Pro),
        fallback_model_for_tier("openai", ModelTier::Max),
    );
    assert!(fast != pro && pro != max && fast != max);
    let env = Env::new();
    env.ok(&["link", "--target", "codex"]);
    for (agent, want) in [
        ("quick", fast),
        ("steady", pro),
        ("deep", max),
        ("plain", pro),
    ] {
        let toml = env.read(&format!(".codex/agents/{agent}.toml"));
        assert_eq!(
            toml.lines().find(|l| l.starts_with("model")),
            Some(format!("model = \"{want}\"").as_str()),
            "{agent}"
        );
    }
}

#[test]
fn claude_writes_the_alias_of_each_tier_and_none_without_one() {
    let env = Env::new();
    env.ok(&["link", "--target", "claude"]);
    for (agent, want) in [
        ("quick", Some("model: haiku")),
        ("steady", Some("model: sonnet")),
        ("deep", Some("model: opus")),
        ("plain", None),
    ] {
        let md = env.read(&format!(".claude/agents/{agent}.md"));
        assert_eq!(md.lines().find(|l| l.starts_with("model")), want, "{agent}");
    }
}

#[test]
fn copilot_writes_no_model_and_reports_each_tier_not_compiled() {
    let env = Env::new();
    let report = env.report("copilot");
    for (path, bytes) in tree(&env.root().join(".github")) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains("model"),
            "{path:?} mentions a model"
        );
    }
    let mut tiers: Vec<_> = report
        .iter()
        .filter(|l| l["element"].as_str().unwrap().ends_with(" tier"))
        .map(|l| {
            (
                l["element"].as_str().unwrap(),
                l["status"].as_str().unwrap(),
            )
        })
        .collect();
    tiers.sort();
    assert_eq!(
        tiers,
        [
            ("agent deep tier", "not-compiled"),
            ("agent quick tier", "not-compiled"),
            ("agent steady tier", "not-compiled"),
        ]
    );
}

/// A cache that names only made-up models must not change a byte of what `link`
/// prints or writes.
#[test]
fn link_never_reads_the_models_cache() {
    let (without, with) = (Env::new(), Env::new());
    let cache = r#"{"fetched_at":1,"providers":{"openai":[
        {"id":"gpt-99-witness","cost":{"input":0.1,"output":0.2}},
        {"id":"gpt-99-witness-mini","cost":{"input":0.01,"output":0.02}},
        {"id":"gpt-99-witness-pro","cost":{"input":9.0,"output":9.0}}]}}"#;
    std::fs::write(
        with.dir.path().join("config").join("models-cache.json"),
        cache,
    )
    .unwrap();
    let args = ["link", "--target", "codex"];
    // the printed paths carry each project's temp root: compare what follows it
    let shown = |e: &Env| e.ok(&args).replace(e.root().to_str().unwrap(), "<root>");
    assert_eq!(shown(&without), shown(&with));
    // the manifest carries `linked_at`, to the second: the one file that may differ
    let files = |e: &Env| {
        let mut t = tree(&e.root());
        t.remove(Path::new(".armadai/link-manifest.yaml")).unwrap();
        t
    };
    assert_eq!(files(&without), files(&with));
    assert!(!with.read(".codex/agents/deep.toml").contains("witness"));
}

/// `curl` and `git` first on `PATH` would leave a witness file if `link` ran either.
#[test]
fn link_opens_no_connection() {
    let env = Env::new();
    let bin = env.dir.path().join("bin");
    let witness = env.dir.path().join("called");
    for tool in ["curl", "git"] {
        let script = bin.join(tool);
        std::fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", witness.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    for target in ["codex", "claude", "opencode", "copilot"] {
        let out = env
            .command()
            .env("PATH", &path)
            .env("https_proxy", "http://127.0.0.1:9")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .args(["link", "--target", target])
            .output()
            .unwrap();
        assert!(out.status.success(), "{target}: {out:?}");
    }
    assert!(!witness.exists(), "link ran curl or git");
}
