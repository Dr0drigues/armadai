//! `armadai run --replay <run_id>` (OH1 Lot 6, Task 2): reconstruct a
//! finished run's `RunEvent` stream purely from the persisted `execution_events`
//! log, without executing any effect (no provider calls).
//!
//! This is deliberately the read-only counterpart of the ES engines'
//! `run_direct_es`/`run_blackboard_es`/`run_ring_es`/`run_hierarchical_es`,
//! which append to the log AND drive the live provider calls. Replay only
//! ever reads: `EventLog::events(run_id)` back, then projects each
//! `ExecutionEvent` onto zero-or-more `RunEvent`s via
//! [`crate::core::orchestration::es::bridge::map_execution_to_run_events`] —
//! the SAME function `SinkProjectingLog` (the live path's bridge, see
//! `bridge.rs`) uses for every ES run. Reusing it here (rather than forking a
//! second `ExecutionEvent -> RunEvent` mapping) is what guarantees replay
//! emits the same shape of events a live run did for the same log content.
//!
//! ## `RunStart`/`Result` bookends
//!
//! A live run's `RunEvent::RunStart` (head) and `RunEvent::Result` (terminal)
//! are emitted by `run.rs`, NOT by the ES engine: `map_execution_to_run_events`
//! maps `RunStarted`/`Completed` to `[]` (see its doc comment), since building
//! either CLI-shaped event needs context the per-event projection doesn't
//! have. `replay_from_log` therefore synthesizes both itself —
//! [`crate::core::orchestration::es::bridge::synthetic_run_start`] for the
//! head, [`crate::cli::run_es_record::final_content`] (the SAME helper
//! `resume_run` in `run.rs` calls, and the fix for a re-review fidelity gap:
//! this module used to call `to_orchestration_result` unconditionally here,
//! which has no notion of a `ring` run's vote tally — a replayed `ring` run
//! would silently drop the `[votes] …` line live/`--resume` both include)
//! for the terminal `Result` — so `--replay --json` produces the SAME
//! complete `RunEvent` stream a live run did, not just its mid-stream slice.
//!
//! Split in two on purpose:
//! - [`replay_run`] is the CLI-facing entry point: opens the real
//!   `SqliteLog` via `crate::storage::init_db()` (the actual, global,
//!   config-resolved DB — same one every other storage-gated run path
//!   opens) and hands off to...
//! - [`replay_from_log`], generic over any [`EventLog`], which does the
//!   actual read-back + projection + emit. Generic so tests can drive it
//!   against a throwaway in-memory `SqliteLog`/`InMemoryLog` without ever
//!   touching `ARMADAI_CONFIG_DIR`/`XDG_DATA_HOME` or the developer's real
//!   database — see `run.rs`'s
//!   `replay_reproduces_the_live_run_event_sequence` test.
//!
//! ## Known fidelity gap: `AgentStart.prov`/`model`
//!
//! `map_execution_to_run_events` fills `AgentStart.prov`/`model` from an
//! `agent_meta` side table (roster key -> `(provider, configured model)`)
//! that the LIVE path builds from the run's `Agent` definitions *before*
//! invoking anything (see `dispatch_direct_es`/`agent_meta_from_roster` in
//! `run.rs`). `ExecutionEvent` never records an agent's provider name at
//! all, and the model configured for an agent's *first* invocation (which is
//! what `AgentStart` shows, not the resolved tier) isn't reconstructable
//! from the log alone either — only `AgentObserved.model` is logged, and
//! only after the call already happened. Replay therefore calls
//! `map_execution_to_run_events` with an **empty** `agent_meta`, which is
//! the function's documented "no roster available" fallback (see
//! `bridge.rs`'s `agent_invoked_maps_to_agent_start_with_empty_prov_model_when_meta_absent`
//! test): every replayed `AgentStart` carries `prov: ""`, `model: ""`,
//! whatever the live run's actually were. Every other field of every other
//! event (content/tokens/cost/agent names, `AgentEnd`/`Route`/`Board`/`Vote`/
//! `Delegate`/`Warning`/...) round-trips through the log exactly, since it's
//! carried verbatim by the `ExecutionEvent` that produced it.
//!
//! `human_output` gates the two pieces of direct terminal output this
//! module performs (a muted `replay <run_id>` banner and, on success, the
//! run's final answer) — mirroring the `!json && !quiet` gate `run.rs`'s
//! live paths use for their own `run <run_id>` banner. It is independent
//! from `sink`: JSONL consumers (`--json`) get the full replayed `RunEvent`
//! stream from `sink` regardless of `human_output`.

#[cfg(feature = "storage")]
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::events::EventSink;
#[cfg(feature = "storage")]
use crate::core::events::RunEvent;

