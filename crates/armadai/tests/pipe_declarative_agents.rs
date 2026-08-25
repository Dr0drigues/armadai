//! Black-box regressions for `armadai run --pipe` over agents declared in
//! `.armadai/agents.yaml` (#339).
//!
//! `--resume` and `--orchestrate` were wired onto `agent_source::
//! load_agent_by_name` in #337, but the historical chaining loop still
//! resolved a *path* per agent (`resolve_agent_path`), which a declared
//! agent does not have — so any chain containing one died before its first
//! provider call, with a message naming the three library directories and
//! never the `agents.yaml` that declares the agent.
//!
//! Spawns the real binary (like `link_list_gate.rs`) because the defect is
//! in the wiring of `cli::run`'s chain loop, not in any single function:
//! a unit test on `load_agent_for_run` alone would stay green if the loop
//! went back to resolving paths.
//!
//! Every agent here uses `provider: cli` with `command: echo`, so a chain
//! runs with no API key, no network and no fake-binary feature — and the
//! echo makes each agent's composed system prompt visible in the next
//! agent's input, which is how these tests prove *which* agents ran and in
//! what order rather than merely that the command exited 0.

use assert_cmd::Command;
use std::path::Path;

/// The unmistakable string an agent's system prompt carries into the echoed
/// output. Deliberately keyed on the agent name so two agents in one chain
/// can never be confused for each other.
fn marker(agent: &str) -> String {
    format!("PIPE-AGENT-IS-{agent}")
}

/// The prompt fragment every declared agent in this file composes from.
/// `{{name}}` is substituted by `agent_decl::compose_prompt` from the
/// declaration's own name, so one fragment serves every declared agent.
const DECLARED_FRAGMENT: &str = "PIPE-AGENT-IS-{{name}}\n\n\
You are {{name}}. Echo the task back.\n";

/// A file-backed agent's Markdown: the three sections `parse_agent_file`
/// requires, with the same `echo` provider and the same marker shape the
/// declared agents get from [`DECLARED_FRAGMENT`].
fn file_agent_markdown(agent: &str) -> String {
    format!(
        "# {agent}\n\
         \n\
         ## Metadata\n\
         - provider: cli\n\
         - command: echo\n\
         \n\
         ## System Prompt\n\
         PIPE-AGENT-IS-{agent}\n\
         \n\
         You are {agent}. Echo the task back.\n"
    )
}

/// A project whose only agents are declared — no `.md` anywhere, and an
/// absent `agents:` list — plus, optionally, file-backed agents written as
/// `agents/<name>.md` and listed in `armadai.yaml`.
///
/// Returns the tempdir (kept alive by the caller) and the project root.
fn project(declared: &[&str], file_backed: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join(".armadai/prompts")).unwrap();

    let agents_block: String = file_backed
        .iter()
        .map(|a| format!("  - name: {a}\n"))
        .collect();
    let config = if file_backed.is_empty() {
        String::new()
    } else {
        format!("agents:\n{agents_block}")
    };
    std::fs::write(root.join("armadai.yaml"), config).unwrap();

    std::fs::write(
        root.join(".armadai/prompts/pipe-base.md"),
        DECLARED_FRAGMENT,
    )
    .unwrap();
    let decls: String = declared
        .iter()
        .map(|a| format!("  - name: {a}\n    prompt: [pipe-base]\n"))
        .collect();
    std::fs::write(
        root.join(".armadai/agents.yaml"),
        format!("defaults:\n  provider: cli\n  command: echo\nagents:\n{decls}"),
    )
    .unwrap();

    if !file_backed.is_empty() {
        std::fs::create_dir_all(root.join("agents")).unwrap();
        for a in file_backed {
            std::fs::write(
                root.join("agents").join(format!("{a}.md")),
                file_agent_markdown(a),
            )
            .unwrap();
        }
    }

    (dir, root)
}

/// Runs `armadai run <head> <input> --pipe <rest…> --json` in `root`, with
/// the config dir AND the data dir redirected into the tempdir: without the
/// former the shadowing check would scan the developer's real global agent
/// library, and without the latter `record_run` would write this test's runs
/// into the developer's real SQLite database (#267).
fn run_pipe(root: &Path, head: &str, input: &str, rest: &[&str]) -> std::process::Output {
    run_pipe_inner(root, head, input, rest, true)
}

