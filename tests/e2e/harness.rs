//! Generic e2e runner: for a given [`CaseFile`], build an isolated tempdir project
//! (`armadai.yaml` + `agents/*.md`), shadow the `claude` CLI with the `fake-claude`
//! stub (Task 1) on `PATH`, invoke the real `armadai run …` binary against it, and
//! evaluate the result via `runner::evaluate`.
//!
//! # Isolation
//!
//! Every run gets its own `tempfile::tempdir()`. `HOME`, `XDG_CONFIG_HOME`,
//! `XDG_DATA_HOME` are all pointed inside that tempdir (see `src/core/config.rs`
//! `config_dir()`/`data_dir()`), so the harness never reads or writes the real
//! user's `~/.config/armadai/` or `~/.local/share/armadai/armadai.sqlite`. This is
//! the load-bearing property of the whole suite — a bug here would mean e2e cases
//! quietly corrupt the developer's actual ArmadAI config/history.

use std::path::Path;

use assert_cmd::Command;

use super::case::CaseFile;
use super::runner::{self, CaseOutcome};

/// Run one case end-to-end and return its outcome. Never panics: any setup failure
/// (writing the tempdir project, spawning `armadai`) is reported as a failed
/// [`CaseOutcome`] with a diagnostic in `diffs`, so a bad case can't take down a
/// whole suite run.
pub fn run_case(case: &CaseFile) -> CaseOutcome {
    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => return setup_failure(case, format!("tempdir() failed: {e}")),
    };
    let root = tmp.path();

    if case.setup.agents.is_empty() {
        return setup_failure(case, "setup.agents is empty — nothing to run".to_string());
    }

    if let Err(e) = write_project(root, case) {
        return setup_failure(case, format!("writing tempdir project failed: {e:#}"));
    }

    let state_dir = root.join("state");
    if let Err(e) = std::fs::create_dir_all(&state_dir) {
        return setup_failure(case, format!("creating FAKE_STATE_DIR failed: {e}"));
    }

    let mut cmd = match Command::cargo_bin("armadai") {
        Ok(c) => c,
        Err(e) => return setup_failure(case, format!("Command::cargo_bin(\"armadai\"): {e}")),
    };

    configure_invocation(&mut cmd, root, &state_dir, case);

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => return setup_failure(case, format!("spawning armadai failed: {e}")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit = output.status.code().unwrap_or(-1);

    let mut outcome = runner::evaluate(case, &stdout, exit);

    let storage_diffs = runner::check_storage(&case.expect.storage, root);
    if !storage_diffs.is_empty() {
        outcome.diffs.extend(storage_diffs);
        outcome.passed = false;
    }

    if !outcome.passed {
        outcome
            .diffs
            .push(format!("--- stderr ---\n{}", stderr.trim_end()));
    }

    outcome
}

/// Build a [`CaseOutcome`] for a failure that happened before `armadai` could even be
/// invoked (tempdir/IO/spawn errors) — distinct from an `evaluate` mismatch, but
/// reported the same way so callers don't need to special-case it.
fn setup_failure(case: &CaseFile, reason: String) -> CaseOutcome {
    CaseOutcome {
        name: case.name.clone(),
        weight: case.weight,
        allow_fail: case.allow_fail,
        passed: false,
        diffs: vec![reason],
        ..Default::default()
    }
}

