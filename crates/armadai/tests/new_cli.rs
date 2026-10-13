//! `armadai new agent|flow|automation|skill` without a terminal: nominal paths,
//! collisions, hostile ids, writes that stay under `.armadai/`, and the undo of a
//! creation that would make the project invalid (T8).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    /// A project holding `files` (paths relative to the project root).
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        for (p, c) in files {
            let path = root.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        Env { dir }
    }

    fn project() -> Self {
        Env::new(&[(".armadai/project.yaml", "schema_version: 1\n")])
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("xdg"))
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

    fn ok(&self, args: &[&str]) -> String {
        let (ok, out, err) = self.run(args);
        assert!(ok, "{args:?} failed:\n{out}\n{err}");
        out
    }

    fn fails(&self, args: &[&str]) -> String {
        let (ok, out, err) = self.run(args);
        assert!(!ok, "{args:?} succeeded:\n{out}\n{err}");
        err
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root().join(rel)).unwrap()
    }

    /// Every diagnostic of the project, rendered.
    fn diagnostics(&self) -> Vec<String> {
        armadai_model::validate(&self.root())
            .iter()
            .map(|d| d.render())
            .collect()
    }
}

/// Every path under `dir` (directories as `/`-terminated keys) with file contents.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
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

const AGENTS: &[(&str, &str)] = &[
    (".armadai/project.yaml", "schema_version: 1\n"),
    (
        ".armadai/agents/lead.md",
        "---\ndescription: Leads\n---\nLead.\n",
    ),
    (
        ".armadai/agents/dev.md",
        "---\ndescription: Codes\n---\nCode.\n",
    ),
    (
        ".armadai/agents/qa.md",
        "---\ndescription: Tests\n---\nTest.\n",
    ),
];

#[test]
fn new_agent_writes_a_valid_canonical_agent() {
    let env = Env::project();
    let out = env.ok(&[
        "new",
        "agent",
        "reviewer",
        "--template",
        "dev-review",
        "--stack",
        "rust",
        "--tier",
        "max",
        "--description",
        "Reviews the Rust code",
    ]);
    assert!(out.contains(".armadai/agents/reviewer.md"), "{out}");
    let text = env.read(".armadai/agents/reviewer.md");
    assert!(
        text.starts_with("---\ndescription: Reviews the Rust code\n"),
        "{text}"
    );
    assert!(
        text.contains("tier: max") && text.contains("working on rust"),
        "{text}"
    );
    assert!(env.diagnostics().is_empty(), "{:?}", env.diagnostics());
}

#[test]
fn an_existing_file_is_kept_unless_forced() {
    let env = Env::new(AGENTS);
    let err = env.fails(&["new", "agent", "dev"]);
    assert!(
        err.contains("already exists") && err.contains("--force"),
        "{err}"
    );
    assert_eq!(env.read(".armadai/agents/dev.md"), AGENTS[2].1);
    env.ok(&[
        "new",
        "agent",
        "dev",
        "--force",
        "--description",
        "Codes better",
    ]);
    assert!(env.read(".armadai/agents/dev.md").contains("Codes better"));
}

#[test]
fn hostile_ids_are_refused_and_nothing_is_written() {
    let env = Env::project();
    let long = "a".repeat(65);
    let before = tree(env.dir.path());
    for id in [
        "../x",
        "a/b",
        ".git",
        "Bad_Name",
        "Mon Agent",
        "é",
        "",
        "a\\b",
        "..",
        &long,
        "con",
        "com1",
        "lpt9",
        "nul",
        "aux",
        "prn",
    ] {
        for kind in [
            &["new", "agent", id][..],
            &["new", "skill", id, "--description", "d"][..],
            &[
                "new",
                "automation",
                id,
                "--event",
                "after-tool",
                "--run",
                "x",
            ][..],
        ] {
            let err = env.fails(kind);
            assert!(
                err.contains("not a valid id") || err.contains("reserves"),
                "{id}: {err}"
            );
        }
    }
    assert_eq!(tree(env.dir.path()), before);
}

#[test]
fn outside_a_project_new_points_to_init() {
    let env = Env::new(&[]);
    let err = env.fails(&["new", "agent", "lonely"]);
    assert!(err.contains("armadai init --project"), "{err}");
    assert!(!env.root().join(".armadai").exists());
}

