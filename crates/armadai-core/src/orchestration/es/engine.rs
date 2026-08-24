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
use futures_util::StreamExt;

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
    /// Invoke several agents concurrently. The loop records one
    /// `AgentInvoked` per `batch` entry in `Vec` order, runs the effects
    /// concurrently (at most `max_concurrency` in flight), then records each
    /// outcome back in `Vec` order — independent of completion order, so
    /// replay/resume stay deterministic. A per-entry failure is recorded as
    /// `AgentFailed` and the run continues (collect-and-record).
    InvokeParallel {
        batch: Vec<InvokeSpec>,
        max_concurrency: usize,
    },
}

/// One unit of work inside an [`Action::InvokeParallel`] batch. Named
/// distinctly from the `Action::Invoke` variant to avoid confusion.
#[derive(Debug, Clone)]
pub struct InvokeSpec {
    pub agent: String,
    pub input: String,
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
    ///
    /// `batch_len` is the number of sibling invocations dispatched together
    /// with this one: `1` for a solitary `Action::Invoke`, or the batch size
    /// for every entry of an `Action::InvokeParallel`. It exists so an
    /// implementation that derives a per-invocation resource ceiling from
    /// `state` (e.g. `HierarchicalEffectRunner::run_nested`'s remaining
    /// token budget) can partition that ceiling across concurrent siblings
    /// without any shared mutable state — every entry in a parallel batch
    /// sees the exact same `state` snapshot (see `Action::InvokeParallel`
    /// below), so `batch_len` is the only signal available to tell them
    /// apart from a solo invocation. Most implementations have no use for
    /// it and simply ignore it.
    async fn run_invoke(
        &self,
        agent: &str,
        input: &str,
        state: &ExecutionState,
        batch_len: usize,
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
                    let observed = effects.run_invoke(&agent, &input, state, 1).await?;
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
                Action::InvokeParallel {
                    batch,
                    max_concurrency,
                } => {
                    // 1. Record every invocation up-front, in Vec order
                    //    (deterministic). Emitting all AgentInvoked before any
                    //    outcome also makes every agent read as "working" at
                    //    once in the Workroom.
                    for spec in &batch {
                        append_and_apply(
                            log,
                            run_id,
                            state,
                            ExecutionEvent::AgentInvoked {
                                agent: spec.agent.clone(),
                                input: spec.input.clone(),
                            },
                        )?;
                    }

                    // 2. Run effects concurrently over a shared, immutable
                    //    snapshot of the now-updated state — every entry sees
                    //    the exact same `state`, regardless of concurrency or
                    //    completion order (`batch_len` below is how an
                    //    `EffectRunner` can still tell a parallel sibling
                    //    apart from a solo `Action::Invoke` — see its doc
                    //    comment; issue #291). `buffer_unordered`
                    //    polls the borrowing futures in place (no spawn, no
                    //    'static bound). Nothing is appended during this phase,
                    //    so only shared borrows of `state`/`effects` are live.
                    let snapshot: &ExecutionState = state;
                    let cap = max_concurrency.max(1);
                    let batch_len = batch.len();
                    // NOTE: iterate over owned clones (`InvokeSpec` is two
                    // `String`s — cheap) rather than `batch.iter()`. Borrowing
                    // the batch here makes rustc infer the closure's argument
                    // as a higher-ranked `for<'r> &'r InvokeSpec` (it must
                    // unify with `Map`'s generic `Stream::Item` bound used by
                    // `buffer_unordered`), which then fails to unify with the
                    // concrete lifetime borrowed by the returned async block's
                    // captured `spec` — "implementation of `FnOnce` is not
                    // general enough". Owning the item removes the borrowed
                    // lifetime from the closure signature entirely. Index
                    // tagging + the later `sort_by_key` still restore Vec
                    // order, so this changes nothing about ordering/semantics.
                    let mut outcomes: Vec<(usize, anyhow::Result<ExecutionEvent>)> =
                        futures_util::stream::iter(batch.iter().cloned().enumerate())
                            .map(|(i, spec)| async move {
                                (
                                    i,
                                    effects
                                        .run_invoke(&spec.agent, &spec.input, snapshot, batch_len)
                                        .await,
                                )
                            })
                            .buffer_unordered(cap)
                            .collect()
                            .await;

                    // 3. Restore Vec order (buffer_unordered yields in
                    //    completion order), then append outcomes in Vec order.
                    //    A failure becomes AgentFailed; the run continues.
                    outcomes.sort_by_key(|(i, _)| *i);
                    for (i, res) in outcomes {
                        let event = match res {
                            Ok(ev) => ev,
                            Err(e) => ExecutionEvent::AgentFailed {
                                agent: batch[i].agent.clone(),
                                error: e.to_string(),
                            },
                        };
                        append_and_apply(log, run_id, state, event)?;
                    }
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
    use crate::orchestration::es::event::ExecutionEvent as E;
    use crate::orchestration::es::log::InMemoryLog;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct ParDecider {
        batch: Vec<InvokeSpec>,
        cap: usize,
    }
    impl Decider for ParDecider {
        fn decide(&self, s: &ExecutionState) -> Vec<Action> {
            // Once any agent has an assistant turn, the batch has run: complete.
            let ran = s
                .conversations
                .values()
                .any(|c| c.iter().any(|m| m.role == "assistant"));
            if ran {
                vec![Action::Complete {
                    content: "done".into(),
                }]
            } else {
                vec![Action::InvokeParallel {
                    batch: self.batch.clone(),
                    max_concurrency: self.cap,
                }]
            }
        }
    }

    // Runner: sleeps longer for lower-index agents so completions arrive in
    // reverse Vec order; records the completion order it actually observed.
    struct OrderedEff {
        order: Vec<String>, // Vec order a,b,c → delays 30,20,10ms
        completions: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl EffectRunner for OrderedEff {
        async fn run_invoke(
            &self,
            agent: &str,
            _input: &str,
            _s: &ExecutionState,
            _batch_len: usize,
        ) -> anyhow::Result<E> {
            let idx = self.order.iter().position(|a| a == agent).unwrap_or(0);
            let delay_ms = 30u64.saturating_sub(idx as u64 * 10);
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            self.completions.lock().unwrap().push(agent.to_string());
            Ok(E::AgentObserved {
                agent: agent.into(),
                content: format!("resp-{agent}"),
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
                model: "m".into(),
            })
        }
    }

    #[tokio::test]
    async fn invoke_parallel_records_in_vec_order_not_completion_order() {
        let batch = vec![
            InvokeSpec {
                agent: "a".into(),
                input: "x".into(),
            },
            InvokeSpec {
                agent: "b".into(),
                input: "x".into(),
            },
            InvokeSpec {
                agent: "c".into(),
                input: "x".into(),
            },
        ];
        let completions = Arc::new(Mutex::new(Vec::new()));
        let decider = ParDecider {
            batch: batch.clone(),
            cap: 4,
        };
        let eff = OrderedEff {
            order: vec!["a".into(), "b".into(), "c".into()],
            completions: completions.clone(),
        };
        let mut log = InMemoryLog::default();
        let init = vec![E::RunStarted {
            run_id: "r".into(),
            pattern: "test".into(),
            agents: vec!["a".into(), "b".into(), "c".into()],
            input: "go".into(),
            project: None,
            roster: Default::default(),
        }];
        run_event_sourced("r", init, &decider, &eff, &mut log)
            .await
            .unwrap();

        let events = log.events("r").unwrap();
        // Recorded observation order == Vec order a,b,c.
        let observed: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                E::AgentObserved { agent, .. } => Some(agent.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(observed, vec!["a", "b", "c"]);
        // Recorded invocation order == Vec order a,b,c too.
        let invoked: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                E::AgentInvoked { agent, .. } => Some(agent.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(invoked, vec!["a", "b", "c"]);
        // Sanity: completions actually arrived in a different (reverse) order,
        // proving the ordering above is not incidental.
        let comp = completions.lock().unwrap().clone();
        assert_eq!(comp, vec!["c", "b", "a"]);
    }

    struct CapEff {
        live: std::sync::atomic::AtomicUsize,
        max_seen: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl EffectRunner for CapEff {
        async fn run_invoke(
            &self,
            agent: &str,
            _input: &str,
            _s: &ExecutionState,
            _batch_len: usize,
        ) -> anyhow::Result<E> {
            let now = self.live.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            self.max_seen
                .fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            self.live.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(E::AgentObserved {
                agent: agent.into(),
                content: "r".into(),
                tokens_in: 0,
                tokens_out: 0,
                cost: 0.0,
                model: "m".into(),
            })
        }
    }

    #[tokio::test]
    async fn invoke_parallel_respects_concurrency_cap() {
        let batch: Vec<InvokeSpec> = (0..6)
            .map(|i| InvokeSpec {
                agent: format!("a{i}"),
                input: "x".into(),
            })
            .collect();
        let decider = ParDecider {
            batch: batch.clone(),
            cap: 2,
        };
        let eff = CapEff {
            live: std::sync::atomic::AtomicUsize::new(0),
            max_seen: std::sync::atomic::AtomicUsize::new(0),
        };
        let mut log = InMemoryLog::default();
        let init = vec![E::RunStarted {
            run_id: "r".into(),
            pattern: "test".into(),
            agents: batch.iter().map(|s| s.agent.clone()).collect(),
            input: "go".into(),
            project: None,
            roster: Default::default(),
        }];
        run_event_sourced("r", init, &decider, &eff, &mut log)
            .await
            .unwrap();
        assert!(
            eff.max_seen.load(std::sync::atomic::Ordering::SeqCst) <= 2,
            "observed {} concurrent invocations, cap was 2",
            eff.max_seen.load(std::sync::atomic::Ordering::SeqCst)
        );
    }

    struct FailBEff;
    #[async_trait]
    impl EffectRunner for FailBEff {
        async fn run_invoke(
            &self,
            agent: &str,
            _input: &str,
            _s: &ExecutionState,
            _batch_len: usize,
        ) -> anyhow::Result<E> {
            if agent == "b" {
                anyhow::bail!("boom");
            }
            Ok(E::AgentObserved {
                agent: agent.into(),
                content: format!("resp-{agent}"),
                tokens_in: 0,
                tokens_out: 0,
                cost: 0.0,
                model: "m".into(),
            })
        }
    }

    #[tokio::test]
    async fn invoke_parallel_records_failure_and_continues() {
        let batch = vec![
            InvokeSpec {
                agent: "a".into(),
                input: "x".into(),
            },
            InvokeSpec {
                agent: "b".into(),
                input: "x".into(),
            },
            InvokeSpec {
                agent: "c".into(),
                input: "x".into(),
            },
        ];
        let decider = ParDecider {
            batch: batch.clone(),
            cap: 4,
        };
        let mut log = InMemoryLog::default();
        let init = vec![E::RunStarted {
            run_id: "r".into(),
            pattern: "test".into(),
            agents: vec!["a".into(), "b".into(), "c".into()],
            input: "go".into(),
            project: None,
            roster: Default::default(),
        }];
        let state = run_event_sourced("r", init, &decider, &FailBEff, &mut log)
            .await
            .unwrap();

        // Run still completed (failure did not abort the loop).
        assert_eq!(state.status, RunStatus::Completed);

        // Outcomes recorded in Vec order: observed a, failed b, observed c.
        let events = log.events("r").unwrap();
        let outcome_kinds: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                E::AgentObserved { agent, .. } => Some(agent.as_str()),
                E::AgentFailed { agent, .. } => Some(agent.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(outcome_kinds, vec!["a", "b", "c"]);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, E::AgentFailed { agent, .. } if agent == "b"))
        );

        // b reads as settled: last turn is the assistant failure marker.
        let convo_b = state.conversations.get("b").unwrap();
        assert_eq!(convo_b.last().unwrap().role, "assistant");
        assert_eq!(convo_b.last().unwrap().content, "[Delegation failed: boom]");
    }

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
            _batch_len: usize,
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
            roster: Default::default(),
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
            _batch_len: usize,
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
                roster: Default::default(),
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
                roster: Default::default(),
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
                roster: Default::default(),
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