/// Set up the `armadai` process invocation: working directory, isolation env vars,
/// and the `run <agent> <input> [flags…]` argument line for `case.setup.pattern`.
fn configure_invocation(cmd: &mut Command, root: &Path, state_dir: &Path, case: &CaseFile) {
    let path_var = std::env::var("PATH").unwrap_or_default();

    cmd.current_dir(root)
        // Shadow `claude` with `fake-claude` (see `write_project`'s `<root>/bin/claude`
        // symlink) by putting our stub dir first on PATH.
        .env("PATH", format!("{}/bin:{path_var}", root.display()))
        .env("FAKE_SCENARIO", root.join("scenario.yaml"))
        .env("FAKE_STATE_DIR", state_dir)
        // Isolation (see module doc): never touch the real user config/DB.
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join(".config"))
        .env("XDG_DATA_HOME", root.join(".local/share"))
        // `config_dir()` in src/core/config.rs checks this ahead of XDG/HOME — clear
        // it in case the harness itself is invoked from an environment that sets it,
        // so isolation can't be silently bypassed.
        .env_remove("ARMADAI_CONFIG_DIR");

    let coordinator = &case.setup.agents[0];
    let rest: Vec<&str> = case.setup.agents[1..].iter().map(String::as_str).collect();

    cmd.arg("run").arg(coordinator).arg(&case.setup.input);

    match case.setup.pattern.as_str() {
        // Hierarchical topology comes from `armadai.yaml`'s `orchestration:` block
        // (see `write_project`/`hierarchical_yaml`) and is auto-detected by
        // `armadai run` — no `--orchestrate`/`--pipe` needed on the CLI.
        "hierarchical" => {}
        // `--orchestrate` only accepts blackboard|ring (see `src/cli/mod.rs`
        // `Command::Run`); the remaining agents ride along via `--pipe`.
        "blackboard" | "ring" => {
            if !rest.is_empty() {
                cmd.arg("--pipe").args(&rest);
            }
            cmd.arg("--orchestrate").arg(&case.setup.pattern);
        }
        // "direct" (or anything else): plain sequential chain, no orchestration.
        _ => {
            if !rest.is_empty() {
                cmd.arg("--pipe").args(&rest);
            }
        }
    }

    for flag in &case.setup.flags {
        cmd.arg(flag);
    }
}

/// Write the tempdir project: `armadai.yaml`, one `agents/<name>.md` per
/// `setup.agents`, the `fake-claude` scenario, and the `claude` → `fake-claude`
/// shadow symlink.
fn write_project(root: &Path, case: &CaseFile) -> anyhow::Result<()> {
    use anyhow::Context;

    std::fs::create_dir_all(root.join("agents")).context("creating agents/ dir")?;
    std::fs::write(root.join("armadai.yaml"), project_yaml(case))
        .context("writing armadai.yaml")?;

    for agent in &case.setup.agents {
        let path = root.join("agents").join(format!("{agent}.md"));
        std::fs::write(&path, agent_markdown(agent))
            .with_context(|| format!("writing {}", path.display()))?;
    }

    let scenario_yaml =
        serde_yaml_ng::to_string(&case.fake).context("serializing fake scenario to YAML")?;
    std::fs::write(root.join("scenario.yaml"), scenario_yaml).context("writing scenario.yaml")?;

    let bin_dir = root.join("bin");
    std::fs::create_dir_all(&bin_dir).context("creating bin/ dir")?;
    let claude_stub = bin_dir.join("claude");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_fake-claude"), &claude_stub)
        .context("symlinking bin/claude -> fake-claude")?;

    Ok(())
}

