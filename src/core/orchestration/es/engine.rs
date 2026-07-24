//! Generic event-sourced run loop (OH1 Lot 1 socle).
//!
//! This module wires the pure pieces from `event`/`state`/`log` into a
//! single generic loop: `decide → run the effect → append the event(s) →
//! fold`. No concrete orchestration pattern is wired to it yet — Lots 2-4
//! provide real `Decider`/`EffectRunner` implementations per pattern; this
//! lot only proves the loop with a mock decider/effect in tests.
//!
//! `replay` reconstructs an `ExecutionState` purely from the log, executing
//! no effects at all — this is what makes the log the source of truth.

use super::event::ExecutionEvent;
use super::log::EventLog;
use super::state::{ExecutionState, RunStatus, apply, fold};

/// Maximum number of loop iterations `run_event_sourced` will perform before
/// giving up and halting the run.
///
/// This is an anti-infinite-loop guard rail, not a business limit: a
/// well-behaved `Decider` should terminate (via `Complete`/`Halt`, or by
/// returning no actions) long before this cap is reached. Chosen as a
/// generous constant so it never fires on legitimate runs while still
/// bounding worst-case work if a `Decider` is buggy (e.g. never observes the
/// state change that would make it stop).
const MAX_ITERATIONS: usize = 500;

/// A single unit of work the loop should perform, as decided by a
/// `Decider` from the current `ExecutionState`.
#[derive(Debug, Clone)]
pub enum Action {
    /// Invoke `agent` with `input`. The loop records an `AgentInvoked` event
    /// before delegating to `EffectRunner::run_invoke`, then records the
    /// event it returns (typically `AgentObserved`).
    Invoke { agent: String, input: String },
    /// Record `event` verbatim, with no associated effect.
    Emit(ExecutionEvent),
    /// Halt the run with `reason` (terminal — recorded as `Halted`).
    Halt { reason: String },
    /// Complete the run with final `content` (terminal — recorded as
    /// `Completed`).
    Complete { content: String },
}

/// Pure, deterministic decision function: given the current projected
/// state, decide what to do next.
///
/// Implementations must not perform I/O, block, or otherwise depend on
/// anything outside `state` — this is what keeps replay deterministic. Only
/// `EffectRunner` (and the loop driving it) is allowed to be async/impure.
pub trait Decider {
    /// Decide the next batch of actions given the current state. Returning
    /// an empty vec tells the loop there is nothing left to do (equivalent
    /// to halting the run, without recording a terminal event).
    fn decide(&self, state: &ExecutionState) -> Vec<Action>;
}

/// Executes the actual side-effecting work behind an `Action::Invoke` (e.g.
/// calling an LLM provider). Concrete orchestration patterns supply the
/// real implementation in later lots; this lot only proves the loop against
/// a mock in tests.
#[async_trait::async_trait]
pub trait EffectRunner {
    /// Run the effect for invoking `agent` with `input` against the current
    /// `state`, returning the event to record (typically `AgentObserved`).
    async fn run_invoke(
        &self,
        agent: &str,
        input: &str,
        state: &ExecutionState,
    ) -> anyhow::Result<ExecutionEvent>;
}

/// Append `event` to `log` for `run_id` and fold it into `state` in one
/// step, keeping the log and the projection in lockstep.
fn append_and_apply<L: EventLog>(
    log: &mut L,
    run_id: &str,
    state: &mut ExecutionState,
    event: ExecutionEvent,
) -> anyhow::Result<()> {
    log.append(run_id, &event)?;
    apply(state, &event);
    Ok(())
}

/// Drive the generic event-sourced loop for `run_id`.
///
/// `initial` (typically containing at least `RunStarted`) is appended and
/// folded first. Then, while `state.status == RunStatus::Running`, this
/// repeatedly calls `decider.decide(&state)` and executes each returned
/// `Action` in order:
/// - `Invoke { agent, input }`: appends `AgentInvoked { agent, input }`,
///   then calls `effects.run_invoke(...)` and appends the event it returns.
/// - `Emit(event)`: appends `event` as-is.
/// - `Halt { reason }` / `Complete { content }`: appends the corresponding
///   terminal event (`Halted`/`Completed`).
///
/// Every event is appended to `log` and folded into the running state via
/// `apply` before the next action is considered, so a later action in the
/// same batch (or the next `decide` call) always sees an up-to-date state.
///
/// The loop stops when `state.status != RunStatus::Running` (a terminal
/// event was recorded) or when `decide` returns no actions. As an
/// anti-infinite-loop guard, if the loop performs `MAX_ITERATIONS` decide
/// rounds without reaching a terminal status, it force-halts the run by
/// appending `Halted { reason: "iteration_cap" }` and returns — this is
/// treated as a (halted) result rather than an `Err`, since a capped run is
/// still a well-formed, replayable outcome rather than a failure to
/// produce one.
pub async fn run_event_sourced<D, R, L>(
    run_id: &str,
    initial: Vec<ExecutionEvent>,
    decider: &D,
    effects: &R,
    log: &mut L,
) -> anyhow::Result<ExecutionState>
where
    D: Decider,
    R: EffectRunner,
    L: EventLog,
{
    let mut state = ExecutionState::default();

    for event in initial {
        append_and_apply(log, run_id, &mut state, event)?;
    }

    run_loop(run_id, &mut state, decider, effects, log).await?;

    Ok(state)
}

