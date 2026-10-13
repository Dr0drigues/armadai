//! `armadai init --pack` with a native pack (`format: 1`: copied by a white list,
//! validated before anything is written) and with a legacy one (converted as before),
//! plus `validate <pack>`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("project/.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn pack(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = self.dir.path().join("packs").join(name);
        for (p, c) in files {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        dir
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
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
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn init(&self, pack: &Path, extra: &[&str]) -> (bool, String, String) {
        let mut args = vec!["init", "--pack", pack.to_str().unwrap()];
        args.extend_from_slice(extra);
        self.run(&args)
    }
}

fn tree(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            if std::fs::symlink_metadata(&path).unwrap().is_dir() {
                out.insert(rel, None);
                walk(root, &path, out);
            } else {
                out.insert(rel, Some(std::fs::read(&path).unwrap_or_default()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

const NATIVE: &[(&str, &str)] = &[
    (
        "pack.yaml",
        "format: 1\nname: demo\ndescription: A demo pack\n",
    ),
    (
        "project.yaml",
        "schema_version: 1\ntargets: [claude]\ncoordinator: lead\n",
    ),
    ("agents/lead.md", "---\ndescription: Leads\n---\nLead.\n"),
    (
        "agents/dev.md",
        "---\ndescription: Codes\nskills: [notes]\n---\nCode.\n",
    ),
    (
        "skills/notes/SKILL.md",
        "---\nname: notes\ndescription: Notes\n---\nTake notes.\n",
    ),
    ("skills/notes/scripts/n.sh", "echo n\n"),
    (
        "flows/ship.yaml",
        "steps:\n  plan: { agent: lead }\n  build: { agent: dev }\nlinks:\n  - { from: plan, to: build }\n",
    ),
    (
        "automations/fmt.yaml",
        "event: after-tool\nrun: cargo fmt\n",
    ),
    // never copied
    ("README.md", "# Demo\n"),
    (".armadai-version", "0.0.1\n"),
    ("link-manifest.yaml", "x: 1\n"),
    (".gitignore", "*\n"),
    ("agents/notes.txt", "not an agent\n"),
];

#[test]
fn a_native_pack_is_copied_by_its_white_list() {
    let env = Env::new();
    let pack = env.pack("demo", NATIVE);
    let (ok, out, err) = env.init(&pack, &[]);
    assert!(ok, "{out}\n{err}");
    let got: Vec<PathBuf> = tree(&env.root().join(".armadai"))
        .into_iter()
        .filter(|(_, c)| c.is_some())
        .map(|(p, _)| p)
        .collect();
    let want: Vec<PathBuf> = [
        ".gitignore",
        "agents/dev.md",
        "agents/lead.md",
        "automations/fmt.yaml",
        "flows/ship.yaml",
        "project.yaml",
        "skills/notes/SKILL.md",
        "skills/notes/scripts/n.sh",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    assert_eq!(got, want);
    assert_eq!(
        std::fs::read_to_string(env.root().join(".armadai/agents/dev.md")).unwrap(),
        NATIVE[3].1
    );
    assert!(armadai_model::validate(&env.root()).is_empty());
    // the automation becomes a hook: said at install
    assert!(
        err.contains("automation fmt") && err.contains("cargo fmt"),
        "{err}"
    );
}

#[test]
fn a_native_pack_without_project_yaml_gets_the_default_one() {
    let env = Env::new();
    let files: Vec<_> = NATIVE
        .iter()
        .filter(|(p, _)| *p != "project.yaml")
        .copied()
        .collect();
    let pack = env.pack("demo", &files);
    let (ok, out, err) = env.init(&pack, &[]);
    assert!(ok, "{out}\n{err}");
    assert_eq!(
        std::fs::read_to_string(env.root().join(".armadai/project.yaml")).unwrap(),
        "schema_version: 1\ntargets: [claude]\n"
    );
}

#[test]
fn an_invalid_native_pack_is_refused_without_writes() {
    let env = Env::new();
    let mut files = NATIVE.to_vec();
    files.push(("flows/bad.yaml", "steps:\n  x: { agent: ghost }\n"));
    let pack = env.pack("demo", &files);
    let before = tree(&env.root());
    let (ok, _, err) = env.init(&pack, &[]);
    assert!(!ok);
    assert!(err.contains("unknown agent `ghost`"), "{err}");
    // refused from the pack itself, before anything is written
    assert!(
        err.contains("pack 'demo' is invalid; nothing was written"),
        "{err}"
    );
    assert!(err.contains("packs/demo/flows/bad.yaml"), "{err}");
    assert_eq!(tree(&env.root()), before);
}

#[cfg(unix)]
#[test]
fn a_pack_holding_any_symbolic_link_is_refused() {
    for (link, to) in [
        ("skills/notes/inner.md", "SKILL.md"),
        ("agents/evil.md", "/etc/hosts"),
    ] {
        let env = Env::new();
        let pack = env.pack("demo", NATIVE);
        std::os::unix::fs::symlink(to, pack.join(link)).unwrap();
        let before = tree(&env.root());
        let (ok, _, err) = env.init(&pack, &[]);
        assert!(!ok && err.contains("symbolic link"), "{link}: {err}");
        assert_eq!(tree(&env.root()), before, "{link}");
    }
}

#[test]
fn an_existing_project_is_refused_then_merged_under_force() {
    let env = Env::new();
    let pack = env.pack("demo", NATIVE);
    std::fs::create_dir_all(env.root().join(".armadai/agents")).unwrap();
    std::fs::write(
        env.root().join(".armadai/project.yaml"),
        "schema_version: 1\ntargets: [codex]\n",
    )
    .unwrap();
    std::fs::write(
        env.root().join(".armadai/agents/dev.md"),
        "---\ndescription: Mine\nskills: [notes]\n---\nMine.\n",
    )
    .unwrap();
    let (ok, _, err) = env.init(&pack, &[]);
    assert!(!ok && err.contains("already holds"), "{err}");

    let (ok, out, err) = env.init(&pack, &["--force"]);
    assert!(ok, "{out}\n{err}");
    assert!(err.contains("project.yaml of the pack is ignored"), "{err}");
    assert!(err.contains("agents/dev.md exists, kept"), "{err}");
    assert_eq!(
        std::fs::read_to_string(env.root().join(".armadai/project.yaml")).unwrap(),
        "schema_version: 1\ntargets: [codex]\n"
    );
    assert!(
        std::fs::read_to_string(env.root().join(".armadai/agents/dev.md"))
            .unwrap()
            .contains("Mine.")
    );
    assert!(env.root().join(".armadai/agents/lead.md").is_file());
}

#[test]
fn a_legacy_pack_is_still_converted() {
    let env = Env::new();
    let pack = env.pack(
        "old",
        &[
            ("pack.yaml", "name: old\ndescription: Old\nagents: [dev]\n"),
            (
                "agents/dev.md",
                "# Dev\n\n## Metadata\n- provider: anthropic\n\n## System Prompt\n\nYou code.\n",
            ),
        ],
    );
    let (ok, out, err) = env.init(&pack, &[]);
    assert!(ok, "{out}\n{err}");
    assert!(
        format!("{out}{err}").contains("legacy pack converted"),
        "{out}\n{err}"
    );
    assert!(env.root().join(".armadai/agents/dev.md").is_file());
    let ds = armadai_model::validate(&env.root());
    assert!(!armadai_model::has_errors(&ds), "{ds:?}");
}

#[test]
fn an_unknown_pack_format_is_refused() {
    let env = Env::new();
    let pack = env.pack(
        "next",
        &[("pack.yaml", "format: 2\nname: n\ndescription: d\n")],
    );
    let (ok, _, err) = env.init(&pack, &[]);
    assert!(!ok && err.contains("format 2"), "{err}");
    assert!(!env.root().join(".armadai").exists());
}

#[test]
fn validate_checks_a_native_pack() {
    let env = Env::new();
    let pack = env.pack("demo", NATIVE);
    let (ok, out, err) = env.run(&["validate", pack.to_str().unwrap()]);
    assert!(ok, "{out}\n{err}");
    assert!(out.contains("0 error(s)"), "{out}");
    std::fs::write(
        pack.join("flows/bad.yaml"),
        "steps:\n  x: { agent: ghost }\n",
    )
    .unwrap();
    let (ok, out, _) = env.run(&["validate", pack.to_str().unwrap()]);
    assert!(!ok && out.contains("unknown agent `ghost`"), "{out}");
    // positioned in the pack, not in the scratch copy it was validated in
    assert!(out.contains("packs/demo/flows/bad.yaml:2:15"), "{out}");
}

/// An install that fails halfway takes back every file and folder it created.
#[test]
fn a_failed_install_leaves_the_project_as_it_was() {
    let env = Env::new();
    let pack = env.pack("demo", NATIVE);
    std::fs::create_dir_all(env.root().join(".armadai")).unwrap();
    std::fs::write(
        env.root().join(".armadai/project.yaml"),
        "schema_version: 1\n",
    )
    .unwrap();
    // `flows` is a file: the copy of flows/ship.yaml fails after agents/ and automations/
    std::fs::write(env.root().join(".armadai/flows"), "not a folder\n").unwrap();
    let before = tree(&env.root());
    let (ok, _, err) = env.init(&pack, &["--force"]);
    assert!(!ok, "{err}");
    assert_eq!(tree(&env.root()), before);
}
