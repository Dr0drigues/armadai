//! `list`, `inspect`, `init` on the canonical model (step 2, Task 7).

use std::path::{Path, PathBuf};

use assert_cmd::Command;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        for (p, c) in files {
            let path = root.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
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
}

const TEAM: &[(&str, &str)] = &[
    (
        ".armadai/project.yaml",
        "schema_version: 1\ncoordinator: lead\n",
    ),
    (
        ".armadai/agents/lead.md",
        "---\ndescription: Leads\ntier: max\n---\nYou lead.\n",
    ),
    (
        ".armadai/agents/dev.md",
        "---\ndescription: Writes code\ntags: [dev]\nskills: [notes]\n---\nYou write code.\n",
    ),
    (
        ".armadai/skills/notes/SKILL.md",
        "---\nname: notes\ndescription: Notes\n---\nB\n",
    ),
    (
        ".armadai/flows/ship.yaml",
        "steps:\n  plan: { agent: lead }\n  build: { agent: dev }\nlinks:\n  - { from: plan, to: build }\n",
    ),
    (
        ".armadai/automations/fmt.yaml",
        "event: on-stop\nrun: cargo fmt\n",
    ),
];

#[test]
fn list_shows_agents_skills_flows_and_automations() {
    let env = Env::new(TEAM);
    let (ok, out, err) = env.run(&["list"]);
    assert!(ok, "{err}");
    assert!(
        out.contains("lead") && out.contains("max") && out.contains("(coordinator)"),
        "{out}"
    );
    assert!(out.contains("Writes code"), "{out}");
    assert!(out.contains("2 agent(s) found."), "{out}");
    for line in ["Skills: notes", "Flows: ship", "Automations: fmt"] {
        assert!(out.contains(line), "{line} missing:\n{out}");
    }
    let (_, out, _) = env.run(&["list", "--tags", "dev"]);
    assert!(
        out.contains("1 agent(s) found.") && !out.contains("Leads"),
        "{out}"
    );
}

#[test]
fn list_warns_about_an_invalid_file_and_shows_the_rest() {
    let mut files = TEAM.to_vec();
    files.push((".armadai/agents/broken.md", "no frontmatter\n"));
    let env = Env::new(&files);
    let (ok, out, err) = env.run(&["list"]);
    assert!(ok, "list is read-only and never refuses: {err}");
    assert!(
        err.contains("broken.md") && err.contains("missing-frontmatter"),
        "{err}"
    );
    assert!(out.contains("2 agent(s) found."), "{out}");
}

#[test]
fn inspect_shows_one_agent_and_its_flows() {
    let env = Env::new(TEAM);
    let (ok, out, err) = env.run(&["inspect", "dev"]);
    assert!(ok, "{err}");
    for needle in [
        "Agent: dev",
        "Writes code",
        "Skills:",
        "notes",
        "Flows:",
        "ship",
        "You write code.",
    ] {
        assert!(out.contains(needle), "{needle} missing:\n{out}");
    }
    let (ok, _, err) = env.run(&["inspect", "ghost"]);
    assert!(!ok && err.contains("dev, lead"), "{err}");
}

#[test]
fn list_and_inspect_refuse_a_legacy_project_with_the_migrate_hint() {
    let env = Env::new(&[(".armadai/config.yaml", "agents: []\n")]);
    for args in [&["list"][..], &["inspect", "x"][..]] {
        let (ok, _, err) = env.run(args);
        assert!(
            !ok && err.contains("run `armadai migrate`"),
            "{args:?}: {err}"
        );
    }
}

fn exists(root: &Path, files: &[&str]) {
    for f in files {
        assert!(root.join(f).exists(), "{f} missing");
    }
}

