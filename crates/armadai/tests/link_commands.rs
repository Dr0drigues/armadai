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

/// Every file under `root` with its bytes (`.git/` aside).
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
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
    assert_eq!(
        env.read(".claude/agents/dev.md"),
        "---\nname: dev\ndescription: Writes code\nskills:\n- a-std\n---\nYou write code.\n"
    );
    env.ok(&["unlink", "--target", "claude"]);
    let mut after = tree(&env.root());
    // the one file link leaves on purpose: it keeps the manifest out of git
    after.remove(Path::new(".armadai/.gitignore"));
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
    assert!(
        out.contains("1 agent(s), 0 skill(s), 0 flow(s), 0 automation(s), 1 command(s)"),
        "{out}"
    );
}
