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
        self.run_env(args, &[])
    }

    /// Every file the binary left under `XDG_DATA_HOME`, relative to it.
    ///
    /// Empty before any command runs (`with_config` creates the directory
    /// and nothing else), so a non-empty result is always something the
    /// command under test wrote.
    fn data_files(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, base, out);
                } else {
                    out.push(
                        path.strip_prefix(base)
                            .unwrap_or(&path)
                            .display()
                            .to_string(),
                    );
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.data, &self.data, &mut out);
        out.sort();
        out
    }

    /// [`Sandbox::run`] plus extra environment variables — an API key, for a
    /// test whose agent is on an HTTP provider (`create_provider` builds it
    /// even on the preview, and refuses without one).
    fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&self.root)
            .env("ARMADAI_CONFIG_DIR", &self.config)
            .env("XDG_DATA_HOME", &self.data)
            .env("NO_COLOR", "1")
            .args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
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

/// Record a run that dies mid-flight and return its id — i.e. a run the
/// event log left in `Running`, the only kind `--resume` accepts.
///
/// The agent is rewritten to [`ECHO`] on the way out, so the caller resumes
/// a roster that can actually execute. Four tests need this exact fixture;
/// it is a function because the file already carried three copies of it and
/// a fourth is how a fixture starts drifting from its siblings.
#[cfg(feature = "storage")]
fn resumable_run_id(sb: &Sandbox) -> String {
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
    run_id
}

/// The agent of [`resumable_run_id`]'s setup run: a `cli` relay pointed at a
/// command that cannot execute, so the run fails after `RunStarted` is
/// persisted and before anything completes.
#[cfg(feature = "storage")]
const BROKEN: &str = "- provider: cli\n- command: /nonexistent-armadai-cmd\n";

