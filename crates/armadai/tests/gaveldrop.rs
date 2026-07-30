#![cfg(feature = "e2e-fake")]
//! `Armadai` adapter for the `gaveldrop` YAML e2e engine.
//!
//! `gaveldrop` owns isolation (`Isolation`), discovery/scheduling (`Config`) and
//! verdict evaluation (`verdict::evaluate`) — this file only teaches it how to
//! *invoke* an armadai project: write a temp project (`armadai.yaml` +
//! `agents/*.md` + the `fake-claude` scenario) and run the real `armadai` binary
//! against it. See `.superpowers/sdd/2026-07-30-gaveldrop-migration/` for the
//! migration plan this task (T4) belongs to.
//!
//! # Two invocation branches, one exit
//!
//! [`Armadai::invoke`] has two branches — a conformance probe (`setup.probe_script`,
//! used by the gaveldrop conformance kit to certify isolation itself, with no
//! armadai project involved) and the real armadai run (`setup.pattern` + friends).
//! Both funnel through [`run_in_iso`], the single place that actually applies the
//! isolation's environment and spawns a process. Splitting that into two code
//! paths would make the conformance kit's certification vacuous for the branch
//! real armadai cases take — see the plan's F3.

use std::path::{Path, PathBuf};
use std::process::Command;

use gaveldrop::adapters::{Adapter, AdapterError};
use gaveldrop::case::Case;
use gaveldrop::iso::Isolation;
use gaveldrop::observations::Observations;
use serde_json::Value;

/// The single exit both branches of [`Armadai::invoke`] funnel through.
///
/// Applies the isolation's environment (plus any adapter-owned `extra_env`), clears
/// what isolation asked to clear, runs `argv` with the isolated root as the working
/// directory, and reads back stdout/stderr/exit plus the call journal and file
/// effects. The conformance-probe branch passes an empty `extra_env`; the real
/// armadai branch passes `ARMADAI_FAKE_SCENARIO` pointing `fake-claude` at the
/// scenario written for the case.
fn run_in_iso(
    iso: &Isolation,
    argv: &[String],
    extra_env: &[(&str, PathBuf)],
) -> Result<Observations, AdapterError> {
    let (program, arguments) = argv
        .split_first()
        .expect("argv must not be empty — callers always prepend a program name");

    let mut cmd = Command::new(program);
    cmd.args(arguments).current_dir(iso.root());
    for (key, value) in iso.env() {
        cmd.env(key, value);
    }
    for key in iso.cleared() {
        cmd.env_remove(key);
    }
    for (key, value) in extra_env {
        cmd.env(key, value);
    }

    let output = cmd.output().map_err(|source| AdapterError::Spawn {
        program: program.clone(),
        source,
    })?;

    Ok(Observations {
        exit: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        calls: gaveldrop_fake::Journal::read(&iso.journal_path())?,
        events: Vec::new(),
        files: iso.changes(),
        ..Observations::default()
    })
}

/// Reads a string from `case.setup.extra`, absent when the key is missing or not a string.
fn str_field<'a>(case: &'a Case, key: &str) -> Option<&'a str> {
    case.setup.extra.get(key).and_then(Value::as_str)
}

