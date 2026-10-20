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
        CONFIG.replace(
            "starters_dirs:   # mine\r\n  - /opt/a   # first\r\n",
            "starters_dirs: []   # mine\r\n"
        )
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
            "costs: 1\nstarters_dirs: &d\n- /opt/a\n",
            ["add", "/opt/b"],
            "line 2",
        ),
        (
            "{a: 1}\n",
            ["add", "/opt/b"],
            "did not find expected <document start>",
        ),
        (
            "{starters_dirs: [/opt/a]}\n",
            ["add", "/opt/b"],
            "did not find expected <document start>",
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

/// The rename replaces the file: what writing it in place could not do is refused.
#[cfg(unix)]
#[test]
fn a_file_the_user_cannot_write_is_refused_untouched() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o464, 0o444, 0o400] {
        let (dir, path) = setup(CONFIG);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let (ok, err) = run(dir.path(), &["add", "/opt/b"]);
        assert!(!ok, "{mode:o}: the add must fail");
        assert!(
            err.contains("not writable") && err.contains("chmod u+w"),
            "{mode:o}: {err}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), CONFIG, "{mode:o}");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn a_read_only_directory_is_refused_untouched() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, path) = setup(CONFIG);
    let cfg = dir.path().join("cfg");
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o555)).unwrap();
    let (ok, err) = run(dir.path(), &["add", "/opt/b"]);
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!ok, "the add must fail");
    assert!(err.contains("cannot write next to"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CONFIG);
}

#[cfg(unix)]
#[test]
fn a_hard_linked_file_is_refused_untouched() {
    let (dir, path) = setup(CONFIG);
    std::fs::hard_link(&path, dir.path().join("hard.yaml")).unwrap();
    let (ok, err) = run(dir.path(), &["add", "/opt/b"]);
    assert!(!ok, "the add must fail");
    assert!(
        err.contains("has 2 hard links") && err.contains("by hand"),
        "{err}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CONFIG);
}

/// Needs a second group of the current user; skipped without one.
#[cfg(unix)]
#[test]
fn the_group_of_the_file_is_kept() {
    use std::os::unix::fs::MetadataExt;
    let (dir, path) = setup(CONFIG);
    let ids = std::process::Command::new("id").arg("-G").output().unwrap();
    let own = std::fs::metadata(&path).unwrap().gid();
    let Some(other) = String::from_utf8_lossy(&ids.stdout)
        .split_whitespace()
        .filter_map(|g| g.parse::<u32>().ok())
        .find(|&g| g != own && std::os::unix::fs::chown(&path, None, Some(g)).is_ok())
    else {
        eprintln!("skipped: no second group to test with");
        return;
    };
    let (ok, err) = run(dir.path(), &["add", "/opt/b"]);
    assert!(ok, "{err}");
    assert_eq!(std::fs::metadata(&path).unwrap().gid(), other);
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

/// A `..` after a missing directory creates nothing outside the resolved target.
#[cfg(unix)]
#[test]
fn a_dangling_symlink_through_a_missing_directory_creates_only_the_target_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("cfg")).unwrap();
    std::os::unix::fs::symlink("sub/../../z/real.yaml", dir.path().join("cfg/config.yaml"))
        .unwrap();
    let (ok, err) = run(dir.path(), &["add", "/opt/a"]);
    assert!(ok, "{err}");
    assert!(!dir.path().join("cfg/sub").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("z/real.yaml")).unwrap(),
        "starters_dirs:\n- /opt/a\n"
    );
}
