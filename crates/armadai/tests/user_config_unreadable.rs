//! A `config.yaml` that exists but does not read is never silently replaced by defaults:
//! a write refuses and leaves it untouched, a read warns once.

use assert_cmd::Command;

const TRIGGERS: [(&str, &str); 3] = [
    ("duplicate key", "costs:\n  limit: 1\n  limit: 2\n"),
    ("non-finite float", "costs:\n  limit: .inf\n"),
    ("unknown tag", "costs:\n  key: !env ANTHROPIC_KEY\n"),
];

fn run(config_dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(config_dir)
        .env("ARMADAI_CONFIG_DIR", config_dir)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_write_refuses_an_unreadable_config_and_leaves_it_untouched() {
    for (what, trigger) in TRIGGERS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let text = format!("starters_dirs:\n  - /opt/mine\n{trigger}");
        std::fs::write(&path, &text).unwrap();
        let (ok, stderr) = run(dir.path(), &["config", "starters-dir", "add", "/opt/other"]);
        assert!(!ok, "{what}: the add must fail\n{stderr}");
        assert!(stderr.contains("config.yaml"), "{what}: {stderr}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{what}");
    }
}

#[test]
fn a_read_warns_once_about_an_unreadable_config() {
    for (what, trigger) in TRIGGERS {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.yaml"),
            format!("starters_dirs:\n  - /opt/mine\n{trigger}"),
        )
        .unwrap();
        let (ok, stderr) = run(dir.path(), &["config", "starters-dir", "list"]);
        assert!(ok, "{what}: a read still works\n{stderr}");
        let warnings: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("warning") && l.contains("config.yaml"))
            .collect();
        assert_eq!(warnings.len(), 1, "{what}: {stderr}");
    }
}

#[test]
fn an_empty_or_comment_only_config_is_the_default_without_warning() {
    for text in ["", "# nothing yet\n"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.yaml"), text).unwrap();
        let (ok, stderr) = run(dir.path(), &["config", "starters-dir", "add", "/opt/other"]);
        assert!(ok, "{text:?}: {stderr}");
        assert!(!stderr.contains("warning"), "{text:?}: {stderr}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config.yaml")).unwrap(),
            format!("{text}starters_dirs:\n- /opt/other\n")
        );
    }
}

#[test]
fn a_remove_refuses_an_unreadable_config_and_leaves_it_untouched() {
    for (what, trigger) in TRIGGERS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let text = format!("starters_dirs:\n  - /opt/mine\n{trigger}");
        std::fs::write(&path, &text).unwrap();
        let (ok, stderr) = run(
            dir.path(),
            &["config", "starters-dir", "remove", "/opt/mine"],
        );
        assert!(!ok, "{what}: the remove must fail\n{stderr}");
        assert!(stderr.contains("config.yaml"), "{what}: {stderr}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{what}");
    }
}

/// `registries.yaml` is only read: an unreadable one is named once, then ignored.
#[test]
fn an_unreadable_registries_yaml_is_named_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("registries.yaml"),
        "skills: []\nskills: []\n",
    )
    .unwrap();
    let (_, stderr) = run(dir.path(), &["skills", "search", "anything"]);
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("warning") && l.contains("registries.yaml"))
        .collect();
    assert_eq!(warnings.len(), 1, "{stderr}");
    assert!(warnings[0].contains("line 2"), "{stderr}");
}
