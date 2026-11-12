//! `link` shows the warnings of permission levels (`grant-*`) on stderr, one rendered
//! diagnostic each, and nothing else: not the other warnings, not on stdout.

use assert_cmd::Command;
use std::path::PathBuf;

struct Env(tempfile::TempDir);

impl Env {
    fn new(project_yaml: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai/agents")).unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(root.join(".armadai/project.yaml"), project_yaml).unwrap();
        std::fs::write(
            root.join(".armadai/agents/lead.md"),
            "---\ndescription: Leads\n---\nYou lead.\n",
        )
        .unwrap();
        Env(dir)
    }

    /// The project root as the binary prints it (macOS temp dirs are symlinks).
    fn root(&self) -> PathBuf {
        self.0.path().join("project").canonicalize().unwrap()
    }

    fn link(&self, args: &[&str]) -> (String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("HOME", self.0.path().join("home"))
            .env("ARMADAI_CONFIG_DIR", self.0.path().join("config"))
            .arg("link")
            .args(["--target", "claude"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

/// A level with a broad rule and one a level never answers, plus a warning of another
/// family (`unknown-free-agent`) that `link` must keep not showing.
const WIDE: &str = "schema_version: 1\ncoordinator: lead\npolicy:\n  free_agents: [Ghost]\n  grants:\n    wide:\n      allow: [\"Bash(*)\", \"Edit(/src/*)\"]\n";

#[test]
fn link_shows_the_grant_warnings_and_only_those_on_stderr() {
    let env = Env::new(WIDE);
    let file = env.root().join(".armadai/project.yaml");
    let file = file.display();
    let want = format!(
        "{file}:7:27: warning[grant-unsupported-rule]: grant `wide` lists `Edit(/src/*)`, and a level answers `Bash` rules only\n  hint: the rule is never granted: write a `Bash(...)` rule, or put it in native.claude.permissions.allow\n\
         {file}:7:16: warning[grant-broad-rule]: grant `wide` rule `Bash(*)` is broad: it lets through any command it matches\n  hint: a grant is a convenience, not a boundary; name the exact command, such as `Bash(python3 scripts/release.py)`\n"
    );
    let (stdout, stderr) = env.link(&["--json"]);
    assert_eq!(stderr, want);
    // the machine report stays valid JSON without them
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(report.is_array());
    assert!(!stdout.contains("grant-"), "{stdout}");
    // and the plain report does not carry them either
    let (stdout, stderr) = env.link(&[]);
    assert_eq!(stderr, want);
    assert!(!stdout.contains("warning["), "{stdout}");
}

#[test]
fn link_without_a_level_shows_no_grant_line() {
    let env = Env::new("schema_version: 1\ncoordinator: lead\npolicy:\n  free_agents: [Ghost]\n");
    let (_, stderr) = env.link(&["--json"]);
    assert_eq!(stderr, "");
}
