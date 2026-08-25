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
        .args(rest)
        .arg("--json");
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