#[test]
fn init_project_writes_a_valid_empty_project() {
    let env = Env::new(&[]);
    let (ok, _, err) = env.run(&["init", "--project"]);
    assert!(ok, "{err}");
    exists(
        &env.root(),
        &[
            ".armadai/project.yaml",
            ".armadai/agents/.gitkeep",
            ".armadai/skills/.gitkeep",
            ".armadai/flows/.gitkeep",
            ".armadai/automations/.gitkeep",
            ".armadai/.gitignore",
        ],
    );
    let (ok, _, err) = env.run(&["validate"]);
    assert!(ok, "{err}");
    let (ok, _, _) = env.run(&["init", "--project"]);
    assert!(!ok, "an existing project is never overwritten");
}

#[test]
fn init_refuses_a_legacy_project() {
    let env = Env::new(&[("armadai.yaml", "agents: []\n")]);
    let (ok, _, err) = env.run(&["init", "--project"]);
    assert!(!ok && err.contains("armadai migrate"), "{err}");
}

#[test]
fn init_pack_writes_a_project_that_validates_and_links() {
    let env = Env::new(&[]);
    let pack = Path::new(env!("CARGO_MANIFEST_DIR")).join("../armadai-core/starters/rust-dev");
    let (ok, out, err) = env.run(&["init", "--pack", pack.to_str().unwrap()]);
    assert!(ok, "{out}\n{err}");
    exists(
        &env.root(),
        &[
            ".armadai/project.yaml",
            ".armadai/agents/code-reviewer.md",
            ".armadai/agents/test-writer.md",
            ".armadai/agents/debug.md",
            ".armadai/.gitignore",
        ],
    );
    assert!(
        !env.root().join(".armadai/.migrated").exists(),
        "no backup copied"
    );
    assert!(
        !env.root().join(".armadai/config.yaml").exists(),
        "no legacy config"
    );
    let (ok, _, err) = env.run(&["validate"]);
    assert!(ok, "{err}");
    let (ok, _, err) = env.run(&["link", "--target", "claude"]);
    assert!(ok, "{err}");
    assert!(env.root().join(".claude/agents/code-reviewer.md").is_file());
}

#[test]
fn skills_list_reads_the_project_skills_folder() {
    let env = Env::new(TEAM);
    let (ok, out, err) = env.run(&["skills", "list"]);
    assert!(ok, "{err}");
    assert!(out.contains("notes"), "{out}");
}

/// `armadai init` (the user configuration) creates no agent or prompt library:
/// nothing reads one any more but `migrate`, for a legacy project that already has it.
/// Nor a `providers.yaml`: nothing has read it since agents stopped running.
#[test]
fn init_creates_no_user_library() {
    let env = Env::new(&[]);
    let (ok, out, err) = env.run(&["init"]);
    assert!(ok, "{out}{err}");
    let config = env.dir.path().join("config");
    assert!(config.join("config.yaml").is_file(), "{out}");
    assert!(!config.join("providers.yaml").exists(), "{out}");
    assert!(!out.contains("providers"), "{out}");
    assert!(!config.join("agents").exists() && !config.join("prompts").exists());
}

/// `config providers` and `config secrets` are gone: the first printed a file
/// nothing reads, the second wrote an age private key under `.armadai/`, where a
/// project would commit it. `starters-dir` stays.
#[test]
fn config_has_only_starters_dir() {
    let env = Env::new(&[]);
    for gone in [&["config", "providers"][..], &["config", "secrets", "init"]] {
        let (ok, _, err) = env.run(gone);
        assert!(
            !ok && err.contains("unrecognized subcommand"),
            "{gone:?}: {err}"
        );
    }
    assert!(!env.root().join(".armadai/age-key.txt").exists());
    let (ok, out, err) = env.run(&["config", "starters-dir", "list"]);
    assert!(ok, "{out}{err}");
}