/// Drive the generic event-sourced loop body for `run_id`, given an
/// already-seeded `state` — the part of [`run_event_sourced`] that runs
/// `while state.status == RunStatus::Running`, factored out so
/// [`resume_event_sourced`] can seed `state` via [`replay`] (no re-append of
/// `initial`) instead of `ExecutionState::default()`, while sharing the
/// exact same decide/act/append/fold sequence — see [`run_event_sourced`]'s
/// own doc comment for the full behavior this implements (iteration cap,
/// per-`Action` handling, etc.), which is unchanged by this extraction.
async fn run_loop<D, R, L>(
    run_id: &str,
    state: &mut ExecutionState,
    decider: &D,
    effects: &R,
    log: &mut L,
) -> anyhow::Result<()>
where
    D: Decider,
    R: EffectRunner,
    L: EventLog,
{
    let mut iterations = 0usize;
    while state.status == RunStatus::Running {
        if iterations >= MAX_ITERATIONS {
            append_and_apply(
                log,
                run_id,
                state,
                ExecutionEvent::Halted {
                    reason: "iteration_cap".to_string(),
                },
            )?;
            break;
        }
        iterations += 1;

        let actions = decider.decide(state);
        if actions.is_empty() {
            break;
        }

        for action in actions {
            if state.status != RunStatus::Running {
                break;
            }
            match action {
                Action::Invoke { agent, input } => {
                    append_and_apply(
                        log,
                        run_id,
                        state,
                        ExecutionEvent::AgentInvoked {
                            agent: agent.clone(),
                            input: input.clone(),
                        },
                    )?;
                    let observed = effects.run_invoke(&agent, &input, state).await?;
                    append_and_apply(log, run_id, state, observed)?;
                }
                Action::Emit(event) => {
                    append_and_apply(log, run_id, state, event)?;
                }
                Action::Halt { reason } => {
                    append_and_apply(log, run_id, state, ExecutionEvent::Halted { reason })?;
                }
                Action::Complete { content } => {
                    append_and_apply(log, run_id, state, ExecutionEvent::Completed { content })?;
                }
            }
        }
    }

    Ok(())
}

/// Resume a previously interrupted event-sourced run: seed `state` by
/// [`replay`]ing `run_id` from `log` (a pure fold — NO re-append of the
/// events already recorded), then drive the same [`run_loop`] a fresh run
/// would use, appending only the NEW events resuming produces.
///
/// Bails if `run_id` has no recorded events at all (unknown run — `replay`
/// alone can't distinguish "unknown id" from "a real, empty-by-construction
/// state", since both fold to `ExecutionState::default()`), or if the
/// replayed run's status isn't [`RunStatus::Running`] (already
/// `Completed`/`Halted` — nothing to resume).
///
/// `decider`/`effects` must be reconstructed by the caller (see the
/// `resume_*_es` entry points in `es::direct`/`blackboard`/`ring`/
/// `hierarchical`) from data recoverable at resume time: the roster/input
/// carried by the log's `RunStarted` event, the pattern config carried by
/// `ConfigSnapshot`, and the agents/providers reloaded from the project on
/// disk — never from anything the caller must have kept around since the
/// original run started.
pub async fn resume_event_sourced<D, R, L>(
    run_id: &str,
    decider: &D,
    effects: &R,
    log: &mut L,
) -> anyhow::Result<ExecutionState>
where
    D: Decider,
    R: EffectRunner,
    L: EventLog,
{
    if log.events(run_id)?.is_empty() {
        anyhow::bail!("no run found for id {run_id}");
    }

    let mut state = replay(run_id, log)?;
    if state.status != RunStatus::Running {
        anyhow::bail!("run {run_id} is not resumable (status: {:?})", state.status);
    }

    run_loop(run_id, &mut state, decider, effects, log).await?;

    Ok(state)
}

