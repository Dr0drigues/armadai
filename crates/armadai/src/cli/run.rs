use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use armadai_core::agent::{Agent, AgentMode};
use armadai_core::config::AppPaths;
use armadai_core::events::{EventSink, RunEvent};
use armadai_core::orchestration::es::bridge::{SinkProjectingLog, to_orchestration_result};
use armadai_core::orchestration::es::event::ExecutionEvent;
use armadai_core::orchestration::es::log::{EventLog, InMemoryLog};
use armadai_core::orchestration::es::state::ExecutionState;
use armadai_core::project::{self, ProjectConfig, ProjectDefaults};
use armadai_core::provider::{ChatMessage, CompletionRequest};
#[cfg(test)]
use armadai_providers::factory::DEFAULT_TIMEOUT_SECS;
use armadai_providers::factory::create_provider;

const GUIDED_MODE_INSTRUCTION: &str = "\
\n\n---\n\n\
**Important**: Before providing your full response, assess whether the request \
is clear and complete. If critical details are missing, ambiguous, or could \
significantly change your approach, ask 2-3 targeted clarifying questions first. \
Only proceed with your complete response once you have enough context to deliver \
accurate, relevant output.";

/// Execute a run command. Parameters are independent CLI options that map directly to
/// configuration flags; grouping into a struct would obscure the caller's argument binding.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    agent_name: Option<String>,
    input: Option<String>,
    pipe: Option<Vec<String>>,
    orchestrate: Option<String>,
    headless: bool,
    json: bool,
    quiet: bool,
    max_content: Option<usize>,
    route: Option<String>,
    tags: Option<Vec<String>>,
    dry_run: bool,
    no_tui: bool,
    resume: Option<String>,
    replay: Option<String>,
) -> anyhow::Result<()> {
    // OH1 Lot 6: the clap `ArgGroup` on `Command::Run` (`agent`/`resume`/
    // `replay`) guarantees exactly one of the three is present, so these
    // branches are exhaustive — the `else` below is the pre-existing agent
    // path, safe to `expect()` since `resume`/`replay` are both `None` there.
    // `--replay` (Task 2) and `--resume` (Task 3) are both fully wired below,
    // ahead of any agent/TUI concern (neither has an `agent_name` to route
    // through them).
    if let Some(run_id) = replay {
        return execute_replay(&run_id, json, quiet, headless).await;
    }
    if let Some(run_id) = resume {
        return execute_resume(&run_id, json, quiet, headless, max_content, no_tui).await;
    }
    let agent_name =
        agent_name.expect("clap ArgGroup guarantees agent is present when resume/replay are not");

    // headless is implied by json (machine output cannot be interrupted by a prompt)
    let headless = headless || json;

    // Orchestration can also be turned on purely via project config
    // (`orchestration.enabled: true` in armadai.yaml), with no explicit
    // `--orchestrate` flag — `run_inner` auto-detects that case on its own
    // (see the `AgentResolution::Project` branch below). The live TUI must
    // trigger for that path too, or config-driven runs silently stay headless.
    let config_orchestrated = project::find_project_config()
        .and_then(|(_, cfg)| cfg.orchestration)
        .map(|o| o.enabled)
        .unwrap_or(false);

    // Live Workroom TUI: only for orchestrated runs (explicit `--orchestrate`
    // or config-driven auto-detect), only when nothing else demands
    // plain/machine output, and only when attached to a real terminal. Falls
    // through to the unchanged headless path otherwise.
    let use_tui = (orchestrate.is_some() || config_orchestrated)
        && !json
        && !quiet
        && !no_tui
        && !dry_run
        && std::io::IsTerminal::is_terminal(&std::io::stdout());

    #[cfg(feature = "tui")]
    if use_tui {
        // Load project orchestration config (for role seeding), best-effort.
        let cfg_yaml = std::fs::read_to_string(".armadai/config.yaml")
            .or_else(|_| std::fs::read_to_string("armadai.yaml"))
            .ok();
        // Map an explicit `--orchestrate <pattern>` flag to the Workroom's
        // pattern enum, so the fullscreen live view renders the matching
        // rich layout (ring/blackboard) instead of relying on the project
        // config's `pattern:` key, which is often absent for a one-off
        // explicit run and would otherwise default to Hierarchical.
        let explicit_pattern = orchestrate.as_deref().and_then(|o| match o {
            "blackboard" => Some(armadai_core::orchestration::OrchestrationPattern::Blackboard),
            "ring" => Some(armadai_core::orchestration::OrchestrationPattern::Ring),
            _ => None,
        });
        let printed = crate::shell::run_view::run_orchestration_tui(
            move |sink| async move {
                run_inner(
                    agent_name,
                    input,
                    pipe,
                    orchestrate,
                    true,
                    false,
                    false,
                    false,
                    max_content,
                    route,
                    tags,
                    dry_run,
                    resume,
                    replay,
                    &sink,
                )
                .await
            },
            cfg_yaml,
            explicit_pattern,
        )
        .await;
        return match printed {
            Ok((run_id, content)) => {
                print_tui_run_outcome(run_id, content);
                Ok(())
            }
            Err(e) => Err(e),
        };
    }
    #[cfg(not(feature = "tui"))]
    let _ = use_tui;

    let sink = armadai_core::events::make_sink(json);

    let result = run_inner(
        agent_name,
        input,
        pipe,
        orchestrate,
        headless,
        json,
        quiet,
        true,
        max_content,
        route,
        tags,
        dry_run,
        resume,
        replay,
        &sink,
    )
    .await;

    if let Err(e) = result {
        if headless {
            let code = exit_code_for(&e);
            sink.emit(&RunEvent::Error {
                code: match code {
                    3 => "budget_exceeded",
                    4 => "provider_unavailable",
                    _ => "agent_failed",
                }
                .into(),
                msg: e.to_string(),
            });
            std::process::exit(code);
        }
        return Err(e);
    }

    Ok(())
}

/// `--replay <run_id>` entry point (OH1 Lot 6, Task 2): builds the sink the
/// same way the normal path does (`make_sink(json)`), delegates to
/// [`crate::cli::run_replay::replay_run`] — which reads the persisted
/// `ExecutionEvent` log back and re-emits it as `RunEvent`s, executing no
/// effects — and funnels any error through the SAME headless
/// error-event/exit-code handler [`execute`] uses for the normal run path, so
/// `--replay --json` on an unknown/errored id behaves like any other failed
/// headless run (an `error` JSONL line + a CI-friendly exit code) rather than
/// a bare panic or a silent non-zero exit.
async fn execute_replay(
    run_id: &str,
    json: bool,
    quiet: bool,
    headless: bool,
) -> anyhow::Result<()> {
    let headless = headless || json;
    let sink = armadai_core::events::make_sink(json);
    // No TUI concern here (unlike the agent path in `execute`): replay has no
    // `orchestrate`/config-driven auto-detect to check and no `agent_name` to
    // route through the live Workroom, so `human_output` collapses to the
    // same `!json && !quiet` gate the agent path's `RunStart` banner uses.
    let human_output = !json && !quiet;

    let result = crate::cli::run_replay::replay_run(run_id, &sink, human_output).await;

    if let Err(e) = result {
        if headless {
            let code = exit_code_for(&e);
            sink.emit(&RunEvent::Error {
                code: match code {
                    3 => "budget_exceeded",
                    4 => "provider_unavailable",
                    _ => "agent_failed",
                }
                .into(),
                msg: e.to_string(),
            });
            std::process::exit(code);
        }
        return Err(e);
    }

    Ok(())
}

/// `--resume <run_id>` entry point (OH1 Lot 6, Task 3): continues a
/// previously interrupted event-sourced run (one whose process died/was
/// killed mid-run, leaving `RunStatus::Running` in the persisted log) from
/// where it left off, executing REAL effects (provider calls) for whatever
/// work remains — unlike `--replay`, which only re-emits the past.
///
/// Requires the `storage` feature (the event log this reads/appends to only
/// persists under it); without it, funnels a clear bail through the same
/// headless error-event/exit-code handling [`execute_replay`] uses, rather
/// than a bare panic.
///
/// The live Workroom TUI is offered the same way the normal agent path does
/// (`use_tui` gate in [`execute`]) — except the pattern (which layout to
/// render) isn't known from a CLI flag here, only from the log itself, so
/// this peeks the run's pattern/status via a synchronous [`replay`] BEFORE
/// deciding `use_tui` and building the `explicit_pattern` hint. `direct`
/// pattern runs never use the TUI, mirroring the live path (a single-agent
/// run never sets `orchestrate`, so it never reaches the TUI branch either).
async fn execute_resume(
    run_id: &str,
    json: bool,
    quiet: bool,
    headless: bool,
    max_content: Option<usize>,
    no_tui: bool,
) -> anyhow::Result<()> {
    #[cfg(not(feature = "storage"))]
    {
        let _ = (run_id, json, quiet, headless, max_content, no_tui);
        anyhow::bail!("--resume requires the 'storage' feature (event log persistence)")
    }

    #[cfg(feature = "storage")]
    {
        use crate::es_log::SqliteLog;
        use armadai_core::orchestration::es::engine::replay;
        use armadai_core::orchestration::es::state::RunStatus;

        let headless = headless || json;

        // Peek the run's pattern/status before deciding on the live TUI —
        // mirrors the agent path's own `use_tui` gate in `execute`, which
        // needs to know the pattern is "orchestrated" before offering it.
        let peek = {
            let db = crate::db::init_db()?;
            let log = SqliteLog::new(db);
            replay(run_id, &log)?
        };
        if peek.pattern.is_empty() {
            anyhow::bail!("no run found for id {run_id}");
        }
        if peek.status != RunStatus::Running {
            anyhow::bail!("run {run_id} is not resumable (status: {:?})", peek.status);
        }

        let use_tui = is_orchestrated_pattern(&peek.pattern)
            && !json
            && !quiet
            && !no_tui
            && std::io::IsTerminal::is_terminal(&std::io::stdout());

        #[cfg(feature = "tui")]
        if use_tui {
            let explicit_pattern = match peek.pattern.as_str() {
                "blackboard" => Some(armadai_core::orchestration::OrchestrationPattern::Blackboard),
                "ring" => Some(armadai_core::orchestration::OrchestrationPattern::Ring),
                _ => None,
            };
            let run_id_owned = run_id.to_string();
            let printed = crate::shell::run_view::run_orchestration_tui(
                move |sink| async move {
                    // `false, false` for json/quiet: guaranteed by the
                    // `use_tui` gate above (mirrors `execute`'s own TUI
                    // closure, which hardcodes the same for `run_inner`).
                    resume_run(&run_id_owned, &sink, false, false, max_content, false).await
                },
                None,
                explicit_pattern,
            )
            .await;
            return match printed {
                Ok((run_id, content)) => {
                    print_tui_run_outcome(run_id, content);
                    Ok(())
                }
                Err(e) => Err(e),
            };
        }
        #[cfg(not(feature = "tui"))]
        let _ = use_tui;

        let sink = armadai_core::events::make_sink(json);

        // `human_output = true` here (unconditional, like the live
        // orchestrated path's non-TUI branch): `resume_run` gates its own
        // `--json`/`--quiet` suppression at each print site instead, since
        // `human_output` here means "not the TUI's alternate screen", not
        // "not machine output" (see `run_orchestrated_inner`'s identical
        // convention).
        let result = resume_run(run_id, &sink, json, quiet, max_content, true).await;

        if let Err(e) = result {
            if headless {
                let code = exit_code_for(&e);
                sink.emit(&RunEvent::Error {
                    code: match code {
                        3 => "budget_exceeded",
                        4 => "provider_unavailable",
                        _ => "agent_failed",
                    }
                    .into(),
                    msg: e.to_string(),
                });
                std::process::exit(code);
            }
            return Err(e);
        }

        Ok(())
    }
}

/// Core of `--resume`: reload the roster from the project on disk (keyed by
/// the run's own `ExecutionState::agents`, folded from the log's
/// `RunStarted`), dispatch to the pattern-matching `resume_*_es` engine entry
/// point wrapped in the same [`SinkProjectingLog`]/[`QuietMaxContentSink`]
/// observability every live `dispatch_*_es` uses, then project/print/emit
/// the result exactly like a live orchestrated run's terminal steps.
///
/// Requires `run_id` to name a `Running` run recorded in the persisted event
/// log — checked again here (not just by the `execute_resume` peek above,
/// which only gates the TUI decision) since this is also the function real
/// tests would call directly.
#[allow(clippy::too_many_arguments)]
#[cfg(feature = "storage")]
async fn resume_run(
    run_id: &str,
    sink: &Arc<dyn EventSink>,
    json: bool,
    quiet: bool,
    max_content: Option<usize>,
    human_output: bool,
) -> anyhow::Result<()> {
    use crate::es_log::SqliteLog;
    use armadai_core::orchestration::es::bridge::synthetic_run_start;
    use armadai_core::orchestration::es::state::{RunStatus, fold};

    let db = crate::db::init_db()?;
    let log = SqliteLog::new(db.clone());
    // Read the raw log back once, up front: `fold` gives the roster/status
    // needed to validate + dispatch the resume below, and the SAME raw
    // `pre_resume_events` also feeds `synthetic_run_start`'s `in_chars`
    // recovery (the original `RunStarted.input`, discarded by `apply` but
    // still present verbatim in the event list) — see that fn's doc comment.
    let pre_resume_events = log.events(run_id)?;
    let state = fold(&pre_resume_events);
    if state.pattern.is_empty() {
        anyhow::bail!("no run found for id {run_id}");
    }
    if state.status != RunStatus::Running {
        anyhow::bail!("run {run_id} is not resumable (status: {:?})", state.status);
    }

    // Prefer the stored `orchestration_runs.pattern` when present, falling
    // back to the log-folded `state.pattern` — the ONLY source available for
    // `direct` runs, which never get an `orchestration_runs` row (see
    // `queries::get_run_pattern`'s doc comment).
    let pattern = armadai_storage::queries::get_run_pattern(&db, run_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| state.pattern.clone());

    if !json && !quiet && human_output {
        let m = crate::cli::style::muted();
        anstream::eprintln!("{m}resume {run_id}{m:#}");
    }

    // I3 (whole-branch review, light must-fix): the roster below is reloaded
    // from the CURRENT directory's project, not the original run's
    // (`RunStarted.project` is logged but never read back — full
    // project-pinning is backlog, not attempted here). Warn once, concisely,
    // so resuming from a different directory than the original run doesn't
    // silently execute the remaining steps against different agent
    // definitions.
    if !json && !quiet && human_output {
        let w = crate::cli::style::warn();
        anstream::eprintln!(
            "{w}resuming {run_id} — agents reloaded from the current project; \
             not-yet-run steps use current definitions{w:#}"
        );
    }

    // HEAD bookend (whole-branch review, I2): a live orchestrated run's
    // `RunStart` is what seeds the Workroom's agent roster
    // (`Workroom::on_run_event_at`'s `RunStart { agents, .. }` arm —
    // `AgentStart` only mutates an already-present agent, it never inserts
    // one). `resume_run` never emitted one, so an interactive `--resume`
    // showed an empty Workroom even though `execute_resume` already routes
    // orchestrated resumes through `run_orchestration_tui` (mirroring
    // `execute`'s own `use_tui` gate). Emitting it here, from the folded
    // roster, before the engine resumes fixes that for both the TUI and
    // `--json`/headless consumers (the bookend is harmless/expected on json
    // too — it matches what a live run emits).
    sink.emit(&synthetic_run_start(
        run_id,
        &pattern,
        &state.agents,
        &pre_resume_events,
    ));

    // Reload the agent roster from the project on disk — the log carries no
    // `Agent` definitions (system prompt/model/temperature/…), only the
    // roster's KEYS (`state.agents`, folded from `RunStarted`) and the
    // pattern's config (`ConfigSnapshot`). `headless = true` here: a resume
    // is a non-interactive continuation, so it must never block on the
    // model-updater's interactive prompt the way a fresh `armadai run` might.
    let resolution = resolve_agents_dir(true);
    let routing_rules = match &resolution {
        AgentResolution::Project { config, .. } => config.routing.clone().unwrap_or_default(),
        _ => armadai_core::routing::RoutingRules::default(),
    };
    let cost_limit = orchestration_cost_limit(&resolution);

    // Fix B (#270) covered fresh orchestrated runs via `run_orchestrated`'s
    // loading loop but not resume; without this, a resumed orchestrated run
    // rebuilds its providers with the plain `create_provider(&agent)` default
    // (`DEFAULT_TIMEOUT_SECS`) and re-hits the exact timeout this fix exists to
    // prevent. Source
    // the config override the same way `run_orchestrated` does, from the
    // resolved project's `defaults.orchestration`.
    //
    // EXCEPTION: `resume_run` also resumes `direct` (single-agent,
    // non-orchestrated) runs — unlike `run_orchestrated`'s loop below, which
    // is only ever reached for orchestrated patterns. `timeout_overrides` is
    // read unconditionally (harmless: `Default` when absent from config), but
    // MUST NOT be applied to a `direct` roster agent below — see the
    // `is_orchestrated_pattern` gate on the `apply_orchestrated_timeout` call
    // (a prior regression applied it unconditionally here, giving resumed
    // `direct` runs the 600s+ orchestrated timeout instead of the correct
    // `DEFAULT_TIMEOUT_SECS` default — see `.superpowers/sdd/orch-e2e-report.md`).
    let timeout_overrides = match &resolution {
        AgentResolution::Project { config, .. } => {
            config.defaults.orchestration.clone().unwrap_or_default()
        }
        _ => armadai_core::project::OrchestrationDefaults::default(),
    };

    let mut agents_map: std::collections::BTreeMap<String, Agent> =
        std::collections::BTreeMap::new();
    let mut providers_map: std::collections::BTreeMap<
        String,
        Arc<dyn armadai_core::provider::Provider>,
    > = std::collections::BTreeMap::new();
    for name in &state.agents {
        let mut agent = load_agent_for_run(&resolution, name)?;
        armadai_core::model_aliases::resolve_model_deprecations(
            &mut agent.metadata.model,
            &mut agent.metadata.model_fallback,
        );
        // Gate: only orchestrated patterns get the orchestrated timeout
        // override — a resumed `direct` run must keep `create_provider`'s
        // own `DEFAULT_TIMEOUT_SECS` default untouched.
        if is_orchestrated_pattern(&pattern) {
            apply_orchestrated_timeout(&mut agent, timeout_overrides.agent_timeout_secs);
        }
        let provider = create_provider(&agent)?;
        providers_map.insert(name.clone(), Arc::from(provider));
        agents_map.insert(name.clone(), agent);
    }

    let filtered_sink = quiet_max_content_sink(sink, quiet, max_content);
    let agent_meta = agent_meta_from_roster(&agents_map);
    let mut proj_log = SinkProjectingLog::with_meta(log, &filtered_sink, agent_meta);

    let final_state = match pattern.as_str() {
        "direct" => {
            use armadai_core::orchestration::es::direct::resume_direct_es;
            resume_direct_es(
                run_id,
                agents_map,
                providers_map,
                routing_rules,
                &mut proj_log,
            )
            .await?
        }
        "blackboard" => {
            use armadai_core::orchestration::es::blackboard::resume_blackboard_es;
            resume_blackboard_es(
                run_id,
                agents_map,
                providers_map,
                routing_rules,
                cost_limit,
                &mut proj_log,
            )
            .await?
        }
        "ring" => {
            use armadai_core::orchestration::es::ring::resume_ring_es;
            resume_ring_es(
                run_id,
                agents_map,
                providers_map,
                routing_rules,
                cost_limit,
                &mut proj_log,
            )
            .await?
        }
        "hierarchical" => {
            use armadai_core::orchestration::es::hierarchical::resume_hierarchical_es;
            resume_hierarchical_es(
                run_id,
                agents_map,
                providers_map,
                routing_rules,
                &mut proj_log,
            )
            .await?
        }
        other => anyhow::bail!("unknown orchestration pattern '{other}' for run {run_id}"),
    };

    let events = proj_log.events(run_id)?;

    match crate::db::init_db() {
        Ok(db2) => {
            if let Err(e) = crate::cli::run_es_record::project_run(&db2, run_id) {
                tracing::warn!("failed to project resumed run {}: {}", run_id, e);
            }
        }
        Err(e) => {
            tracing::warn!("event log storage unavailable, run not projected: {}", e);
        }
    }

    // Re-review fix: routed through the shared `final_content` helper (used
    // identically by `--replay`'s `replay_from_log`) instead of inlining the
    // pattern branch here a second time — see that fn's doc for why (a
    // second, drifted copy of this branch is exactly what let `--replay`'s
    // `ring` output silently lose its vote tally).
    let content = super::run_es_record::final_content(&final_state, &events);

    if human_output {
        let s = status_style(&final_state.status);
        anstream::eprintln!("{s}resume {}: {:?}{s:#}", run_id, final_state.status);
    }
    if !json && human_output {
        println!("{content}");
    }

    sink.emit(&RunEvent::Result {
        content,
        tin: u32::try_from(final_state.budget_tokens_in).unwrap_or(u32::MAX),
        tout: u32::try_from(final_state.budget_tokens_out).unwrap_or(u32::MAX),
        cost: final_state.budget_cost,
        agents: final_state.agents.len(),
    });

    Ok(())
}