/// Every preview signs off on the three guarantees `--dry-run`'s help makes,
/// asserted **clause by clause and per site**.
///
/// Each clause was false at some point in this command's history (#403): the
/// preview registered the project, and on a terminal it offered to rewrite
/// the very agent files it was previewing. The line was widened to say so —
/// and nothing held it there. Restoring the older, measured-false wording
/// ("no provider was called; nothing was recorded or billed" — true of the
/// provider, silent about the disk) left all 19 tests in this file green,
/// as did deleting the line outright from the `--resume` site.
///
/// Naming each clause pins the *promise* rather than the punctuation, and
/// asserting it per site is the same discipline the terminal-event tests
/// already apply: there are three preview sites and dropping the sign-off
/// from any one of them must be red on its own.
///
/// The clauses are spelled out rather than compared against the binary's own
/// `DRY_RUN_NO_EFFECTS`: `crates/armadai` has no `lib.rs`, so an integration
/// test cannot import that constant at all — and a test that compared the
/// output to the constant would agree with any rewording of it, which is the
/// regression being guarded.
fn assert_signs_off_on_every_guarantee(out: &Output, site: &str) {
    let err = out.stderr();
    for clause in [
        "no provider called",
        "no project registered",
        "no agent file rewritten",
    ] {
        assert!(
            err.contains(clause),
            "the {site} preview never promises `{clause}`; stderr was:\n{err}"
        );
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
    let sb = Sandbox::new(&[("alpha", BROKEN)]);
    let run_id = resumable_run_id(&sb);

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

// ── What the preview says the run would use (#398 review, F2/F3) ─────

/// The preview line for a link, e.g. `1/2 alpha — provider=…, model=…`.
fn preview_line(out: &Output, name: &str) -> String {
    out.stderr()
        .lines()
        .find(|l| l.contains(&format!(" {name} — ")))
        .unwrap_or_else(|| panic!("no preview line for {name}:\n{}", out.stderr()))
        .to_string()
}

/// Just the `model=…` half of a preview line. The provider half can legally
/// contain a vendor's name (`provider: claude`), so a naive substring check
/// over the whole line would answer about the wrong column.
fn previewed_model(out: &Output, name: &str) -> String {
    let line = preview_line(out, name);
    let at = line
        .find("model=")
        .unwrap_or_else(|| panic!("no model column in: {line}"));
    line[at + "model=".len()..].to_string()
}

/// A CLI-relayed agent's declared model is never sent: `CliProvider` spawns
/// the command and reads `request.model` nowhere, reporting the command name
/// back instead. The preview used to resolve the placeholder anyway and
/// announce an Anthropic id — the most misleading possible answer to "which
/// model will I pay for", on ArmadAI's own reference configuration.
///
/// The real run is executed straight after, so the two are compared rather
/// than the preview being asserted against a hardcoded expectation. It is a
/// `--pipe` run because `run_single_agent`'s `[name] model=…` summary is the
/// only place ArmadAI prints the model a run actually used, and `--pipe` is
/// the only path that reaches it.
#[test]
fn the_preview_does_not_name_a_model_a_cli_relay_would_ignore() {
    // `provider: claude` + `command: echo`: a unified tool name whose CLI is
    // present, so `create_provider` builds a relay — while the metadata
    // still names a vendor, so the model column has something concrete to
    // get wrong. This is the residual case: for `provider: cli` the tier no
    // longer resolves at all (no vendor is named), but a unified name does
    // name one, and the run still never sends it.
    let sb = Sandbox::new(&[
        (
            "alpha",
            "- provider: claude\n- command: echo\n- model: latest:pro\n",
        ),
        ("beta", ECHO),
    ]);

    let dry = sb.run(&["run", "alpha", "hello", "--pipe", "beta", "--dry-run"]);
    assert!(dry.succeeded(), "dry-run failed: {}", dry.stderr());
    let previewed = previewed_model(&dry, "alpha");
    assert!(
        !previewed.contains("claude-"),
        "the preview named a model the CLI relay never sends: model={previewed}"
    );
    assert!(
        previewed.contains("echo"),
        "the preview should name the relay that actually chooses: model={previewed}"
    );

    let real = sb.run(&["run", "alpha", "hello", "--pipe", "beta"]);
    assert!(real.succeeded(), "run failed: {}", real.stderr());
    assert!(
        real.stderr().contains("model=echo"),
        "expected the relay to report its own command as the model:\n{}",
        real.stderr()
    );
}

/// An API-backed agent still gets its resolved id — the guard above must not
/// have turned the model column into a blanket "unknown".
#[cfg(feature = "providers-api")]
#[test]
fn the_preview_names_the_resolved_model_for_an_api_agent() {
    let sb = Sandbox::new(&[("alpha", "- provider: anthropic\n- model: latest:pro\n")]);

    let dry = sb.run_env(
        &["run", "alpha", "hello", "--dry-run"],
        &[("ANTHROPIC_API_KEY", "sk-not-a-real-key")],
    );
    assert!(dry.succeeded(), "dry-run failed: {}", dry.stderr());
    assert_nothing_ran(&dry);
    let previewed = previewed_model(&dry, "alpha");
    assert!(
        previewed.starts_with("claude-") && !previewed.contains("latest:pro"),
        "expected the resolved Anthropic id, got: model={previewed}"
    );
}

/// `--dry-run`'s help promises "agents, providers and models … on every
/// path". The orchestrated preview printed the roster and nothing else, so
/// two of those words were false on two paths out of four (#398 review, F3).
#[test]
fn an_orchestrated_preview_names_each_agents_provider_and_model() {
    let sb = Sandbox::new(&[("alpha", ECHO), ("beta", ECHO)]);

    let dry = sb.run(&[
        "run",
        "alpha",
        "hello",
        "--pipe",
        "beta",
        "--orchestrate",
        "ring",
        "--dry-run",
        "--json",
        "--no-tui",
    ]);
    assert!(dry.succeeded(), "dry-run failed: {}", dry.stderr());
    assert_nothing_ran(&dry);

    for name in ["alpha", "beta"] {
        let line = preview_line(&dry, name);
        assert!(
            line.contains("provider=cli") && line.contains("model="),
            "orchestrated preview should name provider and model for {name}: {line}"
        );
    }
}

// ── The preview writes nothing to disk (#403) ────────────────────────

/// `--dry-run` promises "0 tokens" and signs off with "nothing was recorded
/// or billed". That was true of the provider and false of the disk:
/// `resolve_agents_dir` ran BEFORE the `dry_run` branch on every path, and
/// registered the project in `projects.json` on the way through.
///
/// The control run is the point of the test. An assertion that a file is
/// absent is worth nothing against a fixture that would never have written
/// it, so the same sandbox is run for real straight after: the registry
/// must appear then, and only then.
#[test]
fn a_sequential_dry_run_does_not_register_the_project() {
    let sb = Sandbox::new(&[("alpha", ECHO)]);
    let registry = sb.config.join("projects.json");

    let dry = sb.run(&["run", "alpha", "hello", "--dry-run", "--json"]);
    assert!(dry.succeeded(), "dry-run failed: {}", dry.stderr());
    assert!(
        !registry.exists(),
        "--dry-run registered the project:\n{}",
        std::fs::read_to_string(&registry).unwrap_or_default()
    );

    let real = sb.run(&["run", "alpha", "hello", "--json"]);
    assert!(real.succeeded(), "run failed: {}", real.stderr());
    assert!(
        registry.exists(),
        "the fixture never registers a project at all — the assertion above \
         proves nothing"
    );
}

/// `--resume` builds its roster through its own `resolve_agents_dir` call,
/// so the sequential fix does not reach it: it needs its own measurement.
///
/// The setup run registers the project (it is a real run), so the registry
/// is deleted before the preview — otherwise "the file exists" would be
/// true whatever the preview does.
#[cfg(feature = "storage")]
#[test]
fn a_resume_dry_run_does_not_register_the_project() {
    let sb = Sandbox::new(&[("alpha", BROKEN)]);
    let registry = sb.config.join("projects.json");
    let run_id = resumable_run_id(&sb);
    std::fs::remove_file(&registry).expect("the setup run should have registered the project");

    let dry = sb.run(&[
        "run",
        "--resume",
        &run_id,
        "--dry-run",
        "--json",
        "--no-tui",
    ]);
    assert!(dry.succeeded(), "resume dry-run failed: {}", dry.stderr());
    assert!(
        !registry.exists(),
        "--resume --dry-run registered the project:\n{}",
        std::fs::read_to_string(&registry).unwrap_or_default()
    );

    // Control: a real resume does register it.
    let real = sb.run(&["run", "--resume", &run_id, "--json", "--no-tui"]);
    assert!(real.succeeded(), "resume failed: {}", real.stderr());
    assert!(
        registry.exists(),
        "a real resume does not register either — the assertion above proves \
         nothing"
    );
}

// ── The preview on a real terminal (#398 review, #403) ───────────────

/// Spawn the binary with an actual TTY on all three stdio ends, feed it
/// `stdin`, and return `(status, everything the terminal saw)`.
///
/// `openpty`, no fork — the child simply gets the slave side as its stdio.
/// Two behaviours under test in this file are unreachable without it, and
/// both fail as *hangs*: the live Workroom's `IsTerminal` gate, and
/// `dialoguer`'s confirmation prompt, which only prompts on a terminal. The
/// deadline below is therefore part of the assertion — it turns a hang into
/// a red test instead of a stuck run.
///
/// The master is drained continuously by a thread: a pty buffer is a few KB,
/// and a child blocked on a full one would look exactly like the hang under
/// test.
/// A pty transcript with its control sequences made readable, keeping real
/// newlines so it still reads as lines.
///
/// The diagnostic below is printed only when the child hung, and what a hung
/// `armadai` has written by then begins with `ESC [ ? 1049 h` — "switch to
/// the alternate screen", emitted by the live Workroom on the way in.
/// Interpolated raw into a panic message, every byte after that one is
/// painted into the alternate buffer and is gone the instant the harness
/// restores the primary screen: the #413 reviewer read "Output so far:" as
/// empty while the pty had in fact captured 1.6 KB, and concluded the
/// diagnostic was less useful than it looked. It was the escape sequence
/// hiding it, not a missing capture.
#[cfg(unix)]
fn visible(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c == '\n' || !c.is_control() {
            out.push(c);
        } else {
            out.extend(c.escape_debug());
        }
    }
    out
}

/// The escaping above is the whole diagnostic, and it is only ever exercised
/// on a hang — which no green run produces. So it is asserted directly, on
/// the exact byte that caused the problem.
#[cfg(unix)]
#[test]
fn a_hung_pty_transcript_survives_being_printed() {
    let shown = visible("before\x1b[?1049hafter\r\nnext\n");

    assert!(
        !shown.contains('\x1b'),
        "an ESC survived into the panic message, which will swallow \
         everything after it: {shown:?}"
    );
    assert!(
        shown.contains("before") && shown.contains("after") && shown.contains("next"),
        "the transcript lost text it was supposed to show: {shown:?}"
    );
    assert!(
        shown.ends_with('\n'),
        "real newlines must survive, or the transcript stops reading as \
         lines: {shown:?}"
    );
}

#[cfg(unix)]
fn run_on_a_pty(sb: &Sandbox, args: &[&str], stdin: &str) -> (std::process::ExitStatus, String) {
    use std::io::{Read, Write};
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    let (master, slave) = {
        let (mut m, mut s) = (0, 0);
        // SAFETY: `openpty` writes two fresh fds through the out-params and
        // returns 0 on success; all other args are the documented "defaults"
        // null pointers.
        let rc = unsafe {
            libc::openpty(
                &mut m,
                &mut s,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty failed: {}", std::io::Error::last_os_error());
        // SAFETY: both fds are owned by this process and handed over exactly
        // once each.
        unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) }
    };

    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("armadai"));
    cmd.current_dir(&sb.root)
        .env("ARMADAI_CONFIG_DIR", &sb.config)
        .env("XDG_DATA_HOME", &sb.data)
        .env("NO_COLOR", "1")
        .args(args)
        .stdin(std::process::Stdio::from(slave.try_clone().unwrap()))
        .stdout(std::process::Stdio::from(slave.try_clone().unwrap()))
        .stderr(std::process::Stdio::from(slave.try_clone().unwrap()));
    let mut child = cmd.spawn().expect("spawn armadai on a pty");
    // The parent must not keep the slave open, or the master never sees EOF.
    drop(slave);

    let mut master = std::fs::File::from(master);
    if !stdin.is_empty() {
        master
            .write_all(stdin.as_bytes())
            .expect("write to the pty");
        master.flush().unwrap();
    }

    let seen: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let mut drain = master.try_clone().expect("clone the pty master");
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        // Linux reports EIO (not EOF) once the last slave closes; any error
        // is the end as far as this drain is concerned.
        while let Ok(n) = drain.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().unwrap() {
            Some(st) => break st,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "armadai never returned on a terminal — a preview that enters \
                     the live Workroom, and a prompt nobody answers, both hang \
                     exactly here. Output so far:\n{}",
                    visible(&String::from_utf8_lossy(&seen.lock().unwrap()))
                );
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    // Let the drain thread pick up whatever was still in the pty buffer.
    std::thread::sleep(Duration::from_millis(100));
    let out = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
    (status, out)
}