#[cfg(unix)]
#[test]
fn no_write_goes_through_a_symbolic_link() {
    let env = Env::project();
    let outside = env.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, env.root().join(".armadai/agents")).unwrap();
    let err = env.fails(&["new", "agent", "x"]);
    assert!(err.contains("symbolic link"), "{err}");
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);

    // a link posed at the destination itself is never followed, even under --force
    let env = Env::project();
    std::fs::create_dir_all(env.root().join(".armadai/skills")).unwrap();
    let target = env.dir.path().join("victim.md");
    std::fs::write(&target, "keep me\n").unwrap();
    std::fs::create_dir_all(env.root().join(".armadai/agents")).unwrap();
    std::os::unix::fs::symlink(&target, env.root().join(".armadai/agents/x.md")).unwrap();
    for args in [
        &["new", "agent", "x"][..],
        &["new", "agent", "x", "--force"][..],
    ] {
        let err = env.fails(args);
        assert!(err.contains("symbolic link"), "{err}");
    }
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me\n");
}

#[test]
fn new_flow_lays_a_template_over_project_agents() {
    let env = Env::new(AGENTS);
    env.ok(&["new", "flow", "ship", "--agents", "lead,dev,qa"]);
    let text = env.read(".armadai/flows/ship.yaml");
    assert!(
        text.contains("from: lead") && text.contains("to: qa"),
        "{text}"
    );
    env.ok(&[
        "new",
        "flow",
        "review",
        "--template",
        "review-loop",
        "--agents",
        "dev,qa,lead",
        "--description",
        "Write, review, ship",
    ]);
    assert!(
        env.read(".armadai/flows/review.yaml")
            .contains("when: rejected")
    );
    assert!(env.diagnostics().is_empty(), "{:?}", env.diagnostics());
    let graph = env.ok(&["graph", "review"]);
    assert!(graph.contains("n_qa -->|\"rejected\"| n_dev"), "{graph}");
}

