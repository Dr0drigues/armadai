//! `armadai pack sync` and `armadai pack status`. Origins are local git repositories
//! (`file://`, so that `--depth 1` is honoured); no test reaches a network.

use std::path::{Path, PathBuf};
use std::process::Command as Std;

use assert_cmd::Command;

const URL: &str = "https://example.com/packs.git";
const LOCK_C: &str = "1111111111111111111111111111111111111111";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Std::new("git")
        .args(["-C", dir.to_str().unwrap()])
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn put(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn index(version: &str) -> String {
    format!("format: 1\npacks:\n  - name: demo\n    path: demo\n    version: {version}\n")
}

struct Run {
    code: Option<i32>,
    out: String,
    err: String,
}

impl Run {
    fn err_has(&self, line: &str) -> bool {
        self.err.lines().any(|l| l == line)
    }
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        for d in ["project/.git", "config", "origin"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        git(&dir.path().join("origin"), &["init", "-q"]);
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config")
    }

    fn origin(&self) -> PathBuf {
        self.dir.path().join("origin")
    }

    fn url(&self) -> String {
        format!("file://{}", self.origin().display())
    }

    /// A commit of `demo` in the origin with the index at `version`; returns the commit.
    fn publish(&self, version: &str) -> String {
        put(&self.origin(), "demo/pack.yaml", "name: demo\n");
        put(&self.origin(), "demo/marker", version);
        put(&self.origin(), "armadai-packs.yaml", &index(version));
        git(&self.origin(), &["add", "-A"]);
        git(&self.origin(), &["commit", "-qm", version]);
        git(&self.origin(), &["rev-parse", "HEAD"])
    }

    fn registries(&self, urls: &[&str]) {
        let body: String = urls.iter().map(|u| format!("  - url: '{u}'\n")).collect();
        put(
            &self.config(),
            "registries.yaml",
            &format!("starters:\n{body}"),
        );
    }

    /// The project references `demo` from `url`, locked at `commit` (tag `demo-v1.0.0`).
    fn reference(&self, url: &str, range: &str, commit: &str) {
        put(
            &self.root(),
            ".armadai/project.yaml",
            &format!(
                "schema_version: 1\npack: {{ name: demo, source: '{url}', version: '{range}' }}\n"
            ),
        );
        put(
            &self.root(),
            ".armadai/pack.lock",
            &format!(
                "format: 1\npack: demo\nsource: '{url}'\nresolved:\n  version: 1.0.0\n  tag: demo-v1.0.0\n  commit: '{commit}'\n"
            ),
        );
    }

    fn lock_bytes(&self) -> Vec<u8> {
        std::fs::read(self.root().join(".armadai/pack.lock")).unwrap()
    }

    fn run(&self, args: &[&str], envs: &[(&str, &str)]) -> Run {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(self.root())
            .env("ARMADAI_CONFIG_DIR", self.config())
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args);
        for k in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
            cmd.env_remove(k);
        }
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        Run {
            code: out.status.code(),
            out: String::from_utf8_lossy(&out.stdout).into_owned(),
            err: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    fn versions_path(&self, url: &str) -> PathBuf {
        self.config()
            .join("registry/packs")
            .join(armadai_core::registries::cache_key(url))
            .join("versions.yaml")
    }
}

fn id12(c: &str) -> &str {
    &c[..12]
}

#[test]
fn sync_refreshes_and_reports() {
    let env = Env::new();
    env.publish("1.0.0");
    git(&env.origin(), &["tag", "demo-v1.0.0"]);
    env.registries(&[&env.url()]);

    let first = env.run(&["pack", "sync"], &[]);
    assert_eq!(first.code, Some(0), "{}{}", first.out, first.err);
    assert_eq!(
        first.out,
        format!("Synced {}: 1 pack(s), 1 version(s)\n", env.url())
    );

    env.publish("1.1.0");
    let second = env.run(&["pack", "sync"], &[]);
    assert_eq!(second.code, Some(0), "{}{}", second.out, second.err);
    assert_eq!(
        second.out,
        format!("Synced {}: 1 pack(s), 2 version(s)\n", env.url())
    );
    let versions = std::fs::read_to_string(env.versions_path(&env.url())).unwrap();
    assert!(versions.contains("version: \"1.1.0\""), "{versions}");
}

/// A project locked on `demo-v1.0.0` (commit c1) whose origin also has `1.1.0`.
fn locked() -> (Env, String, String) {
    let env = Env::new();
    let c1 = env.publish("1.0.0");
    git(&env.origin(), &["tag", "demo-v1.0.0"]);
    let c2 = env.publish("1.1.0");
    git(&env.origin(), &["tag", "demo-v1.1.0"]);
    env.registries(&[&env.url()]);
    env.reference(&env.url(), "^1", &c1);
    (env, c1, c2)
}

#[test]
fn sync_signals_a_moved_tag() {
    let (env, c1, c2) = locked();
    let before = env.lock_bytes();
    let ok = env.run(&["pack", "sync"], &[]);
    assert_eq!(ok.code, Some(0), "{}{}", ok.out, ok.err);

    git(&env.origin(), &["tag", "-f", "demo-v1.0.0", &c2]);
    let moved = env.run(&["pack", "sync"], &[]);
    assert_eq!(moved.code, Some(0), "{}{}", moved.out, moved.err);
    let m3 = format!(
        "  warn: tag demo-v1.0.0 of pack demo now points to {}, .armadai/pack.lock says {}: the lock is kept (link uses its commit)",
        id12(&c2),
        id12(&c1)
    );
    assert!(moved.err_has(&m3), "{}", moved.err);
    assert_eq!(env.lock_bytes(), before);
}

#[test]
fn sync_signals_a_gone_tag() {
    let (env, c1, _) = locked();
    let before = env.lock_bytes();
    env.run(&["pack", "sync"], &[]);

    git(&env.origin(), &["tag", "-d", "demo-v1.0.0"]);
    let gone = env.run(&["pack", "sync"], &[]);
    assert_eq!(gone.code, Some(0), "{}{}", gone.out, gone.err);
    let m3b = format!(
        "warn: tag demo-v1.0.0 of pack demo no longer exists, .armadai/pack.lock keeps {}",
        id12(&c1)
    );
    assert!(gone.err_has(&m3b), "{}", gone.err);
    assert!(!gone.err.contains("now points to"), "{}", gone.err);
    assert_eq!(env.lock_bytes(), before);
}

#[test]
fn sync_signals_a_newer_cached_version_and_lays_the_locked_snapshot() {
    let (env, c1, _) = locked();
    let run = env.run(&["pack", "sync"], &[]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    assert!(
        run.err_has(
            "note: pack demo 1.1.0 is in the cache (locked: 1.0.0): run `armadai pack update`"
        ),
        "{}",
        run.err
    );
    let snap = env
        .config()
        .join("registry/packs")
        .join(armadai_core::registries::cache_key(&env.url()))
        .join("demo")
        .join(&c1);
    assert_eq!(
        std::fs::read_to_string(snap.join("marker")).unwrap(),
        "1.0.0"
    );
}

#[test]
fn sync_reports_an_unreadable_file() {
    let env = Env::new();
    env.publish("1.0.0");
    env.registries(&[&env.url()]);
    // an index that is not YAML, in a later commit
    put(&env.origin(), "armadai-packs.yaml", "format: [\n");
    git(&env.origin(), &["add", "-A"]);
    git(&env.origin(), &["commit", "-qm", "bad index"]);
    let run = env.run(&["pack", "sync"], &[]);
    let clone_index = env
        .config()
        .join("registry/starters")
        .join(armadai_core::registries::cache_key(&env.url()))
        .join("armadai-packs.yaml");
    let prefix = format!("  warn: {} is not readable: ", clone_index.display());
    let line = run.err.lines().find(|l| l.starts_with(&prefix));
    assert!(
        line.is_some_and(|l| l.len() > prefix.len()),
        "{}{}",
        run.out,
        run.err
    );

    // and a versions.yaml that is not YAML
    let env = Env::new();
    env.publish("1.0.0");
    env.registries(&[&env.url()]);
    env.run(&["pack", "sync"], &[]);
    let versions = env.versions_path(&env.url());
    std::fs::write(&versions, "format: [\n").unwrap();
    let run = env.run(&["pack", "sync"], &[]);
    let prefix = format!("  warn: {} is not readable: ", versions.display());
    let line = run.err.lines().find(|l| l.starts_with(&prefix));
    assert!(
        line.is_some_and(|l| l.len() > prefix.len()),
        "{}{}",
        run.out,
        run.err
    );
}

#[test]
fn sync_continues_after_a_failing_source() {
    let env = Env::new();
    env.publish("1.0.0");
    let bad = "https://user:s3cret@127.0.0.1:1/nope.git";
    env.registries(&[bad, &env.url()]);
    let run = env.run(&["pack", "sync"], &[]);
    assert_eq!(run.code, Some(1), "{}{}", run.out, run.err);
    assert_eq!(
        run.out,
        format!("Synced {}: 1 pack(s), 1 version(s)\n", env.url())
    );
    assert!(
        run.err
            .lines()
            .any(|l| l.starts_with("  warn: failed to sync starter source ")),
        "{}",
        run.err
    );
    assert!(!run.err.contains("s3cret"), "{}", run.err);
}

/// `git` and `curl` that leave a witness and fail.
fn fake_tools(dir: &Path) -> (String, PathBuf) {
    let bin = dir.join("fakebin");
    let witness = dir.join("witness");
    std::fs::create_dir_all(&bin).unwrap();
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
    (path, witness)
}

fn cached_versions(env: &Env) {
    let p = env.versions_path(URL);
    put(
        p.parent().unwrap(),
        "versions.yaml",
        "format: 1\npacks:\n  demo:\n    path: demo\n    versions:\n      - { version: 1.0.0, tag: demo-v1.0.0, commit: '1111111111111111111111111111111111111111' }\n      - { version: 1.1.0, tag: demo-v1.1.0, commit: '2222222222222222222222222222222222222222' }\n      - { version: 2.0.0, tag: demo-v2.0.0, commit: '3333333333333333333333333333333333333333' }\n",
    );
}

#[test]
fn status_and_the_signal_touch_no_network() {
    let env = Env::new();
    env.reference(URL, "^1", LOCK_C);
    cached_versions(&env);
    let (path, witness) = fake_tools(env.dir.path());
    let run = env.run(&["pack", "status"], &[("PATH", &path)]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    assert_eq!(
        run.out,
        "Pack demo (reference)\n  source:    https://example.com/packs.git\n  range:     ^1\n  locked:    1.0.0 (tag demo-v1.0.0, 111111111111)\n  in cache:  1.1.0 matches the range (newest: 2.0.0)\n"
    );
    assert!(!witness.exists(), "git or curl was started");
}

#[test]
fn status_lists_referents_and_flags_the_missing() {
    let env = Env::new();
    env.reference(URL, "^1", LOCK_C);
    cached_versions(&env);
    let kept = env.dir.path().join("kept");
    put(
        &kept,
        ".armadai/project.yaml",
        &format!("schema_version: 1\npack: {{ name: demo, source: '{URL}', version: latest }}\n"),
    );
    let gone = env.dir.path().join("gone");
    put(
        &env.config(),
        "pack-referents.yaml",
        &format!(
            "format: 1\nreferents:\n  - {{ pack: demo, source: '{URL}', project: '{}' }}\n  - {{ pack: demo, source: '{URL}', project: '{}' }}\n",
            kept.display(),
            gone.display()
        ),
    );
    let run = env.run(&["pack", "status"], &[]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    let tail = format!(
        "Projects referencing it:\n  {}\n  {} (missing)\n",
        kept.display(),
        gone.display()
    );
    assert!(run.out.ends_with(&tail), "{}", run.out);
}

#[test]
fn status_of_a_copy_and_of_a_project_without_pack() {
    let env = Env::new();
    put(&env.root(), ".armadai/project.yaml", "schema_version: 1\n");
    let none = env.run(&["pack", "status"], &[]);
    assert_eq!(none.code, Some(0), "{}{}", none.out, none.err);
    assert_eq!(
        none.out,
        "this project references no pack and records no installed pack: use `armadai pack add <name>` or `armadai init --pack <name>`\n"
    );

    put(
        &env.root(),
        ".armadai/link-manifest.yaml",
        &format!(
            "version: 1\npacks:\n  demo:\n    source: '{URL}'\n    version: 1.0.0\n    id: sha256:abcdef0123456789abcdef\n"
        ),
    );
    cached_versions(&env);
    let copy = env.run(&["pack", "status"], &[]);
    assert_eq!(copy.code, Some(0), "{}{}", copy.out, copy.err);
    assert_eq!(
        copy.out,
        "Pack demo (copy)\n  source:    https://example.com/packs.git\n  installed: 1.0.0 (abcdef012345)\n  in cache:  2.0.0\n"
    );
}

#[test]
fn sync_takes_the_projects_own_source_and_says_so() {
    let env = Env::new();
    let c1 = env.publish("1.0.0");
    git(&env.origin(), &["tag", "demo-v1.0.0"]);
    env.reference(&env.url(), "^1", &c1);
    let run = env.run(&["pack", "sync"], &[]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    assert_eq!(
        run.out,
        format!("Synced {}: 1 pack(s), 1 version(s)\n", env.url())
    );
    let m2 = format!(
        "note: {} comes from .armadai/project.yaml (pack demo), not from your registries.yaml",
        env.url()
    );
    assert!(run.err_has(&m2), "{}", run.err);
}
