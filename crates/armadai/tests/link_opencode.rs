//! `armadai link` / `unlink` for OpenCode, end to end (step 4, Task 5): the owned
//! keys of the root `opencode.json`, the team file, the stale `instructions.md` of
//! the former linker, `--dry-run`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{Value, json};

const PROJECT: &[(&str, &str)] = &[
    (
        ".armadai/project.yaml",
        "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n",
    ),
    (
        ".armadai/agents/lead.md",
        "---\ndescription: Leads\n---\nYou lead.\n",
    ),
    (
        ".armadai/agents/dev.md",
        "---\ndescription: Writes code\n---\nYou write code.\n",
    ),
    (
        ".armadai/skills/notes/SKILL.md",
        "---\nname: notes\ndescription: Notes\n---\nTake notes.\n",
    ),
];

const USER_OPENCODE: &str = r#"{"theme":"x","instructions":["mine.md"]}"#;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for d in ["project", "config", "home"] {
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

    fn opencode_json(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.root().join("opencode.json")).unwrap())
            .unwrap()
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
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// The keys link owns in `opencode.json` for [`PROJECT`].
fn owned_keys(team: &str) -> Value {
    json!({
        "default_agent": "lead",
        "agent": {
            "build": {"permission": {"task": {"*": "deny"}}},
            "plan": {"permission": {"task": {"*": "deny"}}}
        },
        "experimental": {"subagent_depth": 1}
    })
    .as_object()
    .unwrap()
    .clone()
    .into_iter()
    .chain([("instructions".to_string(), json!([team]))])
    .collect()
}

#[test]
fn link_then_unlink_restores_a_hand_written_opencode_json() {
    let mut files = PROJECT.to_vec();
    files.push(("opencode.json", USER_OPENCODE));
    files.push(("AGENTS.md", "# Mine\n"));
    files.push(("CLAUDE.md", "# Claude notes\n"));
    let env = Env::new(&files);
    let root = env.root();

    env.ok(&["link", "--target", "opencode"]);
    let mut expected = owned_keys(".opencode/armadai-team.md");
    expected["theme"] = json!("x");
    expected["instructions"] = json!(["mine.md", ".opencode/armadai-team.md"]);
    assert_eq!(env.opencode_json(), expected);
    assert!(root.join(".opencode/armadai-team.md").is_file());
    assert!(root.join(".opencode/agents/lead.md").is_file());
    assert!(root.join(".opencode/skills/notes/SKILL.md").is_file());

    env.ok(&["unlink", "--target", "opencode"]);
    assert_eq!(
        env.opencode_json(),
        serde_json::from_str::<Value>(USER_OPENCODE).unwrap()
    );
    for gone in ["agents", "skills", "armadai-team.md"] {
        assert!(!root.join(".opencode").join(gone).exists(), "{gone}");
    }
    let before: BTreeMap<PathBuf, Vec<u8>> = files
        .iter()
        .filter(|(p, _)| *p != "opencode.json")
        .map(|(p, c)| (PathBuf::from(p), c.as_bytes().to_vec()))
        .collect();
    let mut after = tree(&root);
    after.remove(Path::new("opencode.json"));
    after.remove(Path::new(".armadai/.gitignore"));
    assert_eq!(
        after, before,
        "AGENTS.md, CLAUDE.md and .armadai/ untouched"
    );
}

#[test]
fn link_creates_opencode_json_and_unlink_removes_it() {
    let env = Env::new(PROJECT);
    env.ok(&["link", "--target", "opencode"]);
    assert_eq!(env.opencode_json(), owned_keys(".opencode/armadai-team.md"));
    env.ok(&["unlink", "--target", "opencode"]);
    assert!(!env.root().join("opencode.json").exists());
}

#[test]
fn instructions_name_the_team_file_in_a_custom_output_directory() {
    let env = Env::new(PROJECT);
    env.ok(&["link", "--target", "opencode", "--output", "oc"]);
    assert!(env.root().join("oc/armadai-team.md").is_file());
    assert_eq!(env.opencode_json(), owned_keys("oc/armadai-team.md"));
    env.ok(&["unlink", "--target", "opencode", "--output", "oc"]);
    assert!(!env.root().join("opencode.json").exists());
}

#[test]
fn the_ignored_instructions_md_is_removed_as_stale() {
    let env = Env::new(PROJECT);
    let root = env.root();
    env.ok(&["link", "--target", "opencode"]);
    // what the former linker left: `.opencode/instructions.md`, recorded as created
    let old = b"# Team\n";
    std::fs::write(root.join(".opencode/instructions.md"), old).unwrap();
    let digest = {
        use sha2::{Digest, Sha256};
        let mut s = String::from("sha256:");
        for b in Sha256::digest(old) {
            s.push_str(&format!("{b:02x}"));
        }
        s
    };
    let manifest = root.join(".armadai/link-manifest.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let entry = format!(
        "    entries:\n    - path: .opencode/instructions.md\n      produced_by: \
         {{ kind: coordinator, name: lead }}\n      outcome: created\n      digest: {digest}\n"
    );
    assert_eq!(text.matches("    entries:\n").count(), 1, "{text}");
    std::fs::write(&manifest, text.replace("    entries:\n", &entry)).unwrap();

    let report: Value =
        serde_json::from_str(&env.ok(&["link", "--target", "opencode", "--json"])).unwrap();
    let stale: Vec<&Value> = (report.as_array().unwrap().iter())
        .filter(|l| l["element"] == "stale .opencode/instructions.md")
        .collect();
    assert_eq!(stale.len(), 1, "{report:#}");
    assert_eq!(stale[0]["status"], "compiled");
    assert_eq!(stale[0]["reason"], "removed: no longer produced");
    assert!(!root.join(".opencode/instructions.md").exists());
}

#[test]
fn dry_run_writes_nothing() {
    let mut files = PROJECT.to_vec();
    files.push(("opencode.json", USER_OPENCODE));
    let env = Env::new(&files);
    let before = tree(&env.root());
    env.ok(&["link", "--target", "opencode", "--dry-run"]);
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn an_opencode_jsonc_refuses_the_link_before_any_write() {
    let mut files = PROJECT.to_vec();
    files.push(("opencode.jsonc", "{}"));
    let env = Env::new(&files);
    let before = tree(&env.root());
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(env.root())
        .env("HOME", env.dir.path().join("home"))
        .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
        .args(["link", "--target", "opencode"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected = format!(
        "Error: {} exists: rename it to opencode.json or remove it, link only patches opencode.json\n",
        env.root()
            .canonicalize()
            .unwrap()
            .join("opencode.jsonc")
            .display()
    );
    assert_eq!(stderr, expected);
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn keys_no_longer_wanted_are_removed_on_relink() {
    let mut files = PROJECT.to_vec();
    files.push(("opencode.json", USER_OPENCODE));
    let env = Env::new(&files);
    env.ok(&["link", "--target", "opencode"]);
    // no coordinator, no flow, no strict delegation: link owns nothing in opencode.json
    std::fs::write(
        env.root().join(".armadai/project.yaml"),
        "schema_version: 1\n",
    )
    .unwrap();
    env.ok(&["link", "--target", "opencode"]);
    assert_eq!(
        env.opencode_json(),
        serde_json::from_str::<Value>(USER_OPENCODE).unwrap()
    );
}