#[test]
fn a_flow_naming_unknown_agents_or_a_wrong_count_is_refused() {
    let env = Env::new(AGENTS);
    let before = tree(&env.root());
    let err = env.fails(&["new", "flow", "f", "--agents", "dev,ghost"]);
    assert!(
        err.contains("unknown agent") && err.contains("ghost"),
        "{err}"
    );
    assert!(err.contains("dev, lead, qa"), "{err}");
    let err = env.fails(&[
        "new",
        "flow",
        "f",
        "--template",
        "plan-build-verify",
        "--agents",
        "dev,qa",
    ]);
    assert!(err.contains("at least 3"), "{err}");
    let err = env.fails(&[
        "new",
        "flow",
        "f",
        "--template",
        "loop",
        "--agents",
        "dev,qa",
    ]);
    assert!(err.contains("review-loop"), "{err}");
    let err = env.fails(&["new", "flow", "f", "--agents", "dev,dev"]);
    assert!(err.contains("twice"), "{err}");
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn new_automation_and_skill() {
    let env = Env::project();
    env.ok(&[
        "new",
        "automation",
        "fmt",
        "--event",
        "after-tool",
        "--run",
        "cargo fmt",
        "--description",
        "Format after edits",
    ]);
    let text = env.read(".armadai/automations/fmt.yaml");
    assert!(
        text.contains("event: after-tool") && text.contains("run: cargo fmt"),
        "{text}"
    );
    let err = env.fails(&[
        "new",
        "automation",
        "x",
        "--event",
        "whenever",
        "--run",
        "y",
    ]);
    assert!(err.contains("after-tool"), "{err}");

    env.ok(&["new", "skill", "notes", "--description", "Take notes"]);
    let text = env.read(".armadai/skills/notes/SKILL.md");
    assert!(
        text.contains("name: notes") && text.contains("description: Take notes"),
        "{text}"
    );
    assert!(env.diagnostics().is_empty(), "{:?}", env.diagnostics());
}

/// T8: a creation that introduces an error is undone, created folders included.
#[test]
fn a_creation_that_breaks_the_project_is_undone() {
    let env = Env::project();
    let before = tree(&env.root());
    let err = env.fails(&["new", "agent", "x", "--skills", "ghost"]);
    assert!(err.contains("unknown skill `ghost`"), "{err}");
    assert!(err.contains("nothing was written"), "{err}");
    // the diagnostic names a file that no longer exists: said so
    let at: Vec<&str> = err.lines().filter(|l| l.contains("agents/x.md:")).collect();
    assert!(!at.is_empty(), "{err}");
    assert!(at.iter().all(|l| l.starts_with("(rolled back) ")), "{err}");
    assert_eq!(tree(&env.root()), before, "agents/ itself is gone");

    let err = env.fails(&["new", "automation", "x", "--event", "on-stop", "--run", " "]);
    assert!(err.contains("`run` is empty"), "{err}");
    assert_eq!(tree(&env.root()), before);
}

/// T8 under --force: the previous file comes back, never deleted.
#[test]
fn a_forced_creation_that_breaks_the_project_restores_the_file() {
    let env = Env::new(AGENTS);
    let before = tree(&env.root());
    env.fails(&["new", "agent", "dev", "--force", "--skills", "ghost"]);
    assert_eq!(tree(&env.root()), before);
}

/// An already invalid project does not block a creation that adds no error.
#[test]
fn a_project_already_invalid_does_not_block_new() {
    let mut files = AGENTS.to_vec();
    files.push((".armadai/flows/bad.yaml", "steps:\n  x: { agent: ghost }\n"));
    let env = Env::new(&files);
    let out = env.ok(&["new", "agent", "helper"]);
    assert!(out.contains("helper.md"), "{out}");
}

#[test]
fn the_former_form_points_to_the_new_one() {
    let env = Env::project();
    let err = env.fails(&["new", "my-agent"]);
    assert!(err.contains("armadai new agent my-agent"), "{err}");
    env.fails(&["new", "Bad_Name"]);
    assert!(!env.root().join(".armadai/agents").exists());
}

#[test]
fn an_id_is_required_without_a_terminal() {
    let env = Env::project();
    let err = env.fails(&["new", "agent"]);
    assert!(err.contains("id"), "{err}");
}

/// The wizards need a terminal: without one, a clear error and nothing written.
#[test]
fn interactive_mode_without_a_terminal_is_a_clear_error() {
    let env = Env::project();
    let before = tree(&env.root());
    for args in [
        &["new"][..],
        &["new", "agent", "-i"][..],
        &["new", "flow", "--interactive"][..],
        &["new", "automation", "-i"][..],
        &["new", "skill", "-i"][..],
    ] {
        let err = env.fails(args);
        assert!(
            err.contains("interactive mode needs a terminal"),
            "{args:?}: {err}"
        );
    }
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn new_flow_fan_out_sends_the_lead_to_every_agent() {
    let env = Env::new(AGENTS);
    env.ok(&[
        "new",
        "flow",
        "delivery",
        "--template",
        "fan-out",
        "--agents",
        "lead,dev,qa",
    ]);
    let graph = env.ok(&["graph", "delivery"]);
    assert!(
        graph.contains("n_lead --> n_dev\n  n_lead --> n_qa"),
        "{graph}"
    );
    assert!(env.diagnostics().is_empty(), "{:?}", env.diagnostics());
}

/// A flow is guidance only while the project has neither a coordinator nor a strict
/// policy: said at creation, without blocking it.
#[test]
fn a_flow_link_renders_nowhere_is_announced() {
    let warning = "this flow is guidance only: link renders it nowhere until you set a \
                   coordinator or policy strict";
    let env = Env::new(AGENTS);
    let (ok, _, err) = env.run(&["new", "flow", "ship", "--agents", "dev,qa"]);
    assert!(ok && err.contains(warning), "{err}");
    for manifest in [
        "schema_version: 1\ncoordinator: lead\n",
        "schema_version: 1\npolicy:\n  delegation: strict\n",
    ] {
        let mut files = AGENTS.to_vec();
        files[0] = (".armadai/project.yaml", manifest);
        let env = Env::new(&files);
        let (ok, _, err) = env.run(&["new", "flow", "ship", "--agents", "dev,qa"]);
        assert!(ok && !err.contains("guidance only"), "{manifest}: {err}");
    }
}