/// Reads a list of strings from `case.setup.extra`, empty when the key is missing.
fn arr_field(case: &Case, key: &str) -> Vec<String> {
    case.setup
        .extra
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// `setup.nested_team` (C9): the sub-team a `hierarchical` case's lead runs instead of flat
/// delegation. Ported from the old typed `NestedTeamSetup` (`tests/e2e/case.rs`, deleted in
/// T3) — see `project_yaml`.
#[derive(Debug, serde::Deserialize)]
struct NestedTeamSetup {
    lead: String,
    pattern: String,
    agents: Vec<String>,
}

/// Builds an [`AdapterError`] from an IO failure that isn't a process spawn.
///
/// `AdapterError` has no generic IO variant (only `Spawn`, which names a program rather than a
/// file) — `Unsupported` is the closest fit for "this case's project could not be written",
/// mirroring how `Process` reports a case it cannot run at all.
fn io_err(case: &Case, action: &str, source: std::io::Error) -> AdapterError {
    AdapterError::Unsupported {
        case: case.name.clone(),
        reason: format!("{action}: {source}"),
    }
}

/// Writes the tempdir project (`armadai.yaml`, one `agents/<name>.md` per `setup.agents`) and
/// the `fake-claude` scenario, returning the scenario's path.
///
/// Ported from the old harness's `write_project` (`tests/e2e/harness.rs:147-171`, deleted in
/// T3), minus the `bin/claude` symlink — gaveldrop's own `Isolation::prepare` already shadows
/// `claude` with the fake binary via this project's `gaveldrop.yaml` (`fake.bins: [claude]`).
fn write_project(case: &Case, iso: &Isolation) -> Result<PathBuf, AdapterError> {
    let root = iso.root();
    let agents = arr_field(case, "agents");

    std::fs::create_dir_all(root.join("agents"))
        .map_err(|e| io_err(case, "creating agents/ dir", e))?;
    std::fs::write(root.join("armadai.yaml"), project_yaml(case))
        .map_err(|e| io_err(case, "writing armadai.yaml", e))?;

    for agent in &agents {
        let path = root.join("agents").join(format!("{agent}.md"));
        std::fs::write(&path, agent_markdown(agent))
            .map_err(|e| io_err(case, &format!("writing {}", path.display()), e))?;
    }

    let scenario_value = case
        .setup
        .extra
        .get("scenario")
        .cloned()
        .unwrap_or(Value::Null);
    let scenario: armadai_fake::Scenario =
        serde_json::from_value(scenario_value).map_err(|e| AdapterError::Unsupported {
            case: case.name.clone(),
            reason: format!("setup.scenario does not match armadai_fake::Scenario: {e}"),
        })?;
    let scenario_yaml =
        serde_yaml_ng::to_string(&scenario).map_err(|e| AdapterError::Unsupported {
            case: case.name.clone(),
            reason: format!("serializing setup.scenario to YAML: {e}"),
        })?;

    let scenario_path = root.join("armadai-scenario.yaml");
    std::fs::write(&scenario_path, scenario_yaml)
        .map_err(|e| io_err(case, "writing armadai-scenario.yaml", e))?;

    Ok(scenario_path)
}

/// Renders `armadai.yaml` for `case`. The `agents:` list is always required (an empty list
/// makes `armadai` fall back to the global agent library instead of resolving this project's
/// `agents/*.md` — see `resolve_agents_dir` in `src/cli/run.rs`).
///
/// Only `hierarchical` gets a top-level `orchestration:` block: it's the only pattern selected
/// via project config rather than a `--orchestrate` CLI flag (see `build_command`). When
/// `setup.nested_team` is set (C9), the single `teams:` entry declares that team's
/// `lead`/`pattern`/`agents` instead of the default flat team of every non-coordinator agent.
///
/// Ported verbatim (module-doc caveats included) from the old harness's `project_yaml`
/// (`tests/e2e/harness.rs:185-245`, deleted in T3), reading `setup.extra` instead of a typed
/// `Setup`.
fn project_yaml(case: &Case) -> String {
    let agents = arr_field(case, "agents");
    let pattern = str_field(case, "pattern").unwrap_or_default();

    let agents_block: String = agents.iter().map(|a| format!("  - name: {a}\n")).collect();
    let mut yaml = format!("agents:\n{agents_block}");

    if pattern == "hierarchical" {
        let coordinator = &agents[0];
        let nested_team: Option<NestedTeamSetup> = case
            .setup
            .extra
            .get("nested_team")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .unwrap_or(None);

        let teams_block = match nested_team {
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
                let team_agents = &agents[1..];
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

    // `defaults.orchestration.token_budget` is the key `blackboard`/`ring` actually read (via
    // `OrchestrationDefaults`/`apply_blackboard_overrides`/`apply_ring_overrides` in
    // `src/cli/run.rs`) — distinct from the top-level `orchestration:` block above, which only
    // feeds `hierarchical`. Emitted regardless of `pattern`, so it works whichever orchestrated
    // pattern the case is exercising.
    if let Some(budget) = case.setup.extra.get("token_budget").and_then(Value::as_u64) {
        yaml.push_str(&format!(
            "defaults:\n  \
             orchestration:\n    \
             token_budget: {budget}\n"
        ));
    }

    yaml
}

/// A minimal, valid agent Markdown file: H1 + `## Metadata` + `## System Prompt` (the three
/// sections `src/parser/markdown.rs::parse_agent_file` requires). Uses a concrete `model:`
/// (never `latest:auto`) so the run never touches the dynamic model router/registry. Ported
/// verbatim from the old harness (`tests/e2e/harness.rs:254-268`, deleted in T3).
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

/// Builds the `armadai run …` argv for `case`.
///
/// Ported from the old harness's `configure_invocation` flag logic (`tests/e2e/harness.rs:
/// 113-141`, deleted in T3): `--pipe <rest>` (when there is more than one agent) comes BEFORE
/// `--orchestrate <pattern>` for `blackboard`/`ring`; `hierarchical` gets neither (its topology
/// comes from `armadai.yaml`'s `orchestration:` block, see `project_yaml`); anything else
/// ("direct") is a plain sequential `--pipe` chain with no orchestration flag at all.
fn build_command(case: &Case) -> Vec<String> {
    let agents = arr_field(case, "agents");
    let coordinator = agents.first().cloned().unwrap_or_default();
    let rest: Vec<String> = if agents.len() > 1 {
        agents[1..].to_vec()
    } else {
        Vec::new()
    };
    let input = str_field(case, "input").unwrap_or_default().to_string();
    let pattern = str_field(case, "pattern").unwrap_or_default();

    let mut argv = vec![
        env!("CARGO_BIN_EXE_armadai").to_string(),
        "run".to_string(),
        coordinator,
        input,
    ];

    match pattern {
        "hierarchical" => {}
        "blackboard" | "ring" => {
            if !rest.is_empty() {
                argv.push("--pipe".to_string());
                argv.extend(rest.iter().cloned());
            }
            argv.push("--orchestrate".to_string());
            argv.push(pattern.to_string());
        }
        _ => {
            if !rest.is_empty() {
                argv.push("--pipe".to_string());
                argv.extend(rest.iter().cloned());
            }
        }
    }

    argv.extend(arr_field(case, "flags"));
    argv
}

/// Invokes armadai (or, for a conformance probe, a bare shell script) against a
/// `gaveldrop`-prepared isolation.
struct Armadai;

impl Adapter for Armadai {
    fn claims(&self, case: &Case) -> bool {
        case.setup.extra.contains_key("pattern")
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        // Conformance-probe branch: `setup.probe_script` means the conformance kit is
        // certifying the isolation contract itself, not running a fleet. Same `run_in_iso`
        // exit, no extra env — see the module doc.
        if let Some(script) = str_field(case, "probe_script") {
            return run_in_iso(
                iso,
                &["sh".to_string(), "-c".to_string(), script.to_string()],
                &[],
            );
        }

        // Real branch: write the project + scenario, point fake-claude at it, build the
        // `armadai run …` argv, run through the SAME helper.
        let scenario_path = write_project(case, iso)?;
        let argv = build_command(case);
        run_in_iso(iso, &argv, &[(armadai_fake::SCENARIO_ENV, scenario_path)])
    }
}

#[test]
fn config_loads() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let cfg = gaveldrop::config::Config::load(&root.join("gaveldrop.yaml")).expect("parses");
    assert_eq!(cfg.events.as_ref().unwrap().type_field, "t");
    assert_eq!(cfg.invariants.len(), 5);
}

#[test]
fn all_cases_load() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/cases");
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "yaml") {
            gaveldrop::case::Case::load(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            n += 1;
        }
    }
    assert_eq!(n, 9, "expected 9 migrated cases");
}

