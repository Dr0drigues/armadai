//! Commands (`.armadai/commands/`) end to end for Claude Code: link then unlink,
//! a hand-written command file, `--agents`, and a native pack shipping commands.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{Value, json};

const BRIEF: &str =
    "---\ndescription: Refine an idea\nargument_hint: <idea>\n---\nRefine: $ARGUMENTS\n";
const BRIEF_CLAUDE: &str =
    "---\ndescription: Refine an idea\nargument-hint: <idea>\n---\nRefine: $ARGUMENTS\n";
const DEV_CLAUDE: &str =
    "---\nname: dev\ndescription: Writes code\nskills:\n- a-std\n---\nYou write code.\n";

const PROJECT: &[(&str, &str)] = &[
    (".armadai/project.yaml", "schema_version: 1\n"),
    (".armadai/commands/brief.md", BRIEF),
    (
        ".armadai/agents/dev.md",
        "---\ndescription: Writes code\nslots: [code-standards]\n---\nYou write code.\n",
    ),
    (
        ".armadai/agents/qa.md",
        "---\ndescription: Tests\n---\nYou test.\n",
    ),
    (
        ".armadai/skills/a-std/SKILL.md",
        "---\nname: a-std\ndescription: A\nmetadata:\n  armadai-slot: code-standards\n---\nA\n",
    ),
];

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
            let path = env.root().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        env
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("HOME", self.dir.path().join("home"))
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        let (stdout, stderr) = (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        );
        assert!(out.status.success(), "{args:?} failed:\n{stdout}\n{stderr}");
        stdout
    }

    /// The report line of `element`, from `link --json`.
    fn line(&self, args: &[&str], element: &str) -> Value {
        let mut all = vec!["link", "--json"];
        all.extend_from_slice(args);
        let report: Vec<Value> = serde_json::from_str(&self.ok(&all)).unwrap();
        (report.into_iter())
            .find(|l| l["element"] == element)
            .unwrap_or_else(|| panic!("no `{element}`"))
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root().join(rel)).unwrap()
    }
}

/// Every path under `root` with its bytes, directories included (empty content,
/// `/`-terminated), so an empty directory left behind is seen too.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            if path.is_dir() {
                out.insert(PathBuf::from(format!("{}/", rel.display())), Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn link_then_unlink_brings_the_tree_back() {
    let env = Env::new(PROJECT);
    let before = tree(&env.root());
    env.ok(&["link", "--target", "claude"]);
    assert_eq!(env.read(".claude/commands/brief.md"), BRIEF_CLAUDE);
    assert_eq!(env.read(".claude/agents/dev.md"), DEV_CLAUDE);
    env.ok(&["unlink", "--target", "claude"]);
    let mut after = tree(&env.root());
    // the one file link leaves on purpose: it keeps the manifest out of git
    after.remove(Path::new(".armadai/.gitignore"));
    // left on purpose, empty: unlink never removes the target root
    assert_eq!(after.remove(Path::new(".claude/")), Some(Vec::new()));
    assert_eq!(after, before);
}

/// Review Focus 4: a command file link did not write is never overwritten.
#[test]
fn a_hand_written_command_file_is_left_in_place() {
    let env = Env::new(PROJECT);
    std::fs::create_dir_all(env.root().join(".claude/commands")).unwrap();
    std::fs::write(env.root().join(".claude/commands/brief.md"), "mine\n").unwrap();
    let l = env.line(&["--target", "claude"], "command brief");
    assert_eq!(
        l,
        json!({
            "target": "claude",
            "element": "command brief",
            "status": "not-compiled",
            "reason": "file exists and was not written by link (use --force): .claude/commands/brief.md"
        })
    );
    assert_eq!(env.read(".claude/commands/brief.md"), "mine\n");
}

/// Review Focus 5: commands depend on no agent, `--agents` keeps them.
#[test]
fn link_agents_still_writes_the_commands() {
    let env = Env::new(PROJECT);
    env.ok(&["link", "--target", "claude", "--agents", "dev"]);
    assert_eq!(env.read(".claude/commands/brief.md"), BRIEF_CLAUDE);
    assert_eq!(env.read(".claude/agents/dev.md"), DEV_CLAUDE);
    assert!(!env.root().join(".claude/agents/qa.md").exists());
}

#[test]
fn a_native_pack_installs_its_commands() {
    let env = Env::new(&[]);
    let pack = env.dir.path().join("packs/demo");
    for (p, c) in [
        (
            "pack.yaml",
            "format: 1\nname: demo\ndescription: A demo pack\n",
        ),
        ("agents/dev.md", "---\ndescription: Codes\n---\nCode.\n"),
        ("commands/brief.md", BRIEF),
    ] {
        let path = pack.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, c).unwrap();
    }
    let out = env.ok(&["init", "--pack", pack.to_str().unwrap()]);
    assert_eq!(env.read(".armadai/commands/brief.md"), BRIEF);
    assert_eq!(
        out,
        "Created .armadai/ from pack 'demo': 1 agent(s), 0 skill(s), 0 flow(s), \
         0 automation(s), 1 command(s)\n  Run `armadai link` to generate the target files.\n"
    );
}