/// Recover the roster (`RunStarted.agents`, in original order) and original
/// user `input` from the first `RunStarted` event in `events`. Returns
/// `None` if no `RunStarted` is present (an unknown/empty log) — the only
/// piece of `RunStarted` not preserved by the pure `ExecutionState`
/// projection (`apply` deliberately discards `input`, see `state.rs`), so
/// every `resume_*_es` entry point needs this raw-event scan (mirroring
/// `run_es_record::project_run`'s own `RunStarted` extraction) to rebuild a
/// `Decider` that needs the run's original input text.
pub fn run_started_roster_and_input(events: &[ExecutionEvent]) -> Option<(Vec<String>, String)> {
    events.iter().find_map(|e| match e {
        ExecutionEvent::RunStarted { agents, input, .. } => Some((agents.clone(), input.clone())),
        _ => None,
    })
}

/// Recover a pattern's orchestration config (`BlackboardConfig`/`RingConfig`/
/// `OrchestrationConfig`) from the log's `ConfigSnapshot` event, deserializing
/// its `config_json`. Falls back to `C::default()` when no `ConfigSnapshot`
/// is present (direct runs never emit one) or it fails to deserialize —
/// same fallback `run_es_record::project_run` already relies on via
/// `ExecutionState::config_json`.
pub fn config_snapshot<C: serde::de::DeserializeOwned + Default>(events: &[ExecutionEvent]) -> C {
    events
        .iter()
        .find_map(|e| match e {
            ExecutionEvent::ConfigSnapshot { config_json } => {
                serde_json::from_str(config_json).ok()
            }
            _ => None,
        })
        .unwrap_or_default()
}

