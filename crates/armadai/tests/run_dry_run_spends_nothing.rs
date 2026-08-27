//! Black-box regressions for `armadai run --dry-run` on the paths that are
//! not `--orchestrate` (#378).
//!
//! `dry_run` was read in exactly one place, `run_orchestrated_inner`. The
//! sequential chain never consulted it: `armadai run <a> <task> --pipe b,c
//! --dry-run` emitted a full `agent_start`/`agent_end`/`result` per link,
//! which means every provider call was made and billed — on a command whose
//! own help text promises "0 tokens". The single-agent path had the same
//! hole, and `--resume` never received the flag at all.
//!
//! **What these tests assert is the ABSENCE of a call**, not the exit code:
//! a `--dry-run` that returns 0 while spending money is precisely the bug.
//! Every agent here is `provider: cli` with `command: echo`, so a real run
//! is hermetic (no key, no network, no fake-provider feature) and visibly
//! emits `agent_start` per link — which is exactly the event that must not
//! appear.
//!
//! They spawn the real binary because the defect is in the wiring of
//! `cli::run`'s entry points, one per roster construction; a unit test on
//! any single function would stay green while another entry point kept
//! spending.

use assert_cmd::Command;
use std::path::{Path, PathBuf};

/// A project with `agents/<name>.md` for each `(name, metadata)` pair, plus
/// an isolated config dir and data dir.
///
/// Both redirections matter: without `ARMADAI_CONFIG_DIR` the agent
/// shadowing check scans the developer's real global library, and without
/// `XDG_DATA_HOME` a run that *does* execute writes into the developer's
/// real SQLite database — the `#[cfg(test)]` guard in `db.rs` does not
/// protect a spawned binary (#267). Both matter more than usual here, since
/// half of these tests exist to prove a run did *not* happen.
struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
}

/// The `## Metadata` body of an agent that runs hermetically: `echo` returns
/// the composed prompt verbatim, so a link that ran is unmistakable.
const ECHO: &str = "- provider: cli\n- command: echo\n";

impl Sandbox {
    fn new(agents: &[(&str, &str)]) -> Self {
        Self::with_config(agents, "")
    }

    /// [`Sandbox::new`] plus extra top-level `armadai.yaml` keys (an
    /// `orchestration:` block, say).
    fn with_config(agents: &[(&str, &str)], extra_yaml: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();

        let list: String = agents
            .iter()
            .map(|(name, _)| format!("  - name: {name}\n"))
            .collect();
        std::fs::write(
            root.join("armadai.yaml"),
            format!("agents:\n{list}{extra_yaml}"),
        )
        .unwrap();
        for (name, metadata) in agents {
            write_agent(&root, name, metadata);
        }

        let config = dir.path().join("config");
        let data = dir.path().join("data");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        Self {
            _dir: dir,
            root,
            config,
            data,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&self.root)
            .env("ARMADAI_CONFIG_DIR", &self.config)
            .env("XDG_DATA_HOME", &self.data)
            .env("NO_COLOR", "1")
            .args(args);
        Output(cmd.output().unwrap())
    }
}

fn write_agent(root: &Path, name: &str, metadata: &str) {
    std::fs::write(
        root.join("agents").join(format!("{name}.md")),
        format!("# {name}\n\n## Metadata\n{metadata}\n## System Prompt\nMARKER-{name}\n"),
    )
    .unwrap();
}

struct Output(std::process::Output);

impl Output {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.0.stdout).into_owned()
    }
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.0.stderr).into_owned()
    }
    fn succeeded(&self) -> bool {
        self.0.status.success()
    }
    /// The `agent` of every event of kind `t` on stdout, in emission order.
    fn events(&self, t: &str) -> Vec<String> {
        self.stdout()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["t"] == t)
            .map(|v| v["agent"].as_str().unwrap_or("").to_string())
            .collect()
    }
}