/// Replay a finished run: open the real, config-resolved event log
/// (`crate::storage::init_db()` + `SqliteLog`) and re-emit the same
/// `RunEvent` sequence a live run produced for it, executing no effects.
/// Returns an error if `run_id` has no persisted events (unknown id) or if
/// the `storage` feature isn't compiled in (the event log only persists
/// under `storage`).
#[cfg(feature = "storage")]
pub async fn replay_run(
    run_id: &str,
    sink: &Arc<dyn EventSink>,
    human_output: bool,
) -> anyhow::Result<()> {
    use crate::es_log::SqliteLog;

    let db = crate::storage::init_db()?;
    let log = SqliteLog::new(db);
    replay_from_log(&log, run_id, sink, human_output)
}

/// Without the `storage` feature there is no event log to read back from —
/// `--replay` cannot be honored at all (it needs the persisted
/// `execution_events` table).
#[cfg(not(feature = "storage"))]
pub async fn replay_run(
    _run_id: &str,
    _sink: &Arc<dyn EventSink>,
    _human_output: bool,
) -> anyhow::Result<()> {
    anyhow::bail!("--replay requires the 'storage' feature (event log persistence)")
}

/// Core of `--replay`, generic over any [`EventLog`] — no I/O beyond the log
/// itself (no `init_db()`, no env/config resolution), which is what makes it
/// directly unit-testable against an in-memory log without touching global
/// env/config state (see `run.rs`'s
/// `es_switch_tests::replay_reproduces_the_live_run_event_sequence`, which
/// calls this via `crate::cli::run_replay::replay_from_log` — `pub(crate)`
/// for exactly that cross-module test access; not part of the CLI's public
/// surface, [`replay_run`] is).
///
/// Bails with `"no run found for id {run_id}"` when `log.events(run_id)` is
/// empty — [`EventLog::events`] returns an empty vec (not an error) for an
/// unknown id, so this is the only place that turns "nothing recorded"
/// into a user-facing error.
#[cfg(feature = "storage")]
pub(crate) fn replay_from_log<L: crate::core::orchestration::es::log::EventLog>(
    log: &L,
    run_id: &str,
    sink: &Arc<dyn EventSink>,
    human_output: bool,
) -> anyhow::Result<()> {
    use crate::cli::run_es_record::final_content;
    use crate::core::orchestration::es::bridge::{
        map_execution_to_run_events, synthetic_run_start, to_orchestration_result,
    };
    use crate::core::orchestration::es::state::fold;

    let events = log.events(run_id)?;

    if events.is_empty() {
        anyhow::bail!("no run found for id {run_id}");
    }

    if human_output {
        let m = crate::cli::style::muted();
        anstream::eprintln!("{m}replay {run_id}{m:#}");
    }

    // Folded once up front: the roster (`state.agents`) seeds the synthetic
    // `RunStart` bookend below, and the same `state` feeds
    // `to_orchestration_result` for the terminal `Result` bookend after the
    // mid-stream loop.
    let state = fold(&events);

    // HEAD bookend: a live run's `RunStart` is emitted by `run.rs`, not the
    // ES engine (`RunStarted` maps to `[]` in `map_execution_to_run_events`)
    // — replay must synthesize it so `--replay --json` produces the SAME
    // complete stream a live run does (see `synthetic_run_start`'s doc).
    sink.emit(&synthetic_run_start(
        run_id,
        &state.pattern,
        &state.agents,
        &events,
    ));

    // No roster is available on this read-only path (see module docs): every
    // `AgentInvoked` replays through the empty-`agent_meta` fallback, so its
    // `AgentStart` carries empty `prov`/`model` rather than the live run's.
    let agent_meta: BTreeMap<String, (String, String)> = BTreeMap::new();
    for event in &events {
        for re in map_execution_to_run_events(event, &agent_meta) {
            sink.emit(&re);
        }
    }

    // TERMINAL bookend: same reasoning as the head `RunStart` above — a live
    // run's `Result` is built by `run.rs`, never by the engine projection
    // (`Completed` also maps to `[]`), so replay must build and emit it
    // itself for the stream to end the same way a live run's does.
    //
    // `content` goes through `final_content` (the SAME pattern-branching
    // helper `resume_run` uses) rather than `to_orchestration_result`
    // directly — that fixed a fidelity gap where a replayed `ring` run
    // dropped its vote tally (see module doc). `tin`/`tout`/`cost` stay
    // pattern-agnostic (always `state.budget_*`), so `to_orchestration_result`
    // is still the right source for those.
    let result = to_orchestration_result(&state, &events);
    let content = final_content(&state, &events);
    sink.emit(&RunEvent::Result {
        content: content.clone(),
        tin: result.total_tokens_in,
        tout: result.total_tokens_out,
        cost: result.total_cost,
        agents: state.agents.len(),
    });

    // Human-mode convenience: print the run's final answer, same as a live
    // run's `println!(content)` — reuses `content` above instead of
    // recomputing it. This is a plain stdout print, independent of the
    // `RunEvent` stream above/`sink`.
    if human_output {
        println!("{content}");
    }

    Ok(())
}