/// Render `armadai.yaml` for `case`. The `agents:` list is always required (an empty
/// list makes `armadai` fall back to the global agent library instead of resolving
/// our tempdir's `agents/*.md` — see `resolve_agents_dir` in `src/cli/run.rs`).
///
/// Only `hierarchical` gets a top-level `orchestration:` block: it's the only
/// pattern selected via project config rather than a `--orchestrate` CLI flag (see
/// `configure_invocation`). See `OrchestrationConfig`/`TeamConfig` in
/// `src/core/orchestration/mod.rs` for the schema this must match.
///
/// When `case.setup.nested_team` is set (C9), the single `teams:` entry declares
/// that team's `lead`/`pattern`/`agents` instead of the default flat team of every
/// non-coordinator agent — see `NestedTeamSetup` in `case.rs`.
fn project_yaml(case: &CaseFile) -> String {
    let agents_block: String = case
        .setup
        .agents
        .iter()
        .map(|a| format!("  - name: {a}\n"))
        .collect();

    let mut yaml = format!("agents:\n{agents_block}");

    if case.setup.pattern == "hierarchical" {
        let coordinator = &case.setup.agents[0];
        let teams_block = match &case.setup.nested_team {
            Some(nt) => {
                let members_block: String = nt
                    .agents
                    .iter()
                    .map(|a| format!("        - {a}\n"))
                    .collect();
                format!(
                    "    - lead: {}\n      \
                       pattern: {}\n      \
                       agents:\n{members_block}",
                    nt.lead, nt.pattern
                )
            }
            None => {
                let team_agents = &case.setup.agents[1..];
                let team_agents_block: String = team_agents
                    .iter()
                    .map(|a| format!("        - {a}\n"))
                    .collect();
                format!("    - agents:\n{team_agents_block}")
            }
        };
        yaml.push_str(&format!(
            "orchestration:\n  \
             enabled: true\n  \
             pattern: hierarchical\n  \
             coordinator: {coordinator}\n  \
             teams:\n{teams_block}"
        ));
    }

    // OH1 Lot 4 Task 4: `defaults.orchestration.token_budget` is the key
    // `blackboard`/`ring` actually read (`OrchestrationDefaults`, via
    // `apply_blackboard_overrides`/`apply_ring_overrides` in
    // `src/cli/run.rs`) — distinct from the top-level `orchestration:` block
    // above, which only feeds `hierarchical`. Emitted as its own top-level
    // `defaults:` key regardless of `case.setup.pattern`, so it works
    // whichever orchestrated pattern the case is exercising.
    if let Some(budget) = case.setup.token_budget {
        yaml.push_str(&format!(
            "defaults:\n  \
             orchestration:\n    \
             token_budget: {budget}\n"
        ));
    }

    yaml
}

/// A minimal, valid agent Markdown file: H1 + `## Metadata` + `## System Prompt`
/// (the three sections `src/parser/markdown.rs::parse_agent_file` requires). Uses a
/// concrete `model:` (never `latest:auto`) so the run never touches the dynamic
/// model router/registry — the point of this harness is to exercise the fake
/// provider deterministically, not model resolution. The `FAKE_AGENT_ID:` line in
/// the system prompt is how `fake-claude` identifies which agent is calling it (see
/// `src/bin/fake-claude.rs::agent_id_from_prompt`).
fn agent_markdown(agent: &str) -> String {
    format!(
        "# {agent}\n\
         \n\
         ## Metadata\n\
         - provider: claude\n\
         - model: fake-model\n\
         \n\
         ## System Prompt\n\
         FAKE_AGENT_ID: {agent}\n\
         \n\
         You are `{agent}`, a deterministic test agent used by the e2e harness. Respond \
         concisely to the task given.\n"
    )
}

/// Load every `*.yaml` case file under `dir`, sorted by filename for a deterministic
/// run order (and therefore a deterministic report). Used by `e2e_suite` to discover
/// `tests/e2e/cases/*.yaml`.
///
/// # Panics
/// Panics if `dir` cannot be read, or if any file fails to parse as a [`CaseFile`] —
/// a malformed baseline case file is a harness bug, not a runtime condition to
/// tolerate silently.
pub fn discover_cases(dir: &str) -> Vec<CaseFile> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading case dir {dir}: {e}"))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();

    paths
        .iter()
        .map(|p| {
            super::case::load_case(p)
                .unwrap_or_else(|e| panic!("loading case file {}: {e:#}", p.display()))
        })
        .collect()
}

