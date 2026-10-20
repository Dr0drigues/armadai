//! `config starters-dir add|remove` edits only the `starters_dirs` key of config.yaml:
//! other keys, comments and line endings are kept byte for byte (#476).

use assert_cmd::Command;
use std::path::Path;

const CONFIG: &str = "\
# ArmadAI user configuration\r
defaults:\r
  provider: anthropic   # kept\r
starters_dirs:   # mine\r
  - /opt/a   # first\r
\r
# costs below\r
costs:\r
  enabled: true\r
";

fn run(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(dir)
        .env("HOME", dir)
        .env("ARMADAI_CONFIG_DIR", dir.join("cfg"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("ARMADAI_STARTERS_DIRS")
        .args(["config", "starters-dir"])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn setup(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("cfg")).unwrap();
    let path = dir.path().join("cfg/config.yaml");
    std::fs::write(&path, text).unwrap();
    (dir, path)
}

#[test]
fn add_and_remove_keep_every_other_byte() {
    let (dir, path) = setup(CONFIG);
    let (ok, err) = run(dir.path(), &["add", "/opt/b"]);
    assert!(ok, "{err}");
    let added = CONFIG.replace(
        "  - /opt/a   # first\r\n",
        "  - /opt/a   # first\r\n  - /opt/b\r\n",
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), added);

    let (ok, err) = run(dir.path(), &["remove", "/opt/a"]);
    assert!(ok, "{err}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        CONFIG.replace("  - /opt/a   # first\r\n", "  - /opt/b\r\n")
    );

    let (ok, err) = run(dir.path(), &["remove", "/opt/b"]);
    assert!(ok, "{err}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        CONFIG.replace("starters_dirs:   # mine\r\n  - /opt/a   # first\r\n", "")
    );
}

#[cfg(unix)]
#[test]
fn a_duplicate_add_or_an_absent_remove_does_not_rewrite() {
    use std::os::unix::fs::MetadataExt;
    let (dir, path) = setup(CONFIG);
    let ino = std::fs::metadata(&path).unwrap().ino();
    for args in [["add", "/opt/a"], ["remove", "/opt/zzz"]] {
        let (ok, err) = run(dir.path(), &args);
        assert!(ok, "{args:?}: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), CONFIG, "{args:?}");
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), ino, "{args:?}");
    }
}

#[test]
fn an_absent_file_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, err) = run(dir.path(), &["add", "/opt/a"]);
    assert!(ok, "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("cfg/config.yaml")).unwrap(),
        "starters_dirs:\n- /opt/a\n"
    );
}

#[test]
fn a_style_that_cannot_be_edited_is_refused_untouched() {
    for (text, args, why) in [
        (
            "costs: 1\nstarters_dirs: [/opt/a] # note\n",
            ["add", "/opt/b"],
            "line 2",
        ),
        ("{a: 1}\n", ["add", "/opt/b"], "would not read back"),
        (
            "{starters_dirs: [/opt/a]}\n",
            ["add", "/opt/b"],
            "would not read back",
        ),
        (
            "\u{feff}starters_dirs:\n- /opt/a\n",
            ["add", "/opt/b"],
            "byte order mark",
        ),
        (
            "starters_dirs:\n- |-\n    #x\n- /opt/b\n",
            ["remove", "/opt/b"],
            "one line",
        ),
    ] {
        let (dir, path) = setup(text);
        let (ok, err) = run(dir.path(), &args);
        assert!(!ok, "{text:?}: must fail");
        assert!(
            err.contains("config.yaml") && err.contains("by hand") && err.contains(why),
            "{text:?}: {err}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}

#[cfg(unix)]
#[test]
fn a_read_only_file_is_refused_untouched() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, path) = setup(CONFIG);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let (ok, err) = run(dir.path(), &["add", "/opt/b"]);
    assert!(!ok, "the add must fail");
    assert!(
        err.contains("read-only") && err.contains("chmod u+w"),
        "{err}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CONFIG);
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_stays_one_and_its_target_is_created() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("cfg")).unwrap();
    std::fs::create_dir_all(dir.path().join("dots")).unwrap();
    let link = dir.path().join("cfg/config.yaml");
    // a relative chain: config.yaml -> ../dots/hop.yaml -> real.yaml (missing)
    std::os::unix::fs::symlink("../dots/hop.yaml", &link).unwrap();
    std::os::unix::fs::symlink("real.yaml", dir.path().join("dots/hop.yaml")).unwrap();

    let (ok, err) = run(dir.path(), &["add", "/opt/a"]);
    assert!(ok, "{err}");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("dots/real.yaml")).unwrap(),
        "starters_dirs:\n- /opt/a\n"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_stays_one_and_permissions_are_kept() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("cfg")).unwrap();
    let target = dir.path().join("real.yaml");
    std::fs::write(&target, "costs: 1\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.path().join("cfg/config.yaml");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let (ok, err) = run(dir.path(), &["add", "/opt/a"]);
    assert!(ok, "{err}");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "costs: 1\nstarters_dirs:\n- /opt/a\n"
    );
    let mode = std::fs::metadata(&target).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}
