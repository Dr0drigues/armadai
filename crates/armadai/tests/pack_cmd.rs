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
        self.run_in(&self.root(), args, envs)
    }

    fn run_in(&self, cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> Run {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(cwd)
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
        "  warn: tag demo-v1.0.0 of pack demo no longer exists, .armadai/pack.lock keeps {}",
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

const M_FAIL: &str = "Error: 1 source(s) or check(s) did not complete: see the warnings above";

#[test]
fn sync_continues_after_a_failing_source() {
    let env = Env::new();
    env.publish("1.0.0");
    let bad = format!("file://{}/absent", env.dir.path().display());
    env.registries(&[&bad, &env.url()]);
    let run = env.run(&["pack", "sync"], &[]);
    assert_eq!(run.code, Some(1), "{}{}", run.out, run.err);
    assert_eq!(
        run.out,
        format!("Synced {}: 1 pack(s), 1 version(s)\n", env.url())
    );
    let prefix = format!("  warn: failed to sync starter source {bad}: ");
    assert!(
        run.err.lines().any(|l| l.starts_with(&prefix)),
        "{}",
        run.err
    );
    assert_eq!(run.err.lines().last(), Some(M_FAIL), "{}", run.err);
}

/// The `git` found on the PATH now.
fn real_git() -> String {
    let path = std::env::var("PATH").unwrap();
    path.split(':')
        .map(|d| Path::new(d).join("git"))
        .find(|p| p.is_file())
        .unwrap()
        .display()
        .to_string()
}

/// A `git` in front of the PATH running `body`, with `$REAL` the real one.
fn fake_git(dir: &Path, body: &str) -> String {
    let bin = dir.join("gitbin");
    std::fs::create_dir_all(&bin).unwrap();
    let p = bin.join("git");
    std::fs::write(&p, format!("#!/bin/sh\nREAL='{}'\n{body}\n", real_git())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())
}

#[test]
fn sync_masks_the_secrets_of_a_source() {
    let env = Env::new();
    let c1 = env.publish("1.0.0");
    git(&env.origin(), &["tag", "demo-v1.0.0"]);
    let bad = "https://u:s3cret@example.com/nope.git?token=QSECRET";
    let good = "https://u:s3cret@example.com/p.git?token=QSECRET";
    env.registries(&[bad]);
    env.reference(good, "^1", &c1);
    // `good` is served by the local origin; `bad` fails, echoing its URL as git does
    let path = fake_git(
        env.dir.path(),
        r#"for a in "$@"; do
  case "$a" in
    *nope*) echo "fatal: unable to access '$a'" >&2; exit 128;;
  esac
done
for a in "$@"; do
  case "$a" in
    https://u:s3cret@*) a="$ORIGIN";;
  esac
  set -- "$@" "$a"; shift
done
exec "$REAL" "$@""#,
    );
    let origin = env.url();
    let run = env.run(&["pack", "sync"], &[("PATH", &path), ("ORIGIN", &origin)]);
    assert_eq!(run.code, Some(1), "{}{}", run.out, run.err);
    let shown = armadai_core::registries::redact_url(good);
    assert_eq!(
        run.out,
        format!("Synced {shown}: 1 pack(s), 1 version(s)\n")
    );
    let m2 = format!(
        "note: {shown} comes from .armadai/project.yaml (pack demo), not from your registries.yaml"
    );
    assert!(run.err_has(&m2), "{}", run.err);
    for secret in ["s3cret", "QSECRET"] {
        assert!(
            !run.out.contains(secret) && !run.err.contains(secret),
            "{}{}",
            run.out,
            run.err
        );
    }
    assert!(
        run.err
            .lines()
            .any(|l| l.starts_with("  warn: failed to sync starter source ")),
        "{}",
        run.err
    );
}

#[test]
fn the_source_is_never_read_as_a_git_option() {
    let env = Env::new();
    env.publish("1.0.0");
    env.registries(&[&env.url()]);
    let log = env.dir.path().join("argv");
    let path = fake_git(
        env.dir.path(),
        &format!("echo \"$*\" >> '{}'\nexec \"$REAL\" \"$@\"", log.display()),
    );
    let run = env.run(&["pack", "sync"], &[("PATH", &path)]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    let dest = env
        .config()
        .join("registry/starters")
        .join(armadai_core::registries::cache_key(&env.url()));
    let want = format!("clone --depth 1 -q -- {} {}", env.url(), dest.display());
    let argv = std::fs::read_to_string(&log).unwrap();
    assert!(argv.lines().any(|l| l == want), "{argv}");
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

fn status_env() -> Env {
    let env = Env::new();
    env.reference(URL, "^3", LOCK_C);
    env
}

#[test]
fn status_says_when_no_cached_version_matches_the_range() {
    let env = status_env();
    cached_versions(&env);
    let run = env.run(&["pack", "status"], &[]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    assert_eq!(
        run.out,
        "Pack demo (reference)\n  source:    https://example.com/packs.git\n  range:     ^3\n  locked:    1.0.0 (tag demo-v1.0.0, 111111111111)\n  in cache:  none matches the range (newest: 2.0.0)\n"
    );
}

#[test]
fn status_says_when_nothing_is_cached() {
    let env = status_env();
    let run = env.run(&["pack", "status"], &[]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    assert_eq!(
        run.out,
        "Pack demo (reference)\n  source:    https://example.com/packs.git\n  range:     ^3\n  locked:    1.0.0 (tag demo-v1.0.0, 111111111111)\n  in cache:  nothing (run `armadai pack sync`)\n"
    );
}

#[test]
fn status_without_a_lock_names_pack_update() {
    let env = status_env();
    std::fs::remove_file(env.root().join(".armadai/pack.lock")).unwrap();
    let run = env.run(&["pack", "status"], &[]);
    assert_eq!(run.code, Some(1), "{}{}", run.out, run.err);
    assert_eq!(
        run.err,
        "Error: .armadai/project.yaml references pack demo but .armadai/pack.lock is missing: run `armadai pack update`\n"
    );
}

#[test]
fn status_masks_the_secrets_of_the_source() {
    let env = Env::new();
    let url = "https://u:s3cret@example.com/p.git?token=QSECRET";
    env.reference(url, "^1", LOCK_C);
    let shown = armadai_core::registries::redact_url(url);
    let run = env.run(&["pack", "status"], &[]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    assert!(
        !run.out.contains("s3cret") && !run.out.contains("QSECRET"),
        "{}",
        run.out
    );
    assert_eq!(
        run.out.lines().nth(1),
        Some(format!("  source:    {shown}").as_str())
    );

    // a copy
    let env = Env::new();
    put(&env.root(), ".armadai/project.yaml", "schema_version: 1\n");
    put(
        &env.root(),
        ".armadai/link-manifest.yaml",
        &format!("version: 1\npacks:\n  demo:\n    source: '{url}'\n    version: 1.0.0\n"),
    );
    let run = env.run(&["pack", "status"], &[]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    assert_eq!(
        run.out,
        format!("Pack demo (copy)\n  source:    {shown}\n  installed: 1.0.0\n")
    );
}

#[test]
fn status_of_a_copy_without_a_version() {
    let env = Env::new();
    put(&env.root(), ".armadai/project.yaml", "schema_version: 1\n");
    put(
        &env.root(),
        ".armadai/link-manifest.yaml",
        &format!("version: 1\npacks:\n  demo:\n    source: '{URL}'\n"),
    );
    let run = env.run(&["pack", "status"], &[]);
    assert_eq!(run.code, Some(0), "{}{}", run.out, run.err);
    assert_eq!(
        run.out,
        "Pack demo (copy)\n  source:    https://example.com/packs.git\n  installed: (no version)\n"
    );
}

#[test]
fn sync_warns_on_an_unreadable_lock_and_on_an_unverifiable_tag() {
    let (env, c1, _) = locked();
    put(&env.root(), ".armadai/pack.lock", "format: [\n");
    let run = env.run(&["pack", "sync"], &[]);
    assert_eq!(run.code, Some(1), "{}{}", run.out, run.err);
    let line = run
        .err
        .lines()
        .find(|l| l.starts_with("  warn: .armadai/pack.lock: "));
    assert!(line.is_some_and(|l| l.len() > 28), "{}", run.err);
    assert_eq!(run.err.lines().last(), Some(M_FAIL));

    env.reference(&env.url(), "^1", &c1);
    let lock = std::fs::read_to_string(env.root().join(".armadai/pack.lock")).unwrap();
    put(
        &env.root(),
        ".armadai/pack.lock",
        &lock.replace("tag: demo-v1.0.0", "tag: ../x"),
    );
    let run = env.run(&["pack", "sync"], &[]);
    assert_eq!(run.code, Some(1), "{}{}", run.out, run.err);
    assert!(
        run.err_has("  warn: pack demo has an invalid name or version id: ../x"),
        "{}",
        run.err
    );
    assert_eq!(run.err.lines().last(), Some(M_FAIL));
}

#[test]
fn an_unreadable_versions_file_is_reported_once_when_the_source_fails() {
    let env = Env::new();
    let url = format!("file://{}/absent", env.dir.path().display());
    env.reference(&url, "^1", LOCK_C);
    let versions = env.versions_path(&url);
    put(versions.parent().unwrap(), "versions.yaml", "format: [\n");
    let run = env.run(&["pack", "sync"], &[]);
    let prefix = format!("  warn: {} is not readable: ", versions.display());
    let n = run.err.lines().filter(|l| l.starts_with(&prefix)).count();
    assert_eq!(n, 1, "{}", run.err);
}

#[test]
fn the_signal_survives_an_unrelated_failing_source() {
    let (env, _, _) = locked();
    let bad = format!("file://{}/nowhere", env.dir.path().display());
    env.registries(&[&bad, &env.url()]);
    let run = env.run(&["pack", "sync"], &[]);
    assert_eq!(run.code, Some(1), "{}{}", run.out, run.err);
    assert!(
        run.err_has(
            "note: pack demo 1.1.0 is in the cache (locked: 1.0.0): run `armadai pack update`"
        ),
        "{}",
        run.err
    );
}

#[test]
fn an_unreadable_versions_file_of_a_copy_is_reported_once_too() {
    let env = Env::new();
    let url = format!("file://{}/absent", env.dir.path().display());
    put(&env.root(), ".armadai/project.yaml", "schema_version: 1\n");
    put(
        &env.root(),
        ".armadai/link-manifest.yaml",
        &format!("version: 1\npacks:\n  demo:\n    source: '{url}'\n    version: 1.0.0\n"),
    );
    env.registries(&[&url]);
    let versions = env.versions_path(&url);
    put(versions.parent().unwrap(), "versions.yaml", "format: [\n");
    let run = env.run(&["pack", "sync"], &[]);
    let prefix = format!("  warn: {} is not readable: ", versions.display());
    let n = run.err.lines().filter(|l| l.starts_with(&prefix)).count();
    assert_eq!(n, 1, "{}", run.err);
}

// ---- pack add and pack update (reference mode) ----

impl Env {
    /// A valid `demo` at `version` in the origin (an agent, a skill), tagged `demo-v<version>`;
    /// `extra` files are written, `gone` ones removed. Returns the commit.
    fn release(&self, version: &str, extra: &[(&str, &str)], gone: &[&str]) -> String {
        let o = self.origin();
        put(&o, "demo/pack.yaml", "name: demo\n");
        put(
            &o,
            "demo/project.yaml",
            "schema_version: 1\ntargets: [claude]\n",
        );
        put(
            &o,
            "demo/agents/a.md",
            "---\ndescription: from the pack\n---\nA\n",
        );
        put(
            &o,
            "demo/skills/s/SKILL.md",
            "---\nname: s\ndescription: d\n---\nS\n",
        );
        put(&o, "demo/marker", version);
        put(&o, "armadai-packs.yaml", &index(version));
        for (rel, text) in extra {
            put(&o, &format!("demo/{rel}"), text);
        }
        for rel in gone {
            std::fs::remove_file(o.join("demo").join(rel)).unwrap();
        }
        git(&o, &["add", "-A"]);
        git(&o, &["commit", "-qm", version]);
        git(&o, &["tag", &format!("demo-v{version}")]);
        git(&o, &["rev-parse", "HEAD"])
    }

    /// A second project on its own directory.
    fn other_project(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::create_dir_all(p.join(".git")).unwrap();
        p
    }

    fn text(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root().join(rel)).unwrap()
    }
}

type Tree = std::collections::BTreeMap<String, Vec<u8>>;

/// Every file under `dir` (relative path -> bytes).
fn tree(dir: &Path) -> Tree {
    fn walk(base: &Path, dir: &Path, out: &mut Tree) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().display().to_string();
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = Tree::new();
    walk(dir, dir, &mut out);
    out
}

fn ok(r: &Run) {
    assert_eq!(r.code, Some(0), "{}{}", r.out, r.err);
}

fn released() -> (Env, String, String) {
    let env = Env::new();
    let c1 = env.release("1.0.0", &[], &[]);
    let c2 = env.release("1.1.0", &[], &[]);
    env.registries(&[&env.url()]);
    (env, c1, c2)
}

#[test]
fn add_writes_the_reference_and_the_lock() {
    let (env, _c1, c2) = released();
    let r = env.run(&["pack", "add", "demo@^1.0"], &[]);
    ok(&r);
    let url = env.url();
    assert_eq!(
        env.text(".armadai/project.yaml"),
        format!("schema_version: 1\npack:\n  name: demo\n  source: '{url}'\n  version: '^1.0'\n")
    );
    assert_eq!(
        env.text(".armadai/pack.lock"),
        format!(
            "format: 1\npack: demo\nsource: {url}\nresolved:\n  version: \"1.1.0\"\n  tag: demo-v1.1.0\n  commit: {c2}\n"
        )
    );
    assert_eq!(
        r.out.lines().last().unwrap(),
        format!(
            "Added pack demo 1.1.0 ({}): .armadai/project.yaml (version: ^1.0) and .armadai/pack.lock",
            id12(&c2)
        )
    );
    assert!(
        env.root().join(".claude/agents/a.md").is_file(),
        "{}",
        r.out
    );
    let referents = std::fs::read_to_string(env.config().join("pack-referents.yaml")).unwrap();
    assert!(referents.contains("pack: demo"), "{referents}");
}

#[test]
fn add_defaults_the_range_to_the_caret_of_the_resolved_version() {
    let (env, ..) = released();
    ok(&env.run(&["pack", "add", "demo"], &[]));
    assert!(
        env.text(".armadai/project.yaml")
            .ends_with("  version: '^1.1.0'\n")
    );
}

#[test]
fn add_keeps_the_comments_of_an_existing_project_yaml() {
    let (env, ..) = released();
    let before = "schema_version: 1\n# my own note\ntargets: [claude]";
    put(&env.root(), ".armadai/project.yaml", before);
    ok(&env.run(&["pack", "add", "demo"], &[]));
    assert_eq!(
        env.text(".armadai/project.yaml"),
        format!(
            "{before}\npack:\n  name: demo\n  source: '{}'\n  version: '^1.1.0'\n",
            env.url()
        )
    );
}

#[test]
fn add_refuses_a_second_pack_and_an_unknown_one() {
    let (env, ..) = released();
    ok(&env.run(&["pack", "add", "demo"], &[]));
    let again = env.run(&["pack", "add", "demo"], &[]);
    assert_ne!(again.code, Some(0));
    assert!(
        again.err_has(
            "Error: .armadai/project.yaml already references pack demo: use `armadai pack update`"
        ),
        "{}",
        again.err
    );
    let (env, ..) = released();
    let none = env.run(&["pack", "add", "nope"], &[]);
    assert_ne!(none.code, Some(0));
    assert!(none.err_has(
        "Error: pack nope is in none of your sources (registries.yaml): add its source there, or pass --source <url>"
    ), "{}", none.err);
    assert!(!env.root().join(".armadai/pack.lock").exists());
}

/// Two projects on `demo` 1.0.0.
fn two_projects() -> (Env, PathBuf) {
    let env = Env::new();
    env.release("1.0.0", &[], &[]);
    env.registries(&[&env.url()]);
    let second = env.other_project("project2");
    ok(&env.run(&["pack", "add", "demo"], &[]));
    ok(&env.run_in(&second, &["pack", "add", "demo"], &[]));
    (env, second)
}

#[test]
fn update_advances_the_lock_and_lists_other_projects() {
    let (env, second) = two_projects();
    let c3 = env.release("1.2.0", &[("marker2", "x")], &[]);
    let before = tree(&second);
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(
        env.text(".armadai/pack.lock")
            .contains("version: \"1.2.0\"\n")
    );
    assert!(
        env.text(".armadai/pack.lock")
            .contains(&format!("commit: {c3}"))
    );
    let lines: Vec<&str> = r.out.lines().collect();
    let n = lines.len();
    assert_eq!(
        lines[n - 3],
        format!(
            "Pack demo: 1.0.0 -> 1.2.0 ({}), .armadai/pack.lock updated",
            id12(&c3)
        )
    );
    assert_eq!(
        lines[n - 2],
        "Other projects referencing pack demo (not changed; run `armadai pack update` in each):"
    );
    let canon = std::fs::canonicalize(&second).unwrap();
    assert_eq!(lines[n - 1], format!("  {}", canon.display()));
    assert_eq!(tree(&second), before, "the other project is not touched");
}

#[test]
fn update_that_would_break_changes_nothing() {
    let (env, _second) = two_projects();
    put(
        &env.root(),
        ".armadai/agents/mine.md",
        "---\ndescription: mine\nskills: [s]\n---\nM\n",
    );
    env.release("1.2.0", &[], &["skills/s/SKILL.md"]);
    let before = tree(&env.root());
    let r = env.run(&["pack", "update"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(
        r.err_has("Error: pack demo 1.2.0 would make the project invalid; nothing was changed"),
        "{}",
        r.err
    );
    assert!(r.err.contains("error["), "{}", r.err);
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn update_never_touches_state() {
    let (env, _second) = two_projects();
    put(&env.root(), ".armadai/state/flow.json", "{\"a\":1}");
    put(&env.root(), ".armadai/state/sub/x", "x");
    env.release("1.2.0", &[], &[]);
    let before = tree(&env.root().join(".armadai/state"));
    ok(&env.run(&["pack", "update"], &[]));
    assert_eq!(tree(&env.root().join(".armadai/state")), before);
}

#[test]
fn update_with_an_unknown_version_names_it() {
    let (env, _second) = two_projects();
    let before = env.lock_bytes();
    let r = env.run(&["pack", "update", "9.9.9"], &[]);
    assert_ne!(r.code, Some(0));
    assert!(
        r.err_has("Error: no cached version of pack demo matches 9.9.9 (cached: 1.0.0): run `armadai pack sync`, or change pack.version in .armadai/project.yaml"),
        "{}",
        r.err
    );
    assert_eq!(env.lock_bytes(), before);
}

#[test]
fn update_with_nothing_new_says_so() {
    let (env, _second) = two_projects();
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    let lock = env.text(".armadai/pack.lock");
    let commit = lock
        .lines()
        .find_map(|l| l.trim().strip_prefix("commit: "))
        .unwrap();
    assert_eq!(
        r.out.lines().last().unwrap(),
        format!("Pack demo is up to date: 1.0.0 ({})", id12(commit))
    );
}

#[test]
fn an_overlay_never_reaches_the_other_project() {
    let (env, second) = two_projects();
    let snapshots = env.config().join("registry/packs");
    let before_snap = tree(&snapshots);
    let before_second = tree(&second);
    put(
        &env.root(),
        ".armadai/agents/a.md",
        "---\ndescription: overlaid\n---\nO\n",
    );
    ok(&env.run(&["link"], &[]));
    assert!(env.text(".claude/agents/a.md").contains("O"));
    assert_eq!(tree(&snapshots), before_snap);
    ok(&env.run_in(&second, &["link"], &[]));
    let agent = |t: &Tree| t[".claude/agents/a.md"].clone();
    assert_eq!(agent(&tree(&second)), agent(&before_second));
}