/// Directory the e2e suite writes its `e2e-report.{json,html}` artifacts to
/// (created if missing). Kept under `target/` so it's ignored by git and already
/// present for CI's `actions/upload-artifact` step to pick up.
pub fn report_dir() -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/e2e-report");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::case::load_case_str;

    /// Discovers every case file under `tests/e2e/cases/`, runs each one against
    /// the real `armadai` binary + `fake-claude` stub, and writes the aggregate
    /// report (`e2e-report.json`/`.html`) to `report_dir()` — `if: always()` in CI
    /// uploads it regardless of the outcome below, so a failing run still leaves a
    /// diagnostic artifact behind.
    ///
    /// A case fails the suite unless it is marked `allow_fail: true` in its YAML
    /// (see e.g. `cases/ring.yaml`, which documents a known legacy-engine gap).
    #[test]
    fn e2e_suite() {
        let cases = discover_cases(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/e2e/cases"));
        assert!(
            !cases.is_empty(),
            "no case files found under tests/e2e/cases — the suite would vacuously pass"
        );

        let outcomes: Vec<_> = cases.iter().map(run_case).collect();

        crate::e2e::report::write_reports(&outcomes, &report_dir())
            .expect("writing e2e-report.{json,html}");

        let failed: Vec<&str> = outcomes
            .iter()
            .filter(|o| !o.passed && !o.allow_fail)
            .map(|o| o.name.as_str())
            .collect();
        assert!(
            failed.is_empty(),
            "failed cases (not allow_fail): {failed:?} — see {}/e2e-report.html for diffs",
            report_dir().display()
        );
    }

    /// A single-agent, `direct`-pattern case: no orchestration, no `--pipe`. This is
    /// the harness's own smoke test — the first real run of `armadai` against
    /// `fake-claude` end to end. If agent identity, the composed prompt, or the
    /// stream-json shape don't line up, this is what catches it.
    const DIRECT_YAML: &str = r#"
name: direct-happy
weight: 5
setup: { pattern: direct, agents: [t-writer], flags: ["--json"], input: "hi" }
fake:
  rules:
    - match: { agent: t-writer }
      respond: "done"
    - match: {}
      respond: "unexpected — catch-all should never be hit in this case"
expect:
  exit_code: 0
  events:
    - { t: run_start }
    - { t: agent_start, agent: t-writer }
    - { t: result }
  event_counts: { agent_start: 1, agent_end: 1 }
  invariants: [agent_start_end_symmetric, prov_model_non_empty, single_result]
"#;

    #[test]
    fn direct_baseline_passes_end_to_end() {
        let case = load_case_str(DIRECT_YAML).expect("DIRECT_YAML parses as a CaseFile");
        let outcome = run_case(&case);
        assert!(outcome.passed, "diffs: {:?}", outcome.diffs);
    }

    #[test]
    fn missing_agents_reports_a_diagnostic_instead_of_panicking() {
        let case = load_case_str(
            r#"
name: empty-agents
weight: 1
setup: { pattern: direct, agents: [], flags: [], input: "" }
fake: { rules: [ { match: {}, respond: "ok" } ] }
expect: { exit_code: 0 }
"#,
        )
        .unwrap();
        let outcome = run_case(&case);
        assert!(!outcome.passed);
        assert!(outcome.diffs.iter().any(|d| d.contains("setup.agents")));
    }

    #[test]
    fn project_yaml_emits_agents_list_for_direct() {
        let case = load_case_str(
            r#"
name: t
weight: 1
setup: { pattern: direct, agents: [a, b], flags: [], input: "" }
fake: { rules: [ { match: {}, respond: "ok" } ] }
expect: { exit_code: 0 }
"#,
        )
        .unwrap();
        let yaml = project_yaml(&case);
        assert!(yaml.contains("agents:"));
        assert!(yaml.contains("- name: a"));
        assert!(yaml.contains("- name: b"));
        assert!(!yaml.contains("orchestration:"));
    }

    #[test]
    fn project_yaml_emits_orchestration_block_for_hierarchical() {
        let case = load_case_str(
            r#"
name: t
weight: 1
setup: { pattern: hierarchical, agents: [coord, lead-a], flags: [], input: "" }
fake: { rules: [ { match: {}, respond: "ok" } ] }
expect: { exit_code: 0 }
"#,
        )
        .unwrap();
        let yaml = project_yaml(&case);
        assert!(yaml.contains("enabled: true"));
        assert!(yaml.contains("pattern: hierarchical"));
        assert!(yaml.contains("coordinator: coord"));
        assert!(yaml.contains("- lead-a"));
    }

    #[test]
    fn agent_markdown_has_required_sections_and_marker() {
        let md = agent_markdown("t-writer");
        assert!(md.starts_with("# t-writer"));
        assert!(md.contains("## Metadata"));
        assert!(md.contains("provider: claude"));
        assert!(md.contains("model: fake-model"));
        assert!(md.contains("## System Prompt"));
        assert!(md.contains("FAKE_AGENT_ID: t-writer"));
    }
}