/// `--dry-run` on a **real terminal**.
///
/// Everything above runs with stdout on a pipe, where `IsTerminal` is false
/// and the live Workroom is out of reach whatever the flags say. That makes
/// the `&& !dry_run` term in [`use_live_workroom`](../src/cli/run.rs)
/// invisible to the whole suite: remove it and every test stays green.
///
/// What it guards is not cosmetic. The Workroom drives its own event loop
/// until the run produces a terminal event, and a dry run produces none — it
/// never dispatches anything. On a terminal, without the term, the preview
/// enters the alternate screen and stays there: the process does not exit.
#[cfg(unix)]
#[test]
fn a_dry_run_on_a_terminal_prints_a_preview_and_exits() {
    let sb = Sandbox::new(&[("alpha", ECHO), ("beta", ECHO)]);

    let (status, out) = run_on_a_pty(
        &sb,
        &[
            "run",
            "alpha",
            "hello",
            "--pipe",
            "beta",
            "--orchestrate",
            "ring",
            "--dry-run",
        ],
        "",
    );

    assert!(status.success(), "--dry-run on a terminal failed:\n{out}");
    assert!(
        out.contains("[dry-run] pattern 'ring'"),
        "expected the plain preview on a terminal, got:\n{out}"
    );
    assert!(
        !out.contains("\u{1b}[?1049h"),
        "--dry-run entered the alternate screen (the live Workroom):\n{out}"
    );
}

