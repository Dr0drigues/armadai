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
    // a file inside a pack folder that is not installed is said so; root files are not
    assert!(err.contains("ignored: agents/notes.txt"), "{err}");
    assert!(!err.contains("ignored: README.md"), "{err}");
    assert!(
        out.contains("2 agent(s), 1 skill(s), 1 flow(s), 1 automation(s)"),
        "{out}"
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

/// Every embedded pack installs as a project with no diagnostic at all, links for
/// Claude Code, and holds no CLI relay agent (spec criterion 1).
#[test]
fn every_embedded_pack_installs_validates_and_links() {
    let starters = Path::new(env!("CARGO_MANIFEST_DIR")).join("../armadai-core/starters");
    let mut packs: Vec<PathBuf> = std::fs::read_dir(&starters)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("pack.yaml").is_file())
        .collect();
    packs.sort();
    assert_eq!(packs.len(), 7);
    for pack in packs {
        let name = pack.file_name().unwrap().to_string_lossy().into_owned();
        let env = Env::new();
        let (ok, out, err) = env.run(&["init", "--pack", &name]);
        assert!(ok, "{name}: {out}\n{err}");
        assert!(!format!("{out}{err}").contains("legacy pack"), "{name}");
        let ds = armadai_model::validate(&env.root());
        assert!(ds.is_empty(), "{name}: {ds:?}");
        let (p, _) = armadai_model::load_checked(&env.root()).unwrap();
        assert!(!p.agents.is_empty(), "{name}");
        for (id, a) in &p.agents {
            assert!(
                a.body.trim().len() > 100,
                "{name}: agent {id} has no real prompt"
            );
            // legacy sections as headings (a reviewer may quote them as text)
            for legacy in ["## System Prompt", "## Metadata", "## Pipeline"] {
                assert!(!a.body.lines().any(|l| l.trim() == legacy), "{name}: {id}");
            }
            assert!(!a.body.contains("@agent-name:"), "{name}: {id}");
        }
        // the main session cannot preload skills: a coordinator names its own
        if let Some(c) = &p.manifest.coordinator {
            for skill in &p.agents[c].front.skills {
                assert!(
                    p.agents[c].body.contains(&format!("`{skill}` skill"))
                        || p.agents[c].body.contains(&format!("skill `{skill}`")),
                    "{name}: coordinator {c} does not point to its skill {skill}"
                );
            }
        }
        // a state a flow waits for is one its agent says it emits
        for (fid, f) in &p.flows {
            for (sid, step) in &f.steps {
                let body = &p.agents[&step.agent].body;
                for state in &step.states {
                    assert!(
                        body.contains(&format!("`{state}`")),
                        "{name}: flow {fid} step {sid} waits for `{state}`, which agent {} never emits",
                        step.agent
                    );
                }
            }
        }
        for cli in ["claude-cli-reviewer", "echo-reviewer", "gemini-reviewer"] {
            assert!(!p.agents.contains_key(cli), "{name}: {cli}");
        }
        assert!(
            !env.root().join(".armadai/.armadai-version").exists(),
            "{name}"
        );
        let (ok, out, err) = env.run(&["link", "--target", "claude"]);
        assert!(ok, "{name}: {out}\n{err}");
    }
}

/// Files of a pack folder that the white list leaves out are each reported.
#[test]
fn ignored_pack_files_are_reported() {
    let env = Env::new();
    let mut files = NATIVE.to_vec();
    files.push(("flows/x.yml", "steps: {}\n"));
    files.push(("prompts/p.md", "# p\n"));
    let pack = env.pack("demo", &files);
    let (ok, out, err) = env.init(&pack, &[]);
    assert!(ok, "{out}\n{err}");
    for f in ["flows/x.yml", "prompts/p.md", "agents/notes.txt"] {
        assert!(err.contains(&format!("ignored: {f}")), "{f}: {err}");
    }
}

/// Under --force the summary counts what was created, and what was kept; an
/// automation already there is not announced; a skill already there is not mixed
/// with the pack's.
#[test]
fn a_forced_merge_reports_exactly_what_it_did() {
    let env = Env::new();
    let pack = env.pack("demo", NATIVE);
    for (p, c) in [
        (".armadai/project.yaml", "schema_version: 1\n"),
        (
            ".armadai/agents/dev.md",
            "---\ndescription: Mine\nskills: [notes]\n---\nMine.\n",
        ),
        (
            ".armadai/automations/fmt.yaml",
            "event: after-tool\nrun: mine\n",
        ),
        (
            ".armadai/skills/notes/SKILL.md",
            "---\nname: notes\ndescription: Mine\n---\nMine.\n",
        ),
    ] {
        let path = env.root().join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, c).unwrap();
    }
    let (ok, out, err) = env.init(&pack, &["--force"]);
    assert!(ok, "{out}\n{err}");
    assert!(
        out.contains("1 agent(s), 0 skill(s), 1 flow(s), 0 automation(s)"),
        "{out}"
    );
    assert!(out.contains("5 existing file(s) kept"), "{out}");
    assert!(!err.contains("cargo fmt"), "{err}");
    assert!(err.contains("skills/notes exists"), "{err}");
    assert!(!env.root().join(".armadai/skills/notes/scripts").exists());
}