/// Builds a [`Case`] that [`Armadai`] claims but the built-in `Process` adapter does not.
///
/// `pattern` makes [`Armadai::claims`] return `true`; the script itself lives in
/// `extra["probe_script"]` rather than `setup.run`, so `Process` — which only claims cases
/// carrying a `run:` command line — never also claims it. Passed to
/// [`gaveldrop_conformance::run_with`] as the `Invocation` factory: the kit's checks describe a
/// behaviour ("exit 7", "write to this file"), and this is how that behaviour becomes something
/// `Armadai::invoke`'s conformance-probe branch (see its module doc) will actually run.
fn as_armadai_probe(script: &str) -> Case {
    let mut extra = std::collections::BTreeMap::new();
    extra.insert("pattern".to_string(), serde_json::json!("conformance"));
    extra.insert("probe_script".to_string(), serde_json::json!(script));
    Case {
        name: "conformance".to_string(),
        weight: 1,
        allow_fail: false,
        setup: gaveldrop::case::Setup {
            run: None,
            exec: None,
            env: std::collections::BTreeMap::new(),
            extra,
        },
        fake: None,
        expect: gaveldrop::case::Expect::default(),
        steps: Vec::new(),
    }
}

/// Certifies that [`Armadai`] honours gaveldrop's isolation contract.
///
/// Runs the same battery the kit runs against its own built-in `Process`/`Shell` adapters
/// (`gaveldrop-conformance/tests/shell.rs`), through [`as_armadai_probe`] since `Armadai` claims
/// cases by `pattern` rather than by `run:`. `fake-claude` — the conformance fake, per
/// `armadai-fake`'s doc comment on `SCENARIO_ENV` — journals a catch-all and exits 127 when
/// `ARMADAI_FAKE_SCENARIO` is unset, which is exactly what the kit's catch-all check needs.
#[test]
fn armadai_adapter_is_conformant() {
    let fake = Path::new(env!("CARGO_BIN_EXE_fake-claude"));
    let report = gaveldrop_conformance::run_with(&Armadai, fake, &as_armadai_probe);
    assert!(report.is_conformant(), "\n{}", report.render());
}