/// The second half of #403, and the one no piped test can reach.
///
/// `resolve_agents_dir` calls `model_updater::auto_check_and_prompt` with
/// `interactive = !headless && !atty_is_pipe()`. On a pipe that is always
/// false, so the whole suite above only ever saw the "hint:" branch. On a
/// terminal it prompts, and on confirmation `apply_findings` REWRITES the
/// agent files — under `--dry-run`, on a command whose last line claims
/// nothing was recorded.
///
/// The control run is what makes the dry-run assertion mean anything: the
/// same fixture, the same terminal, the same answer, without `--dry-run`.
/// It must rewrite the file. If it does not, the fixture never reached the
/// prompt and "the file is unchanged" would be true for the wrong reason.
///
/// `gpt-3.5-turbo` is an embedded deprecation in `model_aliases`
/// (→ `gpt-4o-mini`), so nothing here depends on the network or on the
/// user's `model-aliases.json` — `ARMADAI_CONFIG_DIR` points at an empty
/// sandbox, so no local override can be loaded.
#[cfg(unix)]
#[test]
fn a_dry_run_on_a_terminal_never_rewrites_an_agent_file() {
    const DEPRECATED: &str = "- provider: cli\n- command: echo\n- model: gpt-3.5-turbo\n";

    let control = Sandbox::new(&[("alpha", DEPRECATED)]);
    let file = control.root.join("agents").join("alpha.md");
    let before = std::fs::read_to_string(&file).unwrap();

    let (status, out) = run_on_a_pty(&control, &["run", "alpha", "hello"], "y\n");
    assert!(status.success(), "the control run failed:\n{out}");
    let rewritten = std::fs::read_to_string(&file).unwrap();
    assert!(
        rewritten.contains("gpt-4o-mini") && rewritten != before,
        "the control run never reached the interactive prompt, so the \
         dry-run assertion below would prove nothing. Terminal saw:\n{out}"
    );

    let sb = Sandbox::new(&[("alpha", DEPRECATED)]);
    let file = sb.root.join("agents").join("alpha.md");
    let (status, out) = run_on_a_pty(&sb, &["run", "alpha", "hello", "--dry-run"], "y\n");

    assert!(status.success(), "--dry-run on a terminal failed:\n{out}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        before,
        "--dry-run rewrote an agent file. Terminal saw:\n{out}"
    );
    assert!(
        !out.contains("Update deprecated models now?"),
        "--dry-run offered to write:\n{out}"
    );
    // Reporting the finding is the part that belongs in a preview.
    assert!(
        out.contains("gpt-3.5-turbo -> gpt-4o-mini"),
        "the preview stopped reporting the deprecation it would fix:\n{out}"
    );
}