/// The single assertion these tests are about: not one provider call was
/// made. `agent_start` is emitted immediately before the call on every path
/// (`run_single_agent`'s inline emission, and the ES bridge's
/// `AgentInvoked → agent_start` projection), so its absence is the proof.
fn assert_nothing_ran(out: &Output) {
    assert_eq!(
        out.events("agent_start"),
        Vec::<String>::new(),
        "--dry-run called a provider; stdout was:\n{}",
        out.stdout()
    );
    assert_eq!(
        out.events("agent_end"),
        Vec::<String>::new(),
        "--dry-run produced agent output; stdout was:\n{}",
        out.stdout()
    );
}

// ── The preview spends nothing ───────────────────────────────────────

#[test]
fn a_pipe_chain_dry_run_calls_no_provider_and_lists_the_links_in_order() {
    let sb = Sandbox::new(&[("alpha", ECHO), ("beta", ECHO), ("gamma", ECHO)]);

    let out = sb.run(&[
        "run",
        "alpha",
        "hello",
        "--pipe",
        "beta",
        "gamma",
        "--dry-run",
        "--json",
    ]);

    assert!(out.succeeded(), "dry-run failed: {}", out.stderr());
    assert_nothing_ran(&out);

    // The preview still has to say what *would* run, in order.
    let err = out.stderr();
    let positions: Vec<usize> = ["1/3 alpha", "2/3 beta", "3/3 gamma"]
        .iter()
        .map(|s| {
            err.find(s)
                .unwrap_or_else(|| panic!("missing {s} in:\n{err}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "links previewed out of execution order:\n{err}"
    );
}

/// The single-agent invocation is the same defect: `--dry-run` has no
/// `--pipe`/`--orchestrate` qualifier in its help text, and this path ran
/// the agent too.
#[test]
fn a_single_agent_dry_run_calls_no_provider() {
    let sb = Sandbox::new(&[("alpha", ECHO)]);

    let out = sb.run(&["run", "alpha", "hello", "--dry-run", "--json"]);

    assert!(out.succeeded(), "dry-run failed: {}", out.stderr());
    assert_nothing_ran(&out);
    assert!(
        out.stderr().contains("1/1 alpha"),
        "the preview did not name the agent:\n{}",
        out.stderr()
    );
}

// ── The preview refuses what the real pass refuses ───────────────────

/// #348's lesson from `unlink --dry-run`: a preview that always succeeds
/// pre-checks nothing. An unresolvable link must fail the dry run with the
/// same non-zero exit the real run gives — and still call nothing.
#[test]
fn a_dry_run_with_an_unresolvable_link_exits_non_zero_like_the_real_pass() {
    let sb = Sandbox::new(&[("alpha", ECHO)]);

    let dry = sb.run(&[
        "run",
        "alpha",
        "hello",
        "--pipe",
        "nope",
        "--dry-run",
        "--json",
    ]);
    let real = sb.run(&["run", "alpha", "hello", "--pipe", "nope", "--json"]);

    assert!(!dry.succeeded(), "dry-run accepted a missing link");
    assert!(!real.succeeded(), "the real run accepted a missing link");
    assert_nothing_ran(&dry);
    assert_eq!(
        dry.0.status.code(),
        real.0.status.code(),
        "dry-run and the real pass disagree on the exit code"
    );
}

/// The provider is built by the same pre-pass, so a link whose provider
/// cannot be constructed is refused before anything runs — on the preview
/// exactly as on the real pass, word for word.
#[test]
fn a_dry_run_with_an_unbuildable_provider_refuses_exactly_like_the_real_pass() {
    let sb = Sandbox::new(&[("alpha", ECHO), ("bad", "- provider: gtp\n- model: x\n")]);

    let dry = sb.run(&[
        "run",
        "alpha",
        "hello",
        "--pipe",
        "bad",
        "--dry-run",
        "--json",
    ]);
    let real = sb.run(&["run", "alpha", "hello", "--pipe", "bad", "--json"]);

    assert!(!dry.succeeded(), "dry-run accepted an unbuildable provider");
    assert_nothing_ran(&dry);

    let msg = |o: &Output| {
        o.stdout()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|v| v["t"] == "error")
            .and_then(|v| v["msg"].as_str().map(str::to_string))
            .unwrap_or_default()
    };
    assert!(
        msg(&dry).contains("has no usable provider"),
        "unexpected dry-run error: {}",
        msg(&dry)
    );
    assert_eq!(msg(&dry), msg(&real), "the preview refuses in other words");
}

/// A single agent gets the *unprefixed* message the real single-agent pass
/// produces: routing the preview through the chain pre-pass must not make
/// `armadai run <x> --dry-run` talk about "chain link 1/1" when there is no
/// chain and nothing to count.
#[test]
fn a_single_agent_dry_run_refuses_in_the_same_words_as_the_real_pass() {
    let sb = Sandbox::new(&[("alpha", ECHO)]);

    let dry = sb.run(&["run", "nope", "hello", "--dry-run", "--json"]);
    let real = sb.run(&["run", "nope", "hello", "--json"]);

    let msg = |o: &Output| {
        o.stdout()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|v| v["t"] == "error")
            .and_then(|v| v["msg"].as_str().map(str::to_string))
            .unwrap_or_default()
    };
    assert!(!dry.succeeded());
    assert_eq!(dry.0.status.code(), real.0.status.code());
    assert!(!msg(&dry).is_empty(), "no error event on the dry run");
    assert_eq!(msg(&dry), msg(&real));
    assert!(
        !msg(&dry).contains("chain link"),
        "a one-agent run was described as a chain link: {}",
        msg(&dry)
    );
}

// ── --resume ─────────────────────────────────────────────────────────

/// `--resume` has its own roster construction, and until #378 it never even
/// received `dry_run` — `armadai run --resume <id> --dry-run` resumed for
/// real. The preview must call nothing AND leave the run resumable, since a
/// dry run that consumed the resume point would be worse than the bug.
#[cfg(feature = "storage")]
#[test]
fn a_resume_dry_run_calls_no_provider_and_leaves_the_run_resumable() {
    let sb = Sandbox::new(&[(
        "alpha",
        "- provider: cli\n- command: /nonexistent-armadai-cmd\n",
    )]);

    // A run whose provider cannot execute dies mid-flight, leaving the
    // event log in `Running` — i.e. resumable.
    let setup = sb.run(&["run", "alpha", "hello", "--json"]);
    assert!(!setup.succeeded(), "the setup run was supposed to fail");
    let run_id = setup
        .stdout()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["t"] == "run_start")
        .and_then(|v| v["run_id"].as_str().map(str::to_string))
        .expect("no run_start to take a run id from");

    write_agent(&sb.root, "alpha", ECHO);

    let dry = sb.run(&[
        "run",
        "--resume",
        &run_id,
        "--dry-run",
        "--json",
        "--no-tui",
    ]);
    assert!(dry.succeeded(), "resume dry-run failed: {}", dry.stderr());
    assert_nothing_ran(&dry);
    assert!(
        dry.stderr().contains("alpha"),
        "the preview did not name the reloaded roster:\n{}",
        dry.stderr()
    );

    // Still resumable: the preview appended nothing to the log.
    let real = sb.run(&["run", "--resume", &run_id, "--json", "--no-tui"]);
    assert!(real.succeeded(), "resume failed: {}", real.stderr());
    assert_eq!(real.events("agent_start"), vec!["alpha".to_string()]);
}