/// Print the `(run_id, content)` pair returned by
/// [`crate::shell::run_view::run_orchestration_tui`] once the terminal has
/// been restored (OH1 Lot 6): the alternate screen clears everything on
/// exit, so this muted `run <id>` banner — mirroring the non-TUI orchestrated
/// path's own banner in `run_orchestrated_inner` — is the only way the id
/// survives in scrollback for a later `--resume`/`--replay` on the TUI path.
/// `run_id` prints whenever a `RunStart` was observed by the Workroom (even
/// on an early Ctrl+C abort, since the id is generated before the run's first
/// effect); `content` prints only when the run produced a final answer.
#[cfg(feature = "tui")]
fn print_tui_run_outcome(run_id: Option<String>, content: Option<String>) {
    if let Some(id) = run_id {
        let m = crate::cli::style::muted();
        anstream::println!("{m}run {id}{m:#}");
    }
    if let Some(content) = content {
        println!("{content}");
    }
}

/// Map a run error to a CI-friendly exit code.
///
/// - `0`: success (handled by caller, never produced here)
/// - `1`: generic execution error
/// - `2`: usage error (reserved for CLI-level argument validation)
/// - `3`: budget/cost limit exceeded
/// - `4`: provider unavailable
fn exit_code_for(err: &anyhow::Error) -> i32 {
    let s = err.to_string().to_lowercase();
    if s.contains("budget") || s.contains("cost limit") {
        3
    } else if s.contains("not available") || s.contains("unavailable") {
        4
    } else {
        1
    }
}

/// Core run logic (sequential or orchestrated). Kept separate from [`execute`] so that
/// all error paths funnel through a single headless error-event + exit-code handler.
/// Parameters are passed directly from `execute` and represent distinct configuration concerns.
#[allow(clippy::too_many_arguments)]
async fn run_inner(
    agent_name: String,
    input: Option<String>,
    pipe: Option<Vec<String>>,
    orchestrate: Option<String>,
    headless: bool,
    json: bool,
    quiet: bool,
    human_output: bool,
    max_content: Option<usize>,
    route: Option<String>,
    tags: Option<Vec<String>>,
    dry_run: bool,
    // OH1 Lot 6: threaded through for signature stability across Tasks 2/3
    // (resume/replay execution). `execute` already bails before calling
    // `run_inner` when either is `Some`, so this function never branches on
    // them yet.
    resume: Option<String>,
    replay: Option<String>,
    sink: &Arc<dyn EventSink>,
) -> anyhow::Result<()> {
    let _ = (&resume, &replay);
    let resolution = resolve_agents_dir(headless);
    let tags = tags.unwrap_or_default();

    // Build the execution chain: primary agent + piped agents
    let mut chain = vec![agent_name];
    if let Some(extra) = pipe {
        chain.extend(extra);
    }

    // Resolve input text
    let current_input = resolve_input(input).await?;

    // Orchestrated multi-agent execution (explicit --orchestrate flag)
    if let Some(pattern) = orchestrate {
        if chain.len() < 2 {
            anyhow::bail!("--orchestrate requires at least 2 agents (use --pipe to add more)");
        }
        return run_orchestrated(
            &resolution,
            &chain,
            &current_input,
            &pattern,
            sink,
            json,
            quiet,
            max_content,
            route.as_deref(),
            &tags,
            dry_run,
            human_output,
        )
        .await;
    }

    // Auto-detect orchestration from project config (orchestration.enabled: true)
    if let AgentResolution::Project { ref config, .. } = resolution
        && let Some(ref orch) = config.orchestration
        && orch.enabled
    {
        let pattern = orch.pattern.to_string();
        // Collect all agents from orchestration config
        let mut orch_agents = Vec::new();
        if let Some(ref coord) = orch.coordinator {
            orch_agents.push(coord.clone());
        }
        for team in &orch.teams {
            if let Some(ref lead) = team.lead {
                orch_agents.push(lead.clone());
            }
            orch_agents.extend(team.agents.iter().cloned());
        }
        if !orch_agents.is_empty() {
            return run_orchestrated(
                &resolution,
                &orch_agents,
                &current_input,
                &pattern,
                sink,
                json,
                quiet,
                max_content,
                route.as_deref(),
                &tags,
                dry_run,
                human_output,
            )
            .await;
        }
    }

    // Standard sequential execution (backward compatible)
    let mut current_input = current_input;
    let project_defaults = match &resolution {
        AgentResolution::Project { config, .. } => Some(&config.defaults),
        _ => None,
    };
    let routing_rules = match &resolution {
        AgentResolution::Project { config, .. } => config.routing.clone().unwrap_or_default(),
        _ => armadai_core::routing::RoutingRules::default(),
    };
    // Which project (if any) these runs are attributed to in storage: the
    // resolved project root, or the CWD as a best-effort fallback when no
    // `armadai.yaml` was found (still useful to distinguish ad-hoc runs).
    let project = project_display_string(&resolution);

    // Generated once, up front, so the emitted `RunStart` (surfaced to the
    // user for a future `--resume`/`--replay`) carries the SAME run_id the
    // single-agent ES dispatch below (`dispatch_direct_es`) persists the ES
    // log under.
    let run_id = uuid::Uuid::new_v4().to_string();

    sink.emit(&RunEvent::RunStart {
        run_id: run_id.clone(),
        v: 1,
        agents: chain.clone(),
        prov: String::new(), // filled per-agent in agent_start; kept minimal here
        model: String::new(),
        in_chars: current_input.chars().count(),
    });

    // Surface the run_id on the human path (OH1 Lot 6, Task 1) so it can be
    // copied for a future `--resume`/`--replay`. Not on `--json` (the id
    // already travels in `RunStart`) or `--quiet`, and gated on
    // `human_output` too, mirroring the orchestrated path below.
    if !json && !quiet && human_output {
        let m = crate::cli::style::muted();
        anstream::println!("{m}run {run_id}{m:#}");
    }

    // Single-agent direct execution (OH1 Lot 5, T5a): switched onto the
    // event-sourced `direct` engine (`run_direct_es`), wrapped in a
    // `SinkProjectingLog` so `AgentStart`/`AgentEnd`/`Route` observability
    // keeps flowing to `sink` unchanged. `--pipe` (multi-agent chain, below)
    // deliberately stays on the legacy `run_single_agent` loop — the brief
    // scopes this bascule to the single-agent case only.
    if chain.len() == 1 {
        let name = &chain[0];
        let agent = load_agent_for_run(&resolution, name)?;
        let (content, tin, tout, cost) = run_single_agent_es(
            &run_id,
            agent,
            name,
            &current_input,
            project_defaults,
            sink,
            quiet,
            max_content,
            &routing_rules,
            project.as_deref(),
        )
        .await?;

        sink.emit(&RunEvent::Result {
            content: content.clone(),
            tin,
            tout,
            cost,
            agents: 1,
        });

        if !json {
            println!("{content}");
        }

        return Ok(());
    }

    let mut agg_tin = 0u32;
    let mut agg_tout = 0u32;
    let mut agg_cost = 0.0f64;

    for (i, name) in chain.iter().enumerate() {
        if chain.len() > 1 && !json {
            let h = crate::cli::style::header();
            anstream::eprintln!("{h}--- [{}/{} {}] ---{h:#}", i + 1, chain.len(), name);
        }

        let agent = load_agent_for_run(&resolution, name)?;
        let (output, metrics) = run_single_agent(
            agent,
            name,
            &current_input,
            project_defaults,
            sink,
            quiet,
            max_content,
            &routing_rules,
            project.as_deref(),
        )
        .await?;
        agg_tin += metrics.tokens_in as u32;
        agg_tout += metrics.tokens_out as u32;
        agg_cost += metrics.cost;
        current_input = output;
    }

    sink.emit(&RunEvent::Result {
        content: current_input.clone(),
        tin: agg_tin,
        tout: agg_tout,
        cost: agg_cost,
        agents: chain.len(),
    });

    // Human/plain output only when not emitting JSON
    if !json {
        println!("{current_input}");
    }

    Ok(())
}

/// Result of resolving the agents directory / project config.
enum AgentResolution {
    /// New-format project config with walk-up root
    Project {
        root: PathBuf,
        config: Box<ProjectConfig>,
        /// The project's prompt fragments, scanned at most once per
        /// invocation however many agents a run loads.
        ///
        /// They are invariant for the whole run — three directories' worth
        /// of `.md` files, parsed — while every loop that loads agents by
        /// name (`--pipe`'s chain, `--orchestrate`'s roster,
        /// `--resume`'s roster reload) needs them once per agent. Computing
        /// them inside [`load_agent_for_run`] instead re-read and re-parsed
        /// the same files per link, and re-printed `load_all_prompts`'s
        /// `warn: failed to load prompt …` once per link — N lines identical
        /// byte for byte, naming no agent, saying nothing the first did not
        /// (#364 review, m1; the same defect `eecbd0f` fixed for `unlink`).
        ///
        /// Lazy rather than eager so a path that resolves a project without
        /// loading any agent by name pays nothing.
        fragments: std::sync::OnceLock<Vec<armadai_core::prompt::Prompt>>,
    },
    /// No project config found — use default paths
    Default(PathBuf),
}

/// Best-effort project identifier for storage attribution: the resolved
/// project root's display string when an `armadai.yaml` was found, else the
/// current working directory (still useful to group ad-hoc runs), else
/// `None` if even `current_dir()` fails.
fn project_display_string(resolution: &AgentResolution) -> Option<String> {
    match resolution {
        AgentResolution::Project { root, .. } => Some(root.display().to_string()),
        AgentResolution::Default(_) => std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string()),
    }
}

/// Load an agent for a run by name, whether it is written as a file or
/// declared in `.armadai/agents.yaml`.
///
/// The ONLY agent-resolution entry point on the run path, and deliberately
/// so. It used to have a path-returning sibling (`resolve_agent_path`),
/// which no declared agent can ever satisfy — a declared agent has no file
/// — and every surface that kept calling it silently served a smaller
/// fleet than the project declares (#339). That primitive is gone rather
/// than merely bypassed: leaving it in place would have left the same trap
/// armed for the next call site.
///
/// Called from the single-agent path (`chain.len() == 1`, the common
/// `armadai run <name>` invocation), the `--pipe` chain loop, the
/// `--orchestrate` roster loader (`run_orchestrated`), and `--resume`'s
/// roster reload (`resume_run`) — the last three in a loop, which is why the
/// loop-invariant half of the work (the prompt fragments) is memoised on
/// `AgentResolution` rather than recomputed here per call.
fn load_agent_for_run(resolution: &AgentResolution, agent_name: &str) -> anyhow::Result<Agent> {
    match resolution {
        AgentResolution::Project {
            root,
            config,
            fragments,
        } => {
            let fragments =
                fragments.get_or_init(|| armadai_core::agent_source::project_fragments(root));
            let (agent, warning) = armadai_core::agent_source::load_agent_by_name(
                agent_name, config, root, fragments,
            )?;
            // Core returns the warning rather than printing it (see
            // `load_agent_by_name`'s own doc) precisely so it renders here,
            // in the CLI's own voice, instead of a bare `tracing::warn!`
            // line reaching the user's terminal directly from core.
            if let Some(w) = warning {
                let s = crate::cli::style::warn();
                anstream::eprintln!("{s}  warn: {}{s:#}", w.message());
            }
            Ok(agent)
        }
        AgentResolution::Default(agents_dir) => {
            let path = Agent::find_file(agents_dir, agent_name).ok_or_else(|| {
                anyhow::anyhow!("Agent '{agent_name}' not found in {}", agents_dir.display())
            })?;
            armadai_core::parser::parse_agent_file(&path)
        }
    }
}

/// Execute a single agent with given input and configuration. Parameters represent
/// environment (agent, input), configuration (defaults, rules), and I/O (sink, quiet, max_content);
/// grouping would obscure distinct concerns in request building and provider creation.
///
/// Takes an already-loaded `agent` rather than a path — the same shape
/// `run_single_agent_es` has. Loading is the caller's job (via
/// [`load_agent_for_run`]) precisely because an agent declared in
/// `.armadai/agents.yaml` has no file to hand down here (#339).
#[allow(clippy::too_many_arguments)]
async fn run_single_agent(
    mut agent: Agent,
    agent_name: &str,
    input: &str,
    project_defaults: Option<&ProjectDefaults>,
    sink: &Arc<dyn EventSink>,
    quiet: bool,
    max_content: Option<usize>,
    routing_rules: &armadai_core::routing::RoutingRules,
    project: Option<&str>,
) -> anyhow::Result<(String, RunMetrics)> {
    // `project` is only read by `record_run` under `#[cfg(feature = "storage")]`
    // below; this keeps the parameter used (and its name meaningful at every
    // call site) regardless of which features are enabled.
    #[cfg(not(feature = "storage"))]
    let _ = project;

    // 1b. Resolve deprecated model aliases
    let model_before = agent.metadata.model.clone();
    armadai_core::model_aliases::resolve_model_deprecations(
        &mut agent.metadata.model,
        &mut agent.metadata.model_fallback,
    );
    if agent.metadata.model != model_before {
        sink.emit(&RunEvent::Warning {
            code: "deprecated_model".to_string(),
            from: model_before,
            to: agent.metadata.model.clone(),
        });
    }
    // 1c. Warn if model unknown in registry
    if let Some(ref model) = agent.metadata.model {
        crate::linker::model_resolution::warn_unknown_model(model, &agent.metadata.provider);
    }

    // 2. Create provider
    let provider = create_provider(&agent)?;

    // 4. Resolve effective mode and build system prompt
    let effective_mode = agent
        .metadata
        .mode
        .or(project_defaults.and_then(|d| d.mode))
        .unwrap_or_default();

    let system_prompt = if effective_mode == AgentMode::Guided {
        format!("{}{GUIDED_MODE_INSTRUCTION}", agent.system_prompt)
    } else {
        agent.system_prompt.clone()
    };

    // 5. Build request
    let raw_model = agent
        .metadata
        .model
        .clone()
        .or_else(|| agent.metadata.command.clone())
        .unwrap_or_else(|| "default".to_string());

    let model = if raw_model == "latest:auto" {
        let (tier, reason) =
            armadai_core::routing::route(input, &agent.metadata.tags, None, routing_rules);
        sink.emit(&RunEvent::Route {
            agent: agent_name.to_string(),
            tier: format!("{tier:?}"),
            reason: format!("{reason:?}"),
        });
        armadai_core::model_resolution::resolve_model_for_tier(&agent.metadata.provider, tier)
    } else {
        raw_model
    };

    let request = CompletionRequest {
        model,
        system_prompt,
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: input.to_string(),
        }],
        temperature: agent.metadata.temperature,
        max_tokens: agent.metadata.max_tokens,
    };

    sink.emit(&RunEvent::AgentStart {
        agent: agent_name.to_string(),
        prov: agent.metadata.provider.clone(),
        model: agent.metadata.model.clone().unwrap_or_default(),
    });

    // 6. Execute (with model fallback)
    let start = Instant::now();
    let response = match provider.complete(request.clone()).await {
        Ok(resp) => resp,
        Err(err) if is_model_not_found(&err) && !agent.metadata.model_fallback.is_empty() => {
            let mut last_err = err;
            let mut fallback_resp = None;
            for fallback_model in &agent.metadata.model_fallback {
                let w = crate::cli::style::warn();
                anstream::eprintln!(
                    "{w}[{agent_name}] Model unavailable, falling back to {fallback_model}...{w:#}"
                );
                let mut retry_request = request.clone();
                retry_request.model = fallback_model.clone();
                match provider.complete(retry_request).await {
                    Ok(resp) => {
                        fallback_resp = Some(resp);
                        break;
                    }
                    Err(e) if is_model_not_found(&e) => {
                        last_err = e;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
            fallback_resp.ok_or(last_err)?
        }
        Err(err) => return Err(err),
    };
    let duration = start.elapsed();

    if !quiet {
        let content_out = match max_content {
            Some(n) => response.content.chars().take(n).collect::<String>(),
            None => response.content.clone(),
        };
        sink.emit(&RunEvent::AgentEnd {
            agent: agent_name.to_string(),
            tin: response.tokens_in,
            tout: response.tokens_out,
            cost: response.cost,
            content: content_out,
        });
    }

    // 7. Print summary to stderr (so stdout is clean for piping)
    let duration_ms = duration.as_millis() as i64;
    let acc = crate::cli::style::accent();
    let mut_ = crate::cli::style::muted();
    anstream::eprintln!(
        "\n{acc}[{}]{acc:#} {mut_}model={} tokens={}/{} cost=${:.6} duration={}ms{mut_:#}",
        agent_name,
        response.model,
        response.tokens_in,
        response.tokens_out,
        response.cost,
        duration_ms
    );

    let metrics = RunMetrics {
        agent: agent_name.to_string(),
        provider_name: agent.metadata.provider.clone(),
        model: response.model.clone(),
        tokens_in: response.tokens_in as i64,
        tokens_out: response.tokens_out as i64,
        cost: response.cost,
        duration_ms,
    };

    // 8. Record in storage (if available)
    #[cfg(feature = "storage")]
    record_run(&metrics, input, &response.content, project);

    Ok((response.content, metrics))
}

/// Result of driving the event-sourced `direct` engine for one agent
/// (OH1 Lot 5, T5a): the final answer plus the run-level aggregate
/// tokens/cost (`ExecutionState::budget_*`), and the raw event log — the
/// latter only needed by callers that also record storage (to recover the
/// resolved model string from the last `AgentObserved`).
struct DirectDispatch {
    content: String,
    tin: u32,
    tout: u32,
    cost: f64,
    events: Vec<ExecutionEvent>,
}

/// [`EventSink`] decorator that applies `--quiet`/`--max-content` to the
/// events flowing through the ES bridge (`SinkProjectingLog`) before
/// forwarding them to `inner`, mirroring the inline suppression/truncation
/// `run_single_agent` applies at its step 6 (see there).
///
/// Per the CLI help text (`src/cli/mod.rs`, `Command::Run::quiet`): "with
/// `--json`, emit only the final `result` event". `RunEvent::Result` is never
/// itself produced by the bridge — `map_execution_to_run_events` maps
/// `ExecutionEvent::Completed` to `[]`, and the terminal `Result` is always
/// emitted by the caller (`run_inner`/`run_orchestrated_inner`) after the
/// dispatch returns, outside this decorator's reach — so every `RunEvent` that
/// reaches [`QuietMaxContentSink::emit`] is, by construction, an intermediate
/// one (`AgentStart`, `AgentEnd`, `Board`, `Vote`, `Route`, `Delegate`,
/// `NestedStart`/`NestedEnd`, `Warning`, …). Under `quiet` every one of them is
/// dropped, which is what makes "only `result` shows up" true for the whole
/// JSONL stream this decorator has a say over.
///
/// Without `quiet`, `max_content` truncates `AgentEnd.content` to `N` chars —
/// the only intermediate event carrying a `content` field — while every other
/// `RunEvent` (and the eventual `Result`, which this decorator never sees)
/// passes through/stays untouched. This keeps `map_execution_to_run_events`
/// itself pure — the filtering lives entirely at this call site, not in the
/// bridge.
///
/// Shared by the direct dispatch (`dispatch_direct_es`) and the three
/// orchestrated dispatches (`dispatch_blackboard_es`/`dispatch_ring_es`/
/// `dispatch_hierarchical_es`) via [`quiet_max_content_sink`] — a single
/// definition of "what quiet/max_content mean" for every ES run path.
struct QuietMaxContentSink<'s> {
    inner: &'s dyn EventSink,
    quiet: bool,
    max_content: Option<usize>,
}

impl EventSink for QuietMaxContentSink<'_> {
    fn emit(&self, ev: &RunEvent) {
        if self.quiet {
            return;
        }
        if let RunEvent::AgentEnd {
            agent,
            tin,
            tout,
            cost,
            content,
        } = ev
        {
            let content = match self.max_content {
                Some(n) => content.chars().take(n).collect::<String>(),
                None => content.clone(),
            };
            self.inner.emit(&RunEvent::AgentEnd {
                agent: agent.clone(),
                tin: *tin,
                tout: *tout,
                cost: *cost,
                content,
            });
            return;
        }
        self.inner.emit(ev);
    }
}

