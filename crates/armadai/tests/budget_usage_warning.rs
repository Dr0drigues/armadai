//! Black-box regression for the notice that a configured budget is being fed
//! no usage at all (#374 review, I3).
//!
//! `token_budget`/`cost_limit` are enforced from `ExecutionState`'s counters,
//! which come from `CompletionResponse`. A provider that reports no usage —
//! the OpenAI-compatible path #368 adds reaches plenty of them, and
//! `CliProvider` does it for any plain-text CLI — leaves those counters at
//! zero, so the limit never breaches and each nested delegation is handed the
//! full original ceiling. The notice is the minimal answer: say once that the
//! budget is inoperative.
//!
//! Spawned as the real binary rather than unit-tested, because the substance
//! under test is the *wiring*: `unreported_usage_warning` is pure and has its
//! own unit tests in `armadai-core`, and a run that never calls it would keep
//! those green. `provider: cli` + `command: echo` gives a real provider
//! reporting real zeros with no API key, no network and no fake-binary
//! feature.

use assert_cmd::Command;
use std::path::Path;

/// A hierarchical project with a coordinator and one team member. `budget` is
/// spliced into the `orchestration:` block, so the same fixture serves the
/// configured and the unconfigured case — hierarchical is the only pattern
/// whose `token_budget` is genuinely optional (`blackboard`/`ring` default
/// theirs to a non-zero value, so "no budget configured" cannot exist there).
fn project(budget: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join("agents")).unwrap();

    std::fs::write(
        root.join("armadai.yaml"),
        format!(
            "agents:\n  - name: lead\n  - name: worker\n\
             orchestration:\n  enabled: true\n  pattern: hierarchical\n  \
             coordinator: lead\n  teams:\n    - agents: [worker]\n{budget}"
        ),
    )
    .unwrap();
    // `true`, not `echo`: `CliProvider` passes the composed input as the last
    // argv, and the coordinator's composed prompt contains the delegation
    // *syntax example* (`@agent-name: …`). Echoing it back makes the
    // coordinator appear to delegate to an agent called `agent-name`, and the
    // run dies before it can report anything. `true` answers with an empty
    // string, which is enough: what this file measures is the usage the
    // provider reports (zero, and reported as such), not the content.
    for agent in ["lead", "worker"] {
        std::fs::write(
            root.join(format!("agents/{agent}.md")),
            format!(
                "# {agent}\n\n## Metadata\n- provider: cli\n- command: true\n\n\
                 ## System Prompt\nYou are {agent}. Answer directly.\n"
            ),
        )
        .unwrap();
    }

    (dir, root)
}

/// `armadai run lead <input> --json` in a project whose `orchestration:`
/// block selects the hierarchical pattern (`--orchestrate` only accepts
/// `blackboard`/`ring`), with both the config dir and the data dir redirected
/// into the sandbox — else the shadowing check reads the developer's global
/// agent library and `record_run` writes their real SQLite database.
///
/// Returns stdout, having first insisted the run succeeded: a mistyped flag
/// or a missing agent would otherwise leave every "no warning was emitted"
/// assertion below trivially true.
fn run_orchestrated(root: &Path) -> String {
    let sandbox = root.parent().unwrap();
    let config = sandbox.join("config");
    let data = sandbox.join("data");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(root)
        .env("ARMADAI_CONFIG_DIR", &config)
        .env("XDG_DATA_HOME", &data)
        .args(["run", "lead", "TASK-BUDGET", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the run must succeed — the notice is a warning, not a failure\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.lines().any(|l| l.contains("\"t\":\"result\"")),
        "the run must have reached its result event\nstdout: {stdout}\nstderr: {stderr}"
    );
    stdout
}

/// Every `warning` event carrying `code`, in emission order.
fn warnings_with_code(stdout: &str, code: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["t"] == "warning" && v["code"] == code)
        .collect()
}

const CODE: &str = "budget_usage_unreported";

#[test]
fn a_budget_fed_no_usage_is_reported_exactly_once() {
    let (_dir, root) = project("  token_budget: 100000\n");
    let stdout = run_orchestrated(&root);

    assert_eq!(
        warnings_with_code(&stdout, CODE).len(),
        1,
        "expected exactly one {CODE} warning\nstdout: {stdout}"
    );
}

#[test]
fn a_cost_limit_fed_no_usage_is_reported_too() {
    let (_dir, root) = project("  cost_limit: 5.0\n");
    let stdout = run_orchestrated(&root);

    assert_eq!(
        warnings_with_code(&stdout, CODE).len(),
        1,
        "a cost_limit is as inoperative as a token_budget when usage is never \
         reported\nstdout: {stdout}"
    );
}

/// The half that keeps the notice honest: a project that configures no
/// ceiling has nothing inoperative, so it must stay silent.
#[test]
fn a_run_with_no_budget_configured_says_nothing() {
    let (_dir, root) = project("");
    let stdout = run_orchestrated(&root);

    assert!(
        warnings_with_code(&stdout, CODE).is_empty(),
        "no budget was configured, so nothing is inoperative\nstdout: {stdout}"
    );
}
