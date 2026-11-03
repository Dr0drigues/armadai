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

    fn out_has(&self, line: &str) -> bool {
        self.out.lines().any(|l| l == line)
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

#[test]
fn add_without_a_matching_version_does_not_offer_to_edit_project_yaml() {
    let (env, ..) = released();
    let r = env.run(&["pack", "add", "demo@^9"], &[]);
    assert_ne!(r.code, Some(0));
    assert!(
        r.err_has("Error: no cached version of pack demo matches ^9 (cached: 1.0.0, 1.1.0): run `armadai pack sync`"),
        "{}",
        r.err
    );
    assert!(!env.root().join(".armadai").exists());
}

#[test]
fn update_outside_the_range_is_refused() {
    let (env, _second) = two_projects();
    env.release("2.0.0", &[], &[]);
    let before = tree(&env.root());
    let r = env.run(&["pack", "update", "2.0.0"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(
        r.err_has("Error: pack demo 2.0.0 is outside the range ^1.0.0 of .armadai/project.yaml; nothing was changed: change pack.version there to move to it"),
        "{}",
        r.err
    );
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn update_inside_the_range_by_exact_version_is_allowed() {
    let (env, _second) = two_projects();
    env.release("1.2.0", &[], &[]);
    env.release("1.3.0", &[], &[]);
    ok(&env.run(&["pack", "update", "1.2.0"], &[]));
    assert!(
        env.text(".armadai/pack.lock")
            .contains("version: \"1.2.0\"\n")
    );
}

/// The project already has an error the pack does not cause (`skills: [nope]`).
fn already_invalid() -> (Env, PathBuf) {
    let (env, second) = two_projects();
    put(
        &env.root(),
        ".armadai/agents/mine.md",
        "---\ndescription: mine\nskills: [nope]\n---\nM\n",
    );
    (env, second)
}

const ALREADY_INVALID: &str = "the project is already invalid: nothing was changed; fix the errors above, then run `armadai pack update` again";

#[test]
fn an_already_invalid_project_is_refused_without_blaming_the_new_version() {
    let (env, second) = already_invalid();
    env.release("1.2.0", &[], &[]);
    let before = tree(&env.root());
    let others = tree(&second);
    let referents = std::fs::read(env.config().join("pack-referents.yaml")).unwrap();
    let r = env.run(&["pack", "update"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(r.err_has(&format!("Error: {ALREADY_INVALID}")), "{}", r.err);
    assert!(
        !r.err.contains("would make the project invalid"),
        "{}",
        r.err
    );
    assert!(
        r.err.contains("error["),
        "its diagnostics are shown: {}",
        r.err
    );
    assert_eq!(tree(&env.root()), before, "lock and outputs untouched");
    assert_eq!(tree(&second), others);
    assert_eq!(
        std::fs::read(env.config().join("pack-referents.yaml")).unwrap(),
        referents
    );
}

#[test]
fn add_refuses_an_already_invalid_project_too() {
    let (env, ..) = released();
    put(
        &env.root(),
        ".armadai/project.yaml",
        "schema_version: 1\ntargets: [claude]\n",
    );
    put(
        &env.root(),
        ".armadai/agents/mine.md",
        "---\ndescription: mine\nskills: [nope]\n---\nM\n",
    );
    let before = tree(&env.root());
    let r = env.run(&["pack", "add", "demo"], &[]);
    assert_ne!(r.code, Some(0));
    assert!(
        r.err_has("Error: the project is already invalid: nothing was changed; fix the errors above, then run `armadai pack add demo` again"),
        "{}",
        r.err
    );
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn add_goes_on_after_a_failing_source() {
    let (env, _c1, c2) = released();
    let bad = format!("file://{}/missing", env.dir.path().display());
    env.registries(&[&bad, &env.url()]);
    let r = env.run(&["pack", "add", "demo"], &[]);
    ok(&r);
    let warn = format!("  warn: failed to sync starter source {bad}: ");
    assert_eq!(
        r.err.lines().filter(|l| l.starts_with(&warn)).count(),
        1,
        "{}",
        r.err
    );
    assert!(env.text(".armadai/pack.lock").contains(&c2));
}

fn archive_path(env: &Env) -> String {
    let src = env.dir.path().join("archive-src");
    put(&src, "armadai-packs.yaml", &index("1.0.0"));
    put(&src, "demo/pack.yaml", "name: demo\n");
    put(
        &src,
        "demo/project.yaml",
        "schema_version: 1\ntargets: [claude]\n",
    );
    put(&src, "demo/agents/a.md", "---\ndescription: d\n---\nA\n");
    let tgz = env.dir.path().join("packs.tar.gz");
    let made = Std::new("tar")
        .args([
            "czf",
            tgz.to_str().unwrap(),
            "-C",
            src.to_str().unwrap(),
            ".",
        ])
        .status()
        .unwrap();
    assert!(made.success());
    let bin = env.dir.path().join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/bin/sh\nwhile [ $# -gt 1 ]; do [ \"$1\" = -o ] && out=$2; shift; done\ncp '{}' \"$out\"\n",
            tgz.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())
}

#[test]
fn an_archive_source_shows_twelve_characters_of_the_digest() {
    let env = Env::new();
    let path = archive_path(&env);
    let url = "https://example.com/packs.tar.gz";
    env.registries(&[url]);

    let added = env.run(&["pack", "add", "demo"], &[("PATH", &path)]);
    ok(&added);
    let lock = env.text(".armadai/pack.lock");
    let hex = lock
        .lines()
        .find_map(|l| l.trim().strip_prefix("digest: sha256:"))
        .unwrap_or_else(|| panic!("no digest in {lock}"))
        .trim_matches('"')
        .to_string();
    let id = &hex[..12];
    assert_eq!(
        added.out.lines().last().unwrap(),
        format!(
            "Added pack demo 1.0.0 ({id}): .armadai/project.yaml (version: ^1.0.0) and .armadai/pack.lock"
        )
    );
    let again = env.run(&["pack", "update"], &[("PATH", &path)]);
    ok(&again);
    assert_eq!(
        again.out.lines().last().unwrap(),
        format!("Pack demo is up to date: 1.0.0 ({id})")
    );
    let status = env.run(&["pack", "status"], &[("PATH", &path)]);
    assert!(
        status
            .out
            .lines()
            .any(|l| l == format!("  locked:    1.0.0 ({id})")),
        "{}",
        status.out
    );
}

// ---- review of task 15 ----

#[test]
fn add_keeps_a_newline_terminated_project_yaml() {
    let (env, ..) = released();
    let before = "schema_version: 1\n# my own note\ntargets: [claude]\n";
    put(&env.root(), ".armadai/project.yaml", before);
    ok(&env.run(&["pack", "add", "demo"], &[]));
    assert_eq!(
        env.text(".armadai/project.yaml"),
        format!(
            "{before}pack:\n  name: demo\n  source: '{}'\n  version: '^1.1.0'\n",
            env.url()
        )
    );
}

#[test]
fn update_relinks_the_outputs_on_the_new_version() {
    let (env, _second) = two_projects();
    env.release(
        "1.2.0",
        &[("agents/a.md", "---\ndescription: from the pack\n---\nNEW\n")],
        &[],
    );
    ok(&env.run(&["pack", "update"], &[]));
    assert_eq!(
        env.text(".claude/agents/a.md"),
        "---\nname: a\ndescription: from the pack\n---\nNEW\n"
    );
}

#[test]
fn add_names_the_pack_the_project_already_references() {
    let (env, ..) = released();
    ok(&env.run(&["pack", "add", "demo"], &[]));
    let r = env.run(&["pack", "add", "other"], &[]);
    assert_ne!(r.code, Some(0));
    assert!(
        r.err_has(
            "Error: .armadai/project.yaml already references pack demo: use `armadai pack update`"
        ),
        "{}",
        r.err
    );
}

const SECRET_URL: &str = "https://user:S3CRETTOK@example.com/packs.tar.gz?sig=S3CRETSIG";
const MASKED_URL: &str = "https://***@example.com/packs.tar.gz?***";

fn secret_in(dir: &Path) -> Vec<String> {
    tree(dir)
        .into_iter()
        .filter(|(_, b)| String::from_utf8_lossy(b).contains("S3CRET"))
        .map(|(p, _)| p)
        .collect()
}

#[test]
fn a_source_with_credentials_is_written_masked_and_still_found() {
    let env = Env::new();
    let path = archive_path(&env);
    env.registries(&[SECRET_URL]);
    let envs = [("PATH", path.as_str())];
    let added = env.run(&["pack", "add", "demo"], &envs);
    ok(&added);
    assert_eq!(
        env.text(".armadai/project.yaml"),
        format!(
            "schema_version: 1\npack:\n  name: demo\n  source: '{MASKED_URL}'\n  version: '^1.0.0'\n"
        )
    );
    assert!(
        env.text(".armadai/pack.lock")
            .contains(&format!("source: {MASKED_URL}\n"))
    );
    assert_eq!(
        secret_in(&env.root().join(".armadai")),
        Vec::<String>::new()
    );
    assert_eq!(
        secret_in(&env.config()).len(),
        1,
        "only registries.yaml keeps it"
    );
    // the masked form is matched to the user's entry to sync, link and show
    ok(&env.run(&["pack", "update"], &envs));
    ok(&env.run(&["link"], &envs));
    let status = env.run(&["pack", "status"], &envs);
    assert!(
        status
            .out
            .lines()
            .any(|l| l == "  in cache:  1.0.0 matches the range (newest: 1.0.0)"),
        "{}",
        status.out
    );
    assert_eq!(
        secret_in(&env.root().join(".armadai")),
        Vec::<String>::new()
    );
    for r in [&added, &status] {
        assert!(!format!("{}{}", r.out, r.err).contains("S3CRET"));
    }
}

#[test]
fn a_masked_source_that_matches_no_entry_or_several_says_so() {
    let env = Env::new();
    let path = archive_path(&env);
    env.registries(&[SECRET_URL]);
    let envs = [("PATH", path.as_str())];
    ok(&env.run(&["pack", "add", "demo"], &envs));
    let none = format!(
        "Error: the source {MASKED_URL} matches none of your sources (registries.yaml): add it there, with its credentials"
    );
    env.registries(&[]);
    for args in [&["link"][..], &["pack", "update"][..]] {
        let r = env.run(args, &envs);
        assert_ne!(r.code, Some(0), "{args:?}");
        assert!(r.err_has(&none), "{args:?}: {}", r.err);
    }
    let sync = env.run(&["pack", "sync"], &envs);
    assert_ne!(sync.code, Some(0), "{}{}", sync.out, sync.err);
    assert!(
        sync.err_has(&format!("  warn: {}", none.trim_start_matches("Error: "))),
        "{}",
        sync.err
    );
    let other = "https://user:OTHERTOK@example.com/packs.tar.gz?sig=OTHERSIG";
    env.registries(&[SECRET_URL, other]);
    let several = format!(
        "Error: the source {MASKED_URL} matches 2 entries of your registries.yaml, which differ only by their credentials: keep one"
    );
    for args in [&["link"][..], &["pack", "update"][..]] {
        let r = env.run(args, &envs);
        assert_ne!(r.code, Some(0), "{args:?}");
        assert!(r.err_has(&several), "{args:?}: {}", r.err);
    }
}

fn seed_versions(env: &Env, url: &str, body: &str) {
    let p = env.versions_path(url);
    put(p.parent().unwrap(), "versions.yaml", body);
}

#[test]
fn add_in_several_sources_lists_them_masked() {
    let env = Env::new();
    let (a, b) = (
        "https://u:TOK1@example.com/a.git",
        "https://u:TOK2@example.com/b.git",
    );
    let body = "format: 1\npacks:\n  demo:\n    path: demo\n    versions:\n      - { version: 1.0.0, tag: demo-v1.0.0, commit: '1111111111111111111111111111111111111111' }\n";
    seed_versions(&env, a, body);
    seed_versions(&env, b, body);
    env.registries(&[a, b]);
    let (path, _) = fake_tools(env.dir.path());
    let r = env.run(&["pack", "add", "demo"], &[("PATH", &path)]);
    assert_ne!(r.code, Some(0));
    assert!(
        r.err_has("Error: pack demo is in several of your sources (https://***@example.com/a.git, https://***@example.com/b.git): pass --source <url>"),
        "{}",
        r.err
    );
    assert!(!r.err.contains("TOK"), "{}", r.err);
}

#[test]
fn add_with_nothing_cached_says_none() {
    let env = Env::new();
    let url = "https://example.com/empty.git";
    seed_versions(
        &env,
        url,
        "format: 1\npacks:\n  demo:\n    path: demo\n    versions: []\n",
    );
    env.registries(&[url]);
    let (path, _) = fake_tools(env.dir.path());
    let r = env.run(&["pack", "add", "demo"], &[("PATH", &path)]);
    assert_ne!(r.code, Some(0));
    assert!(
        r.err_has("Error: no cached version of pack demo matches latest (cached: none): run `armadai pack sync`"),
        "{}",
        r.err
    );
}

#[test]
fn update_without_a_lock_says_none_as_the_old_version() {
    let (env, ..) = released();
    ok(&env.run(&["pack", "add", "demo"], &[]));
    std::fs::remove_file(env.root().join(".armadai/pack.lock")).unwrap();
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    let lock = env.text(".armadai/pack.lock");
    let commit = lock
        .lines()
        .find_map(|l| l.trim().strip_prefix("commit: "))
        .unwrap();
    assert_eq!(
        r.out
            .lines()
            .filter(|l| l.starts_with("Pack demo"))
            .collect::<Vec<_>>(),
        [format!(
            "Pack demo: (none) -> 1.1.0 ({}), .armadai/pack.lock updated",
            id12(commit)
        )]
    );
}

#[test]
fn update_of_a_project_without_a_pack_says_how_to_get_one() {
    let env = Env::new();
    put(&env.root(), ".armadai/project.yaml", "schema_version: 1\n");
    let r = env.run(&["pack", "update"], &[]);
    assert_ne!(r.code, Some(0));
    assert!(
        r.err_has("Error: this project references no pack and records no installed pack: use `armadai pack add <name>` or `armadai init --pack <name>`"),
        "{}",
        r.err
    );
}

#[test]
fn update_lists_a_missing_and_a_departed_referent() {
    let (env, second) = two_projects();
    let third = env.other_project("project3");
    ok(&env.run_in(&third, &["pack", "add", "demo"], &[]));
    let second_canon = std::fs::canonicalize(&second).unwrap();
    let third_canon = std::fs::canonicalize(&third).unwrap();
    std::fs::remove_dir_all(&second).unwrap();
    put(&third, ".armadai/project.yaml", "schema_version: 1\n");
    env.release("1.2.0", &[], &[]);
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    let lines: Vec<&str> = r.out.lines().collect();
    let n = lines.len();
    assert_eq!(
        lines[n - 2..],
        [
            format!("  {} (missing)", second_canon.display()),
            format!("  {} (no longer references demo)", third_canon.display()),
        ]
    );
}

#[test]
fn add_in_a_legacy_project_is_refused_and_writes_nothing() {
    let (env, ..) = released();
    put(&env.root(), "armadai.yaml", "agents: []\n");
    let r = env.run(&["pack", "add", "demo"], &[]);
    assert_ne!(r.code, Some(0));
    assert!(
        r.err.lines().any(|l| l.starts_with("Error: ")
            && l.ends_with(" uses the legacy format, run `armadai migrate`")),
        "{}",
        r.err
    );
    assert!(!env.root().join(".armadai").exists());
}

const NOT_APPLIED: &str = "pack demo 1.2.0 was not applied: link failed, so .armadai/pack.lock and the referent record are back as they were";

#[test]
fn a_failing_link_puts_the_old_lock_back() {
    let (env, _second) = two_projects();
    env.release(
        "1.2.0",
        &[("agents/a.md", "---\ndescription: from the pack\n---\nNEW\n")],
        &[],
    );
    let out = env.root().join(".claude/agents/a.md");
    std::fs::remove_file(&out).unwrap();
    std::fs::create_dir(&out).unwrap();
    let before = tree(&env.root().join(".armadai"));
    // as after a clone: the project is on the pack, the record does not know it yet
    std::fs::remove_file(env.config().join("pack-referents.yaml")).unwrap();
    let r = env.run(&["pack", "update"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(r.err_has(NOT_APPLIED), "{}", r.err);
    assert!(!r.out.contains(".armadai/pack.lock updated"), "{}", r.out);
    assert_eq!(tree(&env.root().join(".armadai")), before);
    assert!(!env.config().join("pack-referents.yaml").exists());
}

#[test]
fn a_failing_link_undoes_an_add() {
    let (env, ..) = released();
    let out = env.root().join(".claude/agents/a.md");
    std::fs::create_dir_all(&out).unwrap();
    let r = env.run(&["pack", "add", "demo"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(
        r.err_has("pack demo 1.1.0 was not applied: link failed, so .armadai/pack.lock and the referent record are back as they were"),
        "{}",
        r.err
    );
    for f in [".armadai/project.yaml", ".armadai/pack.lock"] {
        assert!(!env.root().join(f).exists(), "{f}");
    }
    assert!(!env.config().join("pack-referents.yaml").exists());
}

#[test]
fn an_ssh_source_has_no_secret_and_is_written_as_it_is() {
    const SSH: &str = "git@example.com:org/packs.git";
    let (env, ..) = released();
    env.registries(&[SSH]);
    // the clone of the ssh URL reads the local origin instead
    let body = format!(
        "n=$#; i=0\nwhile [ $i -lt $n ]; do a=$1; shift; [ \"$a\" = '{SSH}' ] && a='{}'; set -- \"$@\" \"$a\"; i=$((i+1)); done\nexec \"$REAL\" \"$@\"",
        env.url()
    );
    let path = fake_git(env.dir.path(), &body);
    let envs = [("PATH", path.as_str())];
    ok(&env.run(&["pack", "add", "demo"], &envs));
    assert_eq!(
        env.text(".armadai/project.yaml"),
        format!("schema_version: 1\npack:\n  name: demo\n  source: '{SSH}'\n  version: '^1.1.0'\n")
    );
    assert!(
        env.text(".armadai/pack.lock")
            .contains(&format!("source: {SSH}\n"))
    );
    // not in registries.yaml any more: the project's own source is still synced, and read
    env.registries(&[]);
    let sync = env.run(&["pack", "sync"], &envs);
    ok(&sync);
    assert!(!sync.err.contains("matches none"), "{}", sync.err);
    // a source with no secret is shown as it is written
    assert_eq!(sync.out, format!("Synced {SSH}: 1 pack(s), 2 version(s)\n"));
    assert!(
        sync.err_has(&format!(
            "note: {SSH} comes from .armadai/project.yaml (pack demo), not from your registries.yaml"
        )),
        "{}",
        sync.err
    );
    let status = env.run(&["pack", "status"], &envs);
    assert!(
        status
            .out
            .lines()
            .any(|l| l == format!("  source:    {SSH}")),
        "{}",
        status.out
    );
    ok(&env.run(&["link"], &envs));
}

// --- copy mode: `init --pack` records its origin, `pack update` replaces what is unchanged ---

const NATIVE: &str = "name: demo\ndescription: d\nformat: 1\n";

impl Env {
    /// `demo` at `version` as a native pack of the origin (`format: 1`).
    fn native(&self, version: &str, extra: &[(&str, &str)], gone: &[&str]) -> String {
        let mut all = vec![("pack.yaml", NATIVE)];
        all.extend_from_slice(extra);
        self.release(version, &all, gone)
    }

    /// A project that installed `demo` 1.0.0 by copy, from the origin.
    fn copied(&self, extra: &[(&str, &str)]) {
        self.native("1.0.0", extra, &[]);
        self.registries(&[&self.url()]);
        ok(&self.run(&["init", "--pack", "demo", "--no-link"], &[]));
    }
}

fn agent(body: &str) -> String {
    format!("---\ndescription: d\n---\n{body}\n")
}

fn sha(bytes: &[u8]) -> String {
    use sha2::Digest;
    let hex: String = sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256:{hex}")
}

fn record(env: &Env) -> serde_json::Value {
    armadai_model::yaml::from_str_manifest(&env.text(".armadai/link-manifest.yaml")).unwrap()
}

#[test]
fn update_replaces_unchanged_and_keeps_edited() {
    let env = Env::new();
    env.copied(&[]);
    let mine = agent("MINE");
    put(&env.root(), ".armadai/agents/a.md", &mine);
    let c2 = env.native(
        "1.1.0",
        &[
            ("agents/a.md", &agent("A2")),
            ("agents/b.md", &agent("B")),
            (
                "skills/s/SKILL.md",
                "---\nname: s\ndescription: d\n---\nS2\n",
            ),
        ],
        &[],
    );
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(
        r.out_has("  updated .armadai/skills/s/SKILL.md"),
        "{}",
        r.out
    );
    assert!(r.out_has("  added .armadai/agents/b.md"), "{}", r.out);
    assert!(
        r.err_has("  warn: .armadai/agents/a.md was edited since the pack installed it: kept, the pack's new version is not applied"),
        "{}",
        r.err
    );
    assert_eq!(env.text(".armadai/agents/a.md"), mine);
    assert_eq!(env.text(".armadai/agents/b.md"), agent("B"));
    assert_eq!(
        env.text(".armadai/skills/s/SKILL.md"),
        "---\nname: s\ndescription: d\n---\nS2\n"
    );
    let summary: Vec<&str> = r
        .out
        .lines()
        .filter(|l| l.starts_with("Pack demo"))
        .collect();
    assert_eq!(
        summary,
        [format!("Pack demo: 1.0.0 -> 1.1.0 ({})", id12(&c2))]
    );
    // the edited file stays "edited" for the next update, the others are the pack's now
    let rec = record(&env);
    let files = &rec["packs"]["demo"]["files"];
    assert_eq!(files["agents/b.md"], sha(agent("B").as_bytes()));
    assert_ne!(files["agents/a.md"], sha(mine.as_bytes()));
    assert_ne!(files["agents/a.md"], sha(agent("A2").as_bytes()));
    assert_eq!(rec["packs"]["demo"]["version"], "1.1.0");
}

#[test]
fn update_merges_project_yaml() {
    let env = Env::new();
    env.copied(&[(
        "project.yaml",
        "schema_version: 1\ntargets: [claude]\npolicy:\n  free_agents: [Explore]\n",
    )]);
    put(
        &env.root(),
        ".armadai/project.yaml",
        "schema_version: 1\ntargets: [claude]\npolicy:\n  free_agents: [Explore, Plan]\n",
    );
    env.native(
        "1.1.0",
        &[(
            "project.yaml",
            "schema_version: 1\ntargets: [claude]\npolicy:\n  free_agents: [Explore, general-purpose]\n",
        )],
        &[],
    );
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(
        r.out_has("  merged .armadai/project.yaml (your keys kept, the pack's added)"),
        "{}",
        r.out
    );
    let merged =
        armadai_model::yaml::from_str_manifest(&env.text(".armadai/project.yaml")).unwrap();
    assert_eq!(
        merged["policy"]["free_agents"],
        serde_json::json!(["Explore", "general-purpose", "Plan"])
    );
}

#[test]
fn update_replaces_an_unchanged_project_yaml() {
    let env = Env::new();
    env.copied(&[]);
    let theirs = "schema_version: 1\ntargets: [claude, opencode]\n";
    env.native("1.1.0", &[("project.yaml", theirs)], &[]);
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(r.out_has("  updated .armadai/project.yaml"), "{}", r.out);
    assert_eq!(env.text(".armadai/project.yaml"), theirs);
}

#[test]
fn update_after_a_fresh_clone_refuses() {
    let env = Env::new();
    env.copied(&[]);
    env.native("1.1.0", &[("agents/b.md", &agent("B"))], &[]);
    std::fs::remove_file(env.root().join(".armadai/link-manifest.yaml")).unwrap();
    let before = tree(&env.root());
    let r = env.run(&["pack", "update"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(
        r.err_has("Error: the origin of this project's pack is unknown here (.armadai/link-manifest.yaml has no packs: section, as after a fresh clone): pack update guesses nothing, nothing was changed"),
        "{}",
        r.err
    );
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn update_removes_a_dropped_file_and_its_output() {
    let env = Env::new();
    env.native(
        "1.0.0",
        &[
            ("agents/old.md", &agent("OLD")),
            (
                "skills/gone/SKILL.md",
                "---\nname: gone\ndescription: d\n---\nG\n",
            ),
        ],
        &[],
    );
    env.registries(&[&env.url()]);
    ok(&env.run(&["init", "--pack", "demo", "--link"], &[]));
    assert!(env.root().join(".claude/agents/old.md").is_file());
    env.native("1.1.0", &[], &["agents/old.md", "skills/gone/SKILL.md"]);
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(
        !env.root().join(".armadai/skills/gone").exists(),
        "an emptied folder is removed"
    );
    assert!(
        r.out_has("  removed .armadai/agents/old.md (no longer in the pack)"),
        "{}",
        r.out
    );
    assert!(!env.root().join(".armadai/agents/old.md").exists());
    assert!(
        !env.root().join(".claude/agents/old.md").exists(),
        "{}",
        r.out
    );
}

#[test]
fn update_says_what_it_keeps() {
    let env = Env::new();
    env.copied(&[("agents/old.md", &agent("OLD"))]);
    std::fs::remove_file(env.root().join(".armadai/agents/a.md")).unwrap();
    put(&env.root(), ".armadai/agents/old.md", &agent("MINE"));
    env.native(
        "1.1.0",
        &[("agents/a.md", &agent("A2"))],
        &["agents/old.md"],
    );
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(
        r.err_has("  warn: .armadai/agents/a.md was removed by hand: not added back"),
        "{}",
        r.err
    );
    assert!(
        r.err_has("  warn: .armadai/agents/old.md is no longer in the pack but was edited: kept"),
        "{}",
        r.err
    );
    assert!(!env.root().join(".armadai/agents/a.md").exists());
    assert_eq!(env.text(".armadai/agents/old.md"), agent("MINE"));
}

#[test]
fn update_twice_is_up_to_date() {
    let env = Env::new();
    env.copied(&[]);
    let c2 = env.native("1.1.0", &[], &[]);
    ok(&env.run(&["pack", "update"], &[]));
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(
        r.out_has(&format!("Pack demo is up to date: 1.1.0 ({})", id12(&c2))),
        "{}",
        r.out
    );
}

#[test]
fn a_copy_update_that_would_break_changes_nothing() {
    let env = Env::new();
    env.copied(&[]);
    put(
        &env.root(),
        ".armadai/agents/mine.md",
        "---\ndescription: mine\nskills: [s]\n---\nM\n",
    );
    env.native(
        "1.1.0",
        &[("agents/b.md", &agent("B"))],
        &["skills/s/SKILL.md"],
    );
    let before = tree(&env.root());
    let r = env.run(&["pack", "update"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(
        r.err_has("Error: pack demo 1.1.0 would make the project invalid; nothing was changed"),
        "{}",
        r.err
    );
    assert!(r.err.contains("error["), "{}", r.err);
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn a_copy_update_on_an_invalid_project_blames_nobody() {
    let env = Env::new();
    env.copied(&[]);
    put(
        &env.root(),
        ".armadai/agents/mine.md",
        "---\ndescription: mine\nskills: [nope]\n---\nM\n",
    );
    env.native("1.1.0", &[("agents/b.md", &agent("B"))], &[]);
    let before = tree(&env.root());
    let r = env.run(&["pack", "update"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(
        r.err_has("Error: the project is already invalid: nothing was changed; fix the errors above, then run `armadai pack update` again"),
        "{}",
        r.err
    );
    assert_eq!(tree(&env.root()), before);
}

#[test]
fn a_copy_update_never_touches_state() {
    let env = Env::new();
    env.copied(&[]);
    put(&env.root(), ".armadai/state/flow.json", "{\"a\":1}");
    put(&env.root(), ".armadai/state/sub/x", "x");
    env.native("1.1.0", &[("agents/b.md", &agent("B"))], &[]);
    let before = tree(&env.root().join(".armadai/state"));
    ok(&env.run(&["pack", "update"], &[]));
    assert_eq!(tree(&env.root().join(".armadai/state")), before);
}

#[test]
fn init_pack_records_its_origin() {
    let env = Env::new();
    env.copied(&[]);
    let rec = record(&env);
    let demo = &rec["packs"]["demo"];
    assert_eq!(demo["source"], env.url());
    assert_eq!(demo["version"], "1.0.0");
    assert!(
        demo["id"].as_str().unwrap().starts_with("sha256:"),
        "{demo}"
    );
    assert_eq!(
        demo["files"]["agents/a.md"],
        sha(b"---\ndescription: from the pack\n---\nA\n")
    );
    assert_eq!(
        demo["files"]["project.yaml"],
        sha(b"schema_version: 1\ntargets: [claude]\n")
    );
    let n = demo["files"].as_object().unwrap().len();
    assert_eq!(n, 3, "{demo}"); // project.yaml, agents/a.md, skills/s/SKILL.md
}

#[test]
fn init_pack_from_a_folder_records_the_folder_and_not_a_kept_file() {
    let env = Env::new();
    let dir = env.dir.path().join("mypack");
    put(&dir, "pack.yaml", NATIVE);
    put(&dir, "agents/a.md", &agent("PACK-A"));
    put(&dir, "agents/b.md", &agent("PACK-B"));
    put(
        &env.root(),
        ".armadai/project.yaml",
        "schema_version: 1\ntargets: [claude]\n",
    );
    put(&env.root(), ".armadai/agents/a.md", &agent("MINE"));
    let r = env.run(
        &[
            "init",
            "--pack",
            dir.to_str().unwrap(),
            "--force",
            "--no-link",
        ],
        &[],
    );
    ok(&r);
    let rec = record(&env);
    let demo = &rec["packs"]["demo"];
    assert_eq!(demo["source"], dir.to_str().unwrap());
    assert!(demo.get("version").is_none(), "{demo}");
    let files = demo["files"].as_object().unwrap();
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["agents/b.md"]
    );
    assert_eq!(files["agents/b.md"], sha(agent("PACK-B").as_bytes()));
}

#[test]
fn a_pack_installed_from_a_folder_updates_from_the_folder() {
    let env = Env::new();
    let dir = env.dir.path().join("mypack");
    put(&dir, "pack.yaml", NATIVE);
    put(&dir, "agents/a.md", &agent("A1"));
    ok(&env.run(&["init", "--pack", dir.to_str().unwrap(), "--no-link"], &[]));
    put(&dir, "agents/a.md", &agent("A2"));
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(r.out_has("  updated .armadai/agents/a.md"), "{}", r.out);
    assert_eq!(env.text(".armadai/agents/a.md"), agent("A2"));
    assert!(
        r.out
            .lines()
            .any(|l| l.starts_with("Pack demo: (no version) -> (no version) (")),
        "{}",
        r.out
    );
    // nothing new: up to date, by content
    let again = env.run(&["pack", "update"], &[]);
    ok(&again);
    assert!(
        again
            .out
            .lines()
            .any(|l| l.starts_with("Pack demo is up to date: (no version) (")),
        "{}",
        again.out
    );
}

#[test]
fn the_origin_of_a_copy_survives_link_and_unlink() {
    let env = Env::new();
    env.native("1.0.0", &[], &[]);
    env.registries(&[&env.url()]);
    ok(&env.run(&["init", "--pack", "demo", "--link"], &[]));
    let rec = record(&env);
    assert!(rec["targets"]["claude"].is_object(), "{rec}");
    assert!(rec["packs"]["demo"].is_object(), "{rec}");
    ok(&env.run(&["unlink"], &[]));
    let after = record(&env);
    assert_eq!(after["packs"], rec["packs"]);
    assert!(
        after
            .get("targets")
            .is_none_or(|t| t.as_object().is_some_and(|o| o.is_empty())),
        "{after}"
    );
    let c2 = env.native("1.1.0", &[("agents/b.md", &agent("B"))], &[]);
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert!(
        r.out
            .lines()
            .any(|l| l == format!("Pack demo: 1.0.0 -> 1.1.0 ({})", id12(&c2))),
        "{}",
        r.out
    );
}

#[test]
fn a_merge_with_nothing_to_add_leaves_the_file_as_written() {
    let env = Env::new();
    env.copied(&[]);
    let mine = "# my team\nschema_version: 1\ntargets: [claude]\n";
    put(&env.root(), ".armadai/project.yaml", mine);
    env.native("1.1.0", &[("agents/b.md", &agent("B"))], &[]);
    let r = env.run(&["pack", "update"], &[]);
    ok(&r);
    assert_eq!(env.text(".armadai/project.yaml"), mine);
    assert!(!r.out.contains("merged"), "{}", r.out);
}

#[cfg(unix)]
#[test]
fn a_copy_update_never_writes_through_a_symbolic_link() {
    let env = Env::new();
    env.copied(&[]);
    let target = env.dir.path().join("elsewhere.md");
    std::fs::write(&target, agent("OUTSIDE")).unwrap();
    let link = env.root().join(".armadai/agents/a.md");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    env.native("1.1.0", &[("agents/a.md", &agent("A2"))], &[]);
    let r = env.run(&["pack", "update"], &[]);
    assert_ne!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(
        r.err_has(&format!(
            "Error: {} is a symbolic link; it is not written through",
            std::fs::canonicalize(env.root())
                .unwrap()
                .join(".armadai/agents/a.md")
                .display()
        )),
        "{}",
        r.err
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), agent("OUTSIDE"));
}