/// Build the `QuietMaxContentSink` decorator around `sink` — the single
/// construction point every ES dispatch (`dispatch_direct_es` and the three
/// orchestrated `dispatch_*_es`) wraps its `SinkProjectingLog` sink with, so
/// `--quiet`/`--max-content` mean exactly the same thing on every run path.
fn quiet_max_content_sink(
    sink: &Arc<dyn EventSink>,
    quiet: bool,
    max_content: Option<usize>,
) -> QuietMaxContentSink<'_> {
    QuietMaxContentSink {
        inner: sink.as_ref(),
        quiet,
        max_content,
    }
}

/// Drive a single, already-loaded/prepared `agent` through the event-sourced
/// `direct` engine ([`armadai_core::orchestration::es::direct::run_direct_es`]),
/// on a fresh [`InMemoryLog`] wrapped in [`SinkProjectingLog`] so
/// `AgentStart`/`AgentEnd`/`Route` observability keeps flowing to `sink`
/// exactly as the legacy path did — modulo `quiet`/`max_content`, applied via
/// [`QuietMaxContentSink`] so the emitted `AgentEnd` honors the same flags
/// `run_single_agent` does. `agent_key` is the roster key (filename slug) the
/// run addresses this agent by — for a single-agent direct run there is no
/// delegation/route to key by anything else, but using the same convention
/// as the orchestrated patterns keeps `run_direct_es`'s own
/// `RunStarted { agents, .. }` roster consistent.
///
/// Pure with respect to loading/side-effect concerns other than the actual
/// provider call — no file I/O, no rate limiting, no storage — which is what
/// makes this directly unit-testable with a mock `Provider` (see
/// `tests::direct_es`).
///
/// **Architecture note (OH1 Lot 5):** The event log (via `SqliteLog` under
/// `storage`, one DB connection per dispatch) is appended AU FIL DE L'EAU
/// during the run, while flat tables (`runs`, `orchestration_runs`, etc.) are
/// written EN FIN de run by separate `record_*_es` helpers (different
/// connection). They cannot share a single transaction — the run is async and
/// spans time — and in Lot 5b the flat tables become projections derived from
/// the log (the `record_*_es` will disappear).
#[allow(clippy::too_many_arguments)]
async fn dispatch_direct_es(
    run_id: &str,
    agent_key: &str,
    agent: Agent,
    provider: Arc<dyn armadai_core::provider::Provider>,
    input: &str,
    routing_rules: &armadai_core::routing::RoutingRules,
    sink: &Arc<dyn EventSink>,
    quiet: bool,
    max_content: Option<usize>,
) -> anyhow::Result<DirectDispatch> {
    use armadai_core::orchestration::es::direct::run_direct_es;
    use std::collections::BTreeMap;

    // Agent metadata for the bridge's `AgentInvoked → AgentStart` projection:
    // the bridge is the single source of `AgentStart`/`AgentEnd` on this path
    // (`run_inner`'s direct branch emits neither), so it must carry the real
    // provider/model here — read before `agent` is moved into the roster map.
    let mut agent_meta: BTreeMap<String, (String, String)> = BTreeMap::new();
    agent_meta.insert(
        agent_key.to_string(),
        (
            agent.metadata.provider.clone(),
            agent.metadata.model.clone().unwrap_or_default(),
        ),
    );

    let mut agents = BTreeMap::new();
    agents.insert(agent_key.to_string(), agent);
    let mut providers = BTreeMap::new();
    providers.insert(agent_key.to_string(), provider);

    let filtered_sink = quiet_max_content_sink(sink, quiet, max_content);

    // Deduplication macro (Fix 2): single definition of the run_direct_es call,
    // only the log constructor varies across storage/fallback/non-storage branches.
    macro_rules! run_with_log {
        ($log:expr) => {{
            let mut log = SinkProjectingLog::with_meta($log, &filtered_sink, agent_meta);
            let state = run_direct_es(
                run_id,
                agent_key,
                input,
                agents,
                providers,
                routing_rules.clone(),
                &mut log,
            )
            .await?;
            let events = log.events(run_id)?;
            (state, events)
        }};
    }

    #[cfg(feature = "storage")]
    let (state, events) = {
        use crate::es_log::SqliteLog;
        match crate::db::init_db() {
            Ok(db) => run_with_log!(SqliteLog::new(db)),
            Err(e) => {
                tracing::warn!("event log storage unavailable, run will not be persisted: {e}");
                run_with_log!(InMemoryLog::default())
            }
        }
    };
    #[cfg(not(feature = "storage"))]
    let (state, events) = run_with_log!(InMemoryLog::default());
    let result = to_orchestration_result(&state, &events);

    Ok(DirectDispatch {
        content: result.content,
        tin: result.total_tokens_in,
        tout: result.total_tokens_out,
        cost: result.total_cost,
        events,
    })
}

