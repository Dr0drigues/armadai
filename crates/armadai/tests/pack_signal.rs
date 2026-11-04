//! The "newer version in the cache" note (M4) on `list`, `inspect` and `link`: read from
//! `versions.yaml`, never from the network.

use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;

const URL: &str = "https://example.com/packs.git";
const C1: &str = "1111111111111111111111111111111111111111";
const M4: &str = "note: pack demo 1.1.0 is in the cache (locked: 1.0.0): run `armadai pack update`";

fn put(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    /// A project referencing `demo` at `range`, locked at 1.0.0, whose cache also holds
    /// 1.1.0.
    fn new(range: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let env = Env { dir };
        let root = env.root();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        put(
            &root,
            ".armadai/project.yaml",
            &format!(
                "schema_version: 1\npack: {{ name: demo, source: '{URL}', version: '{range}' }}\n"
            ),
        );
        put(
            &root,
            ".armadai/pack.lock",
            &format!(
                "format: 1\npack: demo\nsource: '{URL}'\nresolved:\n  version: 1.0.0\n  commit: '{C1}'\n"
            ),
        );
        let store = env
            .dir
            .path()
            .join("config/registry/packs")
            .join(armadai_core::registries::cache_key(URL));
        put(
            &store,
            &format!("demo/{C1}/agents/a.md"),
            "---\ndescription: d\n---\nbody\n",
        );
        put(
            &store,
            "versions.yaml",
            &format!(
                "format: 1\npacks:\n  demo:\n    path: demo\n    versions:\n      - {{ version: 1.0.0, tag: demo-v1.0.0, commit: '{C1}' }}\n      - {{ version: 1.1.0, tag: demo-v1.1.0, commit: '2222222222222222222222222222222222222222' }}\n"
            ),
        );
        env
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn run(&self, args: &[&str], path: Option<&str>) -> Output {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(self.root())
            .env("HOME", self.dir.path())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("NO_COLOR", "1")
            .args(args);
        if let Some(p) = path {
            cmd.env("PATH", p);
        }
        cmd.output().unwrap()
    }
}

fn err_has(o: &Output, line: &str) -> bool {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .any(|l| l == line)
}

const COMMANDS: [&[&str]; 3] = [
    &["list"],
    &["inspect", "a"],
    &["link", "--target", "claude"],
];

#[test]
fn the_three_surfaces_signal_a_newer_version() {
    let env = Env::new("^1");
    // git and curl that leave a witness and fail: the signal must not start them
    let bin = env.dir.path().join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let witness = env.dir.path().join("witness");
    for tool in ["git", "curl"] {
        let p = bin.join(tool);
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\necho {tool} >> '{}'\nexit 1\n",
                witness.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    for args in COMMANDS {
        let o = env.run(args, Some(&path));
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(
            err_has(&o, M4),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    assert!(!witness.exists(), "git or curl was started");
}

#[test]
fn no_signal_when_up_to_date_or_outside_the_range() {
    // 1.1.0 is in the cache but outside `~1.0`: the best of the range is the lock
    let env = Env::new("~1.0");
    for args in COMMANDS {
        let o = env.run(args, None);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(!err.contains("is in the cache"), "{args:?}: {err}");
    }
}

#[test]
fn link_json_stays_pure() {
    let env = Env::new("^1");
    let o = env.run(&["link", "--target", "claude", "--json"], None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(err_has(&o, M4), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}",
            String::from_utf8_lossy(&o.stdout)
        )
    });
}

impl Env {
    fn versions_path(&self) -> PathBuf {
        self.dir
            .path()
            .join("config/registry/packs")
            .join(armadai_core::registries::cache_key(URL))
            .join("versions.yaml")
    }
}

fn err_count(o: &Output, line: &str) -> usize {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| *l == line)
        .count()
}

#[test]
fn the_three_surfaces_signal_a_newer_version_of_a_copy() {
    let env = Env::new("^1");
    // a copy: no `pack:` key, the install is recorded in the link manifest
    put(&env.root(), ".armadai/project.yaml", "schema_version: 1\n");
    std::fs::remove_file(env.root().join(".armadai/pack.lock")).unwrap();
    put(
        &env.root(),
        ".armadai/agents/a.md",
        "---\ndescription: d\n---\nbody\n",
    );
    put(
        &env.root(),
        ".armadai/link-manifest.yaml",
        &format!("version: 1\npacks:\n  demo:\n    source: '{URL}'\n    version: 1.0.0\n"),
    );
    let note =
        "note: pack demo 1.1.0 is in the cache (installed: 1.0.0): run `armadai pack update`";
    for args in COMMANDS {
        let o = env.run(args, None);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert_eq!(
            err_count(&o, note),
            1,
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[test]
fn an_unreadable_versions_file_is_reported_once_per_command() {
    let env = Env::new("^1");
    std::fs::write(env.versions_path(), "format: [\n").unwrap();
    let prefix = format!(
        "  warn: {} is not readable: ",
        env.versions_path().display()
    );
    for args in COMMANDS {
        let o = env.run(args, None);
        let err = String::from_utf8_lossy(&o.stderr);
        let n = err.lines().filter(|l| l.starts_with(&prefix)).count();
        assert_eq!(n, 1, "{args:?}: {err}");
    }
}

#[test]
fn no_signal_when_the_cache_holds_only_the_locked_version() {
    let env = Env::new("^1");
    put(
        env.versions_path().parent().unwrap(),
        "versions.yaml",
        &format!(
            "format: 1\npacks:\n  demo:\n    path: demo\n    versions:\n      - {{ version: 1.0.0, tag: demo-v1.0.0, commit: '{C1}' }}\n"
        ),
    );
    for args in COMMANDS {
        let o = env.run(args, None);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(!err.contains("is in the cache"), "{args:?}: {err}");
    }
}
