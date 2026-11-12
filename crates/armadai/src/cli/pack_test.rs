//! `armadai pack test`: runs the scripts a pack lists under `tests:` in `pack.yaml`.
//!
//! The pack (the project's locked snapshot from the cache, or a folder) is copied into a
//! scratch folder, and each script runs there with `bash`, `HOME`, `CLAUDE_CONFIG_DIR` and
//! `ARMADAI_CONFIG_DIR` pointing at scratch folders. Nothing here touches the network, and
//! nothing runs it but this command. The scripts run with your permissions and see your
//! network: there is no isolation beyond the copy and the three variables.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use armadai_core::starter::StarterPack;

/// The longest one test may run.
pub const LIMIT: Duration = Duration::from_secs(60);

/// How many bytes of a test's output are kept (the rest is read and dropped).
const MAX_OUTPUT: u64 = 64 * 1024;

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Ok,
    /// The exit code (`None`: ended by a signal) and what the script printed.
    Failed(Option<i32>, String),
    TimedOut,
}

#[derive(Debug)]
pub struct TestRun {
    pub path: String,
    pub verdict: Verdict,
    pub took: Duration,
}

/// What a run gives: the pack's name and its results (none when it lists no test).
#[derive(Debug)]
pub struct Report {
    pub name: String,
    pub runs: Vec<TestRun>,
}

/// Run the tests of the pack at `dir`, each for at most `limit`.
pub fn run_pack(dir: &Path, limit: Duration) -> anyhow::Result<Report> {
    let pack = StarterPack::load(dir)?;
    if let Some(d) = crate::cli::native_pack::test_path_diagnostics(dir, &pack).first() {
        bail!("{}", d.render());
    }
    if pack.tests.is_empty() {
        return Ok(Report {
            name: pack.name,
            runs: Vec::new(),
        });
    }
    let scratch = tempfile::tempdir()?;
    let copy = scratch.path().join("pack");
    copy_tree(dir, &copy)?;
    let iso = scratch.path().join("iso");
    let [home, claude, config] = ["home", "claude", "armadai"].map(|d| iso.join(d));
    for d in [&home, &claude, &config] {
        std::fs::create_dir_all(d)?;
    }
    let runs = (pack.tests.iter())
        .map(|t| run_one(&copy, t, [&home, &claude, &config], limit))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Report {
        name: pack.name,
        runs,
    })
}

/// Copy `from` to `to` (created), refusing any symbolic link: a link in the pack could
/// point a test, or the copy, outside of it.
fn copy_tree(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let e = e?;
        let (src, dest) = (e.path(), to.join(e.file_name()));
        let meta = std::fs::symlink_metadata(&src)?;
        if meta.file_type().is_symlink() {
            bail!("the pack holds a symbolic link, refused: {}", src.display());
        } else if meta.is_dir() {
            copy_tree(&src, &dest)?;
        } else if meta.is_file() {
            std::fs::copy(&src, &dest)?;
        }
    }
    Ok(())
}

