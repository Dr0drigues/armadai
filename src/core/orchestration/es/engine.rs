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

    let mut iterations = 0usize;
    while state.status == RunStatus::Running {
        if iterations >= MAX_ITERATIONS {
            append_and_apply(
                log,
                run_id,
                &mut state,
                ExecutionEvent::Halted {
                    reason: "iteration_cap".to_string(),
                },
            )?;
            break;
        }
        iterations += 1;

        let actions = decider.decide(&state);
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
                        &mut state,
                        ExecutionEvent::AgentInvoked {
                            agent: agent.clone(),
                            input: input.clone(),
                        },
                    )?;
                    let observed = effects.run_invoke(&agent, &input, &state).await?;
                    append_and_apply(log, run_id, &mut state, observed)?;
                }
                Action::Emit(event) => {
                    append_and_apply(log, run_id, &mut state, event)?;
                }
                Action::Halt { reason } => {
                    append_and_apply(log, run_id, &mut state, ExecutionEvent::Halted { reason })?;
                }
                Action::Complete { content } => {
                    append_and_apply(
                        log,
                        run_id,
                        &mut state,
                        ExecutionEvent::Completed { content },
                    )?;
                }
            }
        }
    }

    Ok(state)
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
}