/// Runs the full 9-case suite through `gaveldrop::runner::run_all_with`, the same entry point
/// `armadai`'s own e2e binary would use in production, with the `Armadai` adapter prepended to
/// gaveldrop's built-in registry. This is the decisive migration gate: it proves the 9 cases
/// reach the SAME verdicts the old hand-rolled harness produced (deleted in T3), now evaluated
/// entirely by gaveldrop's `verdict::evaluate` instead of bespoke assertion code.
#[test]
fn e2e_suite_passes_through_gaveldrop() {
    use gaveldrop::adapters::{self, Adapter};
    use gaveldrop::report::terminal::Terminal;

    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let config = gaveldrop::config::Config::load(&root.join("gaveldrop.yaml")).unwrap();
    let fake = Path::new(env!("CARGO_BIN_EXE_fake-claude"));

    let mut chain: Vec<Box<dyn Adapter>> = vec![Box::new(Armadai)];
    chain.extend(adapters::registry());

    let mut sink = Terminal::plain(std::io::stdout());
    let report =
        gaveldrop::runner::run_all_with(&config, root, fake, &mut sink, None, None, &chain)
            .unwrap();
    assert!(
        report.is_success(),
        "{} case(s) failed",
        report.summary().failed
    );
}

#[cfg(test)]
mod adapter_tests {
    use super::*;

    fn case(yaml: &str) -> Case {
        Case::load_str(yaml, Path::new("inline")).unwrap()
    }

    #[test]
    fn claims_is_true_iff_pattern_is_present() {
        let with_pattern = case("name: t\nweight: 1\nsetup: { pattern: direct }\nexpect: {}\n");
        let without_pattern = case("name: t\nweight: 1\nsetup: { run: [\"true\"] }\nexpect: {}\n");

        assert!(Armadai.claims(&with_pattern));
        assert!(!Armadai.claims(&without_pattern));
    }

    #[test]
    fn build_command_puts_pipe_before_orchestrate_for_ring() {
        let case = case(
            "name: t\nweight: 1\nsetup: { pattern: ring, agents: [a, b, c], input: hi }\nexpect: {}\n",
        );
        let argv = build_command(&case);

        let pipe_at = argv
            .iter()
            .position(|a| a == "--pipe")
            .expect("--pipe must be present for a multi-agent ring");
        let orchestrate_at = argv
            .iter()
            .position(|a| a == "--orchestrate")
            .expect("--orchestrate must be present for ring");

        assert!(
            pipe_at < orchestrate_at,
            "--pipe must come BEFORE --orchestrate: {argv:?}"
        );
        assert_eq!(argv[orchestrate_at + 1], "ring");
    }

    #[test]
    fn hierarchical_adds_no_orchestration_flags() {
        let case = case(
            "name: t\nweight: 1\nsetup: { pattern: hierarchical, agents: [coord, a], input: hi }\nexpect: {}\n",
        );
        let argv = build_command(&case);

        assert!(
            !argv.iter().any(|a| a == "--pipe" || a == "--orchestrate"),
            "hierarchical's topology comes from armadai.yaml, not CLI flags: {argv:?}"
        );
    }

    #[test]
    fn a_direct_case_gets_no_orchestration_flag_either() {
        let case = case(
            "name: t\nweight: 1\nsetup: { pattern: direct, agents: [a], input: hi }\nexpect: {}\n",
        );
        let argv = build_command(&case);

        assert!(!argv.iter().any(|a| a == "--orchestrate"));
        assert!(!argv.iter().any(|a| a == "--pipe"));
    }

    #[test]
    fn project_yaml_lists_every_agent_for_direct() {
        let case =
            case("name: t\nweight: 1\nsetup: { pattern: direct, agents: [a, b] }\nexpect: {}\n");
        let yaml = project_yaml(&case);

        assert!(yaml.contains("- name: a"));
        assert!(yaml.contains("- name: b"));
        assert!(!yaml.contains("orchestration:"));
    }

    #[test]
    fn project_yaml_emits_orchestration_block_for_hierarchical() {
        let case = case(
            "name: t\nweight: 1\nsetup: { pattern: hierarchical, agents: [coord, lead-a] }\nexpect: {}\n",
        );
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
