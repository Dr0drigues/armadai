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

fn tree_of(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
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
    walk(dir, &mut v);
    v.sort();
    v
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
    // a command whose whole body is a variable: empty once rendered, so a resolved read
    // would fail with `command-empty-body`
    put(
        &env.root(),
        ".armadai/commands/c.md",
        "---\ndescription: run it\n---\n{{var.cmd}}\n",
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
    let cache = env.snapshot_dir();
    let cache_before = tree_of(&cache);
    let (code, out, err) = env.run(&["new", "agent", "c", "--skills", "s"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(tree_of(&cache), cache_before, "the cache is never written");
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
        "%% flow: ship\nflowchart TD\n  n_a([\"a\"])\n  n_b[[\"b\"]]\n  n_a --> n_b\n  classDef startStep stroke-width:3px\n  classDef endStep stroke-width:3px,stroke-dasharray:4 2\n  class n_a startStep\n  class n_b endStep\n"
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
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(
        out,
        "  NAME  DESCRIPTION  VERSION  TOOLS\n  ----  -----------  -------  -----\n  s     d            -        -\n\n  1 skill(s) found.\n"
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

impl Env {
    fn snapshot_dir(&self) -> PathBuf {
        self.dir
            .path()
            .join("config/registry/packs")
            .join(armadai_core::registries::cache_key(URL))
    }
}

#[test]
fn list_diagnostics_are_placed_like_validates() {
    let env = Env::reference();
    // the overlay's own lines are not the merged file's: a position must not leak
    put(
        &env.root(),
        ".armadai/project.yaml",
        "\n\n\npack: { name: demo, source: 'https://example.com/packs.git', version: latest }\ntargets: [claude, foo]\nschema_version: 1\n",
    );
    let (code, out, _) = env.run(&["validate"]);
    assert_eq!(code, Some(1), "{out}");
    let diag = out
        .lines()
        .find(|l| l.contains("unknown-target"))
        .unwrap_or_else(|| panic!("{out}"))
        .to_string();
    let (_, _, err) = env.run(&["list"]);
    assert!(err.lines().any(|l| l == diag), "want `{diag}` in:\n{err}");
    let (_, _, err) = env.run(&["inspect", "a"]);
    assert!(err.lines().any(|l| l == diag), "want `{diag}` in:\n{err}");
}

#[test]
fn unlink_without_manifest_and_without_the_pack_cleans_the_project() {
    let env = Env::reference();
    // an agent of the project alone: the fallback sees the project only
    put(
        &env.root(),
        ".armadai/agents/b.md",
        "---\ndescription: own agent\n---\nB\n",
    );
    let (code, out, err) = env.run(&["link", "--target", "claude"]);
    assert_eq!(code, Some(0), "{out}{err}");
    let b = env.root().join(".claude/agents/b.md");
    assert!(b.is_file());
    std::fs::remove_file(env.root().join(".armadai/link-manifest.yaml")).unwrap();
    std::fs::remove_dir_all(env.snapshot_dir()).unwrap();
    let (code, out, err) = env.run(&["unlink", "--target", "claude"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert!(!b.exists(), "{out}{err}");
    let warn = "pack demo 1.0.0 (111111111111) is not in the local cache: run `armadai pack sync`";
    assert_eq!(
        err.lines().filter(|l| l.contains(warn)).count(),
        1,
        "the open error is shown once: {err}"
    );
}

#[test]
fn init_pack_validates_through_the_view() {
    let env = Env::reference();
    let pack = env.dir.path().join("packs/p");
    put(
        &pack,
        "pack.yaml",
        "name: p\nformat: 1\ndescription: d\nversion: 1.0.0\n",
    );
    put(&pack, "project.yaml", "schema_version: 1\n");
    put(&pack, "agents/c.md", "---\ndescription: c\n---\nC\n");
    std::fs::remove_dir_all(env.snapshot_dir()).unwrap();
    let before = env.tree();
    let (code, out, err) = env.run(&["init", "--pack", pack.to_str().unwrap(), "--force"]);
    assert_eq!(code, Some(1), "{out}{err}");
    assert!(
        err.lines().any(|l| l.ends_with(
            "pack demo 1.0.0 (111111111111) is not in the local cache: run `armadai pack sync`"
        )),
        "{err}"
    );
    assert_eq!(env.tree(), before, "nothing is written");
}

#[test]
fn inspect_names_the_layer_of_the_template_of_an_instance() {
    let env = Env::reference();
    let snap = env.snapshot_dir().join("demo").join(COMMIT);
    put(
        &snap,
        "templates/t.md",
        "---\ndescription: from a template\n---\nT\n",
    );
    put(&env.root(), ".armadai/agents/i.md", "---\nfrom: t\n---\n");
    let (code, out, err) = env.run(&["inspect", "i"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert!(
        out.lines()
            .any(|l| l == "Source: .armadai/agents/i.md, from: t (pack demo, templates/t.md)"),
        "{out}"
    );
}

#[test]
fn skills_show_names_the_real_layer() {
    let env = Env::reference();
    put(
        &env.root(),
        ".armadai/skills/own/SKILL.md",
        "---\nname: own\ndescription: mine\n---\nO\n",
    );
    let snap = env.snapshot_dir().join("demo").join(COMMIT);
    let root = env.root().canonicalize().unwrap();
    let files = ["scripts/x.sh", "references/r.md", "assets/i.txt"];
    for f in files {
        put(&env.root(), &format!(".armadai/skills/own/{f}"), "x\n");
        put(&snap, &format!("skills/s/{f}"), "x\n");
    }
    let (code, out, err) = env.run(&["skills", "show", "own"]);
    assert_eq!(code, Some(0), "{out}{err}");
    let want = format!("Source: {}", root.join(".armadai/skills/own").display());
    assert!(out.lines().any(|l| l == want), "want `{want}` in:\n{out}");
    for f in files {
        let want = format!("  - {}", root.join(".armadai/skills/own").join(f).display());
        assert!(out.lines().any(|l| l == want), "want `{want}` in:\n{out}");
    }
    let (code, out, err) = env.run(&["skills", "show", "s"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert!(
        out.lines().any(|l| l == "Source: pack demo, skills/s"),
        "{out}"
    );
    for f in files {
        let want = format!("  - {}", snap.join("skills/s").join(f).display());
        assert!(out.lines().any(|l| l == want), "want `{want}` in:\n{out}");
    }
}

#[test]
fn skills_warns_when_the_pack_cannot_be_read() {
    let env = Env::reference();
    put(
        &env.root(),
        ".armadai/skills/own/SKILL.md",
        "---\nname: own\ndescription: mine\n---\nO\n",
    );
    std::fs::remove_dir_all(env.snapshot_dir()).unwrap();
    let (code, out, err) = env.run(&["skills", "list"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(
        err,
        "warning: pack demo 1.0.0 (111111111111) is not in the local cache: run `armadai pack sync`\n"
    );
    assert!(
        out.lines().any(|l| l.trim_start().starts_with("own ")),
        "{out}"
    );
}

#[test]
fn init_pack_refuses_what_the_pack_it_references_collides_with() {
    let env = Env::reference();
    let pack = env.dir.path().join("packs/p");
    put(
        &pack,
        "pack.yaml",
        "name: p\nformat: 1\ndescription: d\nversion: 1.0.0\n",
    );
    put(&pack, "project.yaml", "schema_version: 1\n");
    put(&pack, "templates/a.md", "---\ndescription: tpl\n---\nT\n");
    let before = env.tree();
    let (code, out, err) = env.run(&[
        "init",
        "--pack",
        pack.to_str().unwrap(),
        "--force",
        "--no-link",
    ]);
    assert_eq!(code, Some(1), "{out}{err}");
    assert!(
        err.lines()
            .any(|l| l == format!(
                "(rolled back) {}: error[template-agent-collision]: template `a` has the same id as agent `a`",
                env.root().canonicalize().unwrap().join(".armadai/templates/a.md").display()
            )),
        "{err}"
    );
    assert_eq!(env.tree(), before);
}