/// An install that passes the pack's own validation but breaks the project it
/// lands in is undone: here the project's skill `notes` has no frontmatter, so the
/// pack's `dev` agent would name a skill that does not load.
#[test]
fn an_install_introducing_an_error_is_rolled_back() {
    let env = Env::new();
    let pack = env.pack("demo", NATIVE);
    for (p, c) in [
        (".armadai/project.yaml", "schema_version: 1\n"),
        (".armadai/skills/notes/SKILL.md", "no frontmatter\n"),
    ] {
        let path = env.root().join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, c).unwrap();
    }
    let before = tree(&env.root());
    let (ok, _, err) = env.init(&pack, &["--force"]);
    assert!(!ok, "{err}");
    assert!(
        err.contains("(rolled back)") && err.contains("unknown skill `notes`"),
        "{err}"
    );
    assert!(err.contains("would make the project invalid"), "{err}");
    assert_eq!(tree(&env.root()), before);
}

fn legacy_pack(env: &Env) -> PathBuf {
    env.pack(
        "old",
        &[
            ("pack.yaml", "name: old\ndescription: Old\nagents: [dev]\n"),
            (
                "agents/dev.md",
                "# Dev\n\n## Metadata\n- provider: anthropic\n\n## System Prompt\n\nYou code.\n",
            ),
        ],
    )
}

/// A legacy pack never writes through a linked `.armadai` or `.armadai/agents`,
/// nor does a native one; a dangling `.armadai` link is named as such.
#[cfg(unix)]
#[test]
fn no_pack_writes_through_a_linked_armadai() {
    for (link, native) in [
        (".armadai", false),
        (".armadai/agents", false),
        (".armadai", true),
    ] {
        let env = Env::new();
        let pack = if native {
            env.pack("demo", NATIVE)
        } else {
            legacy_pack(&env)
        };
        let outside = env.dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(env.root().join(".armadai")).unwrap();
        if link == ".armadai" {
            std::fs::remove_dir(env.root().join(".armadai")).unwrap();
        }
        std::os::unix::fs::symlink(&outside, env.root().join(link)).unwrap();
        let (ok, _, err) = env.init(&pack, &[]);
        assert!(
            !ok && err.contains("symbolic link"),
            "{link} {native}: {err}"
        );
        assert_eq!(
            std::fs::read_dir(&outside).unwrap().count(),
            0,
            "{link} {native}"
        );
    }
    let env = Env::new();
    let pack = legacy_pack(&env);
    std::os::unix::fs::symlink(env.dir.path().join("nowhere"), env.root().join(".armadai"))
        .unwrap();
    let (ok, _, err) = env.init(&pack, &[]);
    assert!(
        !ok && err.contains(".armadai is a dangling symbolic link"),
        "{err}"
    );
    assert!(!env.dir.path().join("nowhere").exists());
}

/// The report's case: `fullstack` extracted under `~/.config/armadai/starters/` by a
/// binary older than the named marker, with an agent the pack no longer ships (no
/// frontmatter). `init --pack fullstack` installs the current pack, and the old
/// extraction is kept aside with the user's files.
#[test]
fn a_legacy_extraction_does_not_break_init_pack() {
    let env = Env::new();
    let old = env.dir.path().join("config/starters/fullstack");
    for (p, c) in [
        (".armadai-version", "1.0.0-rc.2"),
        ("pack.yaml", "format: 1\nname: fullstack\ndescription: d\n"),
        (
            "agents/gemini-reviewer.md",
            "# Gemini Reviewer\n\n## Metadata\n- provider: gemini\n",
        ),
        ("my-notes.md", "mine"),
    ] {
        std::fs::create_dir_all(old.join(p).parent().unwrap()).unwrap();
        std::fs::write(old.join(p), c).unwrap();
    }
    let (ok, out, err) = env.run(&["init", "--pack", "fullstack"]);
    assert!(ok, "{out}\n{err}");
    assert!(
        err.contains("moved the previous extraction of fullstack"),
        "{err}"
    );
    assert!(
        !env.root()
            .join(".armadai/agents/gemini-reviewer.md")
            .exists()
    );
    let starters = env.dir.path().join("config/starters");
    let legacy: Vec<_> = std::fs::read_dir(&starters)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().contains("fullstack.armadai-legacy-"))
        .collect();
    assert_eq!(legacy.len(), 1, "{legacy:?}");
    assert_eq!(
        std::fs::read_to_string(legacy[0].join("my-notes.md")).unwrap(),
        "mine"
    );
    // a second init in another folder moves nothing more
    let other = env.dir.path().join("other");
    std::fs::create_dir_all(other.join(".git")).unwrap();
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(&other)
        .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
        .env("HOME", env.dir.path())
        .args(["init", "--pack", "fullstack"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let count = std::fs::read_dir(&starters)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("armadai-legacy-"))
        .count();
    assert_eq!(count, 1);
}
