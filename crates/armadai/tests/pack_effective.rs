//! The project commands read a pack reference through its effective view: the locked
//! snapshot overlaid by the project's files. The snapshot is laid by hand in a temporary
//! registry; no `git`, no network.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

const URL: &str = "https://example.com/packs.git";
const COMMIT: &str = "1111111111111111111111111111111111111111";
const REF: &str = "schema_version: 1\npack: { name: demo, source: 'https://example.com/packs.git', version: latest }\n";
const LOCK: &str = "format: 1\npack: demo\nsource: https://example.com/packs.git\nresolved:\n  version: 1.0.0\n  commit: '1111111111111111111111111111111111111111'\n";
const PACK_FLOW: &str = "sequence: [a, b]\n";

fn put(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    /// A project referencing `demo` (snapshot: agent `a`, skill `s`, flow `ship`),
    /// holding its own agent `b`.
    fn reference() -> Env {
        let env = Env::bare();
        let snap = env
            .dir
            .path()
            .join("config/registry/packs")
            .join(armadai_core::registries::cache_key(URL))
            .join("demo")
            .join(COMMIT);
        put(&snap, "project.yaml", "schema_version: 1\n");
        put(
            &snap,
            "agents/a.md",
            "---\ndescription: pack agent\n---\nPACK PROMPT\n",
        );
        put(
            &snap,
            "skills/s/SKILL.md",
            "---\nname: s\ndescription: d\n---\nS\n",
        );
        put(&snap, "flows/ship.yaml", PACK_FLOW);
        let root = env.root();
        put(&root, ".armadai/project.yaml", REF);
        put(&root, ".armadai/pack.lock", LOCK);
        put(
            &root,
            ".armadai/agents/b.md",
            "---\ndescription: own agent\nskills: [s]\n---\nB\n",
        );
        env
    }

    fn bare() -> Env {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("project/.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// `(exit code, stdout, stderr)`
    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Every file under `.armadai/` with its bytes.
    fn tree(&self) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(d: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    out.push((p.clone(), std::fs::read(&p).unwrap()));
                }
            }
        }
        let mut v = Vec::new();
        walk(&self.root().join(".armadai"), &mut v);
        v.sort();
        v
    }
}

#[test]
fn validate_accepts_a_reference_project() {
    let env = Env::reference();
    let (code, out, err) = env.run(&["validate"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(out, "0 error(s), 0 warning(s)\n");
}

#[test]
fn validate_keeps_an_unresolved_variable() {
    let env = Env::bare();
    put(
        &env.root(),
        ".armadai/project.yaml",
        "schema_version: 1\nnative:\n  claude:\n    permissions:\n      additionalDirectories: ['{{var.home}}']\n",
    );
    put(
        &env.root(),
        ".armadai/agents/a.md",
        "---\ndescription: '{{var.d}}'\n---\nuse {{var.home}}\n",
    );
    let before = env.tree();
    let (code, out, err) = env.run(&["validate"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(out, "0 error(s), 0 warning(s)\n");
    assert_eq!(env.tree(), before);
}

#[test]
fn list_and_inspect_show_the_pack_agents() {
    let env = Env::reference();
    let (code, out, err) = env.run(&["list"]);
    assert_eq!(code, Some(0), "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines
            .iter()
            .any(|l| l.trim_start().starts_with("a ") && l.contains("pack agent")),
        "{out}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.trim_start().starts_with("b ") && l.contains("own agent")),
        "{out}"
    );
    let (code, out, err) = env.run(&["inspect", "a"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.lines().any(|l| l == "  PACK PROMPT"), "{out}");
}

#[test]
fn new_agent_in_a_reference_project() {
    let env = Env::reference();
    let before = env.tree();
    let (code, out, err) = env.run(&["new", "agent", "c", "--skills", "s"]);
    assert_eq!(code, Some(0), "{out}{err}");
    let after = env.tree();
    let added: Vec<_> = after
        .iter()
        .filter(|e| !before.contains(e))
        .map(|e| e.0.clone())
        .collect();
    assert_eq!(added, [env.root().join(".armadai/agents/c.md")]);
    assert_eq!(after.len(), before.len() + 1);
}

#[test]
fn graph_reads_the_pack_flows() {
    let env = Env::reference();
    let (code, out, err) = env.run(&["graph", "ship"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(
        out,
        "%% flow: ship\nflowchart TD\n  n_a[\"a\"]\n  n_b[\"b\"]\n  n_a --> n_b\n"
    );
}

#[test]
fn flow_status_knows_the_pack_flows() {
    let env = Env::reference();
    let (code, out, err) = env.run(&["flow", "status", "--list"]);
    assert_eq!(code, Some(0), "{out}{err}");
    let (code, out, err) = env.run(&["flow", "status", "nope"]);
    assert_eq!(code, Some(1), "{out}");
    assert!(err.contains("flows: ship"), "{err}");
}

#[test]
fn skills_lists_the_pack_skills_without_a_scratch_path() {
    let env = Env::reference();
    let (code, out, err) = env.run(&["skills", "list"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert!(
        out.lines()
            .any(|l| l.contains(" s ") || l.trim_start().starts_with("s ")),
        "{out}"
    );
    assert!(
        !out.contains("/tmp") && !out.contains("/var/folders"),
        "{out}"
    );
}

#[test]
fn inspect_names_the_layer_of_the_agent() {
    let env = Env::reference();
    let (_, out, _) = env.run(&["inspect", "a"]);
    assert!(
        out.lines().any(|l| l == "Source: pack demo, agents/a.md"),
        "{out}"
    );
    let (_, out, _) = env.run(&["inspect", "b"]);
    assert!(
        out.lines().any(|l| l == "Source: .armadai/agents/b.md"),
        "{out}"
    );
}