/// OpenCode: the command and the slot skill land in the real output directory, and
/// unlink takes them back.
#[test]
fn opencode_link_then_unlink_brings_the_tree_back() {
    let env = Env::new(PROJECT);
    let before = tree(&env.root());
    env.ok(&["link", "--target", "opencode", "--output", "oc"]);
    assert_eq!(
        env.read("oc/commands/brief.md"),
        "---\ndescription: Refine an idea\n---\nRefine: $ARGUMENTS\n"
    );
    assert_eq!(
        env.read("oc/agents/dev.md"),
        "---\ndescription: Writes code\nmode: subagent\n---\nYou write code.\n\n## Skills\n\nBefore you start, load these skills with the `skill` tool: `a-std`.\n"
    );
    assert!(!env.root().join(".opencode").exists());
    env.ok(&["unlink", "--target", "opencode", "--output", "oc"]);
    let mut after = tree(&env.root());
    after.remove(Path::new(".armadai/.gitignore"));
    assert_eq!(after.remove(Path::new("oc/")), Some(Vec::new()));
    assert_eq!(after, before);
}

/// `unlink --agents dev` takes the commands back too, as it does the skills: they
/// depend on no agent, so any agent-filtered unlink removes them.
#[test]
fn unlink_agents_takes_the_commands_back() {
    let env = Env::new(PROJECT);
    env.ok(&["link", "--target", "claude"]);
    env.ok(&["unlink", "--target", "claude", "--agents", "dev"]);
    assert!(!env.root().join(".claude/commands/brief.md").exists());
    assert!(!env.root().join(".claude/agents/dev.md").exists());
    assert!(env.root().join(".claude/agents/qa.md").exists());
}

/// OpenCode: the coordinator is a primary agent, its filled slot is compiled
/// (allowed through `permission.skill`), unlike Claude's main session.
#[test]
fn an_opencode_coordinator_slot_is_compiled() {
    let mut files = PROJECT.to_vec();
    files[0] = (
        ".armadai/project.yaml",
        "schema_version: 1\ncoordinator: dev\n",
    );
    let env = Env::new(&files);
    let l = env.line(&["--target", "opencode"], "agent dev slot code-standards");
    assert_eq!(
        l,
        json!({
            "target": "opencode",
            "element": "agent dev slot code-standards",
            "status": "compiled",
            "reason": "a-std"
        })
    );
}