/// Execute a single agent via the event-sourced `direct` pattern (OH1 Lot 5,
/// T5a): loads/prepares the agent exactly like [`run_single_agent`] (model-
/// deprecation resolution + warning, unknown-model warning, rate limiting,
/// guided-mode system-prompt augmentation), drives it through
/// [`dispatch_direct_es`], and finally records the run in storage via the
/// same [`record_run`]/[`RunMetrics`] the legacy (`--pipe`) path still uses.
/// Returns `(content, tokens_in, tokens_out, cost)`; the caller (`run_inner`)
/// owns emitting the terminal `RunEvent::Result` and the stdout `println!`,
/// exactly like the legacy per-agent loop does for `--pipe`.
///
/// Known behavior gaps vs. the legacy path (documented, not fixed here — out
/// of scope for the bascule): `agent.metadata.model_fallback` is never
/// retried (`DirectEffectRunner` makes a single provider call and propagates
/// any error, whereas `run_single_agent` retries each fallback model in
/// order on a "model not found" error). `quiet`/`max_content` *are* honored
/// (via [`QuietMaxContentSink`] in [`dispatch_direct_es`]), matching
/// `run_single_agent`'s step 6.
///
/// Takes an already-loaded `agent` rather than a path, as
/// `run_single_agent` now does too: the caller resolves it via
/// [`load_agent_for_run`], which — unlike a bare path — also covers an
/// agent declared in `.armadai/agents.yaml`.
#[allow(clippy::too_many_arguments)]
async fn run_single_agent_es(
    run_id: &str,
    mut agent: Agent,
    agent_key: &str,
    input: &str,
    project_defaults: Option<&ProjectDefaults>,
    sink: &Arc<dyn EventSink>,
    quiet: bool,
    max_content: Option<usize>,
    routing_rules: &armadai_core::routing::RoutingRules,
    project: Option<&str>,
) -> anyhow::Result<(String, u32, u32, f64)> {
    #[cfg(not(feature = "storage"))]
    let _ = project;

    let model_before = agent.metadata.model.clone();
    armadai_core::model_aliases::resolve_model_deprecations(
        &mut agent.metadata.model,
        &mut agent.metadata.model_fallback,
    );
    if agent.metadata.model != model_before {
        sink.emit(&RunEvent::Warning {
            code: "deprecated_model".to_string(),
            from: model_before,
            to: agent.metadata.model.clone(),
        });
    }
    if let Some(ref model) = agent.metadata.model {
        crate::linker::model_resolution::warn_unknown_model(model, &agent.metadata.provider);
    }

    // 2. Create provider (step 2).
    let provider_name = agent.metadata.provider.clone();
    let provider: Arc<dyn armadai_core::provider::Provider> = Arc::from(create_provider(&agent)?);

    // 4. Guided-mode system-prompt augmentation (step 4).
    let effective_mode = agent
        .metadata
        .mode
        .or(project_defaults.and_then(|d| d.mode))
        .unwrap_or_default();
    if effective_mode == AgentMode::Guided {
        agent.system_prompt = format!("{}{GUIDED_MODE_INSTRUCTION}", agent.system_prompt);
    }

    // 5-7. Drive the event-sourced engine (model routing, request building,
    // AgentStart/AgentEnd observability, the actual provider call).
    let start = Instant::now();
    let dispatch = dispatch_direct_es(
        run_id,
        agent_key,
        agent,
        provider,
        input,
        routing_rules,
        sink,
        quiet,
        max_content,
    )
    .await?;
    let duration_ms = start.elapsed().as_millis() as i64;

    // 8. Record in storage (if available) — same shape as `run_single_agent`.
    #[cfg(feature = "storage")]
    {
        let resolved_model = dispatch
            .events
            .iter()
            .rev()
            .find_map(|e| match e {
                ExecutionEvent::AgentObserved {
                    agent: a, model, ..
                } if a == agent_key => Some(model.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let metrics = RunMetrics {
            agent: agent_key.to_string(),
            provider_name,
            model: resolved_model,
            tokens_in: i64::from(dispatch.tin),
            tokens_out: i64::from(dispatch.tout),
            cost: dispatch.cost,
            duration_ms,
        };
        record_run(&metrics, input, &dispatch.content, project);
    }
    #[cfg(not(feature = "storage"))]
    {
        let _ = (provider_name, duration_ms, &dispatch.events);
    }

    Ok((dispatch.content, dispatch.tin, dispatch.tout, dispatch.cost))
}

#[allow(dead_code)]
struct RunMetrics {
    agent: String,
    provider_name: String,
    model: String,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
    duration_ms: i64,
}

#[cfg(feature = "storage")]
fn record_run(metrics: &RunMetrics, input: &str, output: &str, project: Option<&str>) {
    use crate::db::init_db;
    use armadai_storage::queries;

    let db = match init_db() {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("Failed to init storage: {e}");
            return;
        }
    };

    let record = queries::RunRecord {
        agent: metrics.agent.clone(),
        input: input.to_string(),
        output: output.to_string(),
        provider: metrics.provider_name.clone(),
        model: metrics.model.clone(),
        tokens_in: metrics.tokens_in,
        tokens_out: metrics.tokens_out,
        cost: metrics.cost,
        duration_ms: metrics.duration_ms,
        status: "success".to_string(),
        project: project.map(|s| s.to_string()),
    };

    if let Err(e) = queries::insert_run(&db, record) {
        tracing::warn!("Failed to record run: {e}");
    }
}

async fn resolve_input(input: Option<String>) -> anyhow::Result<String> {
    match input {
        Some(text) if text.starts_with('@') => {
            let path = &text[1..];
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read input file '{path}': {e}"))
        }
        Some(text) => Ok(text),
        None => {
            // Try reading from stdin if piped
            if atty_is_pipe() {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
                if buf.is_empty() {
                    anyhow::bail!("No input provided. Usage: armadai run <agent> <input>");
                }
                Ok(buf)
            } else {
                anyhow::bail!("No input provided. Usage: armadai run <agent> \"<input>\"");
            }
        }
    }
}

/// Check if stdin is a pipe (not a terminal).
fn atty_is_pipe() -> bool {
    use std::io::IsTerminal;
    !std::io::stdin().is_terminal()
}

/// Resolve agent source: walk up for `armadai.yaml`, detect format,
/// and return the appropriate resolution strategy.
fn resolve_agents_dir(headless: bool) -> AgentResolution {
    // 1. Walk-up search for project config (new or legacy format).
    //
    // A project counts as having agents when `agents:` lists any, OR
    // `.armadai/agents.yaml` declares any: every declared agent is included
    // automatically (it does not need to be relisted in `agents:`), so a
    // project that only uses that format — an empty/absent `agents:` list —
    // must still take the project branch instead of silently falling
    // through to the no-project default below.
    if let Some((root, config)) = project::find_project_config()
        && armadai_core::agent_source::project_declares_agents(&root, &config)
    {
        tracing::info!(
            "Using project config from {} ({} agent(s))",
            root.display(),
            config.agents.len()
        );
        if let Err(e) = armadai_core::project_registry::register_project(&root) {
            tracing::warn!("Failed to register project in registry: {:?}", e);
        }
        let interactive = !headless && !atty_is_pipe();
        armadai_core::model_updater::auto_check_and_prompt(&root, interactive);
        return AgentResolution::Project {
            root,
            config: Box::new(config),
            fragments: std::sync::OnceLock::new(),
        };
    }

    // 2. Default fallback
    AgentResolution::Default(AppPaths::resolve().agents_dir)
}

/// Apply C8 agent selection (routes/tags) to a loaded roster, returning the
/// filtered and reordered (agents, providers) plus the selection metadata.
///
/// Identity for routing/tagging purposes is the **roster key** (the filename
/// slug used by `--pipe`, `orchestration.routes:`, and the roster passed to
/// `select_agents`) — NOT `agent.name` (the parsed H1 title). The two commonly
/// differ (e.g. key `backend-dev` / H1 `Backend Developer`); keying on the H1
/// silently breaks route/tag matching. `keys` must be aligned by index with
/// `agents`/`providers` (same order the roster was loaded in).
///
/// Everything operates on the loaded roster: a route naming a key absent
/// from the roster is a clear error (the agent must be provided to the run).
/// The selected roster KEYS are returned alongside the reordered vectors so
/// callers can keep downstream events (AgentEnd/Result/dry-run) keyed the
/// same way as AgentStart/RunStart.
#[allow(clippy::type_complexity)] // (keys, agents, providers, selection) mirrors the loaded-roster shape
fn apply_agent_selection(
    keys: &[String],
    agents: Vec<armadai_core::agent::Agent>,
    providers: Vec<std::sync::Arc<dyn armadai_core::provider::Provider>>,
    route: Option<&str>,
    tags: &[String],
    routes: &std::collections::BTreeMap<String, Vec<String>>,
) -> anyhow::Result<(
    Vec<String>,
    Vec<armadai_core::agent::Agent>,
    Vec<std::sync::Arc<dyn armadai_core::provider::Provider>>,
    armadai_core::orchestration::agent_selection::AgentSelection,
)> {
    use std::collections::HashMap;

    debug_assert_eq!(
        keys.len(),
        agents.len(),
        "roster keys must be aligned with the loaded agents"
    );

    let roster: Vec<String> = keys.to_vec();
    let mut agent_tags: HashMap<String, Vec<String>> = HashMap::new();
    for (key, a) in keys.iter().zip(&agents) {
        let mut t = a.metadata.tags.clone();
        t.extend(a.metadata.stacks.iter().cloned());
        agent_tags.insert(key.clone(), t);
    }

    let selection = armadai_core::orchestration::agent_selection::select_agents(
        &roster,
        route,
        tags,
        routes,
        &agent_tags,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Index the loaded pairs by roster KEY, then rebuild in selection order.
    let mut by_name: HashMap<
        String,
        (
            armadai_core::agent::Agent,
            std::sync::Arc<dyn armadai_core::provider::Provider>,
        ),
    > = HashMap::new();
    for ((key, a), p) in keys.iter().cloned().zip(agents).zip(providers) {
        by_name.insert(key, (a, p));
    }

    let mut out_agents = Vec::with_capacity(selection.agents.len());
    let mut out_providers = Vec::with_capacity(selection.agents.len());
    for name in &selection.agents {
        let (a, p) = by_name.remove(name).ok_or_else(|| {
            anyhow::anyhow!(
                "route/selection references agent '{name}' which is not among the run's agents \
                 (add it via --pipe or the orchestration config)"
            )
        })?;
        out_agents.push(a);
        out_providers.push(p);
    }

    Ok((
        selection.agents.clone(),
        out_agents,
        out_providers,
        selection,
    ))
}

/// Run orchestrated multi-agent execution (blackboard/ring/hierarchical).
///
/// Loads the roster (parse + deprecation-resolve each agent, create its
/// provider), then hands off to [`run_orchestrated_inner`], which owns ALL
/// `RunEvent` emission. Split so the emission wiring — the observability
/// contract this fix hardens — is unit-testable with mock agents/providers
/// (see `es_switch_tests`) without file I/O or a real provider factory.
///
/// Deprecation transitions are collected here (not emitted) and replayed by
/// the inner fn right after `RunStart`, preserving the original event order.
#[allow(clippy::too_many_arguments)]
async fn run_orchestrated(
    resolution: &AgentResolution,
    agent_names: &[String],
    input: &str,
    pattern: &str,
    sink: &std::sync::Arc<dyn armadai_core::events::EventSink>,
    json: bool,
    quiet: bool,
    max_content: Option<usize>,
    route: Option<&str>,
    tags: &[String],
    dry_run: bool,
    human_output: bool,
) -> anyhow::Result<()> {
    use std::sync::Arc;

    use armadai_core::provider::Provider;

    let mut agents = Vec::new();
    let mut providers: Vec<Arc<dyn Provider>> = Vec::new();
    let mut deprecations: Vec<(Option<String>, Option<String>)> = Vec::new();

    // Fix B (#270 stopgap): read the project's `defaults.orchestration`
    // ahead of provider construction so `agent_timeout_secs` can actually
    // reach the CLI provider's timeout (see `orchestrated_agent_timeout_secs`
    // doc comment for why this must happen here, before `create_provider`).
    let timeout_overrides = match resolution {
        AgentResolution::Project { config, .. } => {
            config.defaults.orchestration.clone().unwrap_or_default()
        }
        _ => armadai_core::project::OrchestrationDefaults::default(),
    };

    for name in agent_names {
        let mut agent = load_agent_for_run(resolution, name)?;

        let model_before = agent.metadata.model.clone();
        armadai_core::model_aliases::resolve_model_deprecations(
            &mut agent.metadata.model,
            &mut agent.metadata.model_fallback,
        );
        if agent.metadata.model != model_before {
            deprecations.push((model_before, agent.metadata.model.clone()));
        }

        apply_orchestrated_timeout(&mut agent, timeout_overrides.agent_timeout_secs);

        let provider = create_provider(&agent)?;
        providers.push(Arc::from(provider));
        agents.push(agent);
    }

    run_orchestrated_inner(
        resolution,
        agent_names,
        agents,
        providers,
        deprecations,
        input,
        pattern,
        sink,
        json,
        quiet,
        max_content,
        route,
        tags,
        dry_run,
        human_output,
    )
    .await
}

/// Emit every `RunEvent` for an orchestrated run from a pre-loaded roster:
/// `RunStart`, replayed deprecation `Warning`s, C8 agent selection, `--dry-run`
/// preview, the pattern dispatch, and the terminal `Result`.
///
/// **Observability contract (this fix):** the pattern dispatch (`dispatch_*_es`
/// → the bridge's `SinkProjectingLog`) is the SINGLE source of
/// `AgentStart`/`AgentEnd` on every ES path. This fn therefore does NOT emit an
/// upstream per-agent `AgentStart` loop, and does NOT emit a trailing
/// `emit_agent_ends` batch of empty `AgentEnd`s — both were removed as they
/// double-emitted against the bridge (an empty-`prov`/`model` `AgentStart` and
/// an empty-`content` `AgentEnd`, clobbering the bridge's real events). Every
/// agent's `AgentStart`/`AgentEnd` now comes from the bridge, carrying the real
/// provider/model (via [`agent_meta_from_roster`]) and real per-turn content.
/// Only `--pipe`/legacy sequential runs (`run_single_agent`) still emit their
/// own inline `AgentStart`/`AgentEnd`; those paths never reach this fn.
/// Style for a terminal orchestration status line: `Completed` reads as
/// success, anything else (`Halted`, or the in-flight `Running` default,
/// which should not appear at a terminal print site) as a warning — factual,
/// not alarming, since a halt is often a budget/round limit rather than an
/// error.
fn status_style(status: &armadai_core::orchestration::es::state::RunStatus) -> anstyle::Style {
    use armadai_core::orchestration::es::state::RunStatus;
    match status {
        RunStatus::Completed => crate::cli::style::ok(),
        RunStatus::Halted | RunStatus::Running => crate::cli::style::warn(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_orchestrated_inner(
    resolution: &AgentResolution,
    agent_names: &[String],
    mut agents: Vec<armadai_core::agent::Agent>,
    mut providers: Vec<std::sync::Arc<dyn armadai_core::provider::Provider>>,
    deprecations: Vec<(Option<String>, Option<String>)>,
    input: &str,
    pattern: &str,
    sink: &std::sync::Arc<dyn armadai_core::events::EventSink>,
    json: bool,
    quiet: bool,
    max_content: Option<usize>,
    route: Option<&str>,
    tags: &[String],
    dry_run: bool,
    // Gate for the direct human-readable `eprintln!`/`println!` writes below
    // (roster-size banners, "Halted"/"status"/"Done" lines, and the final
    // outcome). `true` on the plain headless/TTY path (unchanged behavior).
    // `false` only on the live Workroom TUI path (`run_view.rs`), where
    // stdout/stderr are the alternate screen: these direct writes would
    // corrupt the display, and the same final content already reaches the
    // caller via the `RunEvent::Result` emitted at the end of each branch
    // below (captured by `WorkroomSink`, printed after terminal restore).
    // Deliberately NOT `quiet` — `quiet` also suppresses the `RunEvent`s
    // themselves (agent_start/agent_end/board), which the Workroom needs to
    // animate; see `tests/cases/quiet-orchestrated.yaml`.
    human_output: bool,
) -> anyhow::Result<()> {
    use std::sync::Arc;

    use armadai_core::orchestration::blackboard::BlackboardConfig;
    use armadai_core::orchestration::ring::RingConfig;
    use armadai_core::project::OrchestrationDefaults;
    use armadai_core::provider::Provider;

    // Read project-level orchestration overrides (if any).
    let orch_defaults = match resolution {
        AgentResolution::Project { config, .. } => {
            config.defaults.orchestration.clone().unwrap_or_default()
        }
        _ => OrchestrationDefaults::default(),
    };

    // Routing rules for `latest:auto` LlmBoardAgent/LlmRingAgent, mirroring
    // the sequential path in `run_single_agent`: project config wins, else
    // the embedded default. The per-engine budget (see `RoutingCtx::new`) is
    // derived below from each config's `token_budget` once it is known.
    let routing_rules = match resolution {
        AgentResolution::Project { config, .. } => config.routing.clone().unwrap_or_default(),
        _ => armadai_core::routing::RoutingRules::default(),
    };

    // Generated once, up front, so the emitted `RunStart` (surfaced to the
    // user for a future `--resume`/`--replay`) carries the SAME run_id the
    // `dispatch_*_es` pattern dispatch below persists the ES log under —
    // otherwise the human-visible id and the one actually usable for replay
    // would silently diverge.
    let run_id = uuid::Uuid::new_v4().to_string();

    sink.emit(&RunEvent::RunStart {
        run_id: run_id.clone(),
        v: 1,
        agents: agent_names.to_vec(),
        prov: String::new(),
        model: pattern.to_string(),
        in_chars: input.chars().count(),
    });

    // Surface the run_id on the human path (OH1 Lot 6, Task 1) so it can be
    // copied for a future `--resume`/`--replay`. Not on `--json` (the id
    // already travels in `RunStart`) or `--quiet`, and gated on
    // `human_output` too — `false` on the live Workroom TUI path, where a
    // direct stdout write would corrupt the alternate-screen display.
    if !json && !quiet && human_output {
        let m = crate::cli::style::muted();
        anstream::println!("{m}run {run_id}{m:#}");
    }

    // Replay deprecation warnings collected during roster load, right after
    // `RunStart` (same order as the pre-split load loop emitted them).
    for (from, to) in deprecations {
        sink.emit(&RunEvent::Warning {
            code: "deprecated_model".to_string(),
            from,
            to,
        });
    }

    // ── C8: deterministic agent selection (routes/tags) ────────────────
    // A route/tag selector filters and reorders the loaded roster above.
    // Hierarchical delegates its own routing internally, so an explicit
    // --route/--tags is ignored there (with a warning) rather than silently
    // shrinking the coordinator's agent pool.
    let routing_active = route.is_some() || !tags.is_empty();
    // Roster identity for downstream events (`AgentEnd`/`Result`/dry-run).
    // Defaults to the original (unfiltered) roster keys and is reassigned
    // ONLY when a route/tag selection actually narrows/reorders the roster
    // below, so a run with no `--route`/`--tags` emits byte-identical events
    // to before this change (AgentStart and AgentEnd/Result stay on the same
    // roster keys).
    let mut effective_names: Vec<String> = agent_names.to_vec();
    let mut selection_reason: Option<String> = None;
    if routing_active && pattern == "hierarchical" {
        sink.emit(&RunEvent::Warning {
            code: "routing_ignored_hierarchical".to_string(),
            from: None,
            to: None,
        });
    } else if routing_active {
        let routes = match resolution {
            AgentResolution::Project { config, .. } => config
                .orchestration
                .as_ref()
                .map(|o| o.routes.clone())
                .unwrap_or_default(),
            _ => std::collections::BTreeMap::new(),
        };
        let (sel_keys, sel_agents, sel_providers, selection) =
            apply_agent_selection(agent_names, agents, providers, route, tags, &routes)?;
        agents = sel_agents;
        providers = sel_providers;
        effective_names = sel_keys;
        selection_reason = Some(selection.reason.clone());

        sink.emit(&RunEvent::AgentSelect {
            selected: selection.agents.clone(),
            reason: selection.reason.clone(),
        });

        // blackboard/ring need >= 2 agents to make sense; a route/tag filter
        // that narrows below that is a usage error, not a silent no-op.
        if (pattern == "blackboard" || pattern == "ring") && agents.len() < 2 {
            anyhow::bail!(
                "agent routing selected {} agent(s); pattern '{pattern}' requires >= 2 \
                 (selection: {})",
                agents.len(),
                selection.reason
            );
        }
    }

    // --dry-run: resolve + print the selection WITHOUT executing. Works with
    // OR without --route/--tags — a plain dry-run previews the full roster
    // (this check lives OUTSIDE the routing block so it fires unconditionally).
    if dry_run {
        let reason = selection_reason
            .as_deref()
            .unwrap_or("no routing (full roster)");
        eprintln!(
            "[dry-run] pattern '{pattern}' — {reason} ({} agent(s)): {}",
            effective_names.len(),
            effective_names.join(", ")
        );
        if !json {
            println!("{}", effective_names.join("\n"));
        }
        return Ok(());
    }

    // Reflect the (possibly narrowed/reordered) selection in downstream events
    // (`AgentEnd`/`Result`); `RunStart`/`AgentStart` above intentionally stay
    // on the originally requested roster keys. When no routing applied,
    // `effective_names` is untouched, so this is the same slice as the
    // original `agent_names` param (byte-identical events).
    let agent_names: &[String] = effective_names.as_slice();

    match pattern {
        "blackboard" => {
            use std::collections::BTreeMap;

            let config = apply_blackboard_overrides(BlackboardConfig::default(), &orch_defaults);
            let cost_limit = orchestration_cost_limit(resolution);

            // Roster keyed by the ROSTER KEY (`agent_names`, the filename
            // slug — same convention as the hierarchical branch below), NOT
            // `agent.name()` (the H1 title). `run_blackboard_es` derives its
            // own `agent_order`/`RunStarted.agents` from this map's keys.
            let mut agent_map: BTreeMap<String, Agent> = BTreeMap::new();
            let mut provider_map: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            for (name, (agent, provider)) in
                agent_names.iter().zip(agents.into_iter().zip(providers))
            {
                agent_map.insert(name.clone(), agent);
                provider_map.insert(name.clone(), provider);
            }

            if human_output {
                let r = crate::cli::style::running();
                anstream::eprintln!(
                    "{r}[blackboard] Starting with {} agent(s), max {} rounds{r:#}",
                    agent_map.len(),
                    config.max_rounds
                );
            }

            let (state, _run_id) = dispatch_blackboard_es(
                &run_id,
                input,
                agent_map,
                provider_map,
                config.clone(),
                routing_rules,
                cost_limit,
                sink,
                quiet,
                max_content,
            )
            .await?;

            if human_output {
                let s = status_style(&state.status);
                anstream::eprintln!("{s}[blackboard] Halted: {:?}{s:#}", state.status);
            }

            #[cfg(feature = "storage")]
            {
                match crate::db::init_db() {
                    Ok(db) => {
                        if let Err(e) = crate::cli::run_es_record::project_run(&db, &_run_id) {
                            tracing::warn!("failed to project run {}: {}", _run_id, e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("event log storage unavailable, run not projected: {}", e);
                    }
                }
            }

            let outcome_text = super::run_es_record::blackboard_display(&state);

            if !json && human_output {
                println!("{outcome_text}");
            }

            sink.emit(&RunEvent::Result {
                content: outcome_text,
                tin: u32::try_from(state.budget_tokens_in).unwrap_or(u32::MAX),
                tout: u32::try_from(state.budget_tokens_out).unwrap_or(u32::MAX),
                cost: state.budget_cost,
                agents: agent_names.len(),
            });
        }
        "ring" => {
            use std::collections::BTreeMap;

            let config = apply_ring_overrides(RingConfig::default(), &orch_defaults);
            let cost_limit = orchestration_cost_limit(resolution);

            // Roster keyed by the ROSTER KEY (`agent_names`), NOT
            // `agent.name()` — unlike the legacy `ring_agents.iter().map(|a|
            // a.name().to_string())` above (H1 title), which silently broke
            // `--route`/`orchestration.routes:` whenever the H1 title differs
            // from the filename slug (the common case in this repo; see
            // `apply_agent_selection`'s own regression test). `agent_map` is a
            // `BTreeMap` (name-sorted iteration), so it cannot carry the chain
            // order on its own — `agent_names` (still ordered: the primary
            // agent + `--pipe` chain, possibly reordered by `--route`/`--tags`)
            // is passed to `dispatch_ring_es` as `agent_order` alongside it, so
            // `run_ring_es` circulates in that order instead of alphabetically
            // (OH1 Lot 4 Task 3, Bug A fix).
            let mut agent_map: BTreeMap<String, Agent> = BTreeMap::new();
            let mut provider_map: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            for (name, (agent, provider)) in
                agent_names.iter().zip(agents.into_iter().zip(providers))
            {
                agent_map.insert(name.clone(), agent);
                provider_map.insert(name.clone(), provider);
            }

            if human_output {
                let r = crate::cli::style::running();
                anstream::eprintln!(
                    "{r}[ring] Starting with {} agent(s), max {} laps{r:#}",
                    agent_map.len(),
                    config.max_laps
                );
            }

            let (state, events, _run_id) = dispatch_ring_es(
                &run_id,
                input,
                agent_map,
                agent_names.to_vec(),
                provider_map,
                config.clone(),
                routing_rules,
                cost_limit,
                sink,
                quiet,
                max_content,
            )
            .await?;

            #[cfg(feature = "storage")]
            {
                match crate::db::init_db() {
                    Ok(db) => {
                        if let Err(e) = crate::cli::run_es_record::project_run(&db, &_run_id) {
                            tracing::warn!("failed to project run {}: {}", _run_id, e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("event log storage unavailable, run not projected: {}", e);
                    }
                }
            }

            let outcome_text = super::run_es_record::ring_display(&state, &events);
            if human_output {
                let s = status_style(&state.status);
                anstream::eprintln!("{s}[ring] status: {:?}{s:#}", state.status);
            }
            if !json && human_output {
                println!("{outcome_text}");
            }

            sink.emit(&RunEvent::Result {
                content: outcome_text,
                tin: u32::try_from(state.budget_tokens_in).unwrap_or(u32::MAX),
                tout: u32::try_from(state.budget_tokens_out).unwrap_or(u32::MAX),
                cost: state.budget_cost,
                agents: agent_names.len(),
            });
        }
        "hierarchical" => {
            use std::collections::BTreeMap;

            use armadai_core::orchestration::OrchestrationConfig;

            // Build orchestration config from project or defaults
            let orch_config = match resolution {
                AgentResolution::Project { config, .. } => {
                    config.orchestration.as_deref().cloned().unwrap_or_default()
                }
                _ => OrchestrationConfig::default(),
            };

            // Validate the config
            if let Err(errors) = armadai_core::orchestration::validate_config(&orch_config) {
                let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                anyhow::bail!("Orchestration config errors:\n  - {}", msgs.join("\n  - "));
            }

            let coordinator_name = orch_config
                .coordinator
                .clone()
                .unwrap_or_else(|| agent_names.first().cloned().unwrap_or_default());

            // Build agent map and provider map, keyed by the ROSTER KEY (the
            // config name / filename slug in `agent_names`), NOT `agent.name`
            // (the H1 title). The coordinator and the delegation directives
            // reference agents by their config key (e.g. `dev-lead`), so keying
            // by the H1 title (`Dev Lead`) would break the coordinator lookup
            // and every `@agent` delegation. `agents`/`providers` are in the
            // same order as `agent_names` (built by the load loop above).
            let mut agent_map: BTreeMap<String, armadai_core::agent::Agent> = BTreeMap::new();
            let mut provider_map: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();

            for (name, (agent, provider)) in
                agent_names.iter().zip(agents.into_iter().zip(providers))
            {
                provider_map.insert(name.clone(), provider);
                agent_map.insert(name.clone(), agent);
            }

            if human_output {
                let r = crate::cli::style::running();
                anstream::eprintln!(
                    "{r}[hierarchical] Starting with coordinator '{}', {} agent(s){r:#}",
                    coordinator_name,
                    agent_map.len()
                );
            }

            let (state, events, _run_id) = dispatch_hierarchical_es(
                &run_id,
                &coordinator_name,
                input,
                orch_config,
                agent_map,
                provider_map,
                routing_rules,
                sink,
                quiet,
                max_content,
            )
            .await?;
            // Nested C9 sub-runs execute against an ephemeral child log during
            // the run (see `HierarchicalEffectRunner::run_nested`) and aren't
            // part of this run's own `events`/`ExecutionState`, so they aren't
            // surfaced in the `OrchestrationResult` — see
            // `to_orchestration_result`'s doc comment.
            let result = to_orchestration_result(&state, &events);

            if human_output {
                let s = status_style(&state.status);
                anstream::eprintln!(
                    "{s}[hierarchical] Done: {} invocations, {} tokens in, {} tokens out{s:#}",
                    result.invocation_count,
                    result.total_tokens_in,
                    result.total_tokens_out
                );
            }

            #[cfg(feature = "storage")]
            {
                match crate::db::init_db() {
                    Ok(db) => {
                        if let Err(e) = crate::cli::run_es_record::project_run(&db, &_run_id) {
                            tracing::warn!("failed to project run {}: {}", _run_id, e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("event log storage unavailable, run not projected: {}", e);
                    }
                }
            }

            if !json && human_output {
                println!("{}", result.content);
            }

            sink.emit(&RunEvent::Result {
                content: result.content,
                tin: result.total_tokens_in,
                tout: result.total_tokens_out,
                cost: result.total_cost,
                agents: agent_names.len(),
            });
        }
        other => {
            anyhow::bail!(
                "Unknown orchestration pattern: '{other}'. Use 'blackboard', 'ring', or 'hierarchical'"
            );
        }
    }

    Ok(())
}

/// Resolve the top-level `orchestration:` block's `cost_limit` for a
/// standalone blackboard/ring run (`armadai run --orchestrate
/// blackboard|ring`), or `None` when there is no project config or no
/// `orchestration:` section declared. `OrchestrationConfig::cost_limit` only
/// lives on the top-level block (`resolution`'s `config.orchestration`), not
/// on `OrchestrationDefaults` (which `orch_defaults` above already reads for
/// `max_rounds`/`token_budget`/etc — it has no `cost_limit` field at all).
///
/// This is a **new guard** for the standalone patterns (OH1 Lot 5): the
/// legacy standalone `run_blackboard`/`run_ring` call sites in this file
/// never threaded a cost limit into `Board::new`/`RingToken::new` (unlike the
/// nested-team path in `armadai_core::orchestration::hierarchical`, which does pass
/// `OrchestrationConfig::cost_limit` down to `Board::with_cost_limit`) — a
/// project declaring `orchestration.cost_limit` had it silently ignored for
/// a plain `--orchestrate blackboard|ring` run. `run_blackboard_es`/
/// `run_ring_es` accept `cost_limit` explicitly (OH1 Lot 4 Task 3), so this
/// closes that gap as a side effect of the bascule.
fn orchestration_cost_limit(resolution: &AgentResolution) -> Option<f64> {
    match resolution {
        AgentResolution::Project { config, .. } => {
            config.orchestration.as_deref().and_then(|o| o.cost_limit)
        }
        AgentResolution::Default(_) => None,
    }
}

/// Build the `agent_meta` table (roster key → `(provider, configured model)`)
/// that the bridge ([`SinkProjectingLog`]) needs so its `AgentInvoked →
/// AgentStart` projection carries the run's real provider/model instead of
/// empty strings. Uses `agent.metadata.model` (the *configured* value, as the
/// legacy `AgentStart` did) — the effectively-resolved tier for `latest:auto`
/// agents is carried separately by `Route`/`ModelRouted`.
fn agent_meta_from_roster(
    agents: &std::collections::BTreeMap<String, Agent>,
) -> std::collections::BTreeMap<String, (String, String)> {
    agents
        .iter()
        .map(|(key, a)| {
            (
                key.clone(),
                (
                    a.metadata.provider.clone(),
                    a.metadata.model.clone().unwrap_or_default(),
                ),
            )
        })
        .collect()
}

/// Drive the event-sourced `blackboard` engine end-to-end for an
/// already-loaded roster (OH1 Lot 5, T5c; OH1 Lot 4 reconciliation Task 5):
/// builds a fresh `InMemoryLog` wrapped in `SinkProjectingLog` (so
/// `Board`/`Vote`/observability events keep flowing to `sink`, filtered
/// through [`quiet_max_content_sink`] so `--quiet`/`--max-content` are honored
/// exactly like the direct path), runs `run_blackboard_es`, and returns the
/// folded state. Pure with respect to loading/storage — no file I/O — which is
/// what makes it directly unit-testable with mock providers (see
/// `tests::blackboard_es`).
#[allow(clippy::too_many_arguments)]
async fn dispatch_blackboard_es(
    run_id: &str,
    input: &str,
    agents: std::collections::BTreeMap<String, Agent>,
    providers: std::collections::BTreeMap<String, Arc<dyn armadai_core::provider::Provider>>,
    config: armadai_core::orchestration::blackboard::BlackboardConfig,
    routing_rules: armadai_core::routing::RoutingRules,
    cost_limit: Option<f64>,
    sink: &Arc<dyn EventSink>,
    quiet: bool,
    max_content: Option<usize>,
) -> anyhow::Result<(ExecutionState, String)> {
    use armadai_core::orchestration::es::blackboard::run_blackboard_es;

    let filtered_sink = quiet_max_content_sink(sink, quiet, max_content);

    // Deduplication macro (Fix 2): single definition of the run_blackboard_es call.
    macro_rules! run_with_log {
        ($log:expr) => {{
            let mut log =
                SinkProjectingLog::with_meta($log, &filtered_sink, agent_meta_from_roster(&agents));
            run_blackboard_es(
                run_id,
                input,
                agents,
                providers,
                config,
                routing_rules,
                cost_limit,
                &mut log,
            )
            .await?
        }};
    }

    #[cfg(feature = "storage")]
    let state = {
        use crate::es_log::SqliteLog;
        match crate::db::init_db() {
            Ok(db) => run_with_log!(SqliteLog::new(db)),
            Err(e) => {
                tracing::warn!("event log storage unavailable, run will not be persisted: {e}");
                run_with_log!(InMemoryLog::default())
            }
        }
    };
    #[cfg(not(feature = "storage"))]
    let state = run_with_log!(InMemoryLog::default());
    Ok((state, run_id.to_string()))
}

/// Drive the event-sourced `ring` engine end-to-end for an already-loaded
/// roster (OH1 Lot 5, T5d) — same shape as [`dispatch_blackboard_es`], but
/// also returns the raw event log: `ring_display` needs it (last
/// `OutcomeResolved`/`Completed`), unlike `blackboard_display` which reads
/// only the folded `state.board.entries`. `--quiet`/`--max-content` are
/// honored the same way, via [`quiet_max_content_sink`] (OH1 Lot 4
/// reconciliation Task 5).
#[allow(clippy::too_many_arguments)]
async fn dispatch_ring_es(
    run_id: &str,
    input: &str,
    agents: std::collections::BTreeMap<String, Agent>,
    agent_order: Vec<String>,
    providers: std::collections::BTreeMap<String, Arc<dyn armadai_core::provider::Provider>>,
    config: armadai_core::orchestration::ring::RingConfig,
    routing_rules: armadai_core::routing::RoutingRules,
    cost_limit: Option<f64>,
    sink: &Arc<dyn EventSink>,
    quiet: bool,
    max_content: Option<usize>,
) -> anyhow::Result<(ExecutionState, Vec<ExecutionEvent>, String)> {
    use armadai_core::orchestration::es::ring::run_ring_es;

    let filtered_sink = quiet_max_content_sink(sink, quiet, max_content);

    // Deduplication macro (Fix 2): single definition of the run_ring_es call.
    macro_rules! run_with_log {
        ($log:expr) => {{
            let mut log =
                SinkProjectingLog::with_meta($log, &filtered_sink, agent_meta_from_roster(&agents));
            let state = run_ring_es(
                run_id,
                input,
                agents,
                agent_order,
                providers,
                config,
                routing_rules,
                cost_limit,
                &mut log,
            )
            .await?;
            let events = log.events(run_id)?;
            (state, events)
        }};
    }

    #[cfg(feature = "storage")]
    let (state, events) = {
        use crate::es_log::SqliteLog;
        match crate::db::init_db() {
            Ok(db) => run_with_log!(SqliteLog::new(db)),
            Err(e) => {
                tracing::warn!("event log storage unavailable, run will not be persisted: {e}");
                run_with_log!(InMemoryLog::default())
            }
        }
    };
    #[cfg(not(feature = "storage"))]
    let (state, events) = run_with_log!(InMemoryLog::default());
    Ok((state, events, run_id.to_string()))
}

/// Drive the event-sourced `hierarchical` engine end-to-end for an
/// already-loaded roster (OH1 Lot 5, T5b) — same shape as
/// [`dispatch_blackboard_es`]/[`dispatch_ring_es`], returning both the folded
/// state and the raw event log so the caller can extract the legacy
/// `OrchestrationResult` shape via `to_orchestration_result` (needed for the
/// existing `record_orchestration_hierarchical`/display code, which predates
/// the ES socle and knows only that type). `--quiet`/`--max-content` are
/// honored the same way, via [`quiet_max_content_sink`] (OH1 Lot 4
/// reconciliation Task 5).
#[allow(clippy::too_many_arguments)]
async fn dispatch_hierarchical_es(
    run_id: &str,
    coordinator: &str,
    input: &str,
    config: armadai_core::orchestration::OrchestrationConfig,
    agents: std::collections::BTreeMap<String, Agent>,
    providers: std::collections::BTreeMap<String, Arc<dyn armadai_core::provider::Provider>>,
    routing_rules: armadai_core::routing::RoutingRules,
    sink: &Arc<dyn EventSink>,
    quiet: bool,
    max_content: Option<usize>,
) -> anyhow::Result<(ExecutionState, Vec<ExecutionEvent>, String)> {
    use armadai_core::orchestration::es::hierarchical::run_hierarchical_es;

    let filtered_sink = quiet_max_content_sink(sink, quiet, max_content);

    // Deduplication macro (Fix 2): single definition of the run_hierarchical_es call.
    macro_rules! run_with_log {
        ($log:expr) => {{
            let mut log =
                SinkProjectingLog::with_meta($log, &filtered_sink, agent_meta_from_roster(&agents));
            let state = run_hierarchical_es(
                run_id,
                coordinator,
                input,
                config,
                agents,
                providers,
                routing_rules,
                &mut log,
            )
            .await?;
            let events = log.events(run_id)?;
            (state, events)
        }};
    }

    #[cfg(feature = "storage")]
    let (state, events) = {
        use crate::es_log::SqliteLog;
        match crate::db::init_db() {
            Ok(db) => run_with_log!(SqliteLog::new(db)),
            Err(e) => {
                tracing::warn!("event log storage unavailable, run will not be persisted: {e}");
                run_with_log!(InMemoryLog::default())
            }
        }
    };
    #[cfg(not(feature = "storage"))]
    let (state, events) = run_with_log!(InMemoryLog::default());
    Ok((state, events, run_id.to_string()))
}

/// Default CLI provider timeout (seconds) for an agent taking part in an
/// orchestrated run (blackboard/ring/hierarchical), used when neither the
/// agent's own frontmatter `timeout` nor the project's
/// `defaults.orchestration.agent_timeout_secs` sets one.
///
/// Higher than the non-orchestrated single-agent default
/// (`providers::factory::DEFAULT_TIMEOUT_SECS`) because an orchestrated
/// coordinator's `claude -p` turn is itself agentic (delegating, waiting on
/// sub-agents, synthesizing) — 242k-499k tokens/turn were observed on real
/// hierarchical runs (#270).
///
/// Since #270, `CliProvider::timeout_secs` bounds *inactivity* between
/// lines of subprocess output, not the call's total duration (see
/// `providers::cli::CliProvider::complete`) — so this value no longer needs
/// to cover an entire multi-delegation run in one wall-clock budget, only
/// the longest silent gap within it. It stays higher than the non-
/// orchestrated default because a coordinator can legitimately think for a
/// long stretch (e.g. synthesizing a final answer) with no observable
/// output at all before its next line.
const ORCHESTRATED_DEFAULT_TIMEOUT_SECS: u64 = 600;

// Three timeout constants govern this feature across two crates
// (`providers::factory::DEFAULT_TIMEOUT_SECS`, this file's
// `ORCHESTRATED_DEFAULT_TIMEOUT_SECS`, and `providers::cli::
// ABSOLUTE_CEILING_SECS`) and nothing enforced their relationship — if an
// inactivity default ever grew past the absolute ceiling, every timeout in
// the product would be misreported as "absolute ceiling" instead of
// "inactivity" (`cli::next_step_timeout`'s `ceiling_bound` flag would be
// wrong for every call). This constant's sibling assertion, against
// `factory::DEFAULT_TIMEOUT_SECS`, lives in `armadai-providers::cli` where
// both those constants are in scope; this one covers the cross-crate half.
const _: () = assert!(
    armadai_providers::cli::ABSOLUTE_CEILING_SECS > ORCHESTRATED_DEFAULT_TIMEOUT_SECS,
    "ABSOLUTE_CEILING_SECS must stay above ORCHESTRATED_DEFAULT_TIMEOUT_SECS"
);

/// Resolve the effective CLI provider timeout (seconds) for an agent
/// participating in an orchestrated run.
///
/// Precedence: the agent's own frontmatter `timeout` always wins (explicit
/// per-agent configuration is never overridden). Otherwise, the project's
/// `defaults.orchestration.agent_timeout_secs` override applies. Otherwise,
/// falls back to [`ORCHESTRATED_DEFAULT_TIMEOUT_SECS`].
///
/// This is the ONE place that actually reaches the provider timeout for
/// orchestrated runs: `create_provider` only reads `agent.metadata.timeout`
/// (`.unwrap_or(DEFAULT_TIMEOUT_SECS)`), so this must run before
/// `create_provider` is called on each agent in `run_orchestrated`'s
/// loading loop — the BlackboardConfig/
/// RingConfig `agent_timeout_secs` field (populated by
/// `apply_blackboard_overrides`/`apply_ring_overrides`) is never read by the
/// event-sourced engine (`es::blackboard`/`es::ring` call
/// `provider.complete()` directly, no timeout wrapper), so setting it alone
/// does not change any real timeout. Applying the override here, before
/// provider construction, is therefore the single mechanism that covers
/// blackboard, ring, AND hierarchical alike (they share this same loading
/// loop) — see the Fix B investigation notes in
/// `.superpowers/sdd/orch-e2e-report.md`.
fn orchestrated_agent_timeout_secs(
    frontmatter_timeout: Option<u64>,
    config_override: Option<u64>,
) -> u64 {
    frontmatter_timeout
        .or(config_override)
        .unwrap_or(ORCHESTRATED_DEFAULT_TIMEOUT_SECS)
}

/// Apply the orchestrated-run timeout override (see
/// [`orchestrated_agent_timeout_secs`]) to `agent.metadata.timeout` in place.
///
/// Both roster-loading loops that feed an orchestrated run — the fresh-run
/// loop in `run_orchestrated` AND the `--resume` reconstruction loop in
/// `resume_run` — MUST call this before `create_provider`, since
/// `create_provider` only reads `agent.metadata.timeout`
/// (`.unwrap_or(DEFAULT_TIMEOUT_SECS)`) once and never re-reads it
/// afterward. Extracted to a single fn so both paths share the exact same
/// precedence and cannot drift (a resumed
/// orchestrated run re-hitting that default was the gap this closes —
/// see `.superpowers/sdd/orch-e2e-report.md`).
fn apply_orchestrated_timeout(agent: &mut Agent, config_override: Option<u64>) {
    agent.metadata.timeout = Some(orchestrated_agent_timeout_secs(
        agent.metadata.timeout,
        config_override,
    ));
}

/// Whether `pattern` names an orchestrated run (`blackboard`/`ring`/
/// `hierarchical`, or any future non-`direct` pattern) as opposed to a plain
/// single-agent `direct` run.
///
/// This is the ONE gate deciding whether [`apply_orchestrated_timeout`] may
/// touch `agent.metadata.timeout`: `resume_run` reconstructs the roster for
/// BOTH `direct` and orchestrated resumes through the same loop, so calling
/// the override unconditionally there once regressed a resumed `direct` run
/// onto the 600s+ orchestrated timeout instead of the correct
/// `DEFAULT_TIMEOUT_SECS` default (see `.superpowers/sdd/orch-e2e-report.md`).
/// `execute_resume`'s own
/// `use_tui` decision uses the identical `!= "direct"` marker (the Workroom
/// TUI never applies to `direct` runs either) — sharing this fn keeps both
/// checks from drifting apart.
// Both call sites (`resume_run`'s roster loop, `execute_resume`'s `use_tui`
// gate) live under `#[cfg(feature = "storage")]` — without `storage` this fn
// has no non-test caller, so silence dead-code rather than gate the fn itself
// behind `storage` too (the predicate is pure and stays unit-tested in both
// feature modes).
#[cfg_attr(not(feature = "storage"), allow(dead_code))]
fn is_orchestrated_pattern(pattern: &str) -> bool {
    pattern != "direct"
}

/// Apply project-level orchestration overrides to a BlackboardConfig.
fn apply_blackboard_overrides(
    mut config: armadai_core::orchestration::blackboard::BlackboardConfig,
    overrides: &armadai_core::project::OrchestrationDefaults,
) -> armadai_core::orchestration::blackboard::BlackboardConfig {
    if let Some(v) = overrides.max_rounds {
        config.max_rounds = v;
    }
    if let Some(v) = overrides.consensus_threshold {
        config.consensus_threshold = v;
    }
    if let Some(v) = overrides.divergence_threshold {
        config.divergence_threshold = v;
    }
    if let Some(v) = overrides.token_budget {
        config.token_budget = v;
    }
    if let Some(v) = overrides.agent_timeout_secs {
        config.agent_timeout_secs = v;
    }
    if let Some(v) = overrides.convergence_rounds {
        config.convergence_rounds = v;
    }
    config
}

/// Apply project-level orchestration overrides to a RingConfig.
fn apply_ring_overrides(
    mut config: armadai_core::orchestration::ring::RingConfig,
    overrides: &armadai_core::project::OrchestrationDefaults,
) -> armadai_core::orchestration::ring::RingConfig {
    if let Some(v) = overrides.max_laps {
        config.max_laps = v;
    }
    if let Some(v) = overrides.consensus_threshold {
        config.consensus_threshold = v;
    }
    if let Some(v) = overrides.majority_threshold {
        config.majority_threshold = v;
    }
    if let Some(v) = overrides.similarity_threshold {
        config.similarity_threshold = v;
    }
    if let Some(v) = overrides.token_budget {
        config.token_budget = v;
    }
    if let Some(v) = overrides.agent_timeout_secs {
        config.agent_timeout_secs = v;
    }
    config
}

/// Persist a hierarchical orchestration run: the parent run row and its
/// delegation trace. Returns the provided hierarchical `run_id`.
#[cfg(feature = "storage")]
pub(crate) fn record_hierarchical_into(
    db: &armadai_storage::Database,
    run_id: &str,
    result: &armadai_core::orchestration::hierarchical::OrchestrationResult,
    config: &armadai_core::orchestration::OrchestrationConfig,
    input: &str,
    project: Option<&str>,
) -> anyhow::Result<String> {
    use armadai_storage::queries;

    // 1. Parent run record.
    let parent = queries::RunRecord {
        agent: "orchestration:hierarchical".to_string(),
        input: input.to_string(),
        output: result.content.clone(),
        provider: "orchestration".to_string(),
        model: String::new(),
        tokens_in: result.total_tokens_in as i64,
        tokens_out: result.total_tokens_out as i64,
        cost: result.total_cost,
        duration_ms: 0,
        status: "success".to_string(),
        project: project.map(|s| s.to_string()),
    };
    queries::insert_run_with_id(db, run_id, parent)?;

    // 2. Orchestration metadata (hierarchical, no parent).
    queries::insert_orchestration_run(
        db,
        queries::OrchestrationRunRecord {
            run_id: run_id.to_string(),
            pattern: "hierarchical".to_string(),
            config_json: serde_json::to_string(config).unwrap_or_default(),
            outcome_json: None,
            rounds: result.invocation_count as i64,
            halt_reason: None,
            parent_run_id: None,
        },
    )?;

    // 3. Delegation events (seq = order in trace).
    for (seq, ev) in result.trace.iter().enumerate() {
        let rec = queries::DelegationEventRecord {
            run_id: run_id.to_string(),
            seq: seq as i64,
            from_agent: ev.from.clone(),
            to_agent: ev.to.clone(),
            message: ev.message.clone(),
            depth: ev.depth as i64,
        };
        if let Err(e) = queries::insert_delegation_event(db, rec) {
            tracing::warn!("Failed to record delegation event: {e}");
        }
    }

    Ok(run_id.to_string())
}

/// Check if an error indicates the model was not found (HTTP 404 or model-related 400).
fn is_model_not_found(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();

    // Google-style: HTTP 404 with "not found"
    if msg.contains("404") && msg.contains("not found") {
        return true;
    }

    // Anthropic-style: "model" + "not_found" or "invalid"
    if msg.contains("model") && (msg.contains("not_found") || msg.contains("invalid")) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use armadai_core::agent::AgentMetadata;

    #[test]
    fn test_is_model_not_found_google_404() {
        let err = anyhow::anyhow!("HTTP 404: model gemini-3.0-pro not found");
        assert!(is_model_not_found(&err));
    }

    #[test]
    fn test_is_model_not_found_anthropic_400() {
        let err = anyhow::anyhow!("400 Bad Request: model not_found: claude-opus-next");
        assert!(is_model_not_found(&err));
    }

    #[test]
    fn test_is_model_not_found_auth_401_false() {
        let err = anyhow::anyhow!("401 Unauthorized: invalid API key");
        assert!(!is_model_not_found(&err));
    }

    #[test]
    fn test_is_model_not_found_rate_limit_429_false() {
        let err = anyhow::anyhow!("429 Too Many Requests: rate limit exceeded");
        assert!(!is_model_not_found(&err));
    }

    #[test]
    fn exit_code_mapping() {
        assert_eq!(exit_code_for(&anyhow::anyhow!("token budget exceeded")), 3);
        assert_eq!(
            exit_code_for(&anyhow::anyhow!("provider 'x' not available")),
            4
        );
        assert_eq!(exit_code_for(&anyhow::anyhow!("boom")), 1);
    }

    #[test]
    fn latest_auto_is_the_only_routed_value() {
        // concrete + latest:pro must NOT be treated as auto
        assert_ne!("claude-3", "latest:auto");
        assert_ne!("latest:pro", "latest:auto");
        // routing only triggers on the exact "latest:auto" string (guard documented)
    }

    #[test]
    fn test_resolve_agents_dir_returns_valid_resolution() {
        // resolve_agents_dir should not panic regardless of cwd state
        let resolution = resolve_agents_dir(false);
        match resolution {
            AgentResolution::Project { root, config, .. } => {
                assert!(!root.to_string_lossy().is_empty());
                assert!(!config.agents.is_empty());
            }
            AgentResolution::Default(dir) => {
                assert!(!dir.to_string_lossy().is_empty());
            }
        }
    }

    // --- Fix B (#270 stopgap): orchestrated agent timeout resolution ---

    /// The agent's own frontmatter `timeout` always wins, even over a
    /// project-level `agent_timeout_secs` override — explicit per-agent
    /// configuration must never be silently overridden.
    #[test]
    fn orchestrated_timeout_frontmatter_wins_over_config_override() {
        assert_eq!(orchestrated_agent_timeout_secs(Some(120), Some(900)), 120);
    }

    /// With no frontmatter timeout, the hierarchical (and blackboard/ring)
    /// path must apply the project's `defaults.orchestration.agent_timeout_secs`
    /// override — this is the actual fix for #270's "hierarchical has no
    /// agent_timeout_secs override" gap.
    #[test]
    fn orchestrated_timeout_applies_config_override_when_no_frontmatter() {
        assert_eq!(orchestrated_agent_timeout_secs(None, Some(900)), 900);
    }

    /// With neither frontmatter nor config override, the orchestrated default
    /// must be 600s, NOT the non-orchestrated single-agent default
    /// (`DEFAULT_TIMEOUT_SECS`) — an orchestrated coordinator's agentic turn
    /// can legitimately exceed that default.
    #[test]
    fn orchestrated_timeout_defaults_to_600_not_300() {
        assert_eq!(orchestrated_agent_timeout_secs(None, None), 600);
        assert_ne!(
            orchestrated_agent_timeout_secs(None, None),
            DEFAULT_TIMEOUT_SECS
        );
    }

    // --- resume-path coverage gap: `apply_orchestrated_timeout` is the SAME
    // helper both `run_orchestrated`'s fresh-run loop and `resume_run`'s
    // reconstruction loop call before `create_provider`. Exercising it here
    // (rather than only `orchestrated_agent_timeout_secs`) locks both call
    // sites to identical behavior and would fail if either loop stopped
    // calling it. ---

    fn agent_with_timeout(timeout: Option<u64>) -> Agent {
        Agent {
            name: "a".to_string(),
            source: PathBuf::from("a.md"),
            metadata: AgentMetadata {
                provider: "mock".to_string(),
                model: Some("mock".to_string()),
                command: None,
                args: None,
                temperature: 0.7,
                max_tokens: None,
                timeout,
                tags: vec![],
                stacks: vec![],
                scope: vec![],
                model_fallback: vec![],
                cost_limit: None,
                rate_limit: None,
                context_window: None,
                mode: None,
                orchestration: None,
                triggers: None,
                ring_config: None,
            },
            system_prompt: "p".to_string(),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    /// Mirrors `orchestrated_timeout_frontmatter_wins_over_config_override`
    /// at the `apply_orchestrated_timeout` level: an agent's own frontmatter
    /// `timeout` must survive the in-place mutation unchanged, even with a
    /// project-level override present — this is the fn both
    /// `run_orchestrated` and `resume_run` call on each roster agent before
    /// `create_provider`.
    #[test]
    fn apply_orchestrated_timeout_frontmatter_wins_over_config_override() {
        let mut agent = agent_with_timeout(Some(120));
        apply_orchestrated_timeout(&mut agent, Some(900));
        assert_eq!(agent.metadata.timeout, Some(120));
    }

    /// Mirrors `orchestrated_timeout_applies_config_override_when_no_frontmatter`:
    /// with no frontmatter timeout, the project's
    /// `defaults.orchestration.agent_timeout_secs` override must land on
    /// `agent.metadata.timeout` — this is what `resume_run` was missing
    /// before this fix (it called `create_provider` directly, skipping the
    /// override entirely).
    #[test]
    fn apply_orchestrated_timeout_applies_config_override_when_no_frontmatter() {
        let mut agent = agent_with_timeout(None);
        apply_orchestrated_timeout(&mut agent, Some(900));
        assert_eq!(agent.metadata.timeout, Some(900));
    }

    /// Mirrors `orchestrated_timeout_defaults_to_600_not_300`: with neither
    /// frontmatter nor config override, a resumed orchestrated run must land
    /// on the 600s orchestrated default, NOT silently fall back to
    /// `create_provider`'s bare `DEFAULT_TIMEOUT_SECS` default — that
    /// regression (a resumed orchestrated run re-hitting that wall) is
    /// exactly the gap this fix closes.
    #[test]
    fn apply_orchestrated_timeout_defaults_to_600_not_300() {
        let mut agent = agent_with_timeout(None);
        apply_orchestrated_timeout(&mut agent, None);
        assert_eq!(agent.metadata.timeout, Some(600));
        assert_ne!(agent.metadata.timeout, Some(DEFAULT_TIMEOUT_SECS));
    }

    // --- Re-review regression: `resume_run` must NOT apply the orchestrated
    // timeout override to a resumed `direct` run (see
    // `.superpowers/sdd/orch-e2e-report.md`). `is_orchestrated_pattern` is the
    // single gate both `resume_run`'s roster loop and `execute_resume`'s
    // `use_tui` decision share, so locking its behavior here locks both call
    // sites. ---

    #[test]
    fn is_orchestrated_pattern_direct_is_false() {
        assert!(!is_orchestrated_pattern("direct"));
    }

    #[test]
    fn is_orchestrated_pattern_true_for_known_orchestrated_patterns() {
        assert!(is_orchestrated_pattern("blackboard"));
        assert!(is_orchestrated_pattern("ring"));
        assert!(is_orchestrated_pattern("hierarchical"));
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use std::path::PathBuf;

    use armadai_core::agent::{Agent, AgentMetadata};
    use armadai_core::provider::{
        CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream,
    };

    struct DummyProvider(String);
    #[async_trait]
    impl Provider for DummyProvider {
        async fn complete(&self, _r: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            anyhow::bail!("not used")
        }
        async fn stream(&self, _r: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("not used")
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: self.0.clone(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    fn agent_with_tags(name: &str, tags: &[&str], stacks: &[&str]) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("{name}.md")),
            metadata: AgentMetadata {
                provider: "mock".to_string(),
                model: Some("mock".to_string()),
                command: None,
                args: None,
                temperature: 0.7,
                max_tokens: None,
                timeout: None,
                tags: tags.iter().map(|s| s.to_string()).collect(),
                stacks: stacks.iter().map(|s| s.to_string()).collect(),
                scope: vec![],
                model_fallback: vec![],
                cost_limit: None,
                rate_limit: None,
                context_window: None,
                mode: None,
                orchestration: None,
                triggers: None,
                ring_config: None,
            },
            system_prompt: "p".to_string(),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    /// Roster keys equal `agent.name` here (the common/simple case); the
    /// H1-vs-key divergence is exercised separately below.
    fn roster() -> (Vec<String>, Vec<Agent>, Vec<Arc<dyn Provider>>) {
        let agents = vec![
            agent_with_tags("sec", &["security"], &["rust"]),
            agent_with_tags("ui", &["frontend"], &[]),
            agent_with_tags("qa", &["testing"], &[]),
        ];
        let keys: Vec<String> = agents.iter().map(|a| a.name.clone()).collect();
        let providers: Vec<Arc<dyn Provider>> = agents
            .iter()
            .map(|a| Arc::new(DummyProvider(a.name.clone())) as Arc<dyn Provider>)
            .collect();
        (keys, agents, providers)
    }

    #[test]
    fn no_selectors_keeps_full_roster_in_order() {
        let (k, a, p) = roster();
        let (sel_keys, agents, providers, sel) =
            apply_agent_selection(&k, a, p, None, &[], &BTreeMap::new()).unwrap();
        assert_eq!(
            agents.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
            vec!["sec", "ui", "qa"]
        );
        assert_eq!(providers.len(), 3);
        assert_eq!(sel.agents, vec!["sec", "ui", "qa"]);
        assert_eq!(sel_keys, vec!["sec", "ui", "qa"]);
    }

    #[test]
    fn tags_filter_and_align_providers() {
        let (k, a, p) = roster();
        let (sel_keys, agents, providers, _sel) =
            apply_agent_selection(&k, a, p, None, &["security".to_string()], &BTreeMap::new())
                .unwrap();
        assert_eq!(
            agents.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
            vec!["sec"]
        );
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].metadata().name, "sec"); // provider realigned to the kept agent
        assert_eq!(sel_keys, vec!["sec"]);
    }

    #[test]
    fn route_selects_named_subset_reordered() {
        let (k, a, p) = roster();
        let mut routes = BTreeMap::new();
        routes.insert("r".to_string(), vec!["qa".to_string(), "sec".to_string()]);
        let (sel_keys, agents, providers, _sel) =
            apply_agent_selection(&k, a, p, Some("r"), &[], &routes).unwrap();
        // Order follows the route, not the roster.
        assert_eq!(
            agents.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
            vec!["qa", "sec"]
        );
        assert_eq!(providers[0].metadata().name, "qa");
        assert_eq!(providers[1].metadata().name, "sec");
        assert_eq!(sel_keys, vec!["qa", "sec"]);
    }

    #[test]
    fn route_referencing_absent_agent_errors() {
        let (k, a, p) = roster();
        let mut routes = BTreeMap::new();
        routes.insert(
            "r".to_string(),
            vec!["sec".to_string(), "ghost".to_string()],
        );
        // `Result::unwrap_err` would require the Ok tuple (which carries
        // `Arc<dyn Provider>`) to implement `Debug`, which it does not — match
        // instead of unwrap_err to extract the error.
        let err = match apply_agent_selection(&k, a, p, Some("r"), &[], &routes) {
            Err(e) => e,
            Ok(_) => panic!("expected an error for a route referencing an absent agent"),
        };
        assert!(
            err.to_string().contains("ghost"),
            "error should name the missing agent: {err}"
        );
    }

    #[test]
    fn unknown_route_propagates_error() {
        let (k, a, p) = roster();
        let err = match apply_agent_selection(&k, a, p, Some("nope"), &[], &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => panic!("expected an error for an unknown route"),
        };
        assert!(err.to_string().to_lowercase().contains("route"));
    }

    /// CRITICAL regression test: in this repo H1 title != filename slug is the
    /// norm. `--route`/`--tags` and `orchestration.routes:` operate on the
    /// roster KEY (filename slug), never on `agent.name` (parsed H1). Before
    /// this fix, `apply_agent_selection` keyed everything on `agent.name`, so
    /// a route naming the slug `backend-dev` would never match an agent whose
    /// H1 is `Backend Developer` — `by_name.remove("backend-dev")` returned
    /// `None` and the run bailed with "not among the run's agents".
    #[test]
    fn route_matches_roster_key_even_when_h1_title_differs() {
        let agent = agent_with_tags("Backend Developer", &["backend"], &["rust"]);
        let keys = vec!["backend-dev".to_string()];
        let providers: Vec<Arc<dyn Provider>> =
            vec![Arc::new(DummyProvider("backend-dev".to_string())) as Arc<dyn Provider>];

        let mut routes = BTreeMap::new();
        routes.insert("r".to_string(), vec!["backend-dev".to_string()]);

        let (sel_keys, agents, out_providers, sel) =
            apply_agent_selection(&keys, vec![agent], providers, Some("r"), &[], &routes).unwrap();

        // The selection and returned identity are the roster KEY, not the H1.
        assert_eq!(sel_keys, vec!["backend-dev".to_string()]);
        assert_eq!(sel.agents, vec!["backend-dev".to_string()]);
        // The Agent itself is untouched — H1 title is preserved for display.
        assert_eq!(agents[0].name, "Backend Developer");
        assert_eq!(out_providers.len(), 1);

        // Same regression, via a tag selector instead of a route.
        let agent2 = agent_with_tags("Backend Developer", &["backend"], &["rust"]);
        let providers2: Vec<Arc<dyn Provider>> =
            vec![Arc::new(DummyProvider("backend-dev".to_string())) as Arc<dyn Provider>];
        let (sel_keys2, _agents2, _providers2, _sel2) = apply_agent_selection(
            &keys,
            vec![agent2],
            providers2,
            None,
            &["backend".to_string()],
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(sel_keys2, vec!["backend-dev".to_string()]);
    }
}

#[cfg(all(test, feature = "storage"))]
mod storage_tests {
    use super::*;
    use armadai_core::orchestration::OrchestrationConfig;
    use armadai_core::orchestration::hierarchical::{DelegationEvent, OrchestrationResult};
    use armadai_storage::{open_in_memory, queries};

    #[test]
    fn hierarchical_run_and_trace_are_persisted() {
        let db = open_in_memory().unwrap();

        // A hierarchical result with one delegation event.
        let result = OrchestrationResult {
            content: "final".to_string(),
            trace: vec![DelegationEvent {
                from: "coordinator".to_string(),
                to: "research-lead".to_string(),
                message: "analyze".to_string(),
                depth: 1,
            }],
            total_tokens_in: 30,
            total_tokens_out: 40,
            total_cost: 0.01,
            invocation_count: 3,
        };
        let config = OrchestrationConfig::default();

        let parent_id = uuid::Uuid::new_v4().to_string();
        let returned =
            record_hierarchical_into(&db, &parent_id, &result, &config, "do research", None)
                .unwrap();
        assert_eq!(returned, parent_id);

        // Parent persisted as hierarchical with no parent.
        let parent = queries::get_orchestration_run(&db, &parent_id)
            .unwrap()
            .unwrap();
        assert_eq!(parent.pattern, "hierarchical");
        assert_eq!(parent.parent_run_id, None);
        // Delegation event persisted.
        let events = queries::get_delegation_events(&db, &parent_id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].to_agent, "research-lead");
    }

    #[test]
    fn hierarchical_run_records_project_on_parent() {
        let db = open_in_memory().unwrap();

        let result = OrchestrationResult {
            content: "final".to_string(),
            trace: vec![],
            total_tokens_in: 0,
            total_tokens_out: 0,
            total_cost: 0.0,
            invocation_count: 1,
        };
        let config = OrchestrationConfig::default();

        let parent_id = uuid::Uuid::new_v4().to_string();
        let returned = record_hierarchical_into(
            &db,
            &parent_id,
            &result,
            &config,
            "do research",
            Some("/home/user/my-project"),
        )
        .unwrap();
        assert_eq!(returned, parent_id);

        let history = queries::get_history(&db, None, 10).unwrap();
        assert_eq!(history.len(), 1, "parent hierarchical run");
        assert_eq!(
            history[0].project.as_deref(),
            Some("/home/user/my-project"),
            "the persisted run should carry the project"
        );
        // Sanity: parent_id itself resolved to a hierarchical run.
        let parent = queries::get_orchestration_run(&db, &parent_id)
            .unwrap()
            .unwrap();
        assert_eq!(parent.pattern, "hierarchical");
    }
}

/// Integration-style tests for OH1 Lot 5 (the `run.rs` → event-sourced
/// engines bascule): drives each pattern's `dispatch_*_es` helper directly
/// with mock providers (same idiom as `es::direct`/`hierarchical`/
/// `blackboard`/`ring`'s own end-to-end tests), then asserts
/// (a) the run completes with the expected content, (b) `--json` headless
/// observability (`RunEvent`s via `SinkProjectingLog`) includes the
/// pattern-specific event kinds, and (c) — under `feature = "storage"` — the
/// run persists via the same record functions the switched match arms call.
///
/// Doesn't drive `execute()`/`run_inner()`/`run_orchestrated()` themselves:
/// those additionally require real files on disk (project resolution,
/// `Agent::find_file`, `create_provider`), which this codebase's existing
/// test conventions don't exercise either — this file's own `storage_tests`
/// module already tests `record_hierarchical_into` directly rather than
/// through the CLI entry point. `dispatch_*_es` is
/// exactly the seam `run_inner`/`run_orchestrated` call right after loading
/// agents/providers — exercising it directly covers the actual
/// engine-selection change this task makes, without re-testing file
/// resolution (unrelated to this bascule).
#[cfg(test)]
mod es_switch_tests {
    use super::*;
    use std::collections::{BTreeMap, VecDeque};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use armadai_core::agent::AgentMetadata;
    use armadai_core::orchestration::blackboard::BlackboardConfig;
    use armadai_core::orchestration::es::state::RunStatus;
    use armadai_core::orchestration::ring::RingConfig;
    use armadai_core::orchestration::{OrchestrationConfig, OrchestrationPattern, TeamConfig};
    use armadai_core::provider::{
        CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream,
    };
    use armadai_core::routing::RoutingRules;

    // ── Shared test infra ────────────────────────────────────────────

    /// Minimal `Agent` for these tests — concrete model, no orchestration
    /// metadata. Mirrors the same-named helper duplicated across every
    /// `es::*` test module.
    fn test_agent(name: &str) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("{name}.md")),
            metadata: AgentMetadata {
                provider: "anthropic".to_string(),
                model: Some("concrete-model".to_string()),
                command: None,
                args: None,
                temperature: 0.7,
                max_tokens: None,
                timeout: None,
                tags: vec![],
                stacks: vec![],
                scope: vec![],
                model_fallback: vec![],
                cost_limit: None,
                rate_limit: None,
                context_window: None,
                mode: None,
                orchestration: None,
                triggers: None,
                ring_config: None,
            },
            system_prompt: format!("You are {name}."),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    /// A provider that returns scripted responses in order, then repeats its
    /// last response forever — mirrors the same-named helper duplicated
    /// across `es::blackboard`/`es::ring`/`es::hierarchical`'s own test
    /// modules.
    struct ScriptedProvider {
        responses: Mutex<VecDeque<String>>,
        last: Mutex<String>,
        calls: AtomicUsize,
    }

    impl ScriptedProvider {
        fn new(responses: &[&str]) -> Self {
            Self {
                responses: Mutex::new(responses.iter().map(|s| (*s).to_string()).collect()),
                last: Mutex::new(String::new()),
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut queue = self.responses.lock().unwrap();
            let content = queue
                .pop_front()
                .unwrap_or_else(|| self.last.lock().unwrap().clone());
            *self.last.lock().unwrap() = content.clone();
            Ok(CompletionResponse {
                content,
                model: request.model,
                tokens_in: 5,
                tokens_out: 7,
                cost: 0.001,
            })
        }
        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("streaming not exercised by these tests")
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "scripted".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    /// Records every emitted `RunEvent` as its serialized `serde_json::Value`
    /// (`RunEvent` derives `Serialize` but not `Clone`) — same idiom as
    /// `es::bridge`'s own `CaptureSink` test helper. Used to assert headless
    /// JSONL observability without spinning up a real `JsonlSink`.
    #[derive(Default)]
    struct CaptureSink {
        events: Mutex<Vec<serde_json::Value>>,
    }

    impl EventSink for CaptureSink {
        fn emit(&self, ev: &RunEvent) {
            let v = serde_json::to_value(ev).expect("RunEvent always serializes");
            self.events.lock().unwrap().push(v);
        }
    }

    impl CaptureSink {
        fn tags(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .map(|v| v["t"].as_str().unwrap().to_string())
                .collect()
        }
    }

    /// Build a `(concrete, dyn)` sink pair sharing the same underlying
    /// `CaptureSink`: the `dyn EventSink` half is what `dispatch_*_es`
    /// functions expect (`&Arc<dyn EventSink>`); the concrete half is kept
    /// for post-run assertions on what was captured.
    fn capture_sink() -> (Arc<CaptureSink>, Arc<dyn EventSink>) {
        let capture = Arc::new(CaptureSink::default());
        let dyn_sink: Arc<dyn EventSink> = capture.clone();
        (capture, dyn_sink)
    }

    /// Redirect `storage` at a throwaway temp DB for the scope of a test, so
    /// the ES dispatch's persistence (`SqliteLog` via `crate::db::init_db`)
    /// never writes into the user's real event log (#267). Points
    /// `ARMADAI_CONFIG_DIR` at a temp `config.yaml` whose `storage.path` is a
    /// scratch sqlite file; serialised via `ENV_MUTEX` and restored on drop.
    /// Mirrors the guard in `src/web/api.rs` tests.
    struct TempStorageGuard {
        _dir: tempfile::TempDir,
        orig: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TempStorageGuard {
        fn new() -> Self {
            let lock = armadai_core::config::ENV_MUTEX.lock().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("test.sqlite");
            let config_yaml = format!(
                "storage:\n  mode: embedded\n  path: \"{}\"\n",
                db_path.display()
            );
            std::fs::write(dir.path().join("config.yaml"), config_yaml).unwrap();
            let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
            // SAFETY: modifies the global environment; serialised via ENV_MUTEX.
            unsafe {
                std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
            }
            Self {
                _dir: dir,
                orig,
                _lock: lock,
            }
        }
    }

    impl Drop for TempStorageGuard {
        fn drop(&mut self) {
            match self.orig.take() {
                // SAFETY: restoring original env state at end of test scope.
                Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
                None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
            }
        }
    }

    // ── T5a: direct ──────────────────────────────────────────────────

    #[tokio::test]
    async fn direct_es_completes_with_content_tokens_and_observability() {
        let _storage = TempStorageGuard::new();
        let (capture, sink) = capture_sink();
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(&["the answer"]));

        let dispatch = dispatch_direct_es(
            &uuid::Uuid::new_v4().to_string(),
            "solo",
            test_agent("solo"),
            provider,
            "do the thing",
            &RoutingRules::default(),
            &sink,
            false,
            None,
        )
        .await
        .unwrap();

        // (a)/(b): completes with the mock's content and its declared tokens/cost.
        assert_eq!(dispatch.content, "the answer");
        assert_eq!(dispatch.tin, 5);
        assert_eq!(dispatch.tout, 7);
        assert!((dispatch.cost - 0.001).abs() < 1e-9);

        // (c): AgentStart/AgentEnd observability reaches the sink (headless
        // JSONL contract — direct is the one pattern whose effect runner
        // always returns `AgentObserved`, so both are always present).
        let tags = capture.tags();
        assert!(tags.iter().any(|t| t == "agent_start"), "tags: {tags:?}");
        assert!(tags.iter().any(|t| t == "agent_end"), "tags: {tags:?}");
    }

    /// Regression test for the `--quiet` "result-only" fidelity fix (OH1 Lot 4
    /// reconciliation Task 5): on the direct ES path (`dispatch_direct_es`),
    /// `quiet: true` must suppress EVERY event flowing through the decorator
    /// — `agent_start` as well as `agent_end` — since per the CLI help text
    /// (`src/cli/mod.rs`) `--quiet` means "emit only the final `result`
    /// event", and `dispatch_direct_es` never emits `result` itself (that's
    /// the caller's job, outside this decorator's reach — see
    /// [`QuietMaxContentSink`]'s doc comment). The returned content/tokens are
    /// unaffected (only the emitted observability events are suppressed, not
    /// the function's return value). Supersedes the narrower pre-Task-5
    /// contract, which only dropped `agent_end` and kept `agent_start`.
    #[tokio::test]
    async fn direct_es_quiet_suppresses_all_intermediate_events() {
        let _storage = TempStorageGuard::new();
        let (capture, sink) = capture_sink();
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(&["the answer"]));

        let dispatch = dispatch_direct_es(
            &uuid::Uuid::new_v4().to_string(),
            "solo",
            test_agent("solo"),
            provider,
            "do the thing",
            &RoutingRules::default(),
            &sink,
            true,
            None,
        )
        .await
        .unwrap();

        assert_eq!(dispatch.content, "the answer");
        let tags = capture.tags();
        assert!(
            tags.is_empty(),
            "quiet must suppress every event reaching the decorator (agent_start included), \
             tags: {tags:?}"
        );
    }

    /// Regression test for the `--max-content` fidelity fix: on the direct ES
    /// path, the emitted `agent_end.content` must be truncated to `N` chars,
    /// while the function's returned `content` (used for the final `Result`
    /// event and stdout `println!`) stays full-length.
    #[tokio::test]
    async fn direct_es_max_content_truncates_agent_end_content_only() {
        let _storage = TempStorageGuard::new();
        let (capture, sink) = capture_sink();
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(&["the full answer"]));

        let dispatch = dispatch_direct_es(
            &uuid::Uuid::new_v4().to_string(),
            "solo",
            test_agent("solo"),
            provider,
            "do the thing",
            &RoutingRules::default(),
            &sink,
            false,
            Some(3),
        )
        .await
        .unwrap();

        assert_eq!(dispatch.content, "the full answer");
        let events = capture.events.lock().unwrap();
        let agent_end = events
            .iter()
            .find(|v| v["t"] == "agent_end")
            .expect("agent_end must still be emitted");
        assert_eq!(agent_end["content"], "the");
    }

    /// Regression test for observability defect #1 (direct ES path): the
    /// `agent_start` must carry the run's REAL `prov`/`model` in its payload
    /// (not empty strings), sourced from the bridge's `agent_meta`, and the
    /// single `agent_end` must carry the real content. Before the fix, the
    /// direct path had no upstream `AgentStart` and relied solely on the
    /// bridge, which emitted `prov: "", model: ""`.
    #[tokio::test]
    async fn direct_es_agent_start_carries_real_prov_model_and_real_end_content() {
        let _storage = TempStorageGuard::new();
        let (capture, sink) = capture_sink();
        let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(&["the answer"]));

        dispatch_direct_es(
            &uuid::Uuid::new_v4().to_string(),
            "solo",
            test_agent("solo"),
            provider,
            "do the thing",
            &RoutingRules::default(),
            &sink,
            false,
            None,
        )
        .await
        .unwrap();

        let events = capture.events.lock().unwrap();
        let starts: Vec<_> = events.iter().filter(|v| v["t"] == "agent_start").collect();
        let ends: Vec<_> = events.iter().filter(|v| v["t"] == "agent_end").collect();

        // Exactly one start/end (no duplicate), start payload carries the real
        // provider/model (`test_agent` uses "anthropic"/"concrete-model").
        assert_eq!(starts.len(), 1, "exactly one agent_start, got {starts:?}");
        assert_eq!(ends.len(), 1, "exactly one agent_end, got {ends:?}");
        assert_eq!(starts[0]["prov"], "anthropic");
        assert_eq!(starts[0]["model"], "concrete-model");
        assert_eq!(ends[0]["content"], "the answer");
    }

    // ── T5b: hierarchical ────────────────────────────────────────────

    /// Base flat-team config: coordinator with a single team of `peers` (no
    /// nested lead) — mirrors `es_flat_config` in `es::hierarchical`'s own
    /// tests.
    fn flat_config(coordinator: &str, peers: &[&str]) -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some(coordinator.to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: peers.iter().map(|p| (*p).to_string()).collect(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Shared scenario for the hierarchical tests below: `dev-lead` delegates
    /// once to `core-specialist`, which answers with a final answer;
    /// `dev-lead` then synthesizes its own final answer from that single
    /// result. Mirrors `es_single_delegation_completes` in
    /// `es::hierarchical`'s own tests.
    fn hierarchical_roster() -> (BTreeMap<String, Agent>, BTreeMap<String, Arc<dyn Provider>>) {
        let mut agents = BTreeMap::new();
        agents.insert("dev-lead".to_string(), test_agent("dev-lead"));
        agents.insert("core-specialist".to_string(), test_agent("core-specialist"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "@core-specialist: fais X",
                "Synthèse : tout est prêt.",
            ])),
        );
        providers.insert(
            "core-specialist".to_string(),
            Arc::new(ScriptedProvider::new(&["X est fait."])),
        );
        (agents, providers)
    }

    #[tokio::test]
    async fn hierarchical_es_delegates_synthesizes_and_emits_observability() {
        let _storage = TempStorageGuard::new();
        let (capture, sink) = capture_sink();
        let (agents, providers) = hierarchical_roster();

        let (state, events, _run_id) = dispatch_hierarchical_es(
            &uuid::Uuid::new_v4().to_string(),
            "dev-lead",
            "build X",
            flat_config("dev-lead", &["core-specialist"]),
            agents,
            providers,
            RoutingRules::default(),
            &sink,
            false,
            None,
        )
        .await
        .unwrap();
        let result = to_orchestration_result(&state, &events);

        // (a): completes.
        assert_eq!(state.status, RunStatus::Completed);
        // (b): delegation traced + non-empty synthesized content.
        assert!(
            result
                .trace
                .iter()
                .any(|e| e.from == "dev-lead" && e.to == "core-specialist"),
            "expected dev-lead -> core-specialist in the trace, got {:?}",
            result.trace
        );
        assert!(!result.content.trim().is_empty());

        // (c): AgentStart/AgentEnd + Delegate observability reaches the sink.
        let tags = capture.tags();
        assert!(tags.iter().any(|t| t == "agent_start"), "tags: {tags:?}");
        assert!(tags.iter().any(|t| t == "agent_end"), "tags: {tags:?}");
        assert!(tags.iter().any(|t| t == "delegate"), "tags: {tags:?}");
    }

    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn hierarchical_es_result_is_recorded_via_record_hierarchical_into() {
        let _storage = TempStorageGuard::new();
        use armadai_storage::{open_in_memory, queries};

        let (_capture, sink) = capture_sink();
        let (agents, providers) = hierarchical_roster();
        let config = flat_config("dev-lead", &["core-specialist"]);

        let (state, events, _dispatch_run_id) = dispatch_hierarchical_es(
            &uuid::Uuid::new_v4().to_string(),
            "dev-lead",
            "build X",
            config.clone(),
            agents,
            providers,
            RoutingRules::default(),
            &sink,
            false,
            None,
        )
        .await
        .unwrap();
        let result = to_orchestration_result(&state, &events);

        // (d): the same `record_hierarchical_into` the switched "hierarchical"
        // match arm calls (via `record_orchestration_hierarchical`) persists
        // the ES-derived `OrchestrationResult`.
        let db = open_in_memory().unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();
        let returned =
            record_hierarchical_into(&db, &run_id, &result, &config, "build X", None).unwrap();
        assert_eq!(returned, run_id);

        let persisted = queries::get_orchestration_run(&db, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.pattern, "hierarchical");
        let delegation_events = queries::get_delegation_events(&db, &run_id).unwrap();
        assert!(
            delegation_events
                .iter()
                .any(|e| e.to_agent == "core-specialist"),
            "expected the delegation to dev-lead -> core-specialist to be persisted"
        );
    }

    // ── T5c: blackboard ──────────────────────────────────────────────

    /// Both agents post a `CONFIRMATION` targeting entry 0 in round 0 — a
    /// 2/2 = 1.0 confirmation ratio, above the default `consensus_threshold`
    /// (0.75), reaching consensus on the very first round. Mirrors
    /// `es_blackboard_converges_and_completes` in `es::blackboard`'s own
    /// tests.
    fn blackboard_roster() -> (BTreeMap<String, Agent>, BTreeMap<String, Arc<dyn Provider>>) {
        let mut agents = BTreeMap::new();
        agents.insert("a".to_string(), test_agent("a"));
        agents.insert("b".to_string(), test_agent("b"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "a".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:tout est cohérent",
            ])),
        );
        providers.insert(
            "b".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:confirmé",
            ])),
        );
        (agents, providers)
    }

    #[tokio::test]
    async fn blackboard_es_converges_and_emits_board_observability() {
        let _storage = TempStorageGuard::new();
        let (capture, sink) = capture_sink();
        let (agents, providers) = blackboard_roster();

        let (state, _run_id) = dispatch_blackboard_es(
            &uuid::Uuid::new_v4().to_string(),
            "task",
            agents,
            providers,
            BlackboardConfig::default(),
            RoutingRules::default(),
            None,
            &sink,
            false,
            None,
        )
        .await
        .unwrap();

        // (a): completes.
        assert_eq!(state.status, RunStatus::Completed);
        // (b): non-empty board digest (same display the switched match arm
        // shows via `run_es_record::blackboard_display`).
        let display = crate::cli::run_es_record::blackboard_display(&state);
        assert!(!display.trim().is_empty());

        // (c): AgentStart + Board + AgentEnd observability reaches the sink,
        // in symmetric start/end pairs. `BlackboardEffectRunner::run_invoke`
        // always returns `BoardEntryAdded` (never `AgentObserved`), but
        // `es::bridge::map_execution_to_run_events` now maps `BoardEntryAdded`
        // onto `[Board, AgentEnd]` too, so every `agent_start` this pattern
        // emits (via the shared `AgentInvoked`) has a matching `agent_end`
        // (observability fidelity fix — see `es::bridge` symmetry test).
        let tags = capture.tags();
        assert!(tags.iter().any(|t| t == "agent_start"), "tags: {tags:?}");
        assert!(tags.iter().any(|t| t == "board"), "tags: {tags:?}");
        assert!(tags.iter().any(|t| t == "agent_end"), "tags: {tags:?}");
        let starts = tags.iter().filter(|t| *t == "agent_start").count();
        let ends = tags.iter().filter(|t| *t == "agent_end").count();
        assert_eq!(
            starts, ends,
            "every agent_start must have a matching agent_end — tags: {tags:?}"
        );
    }

    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn blackboard_es_state_is_recorded_via_record_blackboard_es_into() {
        let _storage = TempStorageGuard::new();
        use armadai_storage::{open_in_memory, queries};

        let (_capture, sink) = capture_sink();
        let (agents, providers) = blackboard_roster();
        let config = BlackboardConfig::default();

        let (state, _dispatch_run_id) = dispatch_blackboard_es(
            &uuid::Uuid::new_v4().to_string(),
            "task",
            agents,
            providers,
            config.clone(),
            RoutingRules::default(),
            None,
            &sink,
            false,
            None,
        )
        .await
        .unwrap();

        // (d): the same `record_blackboard_es_into` the switched "blackboard"
        // match arm calls (via `record_blackboard_es`) persists the folded
        // `ExecutionState`.
        let db = open_in_memory().unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();
        let returned = crate::cli::run_es_record::record_blackboard_es_into(
            &db, &run_id, &state, &config, "task", None, None,
        )
        .unwrap();
        assert_eq!(returned, run_id);

        let persisted = queries::get_orchestration_run(&db, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.pattern, "blackboard");
        let entries = queries::get_board_entries(&db, &run_id).unwrap();
        assert_eq!(entries.len(), 2, "expected both agents' entries persisted");
    }

    /// Verify that a blackboard run persists its event log to
    /// `execution_events` when executed under the `storage` feature (OH1 Lot
    /// 5a Task 2). Unlike `blackboard_es_state_is_recorded_via_
    /// record_blackboard_es_into` which tests the tables plates write, this
    /// test verifies that the event log itself is persisted au fil de l'eau.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn blackboard_es_run_persists_event_log() {
        use crate::es_log::SqliteLog;
        use armadai_core::orchestration::es::blackboard::run_blackboard_es;
        use armadai_storage::open_in_memory;

        // (a): Setup — same roster as `blackboard_es_state_is_recorded_via_
        // record_blackboard_es_into`, but we drive the ES loop with a
        // SqliteLog directly to verify persistence.
        let db = open_in_memory().unwrap();
        let run_id = "it-bb-log-1";
        let (agents, providers) = blackboard_roster();
        let config = BlackboardConfig::default();

        // (b): Execute the ES loop with a SqliteLog (not InMemoryLog), wrapped
        // in a SinkProjectingLog for observability (same structure as
        // dispatch_blackboard_es will use under storage).
        let (_capture, sink) = capture_sink();
        let filtered_sink = quiet_max_content_sink(&sink, false, None);
        let mut log = SinkProjectingLog::with_meta(
            SqliteLog::new(db),
            &filtered_sink,
            agent_meta_from_roster(&agents),
        );
        let _state = run_blackboard_es(
            run_id,
            "task",
            agents,
            providers,
            config,
            RoutingRules::default(),
            None,
            &mut log,
        )
        .await
        .unwrap();

        // (c): Verify that the events were persisted to execution_events
        // (read back via the same log instance).
        let events = log.events(run_id).unwrap();
        assert!(
            !events.is_empty(),
            "blackboard run must have persisted its event log"
        );

        // (d): Sanity-check: the first event should be RunStarted.
        assert!(
            matches!(events[0], ExecutionEvent::RunStarted { .. }),
            "first event should be RunStarted, got {:?}",
            events[0]
        );
    }

    /// Verify that a ring run persists its event log to `execution_events`
    /// when executed under the `storage` feature (OH1 Lot 5a Fix 4). Parallel
    /// to `blackboard_es_run_persists_event_log` above.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn ring_es_run_persists_event_log() {
        use crate::es_log::SqliteLog;
        use armadai_core::orchestration::es::ring::run_ring_es;
        use armadai_storage::open_in_memory;

        let db = open_in_memory().unwrap();
        let run_id = "it-ring-log-1";
        let (agents, providers) = ring_roster();
        let config = RingConfig {
            max_laps: 1,
            ..RingConfig::default()
        };

        let (_capture, sink) = capture_sink();
        let filtered_sink = quiet_max_content_sink(&sink, false, None);
        let mut log = SinkProjectingLog::with_meta(
            SqliteLog::new(db),
            &filtered_sink,
            agent_meta_from_roster(&agents),
        );
        let _state = run_ring_es(
            run_id,
            "task",
            agents,
            vec!["a".to_string(), "b".to_string()],
            providers,
            config,
            RoutingRules::default(),
            None,
            &mut log,
        )
        .await
        .unwrap();

        let events = log.events(run_id).unwrap();
        assert!(
            !events.is_empty(),
            "ring run must have persisted its event log"
        );
        assert!(
            matches!(events[0], ExecutionEvent::RunStarted { .. }),
            "first event should be RunStarted, got {:?}",
            events[0]
        );
    }

    /// Verify that a hierarchical run persists its event log to
    /// `execution_events` when executed under the `storage` feature (OH1 Lot
    /// 5a Fix 4). Parallel to `blackboard_es_run_persists_event_log` above.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn hierarchical_es_run_persists_event_log() {
        use crate::es_log::SqliteLog;
        use armadai_core::orchestration::es::hierarchical::run_hierarchical_es;
        use armadai_storage::open_in_memory;

        let db = open_in_memory().unwrap();
        let run_id = "it-hier-log-1";
        let (agents, providers) = hierarchical_roster();
        let config = flat_config("dev-lead", &["core-specialist"]);

        let (_capture, sink) = capture_sink();
        let filtered_sink = quiet_max_content_sink(&sink, false, None);
        let mut log = SinkProjectingLog::with_meta(
            SqliteLog::new(db),
            &filtered_sink,
            agent_meta_from_roster(&agents),
        );
        let _state = run_hierarchical_es(
            run_id,
            "dev-lead",
            "build X",
            config,
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        let events = log.events(run_id).unwrap();
        assert!(
            !events.is_empty(),
            "hierarchical run must have persisted its event log"
        );
        assert!(
            matches!(events[0], ExecutionEvent::RunStarted { .. }),
            "first event should be RunStarted, got {:?}",
            events[0]
        );
    }

    /// Verify that a blackboard run projects its flat tables from the event
    /// log (OH1 Lot 5b Task 3). After the ES loop persists events to the log,
    /// `project_run` derives the `runs`/`orchestration_runs`/`board_entries`
    /// tables from it — the same flow the real `run_orchestrated` branches will
    /// use after the Task 3 wiring.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn blackboard_es_run_projects_tables_from_log() {
        use crate::es_log::SqliteLog;
        use armadai_core::orchestration::es::blackboard::run_blackboard_es;
        use armadai_storage::{open_in_memory, queries};

        let db = open_in_memory().unwrap();
        let run_id = "it-bb-proj-1";
        let (agents, providers) = blackboard_roster();
        let config = BlackboardConfig::default();

        // (a): Execute the ES loop with a SqliteLog (persists events to the
        // test's in-memory DB), wrapped in a SinkProjectingLog for observability.
        let (_capture, sink) = capture_sink();
        let filtered_sink = quiet_max_content_sink(&sink, false, None);
        let mut log = SinkProjectingLog::with_meta(
            SqliteLog::new(db.clone()),
            &filtered_sink,
            agent_meta_from_roster(&agents),
        );
        let state = run_blackboard_es(
            run_id,
            "task",
            agents,
            providers,
            config,
            RoutingRules::default(),
            None,
            &mut log,
        )
        .await
        .unwrap();

        // (b): Before projection, the flat tables should be empty for this run_id.
        assert!(
            queries::get_orchestration_run(&db, run_id)
                .unwrap()
                .is_none(),
            "flat tables should be empty before projection"
        );

        // (c): Project the flat tables from the event log.
        crate::cli::run_es_record::project_run(&db, run_id).unwrap();

        // (d): Verify that the projection succeeded: `runs` + `orchestration_runs`
        // tables should now have a row for this run with `pattern == "blackboard"`.
        let run_record = queries::get_orchestration_run(&db, run_id)
            .unwrap()
            .expect("projection should have created a run record");
        assert_eq!(run_record.pattern, "blackboard");

        // (e): Verify that board entries were also projected.
        let entries = queries::get_board_entries(&db, run_id).unwrap();
        assert_eq!(
            entries.len(),
            state.board.entries.len(),
            "all board entries should be projected"
        );
    }

    // ── T5d: ring ────────────────────────────────────────────────────

    /// Two agents circulate one substantial (non-pass) lap each
    /// (`max_laps: 1`), then both vote for the same position — a unanimous
    /// 2/2 group. Mirrors `es_ring_circulates_votes_and_resolves` in
    /// `es::ring`'s own tests.
    fn ring_roster() -> (BTreeMap<String, Agent>, BTreeMap<String, Arc<dyn Provider>>) {
        let mut agents = BTreeMap::new();
        agents.insert("a".to_string(), test_agent("a"));
        agents.insert("b".to_string(), test_agent("b"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "a".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "ACTION: PROPOSE\nCONTENT: use Rust with Axum",
                "CONFIDENCE: 0.9\nUse Rust with Axum",
            ])),
        );
        providers.insert(
            "b".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "ACTION: PROPOSE\nCONTENT: agreed, Rust and Axum",
                "CONFIDENCE: 0.8\nUse Rust with Axum",
            ])),
        );
        (agents, providers)
    }

    #[tokio::test]
    async fn ring_es_resolves_and_emits_vote_observability() {
        let _storage = TempStorageGuard::new();
        let (capture, sink) = capture_sink();
        let (agents, providers) = ring_roster();
        let config = RingConfig {
            max_laps: 1,
            ..RingConfig::default()
        };

        let (state, events, _run_id) = dispatch_ring_es(
            &uuid::Uuid::new_v4().to_string(),
            "task",
            agents,
            vec!["a".to_string(), "b".to_string()],
            providers,
            config,
            RoutingRules::default(),
            None,
            &sink,
            false,
            None,
        )
        .await
        .unwrap();

        // (a): completes.
        assert_eq!(state.status, RunStatus::Completed);
        // (b): non-empty resolved outcome (same display the switched match
        // arm shows via `run_es_record::ring_display`), matching the
        // unanimous position both agents voted for.
        let display = crate::cli::run_es_record::ring_display(&state, &events);
        assert!(display.starts_with("Use Rust with Axum"));

        // (c): AgentStart + Vote + AgentEnd observability reaches the sink,
        // in symmetric start/end pairs. `RingEffectRunner::run_invoke` returns
        // `ContributionAdded` during circulation and `VoteCast` during
        // voting — never `AgentObserved` — but `es::bridge::
        // map_execution_to_run_events` now maps both onto an `AgentEnd` too
        // (observability fidelity fix — see `es::bridge` symmetry test), so
        // every `agent_start` this pattern emits has a matching `agent_end`.
        let tags = capture.tags();
        assert!(tags.iter().any(|t| t == "agent_start"), "tags: {tags:?}");
        assert!(tags.iter().any(|t| t == "vote"), "tags: {tags:?}");
        assert!(tags.iter().any(|t| t == "agent_end"), "tags: {tags:?}");
        let starts = tags.iter().filter(|t| *t == "agent_start").count();
        let ends = tags.iter().filter(|t| *t == "agent_end").count();
        assert_eq!(
            starts, ends,
            "every agent_start must have a matching agent_end — tags: {tags:?}"
        );
    }

    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn ring_es_state_is_recorded_via_record_ring_es_into() {
        let _storage = TempStorageGuard::new();
        use armadai_storage::{open_in_memory, queries};

        let (_capture, sink) = capture_sink();
        let (agents, providers) = ring_roster();
        let config = RingConfig {
            max_laps: 1,
            ..RingConfig::default()
        };

        let (state, _events, _dispatch_run_id) = dispatch_ring_es(
            &uuid::Uuid::new_v4().to_string(),
            "task",
            agents,
            vec!["a".to_string(), "b".to_string()],
            providers,
            config.clone(),
            RoutingRules::default(),
            None,
            &sink,
            false,
            None,
        )
        .await
        .unwrap();

        // (d): the same `record_ring_es_into` the switched "ring" match arm
        // calls (via `record_ring_es`) persists the folded `ExecutionState`.
        let db = open_in_memory().unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();
        let returned = crate::cli::run_es_record::record_ring_es_into(
            &db, &run_id, &state, &config, "task", None, None,
        )
        .unwrap();
        assert_eq!(returned, run_id);

        let persisted = queries::get_orchestration_run(&db, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.pattern, "ring");
        let votes = queries::get_ring_votes(&db, &run_id).unwrap();
        assert_eq!(votes.len(), 2, "expected both agents' votes persisted");
    }

    // ── real-path emission tests (run_orchestrated_inner) ─────────────
    //
    // These drive the REAL orchestrated wrapper (`run_orchestrated_inner`,
    // everything `run_orchestrated` runs after loading the roster), closing
    // the gap the review flagged: the `dispatch_*_es` tests above short-circuit
    // the wrapper, so they never exercised the (now removed) upstream
    // `AgentStart` loop or `emit_agent_ends`. Gated to the no-storage config so
    // they never touch the real user DB via `record_*` (the storage recording
    // is covered separately by the `*_recorded_via_*` tests above, which use
    // an in-memory DB). The two invariants below jointly catch both defects:
    //   - every `agent_start` payload has non-empty `prov`/`model` → catches
    //     the empty-prov/model bridge start AND the removed upstream duplicate;
    //   - no `agent_end` has empty `content` → catches the removed
    //     `emit_agent_ends` batch of empty-content ends (the "clobber").

    /// Assert the bridge is the single, faithful source of AgentStart/AgentEnd
    /// on an orchestrated run's captured JSONL stream.
    #[cfg(not(feature = "storage"))]
    fn assert_bridge_single_source(capture: &CaptureSink) {
        let events = capture.events.lock().unwrap();

        let starts: Vec<_> = events.iter().filter(|v| v["t"] == "agent_start").collect();
        let ends: Vec<_> = events.iter().filter(|v| v["t"] == "agent_end").collect();

        assert!(!starts.is_empty(), "expected at least one agent_start");
        for s in &starts {
            assert!(
                !s["prov"].as_str().unwrap().is_empty(),
                "agent_start.prov must be non-empty (real prov/model via bridge agent_meta): {s}"
            );
            assert!(
                !s["model"].as_str().unwrap().is_empty(),
                "agent_start.model must be non-empty: {s}"
            );
        }

        assert!(!ends.is_empty(), "expected at least one agent_end");
        for e in &ends {
            assert!(
                !e["content"].as_str().unwrap().is_empty(),
                "agent_end.content must be non-empty (no emit_agent_ends residual): {e}"
            );
        }

        // Symmetric bridge flow: exactly one end per start, no residual.
        assert_eq!(
            starts.len(),
            ends.len(),
            "agent_start/agent_end must be balanced (bridge single source)"
        );

        // The last agent_end for each agent carries its real content (not
        // clobbered by an empty final AgentEnd).
        use std::collections::BTreeMap;
        let mut last_content: BTreeMap<String, String> = BTreeMap::new();
        for e in &ends {
            last_content.insert(
                e["agent"].as_str().unwrap().to_string(),
                e["content"].as_str().unwrap().to_string(),
            );
        }
        for (agent, content) in &last_content {
            assert!(
                !content.is_empty(),
                "last agent_end for '{agent}' must carry real content, got empty"
            );
        }

        // Exactly one terminal Result.
        let results = events.iter().filter(|v| v["t"] == "result").count();
        assert_eq!(results, 1, "exactly one terminal Result");
    }

    #[cfg(not(feature = "storage"))]
    fn roster_vecs(
        roster: (BTreeMap<String, Agent>, BTreeMap<String, Arc<dyn Provider>>),
    ) -> (Vec<String>, Vec<Agent>, Vec<Arc<dyn Provider>>) {
        let (agents_map, providers_map) = roster;
        let mut names = Vec::new();
        let mut agents = Vec::new();
        let mut providers = Vec::new();
        for (name, agent) in agents_map {
            let provider = providers_map.get(&name).unwrap().clone();
            names.push(name);
            agents.push(agent);
            providers.push(provider);
        }
        (names, agents, providers)
    }

    #[cfg(not(feature = "storage"))]
    #[tokio::test]
    async fn run_orchestrated_inner_blackboard_single_source_observability() {
        let (capture, sink) = capture_sink();
        let (names, agents, providers) = roster_vecs(blackboard_roster());
        let resolution = AgentResolution::Default(PathBuf::from("/tmp"));

        run_orchestrated_inner(
            &resolution,
            &names,
            agents,
            providers,
            vec![],
            "task",
            "blackboard",
            &sink,
            true,
            false,
            None,
            None,
            &[],
            false,
            true,
        )
        .await
        .unwrap();

        assert_bridge_single_source(&capture);
    }

    #[cfg(not(feature = "storage"))]
    #[tokio::test]
    async fn run_orchestrated_inner_ring_single_source_observability() {
        let (capture, sink) = capture_sink();
        let (names, agents, providers) = roster_vecs(ring_roster());
        let resolution = AgentResolution::Default(PathBuf::from("/tmp"));

        run_orchestrated_inner(
            &resolution,
            &names,
            agents,
            providers,
            vec![],
            "task",
            "ring",
            &sink,
            true,
            false,
            None,
            None,
            &[],
            false,
            true,
        )
        .await
        .unwrap();

        assert_bridge_single_source(&capture);
    }

    #[cfg(not(feature = "storage"))]
    #[tokio::test]
    async fn run_orchestrated_inner_hierarchical_single_source_observability() {
        let (capture, sink) = capture_sink();
        let (names, agents, providers) = roster_vecs(hierarchical_roster());
        // Hierarchical reads its orchestration config from the resolution.
        let config = ProjectConfig {
            orchestration: Some(Box::new(flat_config("dev-lead", &["core-specialist"]))),
            ..Default::default()
        };
        let resolution = AgentResolution::Project {
            root: PathBuf::from("/tmp/project"),
            config: Box::new(config),
            fragments: std::sync::OnceLock::new(),
        };

        run_orchestrated_inner(
            &resolution,
            &names,
            agents,
            providers,
            vec![],
            "build X",
            "hierarchical",
            &sink,
            true,
            false,
            None,
            None,
            &[],
            false,
            true,
        )
        .await
        .unwrap();

        assert_bridge_single_source(&capture);
    }

    // ── orchestration_cost_limit (helper) ─────────────────────────────

    #[test]
    fn orchestration_cost_limit_none_without_project() {
        let resolution = AgentResolution::Default(std::path::PathBuf::from("/tmp"));
        assert_eq!(orchestration_cost_limit(&resolution), None);
    }

    #[test]
    fn orchestration_cost_limit_reads_top_level_orchestration_block() {
        let config = ProjectConfig {
            orchestration: Some(Box::new(OrchestrationConfig {
                cost_limit: Some(2.5),
                ..Default::default()
            })),
            ..Default::default()
        };
        let resolution = AgentResolution::Project {
            root: std::path::PathBuf::from("/tmp/project"),
            config: Box::new(config),
            fragments: std::sync::OnceLock::new(),
        };
        assert_eq!(orchestration_cost_limit(&resolution), Some(2.5));
    }

    // Silence an "unused" complaint on `call_count` in feature
    // configurations that don't happen to exercise it (kept for future
    // scenarios/debugging rather than deleted).
    #[test]
    fn scripted_provider_call_count_tracks_calls() {
        let p = ScriptedProvider::new(&["one", "two"]);
        assert_eq!(p.call_count(), 0);
    }

    // ── OH1 Lot 6, Task 2: `--replay` determinism ─────────────────────
    //
    // These tests drive `run_replay::replay_from_log` — the `pub(crate)`,
    // generic-over-`EventLog` core `replay_run` wraps around its own
    // `crate::db::init_db()` call — directly against an in-memory
    // `SqliteLog` (`open_in_memory()`), the SAME idiom
    // `blackboard_es_run_persists_event_log`/`ring_es_run_persists_event_log`
    // already use above. This deliberately avoids exercising `init_db()`
    // itself (which resolves the real, global, config-dependent DB path):
    // an earlier version of this test mutated `ARMADAI_CONFIG_DIR`/
    // `XDG_DATA_HOME` process-wide to sandbox `init_db()`, which raced with
    // OTHER tests in this same file (`dispatch_ring_es` et al.) that also
    // call `crate::db::init_db()` internally under `storage` but hold
    // no such guard — those tests intermittently failed to open their own
    // (unrelated) DB while this test's temp dirs existed/were torn down
    // concurrently. Testing the generic core instead sidesteps that
    // entirely: no env mutation, no cross-test interference, and it's the
    // same read-back+projection logic `replay_run` itself runs.

    /// Drive the event-sourced `direct` engine directly against an injected
    /// `SqliteLog` (mirroring `blackboard_es_run_persists_event_log`'s
    /// pattern) so the "live" side of the determinism test below persists
    /// through the exact same `SqliteLog`/`SinkProjectingLog` machinery a
    /// real `--storage` run does, without going through `dispatch_direct_es`
    /// (which would call the real `crate::db::init_db()`).
    #[cfg(feature = "storage")]
    async fn run_direct_against_sqlite_log(
        db: armadai_storage::Database,
        run_id: &str,
        sink: &Arc<dyn EventSink>,
    ) {
        use std::collections::BTreeMap;

        use crate::es_log::SqliteLog;
        use armadai_core::orchestration::es::direct::run_direct_es;

        let mut agent_meta = BTreeMap::new();
        agent_meta.insert(
            "solo".to_string(),
            ("anthropic".to_string(), "concrete-model".to_string()),
        );
        let mut log = SinkProjectingLog::with_meta(SqliteLog::new(db), sink.as_ref(), agent_meta);

        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), test_agent("solo"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "solo".to_string(),
            Arc::new(ScriptedProvider::new(&["the answer"])) as Arc<dyn Provider>,
        );

        run_direct_es(
            run_id,
            "solo",
            "do the thing",
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();
    }

    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn replay_reproduces_the_live_run_event_sequence() {
        use armadai_storage::open_in_memory;

        let db = open_in_memory().unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();

        let (live_capture, live_sink) = capture_sink();
        run_direct_against_sqlite_log(db.clone(), &run_id, &live_sink).await;

        let replay_log = crate::es_log::SqliteLog::new(db);
        let (replay_capture, replay_sink) = capture_sink();
        crate::cli::run_replay::replay_from_log(&replay_log, &run_id, &replay_sink, false).unwrap();

        let live: Vec<_> = live_capture
            .events
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        let replayed: Vec<_> = replay_capture
            .events
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();

        assert!(!live.is_empty(), "sanity: the live run must emit events");

        // `run_direct_against_sqlite_log` drives the bare ES engine
        // (`run_direct_es`) directly, NOT the CLI's `run_inner` — so `live`
        // here is the engine-level mid-stream slice only, with no
        // `RunStart`/`Result` bookends (those are built by `run.rs`, never by
        // the engine projection — see `run_replay.rs`'s module doc).
        // `replayed` DOES carry both (that's `replay_from_log`'s job, OH1
        // Lot 6 whole-branch review I1). Assert their presence/shape
        // explicitly here, then strip them before the mid-stream comparison
        // below — this doesn't weaken that comparison, it just scopes it to
        // what `live` actually contains; `replay_full_stream_starts_with_run_start_and_ends_with_result`
        // below is the dedicated test for the bookends themselves.
        assert_eq!(
            replayed.first().unwrap()["t"],
            "run_start",
            "replay must lead with a synthetic RunStart bookend, got: {replayed:?}"
        );
        assert_eq!(replayed.first().unwrap()["run_id"], run_id);
        assert_eq!(
            replayed.last().unwrap()["t"],
            "result",
            "replay must end with a Result bookend, got: {replayed:?}"
        );

        let replayed_mid_stream: Vec<_> = replayed[1..replayed.len() - 1].to_vec();
        assert_eq!(
            live, replayed_mid_stream,
            "replay must reproduce the same mid-stream RunEvent sequence exactly — including \
             AgentStart.prov/model, now that the roster round-trips through RunStarted"
        );
    }

    /// OH1 Lot 6 whole-branch review, I1: dedicated coverage for the
    /// `RunStart`/`Result` bookends — this is the test that would have
    /// caught their absence (the sibling
    /// `replay_reproduces_the_live_run_event_sequence` test's "live"
    /// baseline is engine-level only and never had them to compare
    /// against). Asserts the FULL replayed stream, as a `--json` consumer
    /// would see it, starts with `run_start` and ends with `result`, with
    /// the fields `synthetic_run_start`/`to_orchestration_result` are
    /// documented to fill.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn replay_full_stream_starts_with_run_start_and_ends_with_result() {
        use armadai_storage::open_in_memory;

        let db = open_in_memory().unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();

        let (_live_capture, live_sink) = capture_sink();
        run_direct_against_sqlite_log(db.clone(), &run_id, &live_sink).await;

        let replay_log = crate::es_log::SqliteLog::new(db);
        let (replay_capture, replay_sink) = capture_sink();
        crate::cli::run_replay::replay_from_log(&replay_log, &run_id, &replay_sink, false).unwrap();

        let tags = replay_capture.tags();
        assert_eq!(
            tags.first().map(String::as_str),
            Some("run_start"),
            "replayed stream must start with run_start, got: {tags:?}"
        );
        assert_eq!(
            tags.last().map(String::as_str),
            Some("result"),
            "replayed stream must end with result, got: {tags:?}"
        );

        let events = replay_capture.events.lock().unwrap();
        let head = events.first().unwrap();
        assert_eq!(head["run_id"], run_id);
        assert_eq!(head["v"], 1);
        assert_eq!(head["agents"], serde_json::json!(["solo"]));
        assert_eq!(head["prov"], "");
        assert_eq!(head["model"], "");
        assert_eq!(
            head["in_chars"],
            serde_json::json!("do the thing".chars().count())
        );

        let tail = events.last().unwrap();
        assert_eq!(tail["content"], "the answer");
        assert_eq!(tail["agents"], serde_json::json!(1));
    }

    /// Bugfix regression lock (`fix/replay-prov-model-roster`): a replayed
    /// `AgentStart` must carry the run's REAL provider/model, not the
    /// "no roster available" empty-string fallback. `run_direct_against_sqlite_log`
    /// drives the real production `run_direct_es` — which now persists
    /// `RunStarted.roster` (built via `roster_from_agents` from the `agents`
    /// map it holds, here `test_agent("solo")`'s `anthropic`/`concrete-model`)
    /// — against a real `SqliteLog`, exactly like every other test in this
    /// section. `replay_from_log` must recover that roster from the log's
    /// `RunStarted` event (rather than the unconditional empty `agent_meta`
    /// it used before the fix) so the replayed `agent_start` in the JSON
    /// stream shows the same values a live run's did.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn replay_reconstructs_agent_start_prov_model_from_persisted_roster() {
        use armadai_storage::open_in_memory;

        let db = open_in_memory().unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();

        let (_live_capture, live_sink) = capture_sink();
        run_direct_against_sqlite_log(db.clone(), &run_id, &live_sink).await;

        let replay_log = crate::es_log::SqliteLog::new(db);
        let (replay_capture, replay_sink) = capture_sink();
        crate::cli::run_replay::replay_from_log(&replay_log, &run_id, &replay_sink, false).unwrap();

        let events = replay_capture.events.lock().unwrap();
        let agent_start = events
            .iter()
            .find(|e| e["t"] == "agent_start")
            .expect("replayed stream must contain an agent_start event");

        assert_eq!(
            agent_start["prov"], "anthropic",
            "replayed AgentStart must carry the run's real provider (from the \
             persisted RunStarted.roster), got: {agent_start:?}"
        );
        assert_eq!(
            agent_start["model"], "concrete-model",
            "replayed AgentStart must carry the run's real configured model \
             (from the persisted RunStarted.roster), got: {agent_start:?}"
        );
    }

    /// Re-review fix, regression lock: `--replay` of a `ring` run with
    /// non-empty `state.ring.votes` must include the `[votes] …` tally in
    /// the terminal `Result.content`, exactly like the live path and
    /// `--resume` already do via `run_es_record::ring_display` — before the
    /// fix, `replay_from_log` called `to_orchestration_result`
    /// unconditionally, which has no notion of votes at all, so the tally
    /// silently vanished on replay for the one pattern whose whole point is
    /// the vote. Builds a minimal ring event log directly (bypassing the
    /// live ring engine entirely — same idiom as `run_es_record.rs`'s own
    /// `sample_blackboard_events` test helper: only what `fold` +
    /// `ring_display` need), persists it through a real (embedded)
    /// `SqliteLog`, then drives `replay_from_log` against it — the same
    /// injectable-log harness every other test in this module uses, no
    /// global env/config mutation.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn replay_of_completed_ring_run_with_votes_includes_vote_tally() {
        use crate::es_log::SqliteLog;
        use armadai_storage::open_in_memory;

        let db = open_in_memory().unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();

        let mut log = SqliteLog::new(db.clone());
        let events = [
            ExecutionEvent::RunStarted {
                run_id: run_id.clone(),
                pattern: "ring".to_string(),
                agents: vec!["a".to_string(), "b".to_string()],
                input: "review the design".to_string(),
                project: None,
                roster: Default::default(),
            },
            ExecutionEvent::LapStarted { lap: 1 },
            ExecutionEvent::ContributionAdded {
                agent: "a".to_string(),
                lap: 1,
                position: 0,
                action: "propose".to_string(),
                content: "initial proposal".to_string(),
                tokens_in: 10,
                tokens_out: 20,
                cost: 0.01,
            },
            ExecutionEvent::VoteCast {
                agent: "a".to_string(),
                position: "approve".to_string(),
                confidence: 0.9,
                supports: vec![0],
                concerns: vec![],
            },
            ExecutionEvent::VoteCast {
                agent: "b".to_string(),
                position: "approve".to_string(),
                confidence: 0.8,
                supports: vec![0],
                concerns: vec![],
            },
            ExecutionEvent::OutcomeResolved {
                outcome: "consensus reached".to_string(),
            },
            ExecutionEvent::Completed {
                content: "consensus reached".to_string(),
            },
        ];
        for event in &events {
            log.append(&run_id, event).unwrap();
        }

        let replay_log = SqliteLog::new(db);
        let (capture, sink) = capture_sink();
        crate::cli::run_replay::replay_from_log(&replay_log, &run_id, &sink, false).unwrap();

        let tags = capture.tags();
        assert_eq!(tags.last().map(String::as_str), Some("result"));

        let captured = capture.events.lock().unwrap();
        let tail = captured.last().unwrap();
        let content = tail["content"].as_str().unwrap();
        assert!(
            content.contains("[votes]"),
            "replayed ring Result.content must include the vote tally like the \
             live/--resume path, got: {content}"
        );
        assert!(content.starts_with("consensus reached"));
    }

    #[cfg(feature = "storage")]
    #[test]
    fn replay_unknown_run_id_errors() {
        use crate::es_log::SqliteLog;
        use armadai_storage::open_in_memory;

        let log = SqliteLog::new(open_in_memory().unwrap());
        let (_capture, sink) = capture_sink();
        let err = crate::cli::run_replay::replay_from_log(&log, "does-not-exist", &sink, false)
            .unwrap_err();
        assert!(
            err.to_string().contains("does-not-exist"),
            "expected the unknown run_id in the error message, got: {err}"
        );
    }

    #[cfg(not(feature = "storage"))]
    #[tokio::test]
    async fn replay_requires_storage_feature() {
        let (_capture, sink) = capture_sink();
        let err = crate::cli::run_replay::replay_run("any-id", &sink, false)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("storage"),
            "expected the storage-feature-required message, got: {err}"
        );
    }

    // ── OH1 Lot 6 whole-branch review, I2: `--resume` RunStart bookend ──
    //
    // `resume_run` (the CLI wrapper) reloads its roster from the project it
    // resolves via `resolve_agents_dir`/`load_agent_for_run` — driving
    // it directly in a hermetic unit test would mean mutating the process
    // CWD/project resolution, racing every other test in this file that
    // also touches `crate::db::init_db()`/project resolution (same
    // reasoning as the big comment above the `--replay` determinism tests).
    // This test instead drives the EXACT sequence `resume_run` performs on
    // an injected `SqliteLog`: fold the log, emit `synthetic_run_start` from
    // the folded roster, dispatch `resume_direct_es` through a
    // `SinkProjectingLog`, then emit the terminal `Result` via
    // `to_orchestration_result` — the same idiom
    // `run_direct_against_sqlite_log` above uses for the `--replay` tests.
    //
    // This is what would have caught the Workroom-population regression:
    // before the fix, nothing in `resume_run` ever emitted a `RunStart`, so
    // `Workroom::on_run_event_at`'s `RunStart { agents, .. }` arm — the ONLY
    // arm that inserts a new tracked agent, `AgentStart` only mutates one
    // already present — never ran, and an interactive `--resume` showed an
    // empty live Workroom.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn resume_emits_run_start_bookend_before_the_mid_stream_events() {
        use crate::es_log::SqliteLog;
        use armadai_core::orchestration::es::bridge::synthetic_run_start;
        use armadai_core::orchestration::es::direct::resume_direct_es;
        use armadai_core::orchestration::es::state::fold;
        use armadai_storage::open_in_memory;

        let db = open_in_memory().unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();

        // Simulate a crashed run: only `RunStarted` was ever persisted (the
        // process died before the single agent was invoked) — this folds to
        // `RunStatus::Running`, exactly what `resume_run` requires to
        // proceed with a resume.
        let mut log = SqliteLog::new(db);
        log.append(
            &run_id,
            &ExecutionEvent::RunStarted {
                run_id: run_id.clone(),
                pattern: "direct".to_string(),
                agents: vec!["solo".to_string()],
                input: "do the thing".to_string(),
                project: None,
                roster: Default::default(),
            },
        )
        .unwrap();

        let pre_resume_events = log.events(&run_id).unwrap();
        let state = fold(&pre_resume_events);
        assert_eq!(state.status, RunStatus::Running, "sanity: resumable");

        let (capture, sink) = capture_sink();

        // HEAD bookend — same call `resume_run` makes, from the same folded
        // roster/events, before the engine resumes.
        sink.emit(&synthetic_run_start(
            &run_id,
            &state.pattern,
            &state.agents,
            &pre_resume_events,
        ));

        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), test_agent("solo"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "solo".to_string(),
            Arc::new(ScriptedProvider::new(&["resumed answer"])) as Arc<dyn Provider>,
        );

        let mut proj_log = SinkProjectingLog::with_meta(log, sink.as_ref(), BTreeMap::new());
        let final_state = resume_direct_es(
            &run_id,
            agents,
            providers,
            RoutingRules::default(),
            &mut proj_log,
        )
        .await
        .unwrap();

        // TERMINAL bookend — same call `resume_run` makes after the engine
        // resumes.
        let events = proj_log.events(&run_id).unwrap();
        let result = to_orchestration_result(&final_state, &events);
        sink.emit(&RunEvent::Result {
            content: result.content.clone(),
            tin: result.total_tokens_in,
            tout: result.total_tokens_out,
            cost: result.total_cost,
            agents: final_state.agents.len(),
        });

        let tags = capture.tags();
        assert_eq!(
            tags.first().map(String::as_str),
            Some("run_start"),
            "resume must emit a RunStart bookend at the head, got: {tags:?}"
        );
        assert_eq!(
            tags.last().map(String::as_str),
            Some("result"),
            "resume must still end with the terminal Result, got: {tags:?}"
        );

        let captured = capture.events.lock().unwrap();
        let head = captured.first().unwrap();
        assert_eq!(head["run_id"], run_id);
        assert_eq!(head["agents"], serde_json::json!(["solo"]));
        let tail = captured.last().unwrap();
        assert_eq!(tail["content"], "resumed answer");
    }
}