/// Reconstruct an `ExecutionState` for `run_id` purely from `log`, executing
/// no effects. This is the read path counterpart to `run_event_sourced`:
/// given the same log, it always produces the same state, since `fold`
/// (via `apply`) is pure and deterministic.
pub fn replay<L: EventLog>(run_id: &str, log: &L) -> anyhow::Result<ExecutionState> {
    Ok(fold(&log.events(run_id)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::orchestration::es::event::ExecutionEvent as E;
    use crate::core::orchestration::es::log::InMemoryLog;
    use async_trait::async_trait;

    // Decider: invoke "a" once (when no observation yet), then complete.
    struct D;
    impl Decider for D {
        fn decide(&self, s: &ExecutionState) -> Vec<Action> {
            let observed = s
                .conversations
                .get("a")
                .map(|c| !c.is_empty())
                .unwrap_or(false);
            if !observed {
                vec![Action::Invoke {
                    agent: "a".into(),
                    input: "go".into(),
                }]
            } else {
                vec![Action::Complete {
                    content: "final".into(),
                }]
            }
        }
    }
    struct Eff;
    #[async_trait]
    impl EffectRunner for Eff {
        async fn run_invoke(
            &self,
            agent: &str,
            _input: &str,
            _s: &ExecutionState,
        ) -> anyhow::Result<E> {
            Ok(E::AgentObserved {
                agent: agent.into(),
                content: "resp".into(),
                tokens_in: 3,
                tokens_out: 4,
                cost: 0.0,
                model: "m".into(),
            })
        }
    }

    #[tokio::test]
    async fn loop_runs_appends_folds_and_terminates() {
        let mut log = InMemoryLog::default();
        let init = vec![E::RunStarted {
            run_id: "r".into(),
            pattern: "direct".into(),
            agents: vec!["a".into()],
            input: "go".into(),
            project: None,
        }];
        let st = run_event_sourced("r", init, &D, &Eff, &mut log)
            .await
            .unwrap();
        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(st.budget_tokens_in, 3);
        // replay from the log reconstructs the same state, no effects re-run
        let replayed = replay("r", &log).unwrap();
        assert_eq!(format!("{st:?}"), format!("{replayed:?}"));
        // log contains RunStarted, AgentInvoked, AgentObserved, Completed
        assert!(log.events("r").unwrap().len() >= 4);
    }

    // ── `resume_event_sourced` (OH1 Lot 6, Task 3) ──────────────────────

    /// Two-step decider: invoke "a", then "b", then complete — needed
    /// (unlike the single-step `D` above) to prove a resume genuinely
    /// *continues* mid-sequence rather than trivially completing on its
    /// first `decide` call.
    struct TwoStep;
    impl Decider for TwoStep {
        fn decide(&self, s: &ExecutionState) -> Vec<Action> {
            let observed = |agent: &str| {
                s.conversations
                    .get(agent)
                    .map(|c| c.iter().any(|m| m.role == "assistant"))
                    .unwrap_or(false)
            };
            if !observed("a") {
                vec![Action::Invoke {
                    agent: "a".into(),
                    input: "go".into(),
                }]
            } else if !observed("b") {
                vec![Action::Invoke {
                    agent: "b".into(),
                    input: "go".into(),
                }]
            } else {
                vec![Action::Complete {
                    content: "final".into(),
                }]
            }
        }
    }

    /// Effect runner that records every agent it's asked to invoke, so tests
    /// can assert an already-observed agent (replayed from the log, not
    /// re-invoked) never appears in the recorded list.
    #[derive(Default)]
    struct TrackingEff {
        invoked: std::sync::Mutex<Vec<String>>,
    }
    #[async_trait]
    impl EffectRunner for TrackingEff {
        async fn run_invoke(
            &self,
            agent: &str,
            _input: &str,
            _s: &ExecutionState,
        ) -> anyhow::Result<E> {
            self.invoked.lock().unwrap().push(agent.to_string());
            Ok(E::AgentObserved {
                agent: agent.into(),
                content: "resp".into(),
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
                model: "m".into(),
            })
        }
    }

    #[tokio::test]
    async fn resume_event_sourced_continues_without_reinvoking_observed_agent() {
        let mut log = InMemoryLog::default();
        // Simulate a crash: the process recorded `RunStarted` + invoked/
        // observed "a", then died before deciding on "b" — status is still
        // `Running` (no terminal event recorded).
        log.append(
            "r",
            &E::RunStarted {
                run_id: "r".into(),
                pattern: "direct".into(),
                agents: vec!["a".into(), "b".into()],
                input: "go".into(),
                project: None,
            },
        )
        .unwrap();
        log.append(
            "r",
            &E::AgentInvoked {
                agent: "a".into(),
                input: "go".into(),
            },
        )
        .unwrap();
        log.append(
            "r",
            &E::AgentObserved {
                agent: "a".into(),
                content: "resp-a".into(),
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
                model: "m".into(),
            },
        )
        .unwrap();

        let eff = TrackingEff::default();
        let state = resume_event_sourced("r", &TwoStep, &eff, &mut log)
            .await
            .unwrap();

        assert_eq!(state.status, RunStatus::Completed);
        // Only "b" was invoked by the resumed run — "a" (already observed
        // before the crash) was never re-invoked.
        assert_eq!(*eff.invoked.lock().unwrap(), vec!["b".to_string()]);
        // The log now also contains the pre-crash events, unchanged.
        let events = log.events("r").unwrap();
        assert!(matches!(events[0], E::RunStarted { .. }));
        assert!(
            events
                .iter()
                .filter(|e| matches!(e, E::AgentInvoked { agent, .. } if agent == "a"))
                .count()
                == 1,
            "\"a\" must appear invoked exactly once across the whole log"
        );
    }

    #[tokio::test]
    async fn resume_event_sourced_bails_on_completed_run() {
        let mut log = InMemoryLog::default();
        log.append(
            "r",
            &E::RunStarted {
                run_id: "r".into(),
                pattern: "direct".into(),
                agents: vec!["a".into()],
                input: "go".into(),
                project: None,
            },
        )
        .unwrap();
        log.append(
            "r",
            &E::Completed {
                content: "done".into(),
            },
        )
        .unwrap();

        let eff = TrackingEff::default();
        let err = resume_event_sourced("r", &D, &eff, &mut log)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not resumable"));
        // No effect should have been run against an already-terminal log.
        assert!(eff.invoked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn resume_event_sourced_bails_on_halted_run() {
        let mut log = InMemoryLog::default();
        log.append(
            "r",
            &E::RunStarted {
                run_id: "r".into(),
                pattern: "direct".into(),
                agents: vec!["a".into()],
                input: "go".into(),
                project: None,
            },
        )
        .unwrap();
        log.append(
            "r",
            &E::Halted {
                reason: "budget".into(),
            },
        )
        .unwrap();

        let eff = TrackingEff::default();
        let err = resume_event_sourced("r", &D, &eff, &mut log)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not resumable"));
    }

    #[tokio::test]
    async fn resume_event_sourced_bails_on_unknown_run() {
        let mut log = InMemoryLog::default();
        let eff = TrackingEff::default();
        let err = resume_event_sourced("nope", &D, &eff, &mut log)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no run found"));
    }
}