/// OpenCode: an agent whose only slot is empty gets no `permission.skill` rule
/// (it keeps every skill, as an agent without `skills:` does).
#[test]
fn an_opencode_agent_with_only_an_empty_slot_has_no_skill_rule() {
    let env = Env::new(&[
        (".armadai/project.yaml", "schema_version: 1\n"),
        (
            ".armadai/agents/dev.md",
            "---\ndescription: Writes code\nslots: [security]\n---\nYou write code.\n",
        ),
    ]);
    let l = env.line(&["--target", "opencode"], "agent dev slot security");
    assert_eq!(l["status"], "not-compiled");
    assert_eq!(
        env.read(".opencode/agents/dev.md"),
        "---\ndescription: Writes code\nmode: subagent\n---\nYou write code.\n"
    );
}

/// A command whose `native.claude` is not a mapping stops link, nothing written.
#[test]
fn a_command_native_claude_that_is_not_a_mapping_is_an_error() {
    let env = Env::new(&[
        (".armadai/project.yaml", "schema_version: 1\n"),
        (
            ".armadai/commands/brief.md",
            "---\ndescription: D\nnative:\n  claude: x\n---\nDo $ARGUMENTS\n",
        ),
    ]);
    let before = tree(&env.root());
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(env.root())
        .env("HOME", env.dir.path().join("home"))
        .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
        .env("NO_COLOR", "1")
        .args(["link", "--target", "claude"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Error: command `brief`: native.claude must be a mapping\n"
    );
    assert_eq!(tree(&env.root()), before);
}

/// OpenCode: a slot never restricts the skills. A coordinator without `skills:`
/// whose slot gets filled later is told to load the new skill but gets no
/// `permission.skill`: it keeps every other skill of the project.
#[test]
fn an_opencode_slot_never_denies_the_other_skills() {
    let env = Env::new(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\ncoordinator: lead\n",
        ),
        (
            ".armadai/agents/lead.md",
            "---\ndescription: Leads\nslots: [code-standards]\n---\nYou lead.\n",
        ),
        (
            ".armadai/skills/notes/SKILL.md",
            "---\nname: notes\ndescription: Notes\n---\nN\n",
        ),
    ]);
    env.ok(&["link", "--target", "opencode"]);
    let lead = "---\ndescription: Leads\nmode: primary\n---\nYou lead.\n\n## Delegation\n\n\
                You coordinate the team described in `.opencode/armadai-team.md`. Delegate \
                with the `subagent` tool, naming the agent by its id.\n";
    assert_eq!(env.read(".opencode/agents/lead.md"), lead);
    std::fs::create_dir_all(env.root().join(".armadai/skills/std")).unwrap();
    std::fs::write(
        env.root().join(".armadai/skills/std/SKILL.md"),
        "---\nname: std\ndescription: Standards\nmetadata:\n  armadai-slot: code-standards\n---\nS\n",
    )
    .unwrap();
    env.ok(&["link", "--target", "opencode"]);
    assert_eq!(
        env.read(".opencode/agents/lead.md"),
        format!(
            "{lead}\n## Skills\n\nBefore you start, load these skills with the `skill` tool: `std`.\n"
        )
    );
}

/// OpenCode: an agent with `skills:` is restricted to them plus its slot skills.
#[test]
fn opencode_declared_skills_and_slot_skills_are_allowed_together() {
    let env = Env::new(&[
        (".armadai/project.yaml", "schema_version: 1\n"),
        (
            ".armadai/agents/dev.md",
            "---\ndescription: Codes\nskills: [x]\nslots: [s]\n---\nCode.\n",
        ),
        (
            ".armadai/skills/x/SKILL.md",
            "---\nname: x\ndescription: X\n---\nX\n",
        ),
        (
            ".armadai/skills/a/SKILL.md",
            "---\nname: a\ndescription: A\nmetadata:\n  armadai-slot: s\n---\nA\n",
        ),
    ]);
    env.ok(&["link", "--target", "opencode"]);
    assert_eq!(
        env.read(".opencode/agents/dev.md"),
        "---\ndescription: Codes\nmode: subagent\npermission:\n  skill:\n    \"*\": deny\n    x: allow\n    a: allow\n---\nCode.\n\n## Skills\n\nBefore you start, load these skills with the `skill` tool: `x`, `a`.\n"
    );
}