/// A `config/settings.yaml` (any third-party project may have one) no longer
/// prints a hint to run `armadai init`, which migrated nothing.
#[test]
fn a_config_settings_yaml_prints_no_hint() {
    let env = Env::new(&[("config/settings.yaml", "a: 1\n")]);
    let (ok, out, err) = env.run(&["config", "starters-dir", "list"]);
    assert!(ok, "{out}{err}");
    assert!(
        !err.contains("settings.yaml") && !err.contains("hint"),
        "{err}"
    );
}

const INSTANCE: &[(&str, &str)] = &[
    (".armadai/project.yaml", "schema_version: 1\n"),
    (
        ".armadai/templates/checker.md",
        "---\ndescription: \"Checks {{subject}}\"\ntier: pro\n---\nYou check {{subject}}.\n",
    ),
    (
        ".armadai/agents/guardian.md",
        "---\nfrom: checker\nvars:\n  subject: the frame\n---\nExtra line.\n",
    ),
    (
        ".armadai/agents/lead.md",
        "---\ndescription: Leads\n---\nLead.\n",
    ),
];

#[test]
fn list_shows_the_templates_and_where_an_instance_comes_from() {
    let env = Env::new(INSTANCE);
    let (ok, out, err) = env.run(&["list"]);
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "  ID                       TIER  TAGS  DESCRIPTION\n  \
         guardian (from checker)  pro   -  Checks the frame\n  \
         lead                     -     -  Leads\n\n  \
         2 agent(s) found.\n\
         Templates: checker\n"
    );
}

#[test]
fn inspect_shows_an_instance_rendered_and_its_template() {
    let env = Env::new(INSTANCE);
    let (ok, out, err) = env.run(&["inspect", "guardian"]);
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "Agent: guardian\n\
         Source: .armadai/agents/guardian.md, from: checker (.armadai/templates/checker.md)\n\n  \
         Description:  Checks the frame\n  \
         Tier:         pro\n\n\
         ## Prompt\n  \
         You check the frame.\n  \n  \
         Extra line.\n"
    );
}

const SLOTTED: &[(&str, &str)] = &[
    (".armadai/project.yaml", "schema_version: 1\n"),
    (
        ".armadai/agents/dev.md",
        "---\ndescription: Writes code\nskills: [notes]\nslots: [security, code-standards]\n---\nCode.\n",
    ),
    (
        ".armadai/skills/notes/SKILL.md",
        "---\nname: notes\ndescription: Notes\n---\nB\n",
    ),
    (
        ".armadai/skills/b-std/SKILL.md",
        "---\nname: b-std\ndescription: B\nmetadata:\n  armadai-slot: code-standards\n---\nB\n",
    ),
    (
        ".armadai/skills/a-std/SKILL.md",
        "---\nname: a-std\ndescription: A\nmetadata:\n  armadai-slot: code-standards\n---\nA\n",
    ),
    (
        ".armadai/commands/brief.md",
        "---\ndescription: Refine an idea\n---\n$ARGUMENTS\n",
    ),
];

#[test]
fn list_shows_the_commands_and_the_skills_of_each_slot() {
    let env = Env::new(SLOTTED);
    let (ok, out, err) = env.run(&["list"]);
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "  ID   TIER  TAGS  DESCRIPTION\n  \
         dev  -     -  Writes code\n\n  \
         1 agent(s) found.\n\
         Skills: a-std, b-std, notes\n\
         Commands: brief\n\
         Slots:\n  \
         code-standards: a-std, b-std\n  \
         security: (none)\n"
    );
}

#[test]
fn inspect_shows_each_slot_and_the_skills_filling_it() {
    let env = Env::new(SLOTTED);
    let (ok, out, err) = env.run(&["inspect", "dev"]);
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "Agent: dev\n\
         Source: .armadai/agents/dev.md\n\n  \
         Description:  Writes code\n  \
         Tier:         -\n  \
         Skills:       notes\n  \
         Slots:        security: (none)\n  \
         \x20             code-standards: a-std, b-std\n\n\
         ## Prompt\n  \
         Code.\n"
    );
}
