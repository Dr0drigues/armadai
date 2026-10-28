//! A mistake under `model_tiers` of config.yaml is a warning for what does not use it
//! (`config starters-dir`, the pack search) and an error only for `link --target opencode` (#506).

use assert_cmd::Command;
use std::path::Path;

const CONFIG_TAIL: &str = "model_tiers:\n  opencode:\n    medium: x\n";

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(env.dir.path().join("cfg")).unwrap();
        std::fs::create_dir_all(env.root().join(".armadai")).unwrap();
        std::fs::write(
            env.root().join(".armadai/project.yaml"),
            "schema_version: 1\n",
        )
        .unwrap();
        env
    }

    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("project")
    }

    fn config(&self) -> std::path::PathBuf {
        self.dir.path().join("cfg/config.yaml")
    }

    fn run(&self, args: &[&str]) -> (Option<i32>, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("HOME", self.dir.path())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("cfg"))
            .env("NO_COLOR", "1")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("ARMADAI_STARTERS_DIRS")
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn warning(&self) -> String {
        format!(
            "warning: {}: model_tiers.opencode.medium is not a tier (fast, pro, max); ignored here, but `armadai link --target opencode` refuses it\n",
            self.config().display()
        )
    }
}

#[test]
fn starters_dir_add_warns_and_keeps_going() {
    let env = Env::new();
    std::fs::write(
        env.config(),
        format!("starters_dirs:\n- /opt/a\n{CONFIG_TAIL}"),
    )
    .unwrap();
    let (code, err) = env.run(&["config", "starters-dir", "add", "/opt/b"]);
    assert_eq!((code, err), (Some(0), env.warning()));
    let text = std::fs::read_to_string(env.config()).unwrap();
    assert_eq!(
        text,
        format!("starters_dirs:\n- /opt/a\n- /opt/b\n{CONFIG_TAIL}")
    );
}

#[test]
fn link_for_opencode_fails_and_writes_nothing() {
    let env = Env::new();
    std::fs::write(env.config(), CONFIG_TAIL).unwrap();
    let (code, err) = env.run(&["link", "--target", "opencode"]);
    let line = format!(
        "{}: model_tiers.opencode.medium is not a tier (fast, pro, max); fix it or remove it",
        env.config().display()
    );
    assert_eq!(code, Some(1), "{err}");
    assert!(err.lines().any(|l| l == format!("Error: {line}")), "{err}");
    assert!(!env.root().join(".opencode").exists());
    assert!(!env.root().join("opencode.json").exists());
}

#[test]
fn the_pack_search_keeps_the_starters_dirs() {
    let env = Env::new();
    let starters = env.dir.path().join("mine");
    let pack: &Path = &starters.join("demo");
    std::fs::create_dir_all(pack).unwrap();
    std::fs::write(
        pack.join("pack.yaml"),
        "format: 1\nname: demo\ndescription: d\n",
    )
    .unwrap();
    std::fs::write(
        env.config(),
        format!("starters_dirs:\n- {}\n{CONFIG_TAIL}", starters.display()),
    )
    .unwrap();
    std::fs::remove_dir_all(env.root().join(".armadai")).unwrap();
    let (code, err) = env.run(&["init", "--pack", "demo", "--no-link"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(err.contains(&env.warning()), "{err}");
}

#[test]
fn a_mistake_under_another_target_is_a_plain_warning() {
    let env = Env::new();
    std::fs::write(env.config(), "model_tiers:\n  codex:\n    medium: x\n").unwrap();
    let (code, err) = env.run(&["config", "starters-dir", "list"]);
    assert_eq!(code, Some(0), "{err}");
    let line = format!(
        "warning: {}: model_tiers.codex.medium is not a tier (fast, pro, max); ignored\n",
        env.config().display()
    );
    assert_eq!(err, line);
    let (code, err) = env.run(&["link", "--target", "opencode"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(err.contains(&line), "{err}");
}