fn run_one(
    copy: &Path,
    test: &str,
    [home, claude, config]: [&PathBuf; 3],
    limit: Duration,
) -> anyhow::Result<TestRun> {
    let mut cmd = Command::new("bash");
    // `--`: a script named like an option is still a script
    cmd.arg("--")
        .arg(test)
        .current_dir(copy)
        .env("HOME", home)
        .env("CLAUDE_CONFIG_DIR", claude)
        .env("ARMADAI_CONFIG_DIR", config)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let start = Instant::now();
    let mut child = cmd.spawn().context("running bash")?;
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|o| Box::new(o) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|o| Box::new(o) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|mut o| {
        std::thread::spawn(move || {
            let mut raw = Vec::new();
            let _ = (&mut o).take(MAX_OUTPUT).read_to_end(&mut raw);
            // keep draining: a chatty script must not block on a full pipe
            let _ = std::io::copy(&mut o, &mut std::io::sink());
            raw
        })
    })
    .collect();
    let exited = loop {
        if crate::claude_adapter::statusline_chain::exited(&mut child) {
            break true;
        }
        if start.elapsed() >= limit {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // the whole group, also after a normal end: no grandchild outlives its test; the
    // leader is not reaped yet, so its pid still names the group
    crate::claude_adapter::statusline_chain::kill_group(&mut child);
    let waited = child.wait();
    let status = if exited { waited.ok() } else { None };
    let output: Vec<u8> = readers
        .into_iter()
        .flat_map(|r| r.join().unwrap_or_default())
        .collect();
    let took = start.elapsed();
    let verdict = match status {
        None => Verdict::TimedOut,
        Some(s) if s.success() => Verdict::Ok,
        Some(s) => Verdict::Failed(s.code(), String::from_utf8_lossy(&output).into_owned()),
    };
    Ok(TestRun {
        path: test.to_string(),
        verdict,
        took,
    })
}

fn seconds(d: Duration) -> String {
    format!("{:.1}s", d.as_secs_f64())
}

/// The limit as written after `timed out after`: whole seconds, else one decimal.
fn limit_text(limit: Duration) -> String {
    if limit.subsec_nanos() == 0 {
        format!("{}s", limit.as_secs())
    } else {
        seconds(limit)
    }
}

/// The text of the command: `header`, one line per test (a failed one followed by what it
/// printed), then the counts; `declares no test` for a pack without any.
pub fn render(header: &str, report: &Report, limit: Duration) -> String {
    if report.runs.is_empty() {
        return format!(
            "pack {} declares no test (tests: in pack.yaml)\n",
            report.name
        );
    }
    let mut out = format!("{header}\n");
    let (mut passed, mut failed) = (0, 0);
    for r in &report.runs {
        match &r.verdict {
            Verdict::Ok => {
                passed += 1;
                out += &format!("ok    {}  {}\n", r.path, seconds(r.took));
            }
            Verdict::Failed(code, text) => {
                failed += 1;
                let code = code.map_or("signal".to_string(), |c| format!("exit {c}"));
                out += &format!("FAIL  {}  {code}  {}\n", r.path, seconds(r.took));
                for line in text.lines() {
                    out += &format!("  | {line}\n");
                }
            }
            Verdict::TimedOut => {
                failed += 1;
                out += &format!("FAIL  {}  timed out after {}\n", r.path, limit_text(limit));
            }
        }
    }
    out + &format!("{passed} passed, {failed} failed\n")
}

/// `armadai pack test [<dir>]`: prints the result; `false` when a test failed.
pub fn execute(dir: Option<PathBuf>, registry: &Path) -> anyhow::Result<bool> {
    let (pack_dir, header) = match dir {
        Some(d) => {
            let header = format!("pack test  {}", d.display());
            (d, header)
        }
        None => {
            let (dir, name, version) = locked_snapshot(registry)?;
            let header = format!("pack test  {name} {version} (locked snapshot)");
            (dir, header)
        }
    };
    let report = run_pack(&pack_dir, LIMIT)?;
    print!("{}", render(&header, &report, LIMIT));
    Ok(report.runs.iter().all(|r| r.verdict == Verdict::Ok))
}

/// The folder of the pack the project locks, its name and version; no network.
fn locked_snapshot(registry: &Path) -> anyhow::Result<(PathBuf, String, String)> {
    let cwd = std::env::current_dir()?;
    let root = crate::compile::find_project(&cwd)?;
    locked_snapshot_of(&root, registry)
}

pub(crate) fn locked_snapshot_of(
    root: &Path,
    registry: &Path,
) -> anyhow::Result<(PathBuf, String, String)> {
    let Some((name, _)) = crate::pack::effective::pack_ref(root) else {
        bail!("this project references no pack: give a pack folder, armadai pack test <dir>");
    };
    let Some(lock) = crate::pack::lock::read(root)? else {
        bail!("{}", crate::pack::effective::lock_missing(&name));
    };
    let (dir, _) = crate::pack::effective::snapshot_path(root, &lock, registry)?;
    let version = lock.resolved.version.clone();
    if !dir.is_dir() {
        bail!(
            "the locked snapshot of {} {version} is not in the cache; run armadai pack sync",
            lock.pack
        );
    }
    Ok((dir, lock.pack, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pack folder named `p` listing `tests` (as written, quoted), holding `files`.
    fn pack(tests: &[&str], files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let list: String = tests
            .iter()
            .map(|t| format!("  - {}\n", serde_json::to_string(t).unwrap()))
            .collect();
        let tests = if tests.is_empty() {
            String::new()
        } else {
            format!("tests:\n{list}")
        };
        let yaml = format!("name: p\ndescription: d\nformat: 1\n{tests}");
        put(dir.path(), "pack.yaml", &yaml);
        for (rel, text) in files {
            put(dir.path(), rel, text);
        }
        dir
    }

    fn put(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn verdicts(r: &Report) -> Vec<(&str, &Verdict)> {
        r.runs
            .iter()
            .map(|t| (t.path.as_str(), &t.verdict))
            .collect()
    }

    #[test]
    fn a_passing_and_a_failing_test_keep_their_order_and_output() {
        let dir = pack(
            &["tests/a.sh", "tests/b.sh"],
            &[
                ("tests/a.sh", "exit 0\n"),
                ("tests/b.sh", "echo boom; echo oops >&2; exit 1\n"),
            ],
        );
        let r = run_pack(dir.path(), LIMIT).unwrap();
        assert_eq!(r.name, "p");
        let boom = Verdict::Failed(Some(1), "boom\noops\n".into());
        assert_eq!(
            verdicts(&r),
            [("tests/a.sh", &Verdict::Ok), ("tests/b.sh", &boom)]
        );
    }

    #[test]
    fn the_text_shows_each_test_its_output_and_the_counts() {
        let ms = Duration::from_millis;
        let report = Report {
            name: "p".into(),
            runs: vec![
                TestRun {
                    path: "tests/permissions.sh".into(),
                    verdict: Verdict::Ok,
                    took: ms(400),
                },
                TestRun {
                    path: "tests/mr-rules.sh".into(),
                    verdict: Verdict::Failed(Some(1), "boom\n".into()),
                    took: ms(2100),
                },
                TestRun {
                    path: "tests/x.sh".into(),
                    verdict: Verdict::TimedOut,
                    took: ms(60_000),
                },
            ],
        };
        assert_eq!(
            render("pack test  demo 1.4.0 (locked snapshot)", &report, LIMIT),
            "pack test  demo 1.4.0 (locked snapshot)\n\
             ok    tests/permissions.sh  0.4s\n\
             FAIL  tests/mr-rules.sh  exit 1  2.1s\n\
             \x20 | boom\n\
             FAIL  tests/x.sh  timed out after 60s\n\
             1 passed, 2 failed\n"
        );
    }

    #[test]
    fn a_pack_without_a_test_says_so() {
        let dir = pack(&[], &[]);
        let r = run_pack(dir.path(), LIMIT).unwrap();
        assert_eq!(
            render("pack test  .", &r, LIMIT),
            "pack p declares no test (tests: in pack.yaml)\n"
        );
    }

    /// The test sees its own `HOME` and config folders, none of the caller's.
    #[test]
    fn a_test_gets_temporary_home_and_config_folders() {
        let outer = tempfile::tempdir().unwrap();
        let script = format!(
            "for v in HOME CLAUDE_CONFIG_DIR ARMADAI_CONFIG_DIR; do\n  \
               eval \"d=\\${{$v}}\"\n  \
               case \"$d\" in {o}*) echo \"$v leaks: $d\"; exit 1;; esac\n  \
               test -d \"$d\" || {{ echo \"$v is not a folder\"; exit 1; }}\n  \
               echo x > \"$d/written\"\n\
             done\n",
            o = outer.path().display()
        );
        let dir = pack(&["tests/h.sh"], &[("tests/h.sh", &script)]);
        let run = || {
            // the child inherits the process environment: set it under the lock
            let _g = armadai_core::test_support::env_lock();
            let set = [
                ("HOME", outer.path().to_path_buf()),
                ("CLAUDE_CONFIG_DIR", outer.path().to_path_buf()),
                ("ARMADAI_CONFIG_DIR", outer.path().to_path_buf()),
            ];
            let old: Vec<_> = set.iter().map(|(k, _)| (*k, std::env::var_os(k))).collect();
            for (k, v) in &set {
                // SAFETY: serialised by `env_lock`, restored below
                unsafe { std::env::set_var(k, v) };
            }
            let r = run_pack(dir.path(), LIMIT).unwrap();
            for (k, v) in old {
                // SAFETY: as above
                unsafe {
                    match v {
                        Some(v) => std::env::set_var(k, v),
                        None => std::env::remove_var(k),
                    }
                }
            }
            r
        };
        let r = run();
        assert_eq!(verdicts(&r), [("tests/h.sh", &Verdict::Ok)], "{r:?}");
        // nothing was written under the caller's folders
        assert!(std::fs::read_dir(outer.path()).unwrap().next().is_none());
    }

    #[test]
    fn no_tilde_or_variable_is_expanded_in_a_test_path() {
        let dir = pack(
            &["~/a.sh", "$HOME/b.sh"],
            &[("~/a.sh", "exit 0\n"), ("$HOME/b.sh", "exit 0\n")],
        );
        let r = run_pack(dir.path(), LIMIT).unwrap();
        assert_eq!(
            verdicts(&r),
            [("~/a.sh", &Verdict::Ok), ("$HOME/b.sh", &Verdict::Ok)]
        );
    }

    #[test]
    fn a_path_is_used_as_written_not_trimmed() {
        // a leading space is part of the name: the file is `<space>a.sh`
        let dir = pack(&[" a.sh"], &[(" a.sh", "exit 0\n"), ("a.sh", "exit 1\n")]);
        let r = run_pack(dir.path(), LIMIT).unwrap();
        assert_eq!(verdicts(&r), [(" a.sh", &Verdict::Ok)]);
    }

    #[test]
    fn an_escaping_or_nul_path_is_refused_with_the_message_not_run() {
        for bad in ["../x.sh", "/etc/x.sh", "tests/a\0b.sh"] {
            let dir = pack(&[bad], &[]);
            let e = run_pack(dir.path(), LIMIT).unwrap_err().to_string();
            let shown = bad.replace('\0', "\\0");
            assert!(
                e.contains(&format!(
                    "test `{shown}` must be a relative path inside the pack"
                )) && e.contains("write it as tests/<name>.sh, without .. or a leading /"),
                "{bad:?}: {e}"
            );
            assert!(!e.contains('\0'));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_in_the_pack_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        put(outside.path(), "x.sh", "exit 0\n");
        for make in [
            |from: &Path, to: &Path| {
                std::os::unix::fs::symlink(from.join("x.sh"), to.join("tests/l.sh"))
            },
            |from: &Path, to: &Path| std::os::unix::fs::symlink(from, to.join("tests/dir")),
        ] {
            let dir = pack(&["tests/l.sh"], &[("tests/ok.sh", "exit 0\n")]);
            make(outside.path(), dir.path()).unwrap();
            let e = run_pack(dir.path(), LIMIT).unwrap_err().to_string();
            assert!(e.contains("the pack holds a symbolic link, refused"), "{e}");
        }
    }

    /// Measured on the threshold of the defect: a script that sleeps 5 s and a grandchild
    /// that sleeps 30 s. A limit that is not enforced waits 5 s (and a group that is not
    /// killed holds the pipes 30 s); 3 s is between the limit and either.
    #[cfg(unix)]
    #[test]
    fn a_test_over_the_limit_is_killed_with_its_children() {
        for pass in 0..5 {
            let marks = tempfile::tempdir().unwrap();
            let pidfile = marks.path().join("pid");
            let script = format!("sleep 30 &\necho $! > {}\nsleep 5\n", pidfile.display());
            let dir = pack(&["tests/slow.sh"], &[("tests/slow.sh", &script)]);
            let start = Instant::now();
            let r = run_pack(dir.path(), Duration::from_secs(1)).unwrap();
            assert!(start.elapsed() < Duration::from_secs(3), "pass {pass}");
            assert_eq!(verdicts(&r), [("tests/slow.sh", &Verdict::TimedOut)]);
            assert_eq!(
                render("h", &r, Duration::from_secs(1)).lines().nth(1),
                Some("FAIL  tests/slow.sh  timed out after 1s")
            );
            let pid = std::fs::read_to_string(&pidfile).unwrap();
            // `kill -0` also succeeds for a killed process until init reaps it: give
            // it 2 s (the sleep lasts 30) before calling it alive
            let gone = (0..200).any(|_| {
                let alive = Command::new("kill")
                    .args(["-0", pid.trim()])
                    .stderr(Stdio::null())
                    .status()
                    .unwrap()
                    .success();
                if alive {
                    std::thread::sleep(Duration::from_millis(10));
                }
                !alive
            });
            let alive = !gone;
            assert!(!alive, "the grandchild {pid} is still alive");
        }
    }

    /// A script that ends normally but leaves a 30 s grandchild: the group is killed after
    /// the normal end too. A group left alone holds the pipes, and the run, for 30 s.
    #[cfg(unix)]
    #[test]
    fn a_grandchild_left_by_a_test_that_passed_is_killed() {
        let marks = tempfile::tempdir().unwrap();
        let pidfile = marks.path().join("pid");
        let script = format!("sleep 30 &\necho $! > {}\nexit 0\n", pidfile.display());
        let dir = pack(&["tests/ok.sh"], &[("tests/ok.sh", &script)]);
        let start = Instant::now();
        let r = run_pack(dir.path(), Duration::from_secs(10)).unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(verdicts(&r), [("tests/ok.sh", &Verdict::Ok)]);
        let pid = std::fs::read_to_string(&pidfile).unwrap();
        // `kill -0` also succeeds on a zombie until init reaps it: allow 2 s (the sleep lasts 30)
        let alive = |pid: &str| {
            Command::new("kill")
                .args(["-0", pid])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        let gone = (0..200).any(|_| {
            let a = alive(pid.trim());
            if a {
                std::thread::sleep(Duration::from_millis(10));
            }
            !a
        });
        if !gone {
            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
        }
        assert!(gone, "the grandchild {pid} is still alive");
    }

    // ---- the locked snapshot ----

    const URL: &str = "https://example.com/packs.git";
    const C1: &str = "1111111111111111111111111111111111111111";

    fn project(reference: bool, lock: bool) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let manifest = if reference {
            format!("pack: {{ name: demo, source: '{URL}', version: latest }}\n")
        } else {
            "schema_version: 1\n".to_string()
        };
        put(root.path(), ".armadai/project.yaml", &manifest);
        if lock {
            put(
                root.path(),
                ".armadai/pack.lock",
                &format!(
                    "format: 1\npack: demo\nsource: '{URL}'\nresolved:\n  version: 1.4.0\n  commit: {C1}\n"
                ),
            );
        }
        root
    }

    #[test]
    fn the_locked_snapshot_is_read_from_the_cache_and_its_absence_says_what_to_do() {
        let registry = tempfile::tempdir().unwrap();
        let root = project(true, true);
        let e = locked_snapshot_of(root.path(), registry.path()).unwrap_err();
        assert_eq!(
            e.to_string(),
            "the locked snapshot of demo 1.4.0 is not in the cache; run armadai pack sync"
        );
        let snap = crate::pack::cache::snapshot_dir_in(registry.path(), URL, "demo", C1);
        put(&snap, "pack.yaml", "name: demo\ndescription: d\n");
        let (dir, name, version) = locked_snapshot_of(root.path(), registry.path()).unwrap();
        assert_eq!(
            (dir, name.as_str(), version.as_str()),
            (snap, "demo", "1.4.0")
        );
    }

    #[test]
    fn a_project_without_a_lock_or_a_pack_is_told_what_to_do() {
        let registry = tempfile::tempdir().unwrap();
        let e = locked_snapshot_of(project(true, false).path(), registry.path())
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("pack.lock is missing: run `armadai pack update`"),
            "{e}"
        );
        let e = locked_snapshot_of(project(false, false).path(), registry.path())
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "this project references no pack: give a pack folder, armadai pack test <dir>"
        );
    }
}
