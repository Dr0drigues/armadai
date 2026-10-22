//! Agent templates are rendered at load time (step 4, lot B0): a project whose
//! agents come from `templates/` links exactly like the same project written by
//! hand with the rendered agents, and a changed template is relinked without
//! `--force`.
//!
//! `tests/fixtures/templates-instances/` and `tests/fixtures/templates-flattened/`
//! are the same project: keep them in step when one changes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

const TARGETS: &[&str] = &["claude", "codex", "copilot", "opencode"];

/// A temp dir holding `project/` (a copy of the fixture), `home/` and `config/`.
struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(fixture: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for d in ["project", "config", "home"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let env = Env { dir };
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture),
            &env.root(),
        );
        env
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// stdout of a successful `armadai <args>`, the project root replaced by `<ROOT>`.
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
        stdout.replace(&self.root().display().to_string(), "<ROOT>")
    }

    /// Every file `link` wrote (all text): all of the project but `.armadai/`.
    fn linked(&self) -> BTreeMap<PathBuf, String> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                let rel = p.strip_prefix(root).unwrap().to_path_buf();
                if rel == Path::new(".armadai") {
                    continue;
                }
                if p.is_dir() {
                    walk(root, &p, out);
                } else {
                    out.insert(rel, std::fs::read_to_string(&p).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.root(), &self.root(), &mut out);
        out
    }
}

// shortcut: each integration test is its own crate; a copy of the helper
// link_golden.rs also carries, until a shared tests/common module is worth it
fn copy_tree(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let dest = to.join(p.file_name().unwrap());
        if p.is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_tree(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}

#[test]
fn instances_link_exactly_like_their_flattened_agents() {
    for target in TARGETS {
        let (inst, flat) = (
            Env::new("templates-instances"),
            Env::new("templates-flattened"),
        );
        assert!(inst.root().join(".armadai/templates/checker.md").is_file());
        assert!(!flat.root().join(".armadai/templates").exists());

        let args = ["link", "--target", target, "--dry-run", "--json"];
        assert_eq!(inst.ok(&args), flat.ok(&args), "{target}: dry-run report");

        let args = ["link", "--target", target];
        assert_eq!(inst.ok(&args), flat.ok(&args), "{target}: link stdout");
        let written = inst.linked();
        // both trees empty would compare equal: guard against a vacuous match
        assert!(
            written.keys().any(|p| p.ends_with("guardian.md")
                || p.ends_with("guardian.toml")
                || p.ends_with("guardian.agent.md")),
            "{target}: no guardian file in {:?}",
            written.keys().collect::<Vec<_>>()
        );
        assert_eq!(written, flat.linked(), "{target}: linked files");
    }
}

/// Review Focus 3: the generated files are link's own, unchanged by hand, so the
/// next link rewrites them from the changed template without `--force`.
#[test]
fn a_changed_template_relinks_without_force() {
    let env = Env::new("templates-instances");
    let mut link = vec!["link", "--target"];
    link.extend(TARGETS);
    env.ok(&link);
    let before = env.linked();

    let template = env.root().join(".armadai/templates/checker.md");
    let text = std::fs::read_to_string(&template).unwrap();
    let (old, new) = ("You check {{subject}}:", "You inspect {{subject}}:");
    assert_eq!(text.matches(old).count(), 1);
    std::fs::write(&template, text.replace(old, new)).unwrap();
    env.ok(&link);

    let expected: BTreeMap<PathBuf, String> = (before.iter())
        .map(|(p, s)| {
            let s = s
                .replace("You check the frame:", "You inspect the frame:")
                .replace("You check the code:", "You inspect the code:");
            (p.clone(), s)
        })
        .collect();
    let changed = (before.iter()).filter(|(p, b)| expected[*p] != **b).count();
    // guardian and validator, for each of the four targets
    assert_eq!(changed, 8, "files the change must reach");
    assert_eq!(env.linked(), expected);
}

/// Spec §8: a generated file edited by hand is refused by the next link, and the
/// report line names the template the agent comes from.
#[test]
fn a_refused_instance_names_its_template() {
    let env = Env::new("templates-instances");
    env.ok(&["link", "--target", "claude"]);
    let generated = env.root().join(".claude/agents/guardian.md");
    let edited = std::fs::read_to_string(&generated).unwrap() + "Edited by hand.\n";
    std::fs::write(&generated, &edited).unwrap();

    let out = env.ok(&["link", "--target", "claude"]);
    let lines: Vec<&str> = (out.lines())
        .filter(|l| l.starts_with("claude: agent guardian:"))
        .collect();
    assert_eq!(
        lines,
        [
            "claude: agent guardian: not-compiled (edited since the last link (use --force): \
          .claude/agents/guardian.md (from template checker))"
        ]
    );
    assert_eq!(std::fs::read_to_string(&generated).unwrap(), edited);
}