// ── The --json stream of a preview terminates (#405) ─────────────────

/// The last line of stdout that parses as JSON.
///
/// `--dry-run --json` emitted `run_start` and then nothing at all, so a
/// consumer could not tell "the preview is over" from "the process died at
/// startup". Reading the LAST line — rather than searching the stream for a
/// `dry_run` anywhere in it — is the whole point: the event has to be
/// terminal to answer that question.
fn terminal_event(out: &Output) -> serde_json::Value {
    out.stdout()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .next_back()
        .unwrap_or_else(|| panic!("no JSON on stdout at all:\n{}", out.stdout()))
}

/// `[(agent, prov, model), …]` of a `dry_run` event, in emission order.
fn roster(ev: &serde_json::Value) -> Vec<(String, String, String)> {
    ev["agents"]
        .as_array()
        .unwrap_or_else(|| panic!("no agents array in {ev}"))
        .iter()
        .map(|a| {
            (
                a["agent"].as_str().unwrap_or_default().to_string(),
                a["prov"].as_str().unwrap_or_default().to_string(),
                a["model"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// One test per preview site, deliberately: an assertion on the whole
/// stream's concatenation would stay green with a single site wired, and
/// there are three (#398 found five sites of that same class, not one of
/// which was covered by any of the others).
#[test]
fn a_pipe_chain_dry_run_ends_its_json_stream_with_a_dry_run_event() {
    let sb = Sandbox::new(&[("alpha", ECHO), ("beta", ECHO)]);

    let out = sb.run(&[
        "run",
        "alpha",
        "hello",
        "--pipe",
        "beta",
        "--dry-run",
        "--json",
    ]);
    assert!(out.succeeded(), "dry-run failed: {}", out.stderr());

    let ev = terminal_event(&out);
    assert_eq!(
        ev["t"],
        "dry_run",
        "the sequential preview's stream does not end on a terminal event:\n{}",
        out.stdout()
    );
    assert_eq!(ev["mode"], "sequential", "in {ev}");
    assert_eq!(
        roster(&ev)
            .iter()
            .map(|(a, _, _)| a.clone())
            .collect::<Vec<_>>(),
        vec!["alpha".to_string(), "beta".to_string()],
        "roster missing or out of execution order in {ev}"
    );
    for (agent, prov, model) in roster(&ev) {
        assert_eq!(prov, "cli", "no provider for {agent} in {ev}");
        assert!(
            model.contains("echo"),
            "no model for {agent} in {ev} (got {model})"
        );
    }
    // The value, not merely its presence. `reason` is picked by a two-branch
    // `if n == 1`, and swapping those branches — a preview of two agents
    // explaining itself as "single agent" — left all 19 tests green while
    // they only asserted the field was non-empty, which any string satisfies.
    assert_eq!(ev["reason"], "explicit chain", "in {ev}");
    assert_signs_off_on_every_guarantee(&out, "sequential");
}

/// The `n == 1` half of that same branch, which no test reached: every
/// sequential preview under test was a `--pipe` chain of two.
#[test]
fn a_single_agent_dry_run_ends_its_json_stream_with_a_dry_run_event() {
    let sb = Sandbox::new(&[("alpha", ECHO)]);

    let out = sb.run(&["run", "alpha", "hello", "--dry-run", "--json"]);
    assert!(out.succeeded(), "dry-run failed: {}", out.stderr());

    let ev = terminal_event(&out);
    assert_eq!(
        ev["t"],
        "dry_run",
        "the single-agent preview's stream does not end on a terminal event:\n{}",
        out.stdout()
    );
    assert_eq!(ev["mode"], "sequential", "in {ev}");
    assert_eq!(
        roster(&ev)
            .iter()
            .map(|(a, _, _)| a.clone())
            .collect::<Vec<_>>(),
        vec!["alpha".to_string()],
        "roster missing in {ev}"
    );
    assert_eq!(ev["reason"], "single agent", "in {ev}");
    assert_signs_off_on_every_guarantee(&out, "single-agent");
}

#[test]
fn an_orchestrated_dry_run_ends_its_json_stream_with_a_dry_run_event() {
    let sb = Sandbox::new(&[("alpha", ECHO), ("beta", ECHO)]);

    let out = sb.run(&[
        "run",
        "alpha",
        "hello",
        "--pipe",
        "beta",
        "--orchestrate",
        "ring",
        "--dry-run",
        "--json",
        "--no-tui",
    ]);
    assert!(out.succeeded(), "dry-run failed: {}", out.stderr());

    let ev = terminal_event(&out);
    assert_eq!(
        ev["t"],
        "dry_run",
        "the orchestrated preview's stream does not end on a terminal event:\n{}",
        out.stdout()
    );
    assert_eq!(ev["mode"], "orchestrated", "in {ev}");
    assert_eq!(ev["pattern"], "ring", "in {ev}");
    assert_eq!(
        roster(&ev)
            .iter()
            .map(|(a, _, _)| a.clone())
            .collect::<Vec<_>>(),
        vec!["alpha".to_string(), "beta".to_string()],
        "roster missing or out of order in {ev}"
    );
    for (agent, prov, model) in roster(&ev) {
        assert_eq!(prov, "cli", "no provider for {agent} in {ev}");
        assert!(
            model.contains("echo"),
            "no model for {agent} in {ev} (got {model})"
        );
    }
    assert_eq!(ev["reason"], "no routing (full roster)", "in {ev}");
    assert_signs_off_on_every_guarantee(&out, "orchestrated");
}

#[cfg(feature = "storage")]
#[test]
fn a_resume_dry_run_ends_its_json_stream_with_a_dry_run_event() {
    let sb = Sandbox::new(&[("alpha", BROKEN)]);
    let run_id = resumable_run_id(&sb);

    let out = sb.run(&[
        "run",
        "--resume",
        &run_id,
        "--dry-run",
        "--json",
        "--no-tui",
    ]);
    assert!(out.succeeded(), "resume dry-run failed: {}", out.stderr());

    let ev = terminal_event(&out);
    assert_eq!(
        ev["t"],
        "dry_run",
        "the resume preview's stream does not end on a terminal event:\n{}",
        out.stdout()
    );
    assert_eq!(ev["mode"], "resume", "in {ev}");
    assert_eq!(ev["pattern"], "direct", "in {ev}");
    assert_eq!(
        roster(&ev),
        vec![(
            "alpha".to_string(),
            "cli".to_string(),
            "(not sent — cli:echo chooses)".to_string()
        )],
        "in {ev}"
    );
    // Names the run it reloaded from: the resume preview's whole claim is
    // that this roster is the *recorded* one, so a reason that does not
    // identify the record explains nothing.
    assert_eq!(
        ev["reason"],
        serde_json::Value::String(format!("roster reloaded from run {run_id}")),
        "in {ev}"
    );
    assert_signs_off_on_every_guarantee(&out, "resume");
}

/// A preview must not be mistakable for a run. `result` is what a consumer
/// bills, records and reports on, and a `result` with zeroed tokens is
/// exactly what a real run that cost nothing looks like — a cached answer,
/// a zero-token relay. So the preview gets its own event and emits no
/// `result` at all.
#[test]
fn a_dry_run_never_emits_a_result_event() {
    let sb = Sandbox::new(&[("alpha", ECHO), ("beta", ECHO)]);

    for args in [
        vec![
            "run",
            "alpha",
            "hello",
            "--pipe",
            "beta",
            "--dry-run",
            "--json",
        ],
        vec![
            "run",
            "alpha",
            "hello",
            "--pipe",
            "beta",
            "--orchestrate",
            "ring",
            "--dry-run",
            "--json",
            "--no-tui",
        ],
    ] {
        let out = sb.run(&args);
        assert!(out.succeeded(), "dry-run failed: {}", out.stderr());
        let kinds: Vec<String> = out
            .stdout()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .map(|v| v["t"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(
            !kinds.iter().any(|k| k == "result"),
            "a preview emitted a `result` a consumer would count as a run \
             ({args:?}): {kinds:?}"
        );
    }
}

/// The same guarantee on the third site, which the loop above cannot reach:
/// `--resume` needs a recorded run, so it needs its own fixture — and having
/// no test at all, it was the one site where emitting a zeroed `result`
/// before the `dry_run` event left every test in this file green.
///
/// That is the same "one test per site" argument the terminal-event tests
/// make against asserting on the concatenated stream, applied to the site it
/// had been left out of.
#[cfg(feature = "storage")]
#[test]
fn a_resume_dry_run_never_emits_a_result_event() {
    let sb = Sandbox::new(&[("alpha", BROKEN)]);
    let run_id = resumable_run_id(&sb);

    let out = sb.run(&[
        "run",
        "--resume",
        &run_id,
        "--dry-run",
        "--json",
        "--no-tui",
    ]);
    assert!(out.succeeded(), "resume dry-run failed: {}", out.stderr());

    let kinds: Vec<String> = out
        .stdout()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| v["t"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        !kinds.iter().any(|k| k == "result"),
        "the resume preview emitted a `result` a consumer would count as a \
         run: {kinds:?}"
    );
}

// ── What the preview promises on disk (#413 review) ──────────────────

/// The three paths whose promise is unqualified write **nothing**.
///
/// `--dry-run`'s help claimed for a while that the preview wrote "nothing"
/// at all. Measured false: `--resume` and `--replay` both call
/// `db::init_db()` before they can read a roster back, and
/// `armadai_storage::open` does `create_dir_all` + `Connection::open` +
/// `schema::apply` — so on a machine with no journal yet, a preview that had
/// just promised to write nothing left an 88 KB SQLite behind with the full
/// schema, then exited 1 on an unknown id. The help was narrowed to the
/// three guarantees it actually keeps.
///
/// This pins the other half of that measurement, which is the half worth
/// keeping: on the paths that consult no journal, the promise is total. An
/// `init_db()` drifting above a `dry_run` branch — exactly how the two
/// journal paths came by theirs — must be red here.
///
/// The control run is the point, as in the registration tests: "no file
/// appeared" is worth nothing against a fixture that never writes one, so
/// the same sandbox then runs for real and must produce the journal.
///
/// That control is also why this is gated on `storage`: without the feature
/// there is no journal for anyone to write, the real run leaves the data
/// directory as empty as the preview did, and the test would pass while
/// proving nothing. It says so out loud rather than being quietly vacuous —
/// run under `--features tui` alone it fails on the control, which is how
/// the gate was found.
#[cfg(feature = "storage")]
#[test]
fn a_preview_that_consults_no_journal_creates_no_journal() {
    for (site, args) in [
        (
            "single agent",
            vec!["run", "alpha", "hello", "--dry-run", "--json"],
        ),
        (
            "pipe chain",
            vec![
                "run",
                "alpha",
                "hello",
                "--pipe",
                "beta",
                "--dry-run",
                "--json",
            ],
        ),
        (
            "orchestrated",
            vec![
                "run",
                "alpha",
                "hello",
                "--pipe",
                "beta",
                "--orchestrate",
                "ring",
                "--dry-run",
                "--json",
                "--no-tui",
            ],
        ),
    ] {
        let sb = Sandbox::new(&[("alpha", ECHO), ("beta", ECHO)]);
        let dry = sb.run(&args);
        assert!(dry.succeeded(), "{site} dry-run failed: {}", dry.stderr());
        assert_eq!(
            sb.data_files(),
            Vec::<String>::new(),
            "the {site} preview wrote to the data directory"
        );

        let real = sb.run(&["run", "alpha", "hello", "--json"]);
        assert!(real.succeeded(), "run failed: {}", real.stderr());
        assert!(
            !sb.data_files().is_empty(),
            "the fixture never writes to the data directory at all — the \
             assertion above proves nothing about {site}"
        );
    }
}

/// `--dry-run` and `--replay` are refused together rather than silently
/// disagreeing.
///
/// `--dry-run` was outside the `agent`/`resume`/`replay` `ArgGroup`, so clap
/// accepted the pair and `execute_replay` — which never took the flag — went
/// on to replay the recorded run in full. The user asking for a preview got
/// a `result` with zeroed tokens: precisely the shape `RunEvent::DryRun`
/// exists to keep out of a consumer's hands, since zeroes are also what a
/// real run that cost nothing looks like.
///
/// Refused rather than honoured, because a replay already calls no provider
/// and spends nothing: previewing one would describe a cheaper operation
/// than simply performing it. Exit 2 is the documented usage-error code.
#[test]
fn a_dry_run_cannot_be_combined_with_replay() {
    let sb = Sandbox::new(&[("alpha", ECHO)]);

    let out = sb.run(&[
        "run",
        "--replay",
        "00000000-0000-0000-0000-000000000000",
        "--dry-run",
        "--json",
    ]);

    assert_eq!(
        out.0.status.code(),
        Some(2),
        "--replay --dry-run was not refused as a usage error; stdout:\n{}\nstderr:\n{}",
        out.stdout(),
        out.stderr()
    );
    assert!(
        out.stderr().contains("--replay") && out.stderr().contains("--dry-run"),
        "the refusal does not name both flags:\n{}",
        out.stderr()
    );
    // The silent-drop symptom itself: not one recorded event was re-emitted.
    assert_nothing_ran(&out);
    assert!(
        !out.stdout().contains("\"result\""),
        "a refused preview still emitted a result:\n{}",
        out.stdout()
    );
}

/// The sign-off is printed by the binary and quoted verbatim in the wiki,
/// and until now nothing linked the two.
///
/// It is exactly the kind of line that goes stale: its whole content is a
/// list of promises, and that list has already grown once (#403 added the
/// two disk clauses). A reader who trusts the page would be told the preview
/// guarantees something it no longer does — or, worse, would not be told
/// about a guarantee it gained.
///
/// `include_str!` rather than a runtime read, so a moved or renamed page is
/// a compile error here instead of a test that quietly stops checking.
#[test]
fn the_wiki_quotes_the_sign_off_the_binary_actually_prints() {
    const WIKI: &str = include_str!("../../../docs/wiki/declarative-agents.md");

    let sb = Sandbox::new(&[("alpha", ECHO)]);
    let out = sb.run(&["run", "alpha", "hello", "--dry-run", "--json"]);
    assert!(out.succeeded(), "dry-run failed: {}", out.stderr());

    let printed = out
        .stderr()
        .lines()
        .find(|l| l.contains("[dry-run] no provider"))
        .unwrap_or_else(|| panic!("the preview printed no sign-off:\n{}", out.stderr()))
        .to_string();

    assert!(
        WIKI.contains(&printed),
        "docs/wiki/declarative-agents.md quotes a `--dry-run` sign-off the \
         binary no longer prints.\n  printed: {printed}"
    );
}