/// [`run_pipe`] without `--json`: the plain human invocation, whose failures
/// go through `main`'s `Debug`-formatted `anyhow::Error` (`Error: …` plus a
/// `Caused by:` section) rather than through an `error` event.
fn run_pipe_human(root: &Path, head: &str, input: &str, rest: &[&str]) -> std::process::Output {
    run_pipe_inner(root, head, input, rest, false)
}

fn run_pipe_inner(
    root: &Path,
    head: &str,
    input: &str,
    rest: &[&str],
    json: bool,
) -> std::process::Output {
    let sandbox = root.parent().unwrap();
    let config = sandbox.join("config");
    let data = sandbox.join("data");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let mut cmd = Command::cargo_bin("armadai").unwrap();
    cmd.current_dir(root)
        .env("ARMADAI_CONFIG_DIR", &config)
        .env("XDG_DATA_HOME", &data)
        .args(["run", head, input, "--pipe"])
        .args(rest);
    if json {
        cmd.arg("--json");
    }
    cmd.output().unwrap()
}

/// The `agent` field of every `agent_start` event on stdout, in emission order.
fn agents_started(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["t"] == "agent_start")
        .map(|v| v["agent"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The single terminal `result` event on stdout.
fn result_event(stdout: &str) -> serde_json::Value {
    let results: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["t"] == "result")
        .collect();
    assert_eq!(
        results.len(),
        1,
        "expected exactly one result event, got: {stdout}"
    );
    results.into_iter().next().unwrap()
}

/// Asserts that `content` shows `outer`'s prompt wrapping `inner`'s output,
/// i.e. that `inner` ran first and `outer` consumed it — the property a
/// chain that merely ran both agents in the wrong order would fail.
fn assert_wraps(content: &str, outer: &str, inner: &str, input: &str) {
    let outer_at = content
        .find(&marker(outer))
        .unwrap_or_else(|| panic!("'{outer}' never ran: {content}"));
    let inner_at = content
        .find(&marker(inner))
        .unwrap_or_else(|| panic!("'{inner}' never ran: {content}"));
    assert!(
        outer_at < inner_at,
        "'{outer}' must consume '{inner}''s output (so its prompt wraps it), got: {content}"
    );
    assert!(
        content.contains(input),
        "the original input must survive the chain, got: {content}"
    );
}

#[test]
fn pipe_chains_two_declared_agents() {
    let (_dir, root) = project(&["alpha-decl", "omega-decl"], &[]);
    let out = run_pipe(&root, "alpha-decl", "TASK-ALL-DECLARED", &["omega-decl"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "a --pipe chain of two declared agents must run.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        agents_started(&stdout),
        vec!["alpha-decl".to_string(), "omega-decl".to_string()],
        "both declared agents must start, in chain order.\nstdout: {stdout}"
    );
    let result = result_event(&stdout);
    assert_eq!(result["agents"], 2);
    assert_wraps(
        result["content"].as_str().unwrap(),
        "omega-decl",
        "alpha-decl",
        "TASK-ALL-DECLARED",
    );
}

#[test]
fn pipe_chains_a_file_backed_head_into_a_declared_tail() {
    let (_dir, root) = project(&["omega-decl"], &["beta-file"]);
    let out = run_pipe(
        &root,
        "beta-file",
        "TASK-FILE-THEN-DECLARED",
        &["omega-decl"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "a declared agent must be usable as a non-head link of a chain.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        agents_started(&stdout),
        vec!["beta-file".to_string(), "omega-decl".to_string()],
        "stdout: {stdout}"
    );
    assert_wraps(
        result_event(&stdout)["content"].as_str().unwrap(),
        "omega-decl",
        "beta-file",
        "TASK-FILE-THEN-DECLARED",
    );
}

#[test]
fn pipe_chains_a_declared_head_into_a_file_backed_tail() {
    let (_dir, root) = project(&["alpha-decl"], &["beta-file"]);
    let out = run_pipe(
        &root,
        "alpha-decl",
        "TASK-DECLARED-THEN-FILE",
        &["beta-file"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "mixing a declared head with a file-backed tail must run.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        agents_started(&stdout),
        vec!["alpha-decl".to_string(), "beta-file".to_string()],
        "stdout: {stdout}"
    );
    assert_wraps(
        result_event(&stdout)["content"].as_str().unwrap(),
        "beta-file",
        "alpha-decl",
        "TASK-DECLARED-THEN-FILE",
    );
}

#[test]
fn pipe_names_the_declarations_file_when_a_chained_agent_is_missing() {
    let (_dir, root) = project(&["alpha-decl"], &[]);
    let out = run_pipe(&root, "alpha-decl", "TASK-MISSING", &["nowhere-agent"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !out.status.success(),
        "a chain naming an unknown agent must still fail.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("nowhere-agent"),
        "the failure must name the agent it could not find, got: {combined}"
    );
    assert!(
        combined.contains(".armadai/agents.yaml") || combined.contains(".armadai\\agents.yaml"),
        "a chain in a project that declares agents must mention the declarations file \
         it also looked in — `list` and `inspect` already do, got: {combined}"
    );
}

// ---------------------------------------------------------------------------
// #364 review, m2: a name that is both declared and written as a file is
// refused on the `--pipe` path too.
// ---------------------------------------------------------------------------

#[test]
fn pipe_refuses_a_name_that_is_both_declared_and_written_as_a_file() {
    // `agent_source::load_agent_by_name` gives declarations and files NO
    // precedence over each other: a colliding name is an error the user has
    // to resolve, not a coin flip. `armadai run <name>` already refused it,
    // but `--pipe` reached it through a path-shaped resolver that never
    // consulted the declarations at all — so the same project refused a
    // single-agent run and accepted the chain. Routing `--pipe` through the
    // by-name loader closed that inconsistency; this pins it.
    let (_dir, root) = project(&["alpha-decl"], &["alpha-decl", "beta-file"]);
    let out = run_pipe(&root, "alpha-decl", "TASK-COLLIDING", &["beta-file"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !out.status.success(),
        "a chain whose head is both declared and written as a file must be refused, \
         exactly as the single-agent path already refuses it.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("also written as"),
        "the refusal must be the collision one — naming both places the name lives — \
         not a generic 'not found', got: {combined}"
    );
    assert!(
        combined.contains("remove one"),
        "the refusal must say what to do about it, got: {combined}"
    );
    assert!(
        agents_started(&stdout).is_empty(),
        "the collision must be caught before the head agent burns a provider call, \
         got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// #364 review, m1: loop-invariant work is done once per run, not once per
// link — measured through the one symptom a user can see.
// ---------------------------------------------------------------------------

#[test]
fn a_broken_prompt_fragment_is_reported_once_for_the_whole_chain() {
    // `project_fragments` scans and parses three prompt directories, and
    // `load_all_prompts` prints `warn: failed to load prompt <file>: <err>`
    // for each one it cannot read. That line names no agent and carries no
    // per-link information, so printing it once per link — three identical
    // lines for a three-link chain — is noise, the same defect `eecbd0f`
    // fixed for `unlink`. One line per broken fragment, however long the
    // chain.
    let (_dir, root) = project(&[], &["chain-1", "chain-2", "chain-3"]);
    std::fs::write(
        root.join(".armadai/prompts/pipe-broken.md"),
        "---\nname: [unterminated\n---\nbody\n",
    )
    .unwrap();

    let out = run_pipe(
        &root,
        "chain-1",
        "TASK-ONE-WARNING",
        &["chain-2", "chain-3"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "a broken fragment no agent references must not fail the chain.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        agents_started(&stdout).len(),
        3,
        "all three links must still run.\nstdout: {stdout}"
    );

    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("failed to load prompt"))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "the broken fragment must be reported ONCE for the whole chain, not once per \
         link — the lines are identical and name no agent, so N of them say nothing \
         the first did not. Got {} line(s): {warnings:#?}",
        warnings.len()
    );
    assert!(
        warnings[0].contains("pipe-broken.md"),
        "the one warning must still name the fragment it could not load, got: {warnings:#?}"
    );
}

// ---------------------------------------------------------------------------
// #366: the whole chain is resolved before the first link runs.
// ---------------------------------------------------------------------------

#[test]
fn pipe_resolves_every_link_before_running_the_first() {
    // The chain used to be resolved lazily, one link at a time, *inside* the
    // execution loop: a typo on link N was only discovered after links
    // 1..N-1 had already run — real provider calls, billed, on a chain that
    // could never complete. Measured on this exact fixture: `agent_start`
    // and `agent_end` for `m3-a` and `m3-b`, and only then the `error`.
    //
    // The `echo` provider makes those calls free here, which is precisely
    // why the assertion is on the events and not on a bill: with a real
    // provider the same two `agent_start`s are two model calls.
    let (_dir, root) = project(&["m3-a", "m3-b", "m3-c"], &[]);
    let out = run_pipe(
        &root,
        "m3-a",
        "TASK-LATE-TYPO",
        &["m3-b", "typo-agent", "m3-c"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !out.status.success(),
        "a chain naming an unknown agent must fail.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        agents_started(&stdout),
        Vec::<String>::new(),
        "NO agent may start when a later link of the chain cannot be resolved — every \
         one of them is a billed provider call on a run that cannot complete.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("typo-agent"),
        "the failure must name the link it could not resolve, got: {combined}"
    );
    assert!(
        combined.contains("3/4"),
        "the failure must place the bad link in the chain (link 3 of 4) — with four \
         names on one command line, the name alone leaves the user counting, \
         got: {combined}"
    );
    assert!(
        combined.contains(".armadai/agents.yaml") || combined.contains(".armadai\\agents.yaml"),
        "positioning the bad link must not swallow the resolution message: a project \
         that declares agents must still be told the declarations file was looked in \
         (#339), got: {combined}"
    );
}

// ---------------------------------------------------------------------------
// #373 review, i1 + m4: the human error path keeps the whole chain of causes.
//
// Every test above passes `--json`, which is exactly how the regression got
// in: the headless `error` event reports `Error::to_string()` — one layer —
// so a flattened error looks identical there, while `main`'s `Debug`-printed
// `anyhow::Error` on the human path lost its whole `Caused by:` section.
// ---------------------------------------------------------------------------

/// Overwrite one of [`project`]'s file-backed agents with Markdown carrying
/// `extra_metadata` as an additional `## Metadata` line, leaving the
/// `armadai.yaml` entry `project` already wrote for it in place.
fn break_file_agent(root: &Path, agent: &str, extra_metadata: &str) {
    let md = file_agent_markdown(agent).replace(
        "- command: echo\n",
        &format!("- command: echo\n{extra_metadata}\n"),
    );
    std::fs::write(root.join("agents").join(format!("{agent}.md")), md).unwrap();
}

/// `armadai run <agent> <input>` — the single-agent path, whose error
/// reporting `--pipe`'s must not be worse than.
fn run_single_human(root: &Path, agent: &str, input: &str) -> std::process::Output {
    let sandbox = root.parent().unwrap();
    let config = sandbox.join("config");
    let data = sandbox.join("data");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(root)
        .env("ARMADAI_CONFIG_DIR", &config)
        .env("XDG_DATA_HOME", &data)
        .args(["run", agent, input])
        .output()
        .unwrap()
}

#[test]
fn a_chain_link_that_fails_to_load_reports_why_not_just_what() {
    // `- temperature: warm` fails deep in the parser: `invalid temperature`
    // wrapping `invalid float literal`. The outer layer alone names the field
    // but not what was wrong with it — and a user who can already see
    // `temperature: warm` in the file learns nothing from it.
    let (_dir, root) = project(&[], &["m4-head", "m4-broken"]);
    break_file_agent(&root, "m4-broken", "- temperature: warm");

    let single = run_single_human(&root, "m4-broken", "TASK-CAUSE-CHAIN");
    let single_err = String::from_utf8_lossy(&single.stderr).to_string();
    assert!(
        single_err.contains("invalid temperature") && single_err.contains("invalid float literal"),
        "fixture check: the single-agent path must report both layers, otherwise this \
         test cannot tell a flattened chain from an already-flat one, got: {single_err}"
    );

    let out = run_pipe_human(&root, "m4-head", "TASK-CAUSE-CHAIN", &["m4-broken"]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a chain whose second link cannot be parsed must fail.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("m4-broken") && stderr.contains("2/2"),
        "the failure must still place and name the bad link, got: {stderr}"
    );
    assert!(
        stderr.contains("invalid temperature"),
        "the failure must carry the resolver's own message, got: {stderr}"
    );
    assert!(
        stderr.contains("invalid float literal"),
        "the failure must carry the ROOT cause too, not only the outermost layer: \
         positioning the bad link inlines the resolver's message into a new error, and a \
         non-alternate `{{e}}` there silently drops everything under it — the same \
         `armadai run m4-broken` prints under `Caused by:`, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// #373 review, i3: the up-front pass validates the provider too, not just the
// agent definition — otherwise #366's own bill survives one gate further on.
// ---------------------------------------------------------------------------

#[test]
fn pipe_builds_every_link_provider_before_running_the_first() {
    // `create_provider` fails deterministically on the agent's own metadata,
    // with nothing an earlier link could have changed. Resolving definitions
    // up front but leaving provider construction inside `run_single_agent`
    // meant a misspelled `provider:` on link 2 was discovered only after link
    // 1 had run: measured as `agent_start`/`agent_end` for the head, then
    // `error: Unknown provider: 'gtp'` — a billed call on a chain that could
    // never complete, which is the exact defect #366 is about.
    let (_dir, root) = project(&[], &["m5-head", "m5-typo-prov", "m5-tail"]);
    break_file_agent(&root, "m5-typo-prov", "- provider: gtp");

    let out = run_pipe(
        &root,
        "m5-head",
        "TASK-BAD-PROVIDER",
        &["m5-typo-prov", "m5-tail"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !out.status.success(),
        "a chain naming an unconstructible provider must fail.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        agents_started(&stdout),
        Vec::<String>::new(),
        "NO agent may start when a later link's provider cannot be built — the failure \
         is decided by that agent's own metadata, so nothing the head produces could \
         have changed it, and running the head only bills a chain that cannot \
         complete.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("m5-typo-prov") && combined.contains("2/3"),
        "the failure must name and place the offending link — `Unknown provider: 'gtp'` \
         on its own names neither, got: {combined}"
    );
    assert!(
        combined.contains("Unknown provider"),
        "the failure must still carry the provider factory's own reason, got: {combined}"
    );
}

// ---------------------------------------------------------------------------
// #373 review, m5: a per-link warning is printed under its own link's header,
// and a project-wide one is printed once.
// ---------------------------------------------------------------------------

#[test]
fn a_project_wide_load_warning_is_printed_once_under_the_first_link() {
    // An unparsable `.armadai/agents.yaml` makes `load_agent_by_name` return
    // `DeclarationsUnreadable` for EVERY link that falls back to a file —
    // ~380 characters restating one project fact, differing only in the
    // served agent's name at the very end, and byte-identical whenever two
    // links name the same agent. Resolving the chain up front also moved the
    // whole block ahead of `--- [1/3 …] ---`, so nothing said which link any
    // of them was about.
    let (_dir, root) = project(&[], &["m6-one", "m6-two"]);
    std::fs::write(
        root.join(".armadai/agents.yaml"),
        "agents:\n  - name: [unterminated\n",
    )
    .unwrap();

    let out = run_pipe_human(&root, "m6-one", "TASK-ONE-WARNING", &["m6-two", "m6-one"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "an unparsable declarations file must not fail a chain of file-backed agents.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );

    let lines: Vec<&str> = stderr.lines().collect();
    let warnings: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("ignoring unparsable"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "the unparsable declarations file is one project fact, not one per link: three \
         links restated it three times, twice byte for byte. Got {} line(s): {:#?}",
        warnings.len(),
        warnings.iter().map(|&i| lines[i]).collect::<Vec<_>>()
    );

    let first_header = lines
        .iter()
        .position(|l| l.contains("[1/3"))
        .unwrap_or_else(|| panic!("no chain header on stderr: {stderr}"));
    assert!(
        warnings[0] > first_header,
        "the warning must sit UNDER the header of the link it is about, not in a block \
         ahead of the whole chain — with the header above it, the trailing agent name is \
         what anchors it. Got the warning at line {} and `[1/3 …]` at line {}: {stderr}",
        warnings[0],
        first_header
    );
}