/// Same refusal, same exit code: an unknown run id is rejected by the
/// preview exactly as by a real resume.
#[cfg(feature = "storage")]
#[test]
fn a_resume_dry_run_of_an_unknown_id_exits_non_zero() {
    let sb = Sandbox::new(&[("alpha", ECHO)]);

    let dry = sb.run(&[
        "run",
        "--resume",
        "no-such-run",
        "--dry-run",
        "--json",
        "--no-tui",
    ]);
    let real = sb.run(&["run", "--resume", "no-such-run", "--json", "--no-tui"]);

    assert!(!dry.succeeded(), "dry-run accepted an unknown run id");
    assert_eq!(dry.0.status.code(), real.0.status.code());
    assert_nothing_ran(&dry);
}

/// The resume preview must list the roster in the order the run recorded,
/// not sorted. For `ring` that order IS the circulation order
/// (`run_ring_es` stores `agent_order` in its `RunStarted`), so an
/// alphabetical listing would describe a run that would not happen. The
/// three names are chosen so run order (`zeta, mid, alpha`) is the exact
/// reverse of alphabetical order — nothing but reading the recorded roster
/// can produce it.
///
/// Interrupting a run is the only way to get a resumable multi-agent one:
/// `ring` deliberately swallows provider failures (it records a `pass` and
/// keeps circulating), so a broken command completes the run instead of
/// leaving it `Running`. The interruption is driven by the run's OWN output
/// rather than by a timer — the process is killed the moment its first
/// `agent_start` reaches stdout, which is after `RunStarted` is persisted
/// and before the first provider call returns. No wall-clock assumption,
/// so nothing here can flake under load.
#[cfg(feature = "storage")]
#[test]
fn a_resume_dry_run_lists_the_roster_in_run_order_not_sorted() {
    use std::io::BufRead;

    let sb = Sandbox::new(&[]);

    // A command that blocks, so the run can be caught mid-flight. Written as
    // a script rather than passed as `args:` so nothing depends on how a
    // multi-word argument survives metadata parsing.
    let blocker = sb.root.join("blocker.sh");
    std::fs::write(&blocker, "#!/bin/sh\nsleep 10\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&blocker, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let names = ["zeta", "mid", "alpha"];
    let list: String = names.iter().map(|n| format!("  - name: {n}\n")).collect();
    std::fs::write(sb.root.join("armadai.yaml"), format!("agents:\n{list}")).unwrap();
    for n in names {
        write_agent(
            &sb.root,
            n,
            &format!("- provider: cli\n- command: {}\n", blocker.display()),
        );
    }

    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("armadai"))
        .current_dir(&sb.root)
        .env("ARMADAI_CONFIG_DIR", &sb.config)
        .env("XDG_DATA_HOME", &sb.data)
        .env("NO_COLOR", "1")
        .args([
            "run",
            "zeta",
            "hello",
            "--pipe",
            "mid",
            "alpha",
            "--orchestrate",
            "ring",
            "--json",
            "--no-tui",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut run_id = None;
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap() > 0 {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if v["t"] == "run_start" {
                run_id = v["run_id"].as_str().map(str::to_string);
            }
            if v["t"] == "agent_start" {
                break;
            }
        }
        line.clear();
    }
    let _ = child.kill();
    let _ = child.wait();
    let run_id = run_id.expect("the interrupted run never announced a run id");

    // Make the roster runnable again, so nothing but the preview's own
    // ordering can be what this test observes.
    for n in names {
        write_agent(&sb.root, n, ECHO);
    }

    let dry = sb.run(&[
        "run",
        "--resume",
        &run_id,
        "--dry-run",
        "--json",
        "--no-tui",
    ]);
    assert!(dry.succeeded(), "resume dry-run failed: {}", dry.stderr());
    assert_nothing_ran(&dry);

    let err = dry.stderr();
    let at = |n: &str| {
        err.find(&format!("[dry-run]   {n} —"))
            .unwrap_or_else(|| panic!("{n} missing from the preview:\n{err}"))
    };
    assert!(
        at("zeta") < at("mid") && at("mid") < at("alpha"),
        "roster previewed sorted instead of in the order the run recorded:\n{err}"
    );
}
