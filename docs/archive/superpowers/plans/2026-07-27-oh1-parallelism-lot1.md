# OH1 Parallelism — Lot 1 (socle) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a socle-level `Action::InvokeParallel` to the event-sourced orchestration engine that runs independent delegations concurrently while recording events in a deterministic order, with bounded concurrency and collect-and-record failure resilience.

**Architecture:** The generic `run_loop` (`src/core/orchestration/es/engine.rs`) gains one new action variant carrying its own concurrency cap. Its handler records all `AgentInvoked` in `Vec` order, runs the effects concurrently via `futures_util`'s `buffer_unordered`, then records each outcome back in `Vec` order (independent of completion order) — a per-item failure becomes a new `ExecutionEvent::AgentFailed` event instead of aborting the run. No orchestration pattern emits the new action in this lot (that is Lot 2); tests drive it against a mock `EffectRunner`.

**Tech Stack:** Rust edition 2024, `tokio`, `async-trait`, `futures-util` (promoted to a direct dependency for `buffer_unordered`), event-sourcing engine under `src/core/orchestration/es/`.

## Global Constraints

- Target branch: `master` (master-only model). Work happens on `feat/oh1-parallel` (already contains the design spec).
- Reference spec: `docs/superpowers/specs/2026-07-27-oh1-parallelism-design.md`.
- The ES engine compiles in **all** CI feature modes → any new dependency (`futures-util`) must be **non-optional** (a plain `[dependencies]` entry, not feature-gated).
- The generic engine signatures `run_event_sourced` / `resume_event_sourced` / `run_loop` MUST stay unchanged — the concurrency cap is carried inside the `InvokeParallel` action, not passed as a parameter.
- Determinism is the cardinal invariant: recorded event order = the decider's `Vec` order, **never** completion order. Replay/resume fold purely over recorded order.
- `apply` is a pure, total, deterministic reducer — no I/O, no clock, no randomness.
- Collect-and-record: a failed `run_invoke` records `AgentFailed` and the run continues; it never propagates an `Err` that aborts the loop.
- Gate per task: `cargo fmt --all` + clippy 3 modes (`--no-default-features --features tui` / `tui,providers-api` / `tui,web,storage`) `-D warnings` + `cargo test --no-default-features --features tui` + `cargo test --no-default-features --features tui,storage`.
- `rust-analyzer` diagnostics are unreliable here (stale ABI / inactive-cfg / E0308 false positives) — **always verify at the compiler** with `cargo`.
- `cat` is aliased to `bat`; use `command cat` if you must cat.
- Conventional Commits, single type per commit, trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- One PR for the whole lot + independent review + Dimitri validation.

---

## File Structure

- `src/core/orchestration/es/event.rs` — add `ExecutionEvent::AgentFailed { agent, error }` variant + a shared `delegation_failed_content(error)` helper (single source of the failure marker string, consumed by both `apply` and the bridge — DRY).
- `src/core/orchestration/es/state.rs` — add the `apply(AgentFailed)` reducer arm (push an `assistant` marker message so the agent reads as "settled"; do **not** touch the budget).
- `src/core/orchestration/es/bridge.rs` — map `AgentFailed` → `RunEvent::AgentEnd` (closes the Workroom tile).
- `src/core/orchestration/es/engine.rs` — add `Action::InvokeParallel { batch, max_concurrency }` + `struct InvokeSpec`, the `run_loop` handler, and mock-driven tests.
- `src/core/orchestration/mod.rs` — add `OrchestrationConfig::max_concurrency: Option<u32>` field + `max_concurrency()` accessor (default 4). Consumed by the hierarchical decider in Lot 2; here it is the config surface only.
- `Cargo.toml` — promote `futures-util` to a direct, non-optional dependency.

---

## Task 1: `AgentFailed` event plumbing (event + reducer + bridge)

**Files:**
- Modify: `src/core/orchestration/es/event.rs` (add variant + helper)
- Modify: `src/core/orchestration/es/state.rs:155-174` (add `apply` arm after the `AgentObserved` arm)
- Modify: `src/core/orchestration/es/bridge.rs:98-111` (add mapping arm) and `src/core/orchestration/es/bridge.rs:190-200` (remove `AgentFailed` from the catch-all if the compiler flags non-exhaustiveness — it will be handled explicitly)

**Interfaces:**
- Produces:
  - `ExecutionEvent::AgentFailed { agent: String, error: String }` — a recorded delegation failure.
  - `pub fn delegation_failed_content(error: &str) -> String` in `event.rs` — returns `format!("[Delegation failed: {error}]")`. The single source of the marker string.
- Consumes: the existing `ChatMessage { role: String, content: String }` (from `src/providers/traits.rs`), `RunEvent::AgentEnd { agent, tin, tout, cost, content }` (from `src/core/events.rs`).

- [ ] **Step 1: Write the failing reducer test**

Add to the existing `#[cfg(test)] mod tests` in `src/core/orchestration/es/state.rs` (it already uses `ExecutionEvent as E` and builds states via `apply`; match that style):

```rust
#[test]
fn agent_failed_pushes_assistant_marker_and_leaves_budget_untouched() {
    let mut st = ExecutionState::default();
    apply(&mut st, &E::AgentInvoked {
        agent: "b".into(),
        input: "do it".into(),
    });
    apply(&mut st, &E::AgentFailed {
        agent: "b".into(),
        error: "boom".into(),
    });

    let convo = st.conversations.get("b").expect("conversation exists");
    assert_eq!(convo.len(), 2);
    assert_eq!(convo[0].role, "user");
    assert_eq!(convo[1].role, "assistant");
    assert_eq!(convo[1].content, "[Delegation failed: boom]");
    // AgentFailed must not move budget counters (unlike AgentObserved).
    assert_eq!(st.budget_tokens_in, 0);
    assert_eq!(st.budget_tokens_out, 0);
    assert_eq!(st.budget_cost, 0.0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui agent_failed_pushes_assistant_marker -- --nocapture`
Expected: FAIL to compile — `no variant named AgentFailed` on `ExecutionEvent`.

- [ ] **Step 3: Add the event variant + marker helper**

In `src/core/orchestration/es/event.rs`, add the variant inside `enum ExecutionEvent` (place it right after the `Completed { content: String }` common variant, before the `// ── Hierarchical ──` section):

```rust
    /// A delegated invocation failed. Recorded instead of aborting the run
    /// (collect-and-record): the reducer pushes an `assistant` marker so the
    /// agent reads as "settled" (a coordinator that awaits this child then
    /// synthesizes over the partial results), and the run continues.
    AgentFailed { agent: String, error: String },
```

Add the shared marker helper at module level in `event.rs` (after the enum):

```rust
/// The `assistant`-role content recorded for a failed delegation. Single
/// source of the marker string, consumed by both the reducer (`apply`) and
/// the `RunEvent` bridge so they never drift. Contains no `@agent:` marker,
/// so the hierarchical `is_final_answer` reads it as a plain final answer.
pub fn delegation_failed_content(error: &str) -> String {
    format!("[Delegation failed: {error}]")
}
```

- [ ] **Step 4: Add the `apply` reducer arm**

In `src/core/orchestration/es/state.rs`, add this arm immediately after the `ExecutionEvent::AgentObserved { .. }` arm (ends at line ~174). Import the helper via the existing `use` of the event module (the arm references `super`-level `delegation_failed_content`; use the crate path):

```rust
        ExecutionEvent::AgentFailed { agent, error } => {
            // Push an `assistant` marker so `latest_response(agent)` is
            // `Some` and the child reads as settled — otherwise a hierarchical
            // coordinator would await a child that never responds
            // (`awaiting_in_flight`). Budget counters are deliberately left
            // untouched (no successful call happened).
            state
                .conversations
                .entry(agent.clone())
                .or_default()
                .push(ChatMessage {
                    role: "assistant".to_string(),
                    content: crate::core::orchestration::es::event::delegation_failed_content(
                        error,
                    ),
                });
        }
```

(If `ChatMessage` is not already imported in `state.rs`, it is — the `AgentObserved` arm above already builds `ChatMessage`. Reuse the same import.)

- [ ] **Step 5: Run the reducer test to verify it passes**

Run: `cargo test --no-default-features --features tui agent_failed_pushes_assistant_marker -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Write the failing bridge test**

Add to the `#[cfg(test)] mod tests` in `src/core/orchestration/es/bridge.rs` (it already exercises `map_execution_to_run_events`; match its style — it builds an empty `BTreeMap` for `agent_meta`):

```rust
#[test]
fn agent_failed_maps_to_agent_end_with_marker() {
    let meta: std::collections::BTreeMap<String, (String, String)> = Default::default();
    let evs = map_execution_to_run_events(
        &ExecutionEvent::AgentFailed {
            agent: "b".into(),
            error: "boom".into(),
        },
        &meta,
    );
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        RunEvent::AgentEnd { agent, tin, tout, cost, content } => {
            assert_eq!(agent, "b");
            assert_eq!(*tin, 0);
            assert_eq!(*tout, 0);
            assert_eq!(*cost, 0.0);
            assert_eq!(content, "[Delegation failed: boom]");
        }
        other => panic!("expected AgentEnd, got {other:?}"),
    }
}
```

- [ ] **Step 7: Run the bridge test to verify it fails**

Run: `cargo test --no-default-features --features tui agent_failed_maps_to_agent_end -- --nocapture`
Expected: FAIL — non-exhaustive `match` / unknown variant, or the mapping returns `[]` if `AgentFailed` fell into the catch-all.

- [ ] **Step 8: Add the bridge mapping arm**

In `src/core/orchestration/es/bridge.rs`, add this arm right after the `ExecutionEvent::AgentObserved { .. }` arm (line ~111):

```rust
        ExecutionEvent::AgentFailed { agent, error } => vec![RunEvent::AgentEnd {
            agent: agent.clone(),
            tin: 0,
            tout: 0,
            cost: 0.0,
            content: crate::core::orchestration::es::event::delegation_failed_content(error),
        }],
```

If the compiler reports the terminal catch-all (lines ~190-200, the `Completed | RunStarted | ... => vec![]` group) is now unreachable/exhaustive-conflicting, no change is needed there — `AgentFailed` is matched explicitly above it. If instead the compiler still complains about non-exhaustiveness, ensure `AgentFailed` is not accidentally listed in the catch-all group.

- [ ] **Step 9: Run the bridge test to verify it passes**

Run: `cargo test --no-default-features --features tui agent_failed_maps_to_agent_end -- --nocapture`
Expected: PASS.

- [ ] **Step 10: Run the gate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage
```
Expected: all green (note: any other `match` over `ExecutionEvent` in the codebase may now need an `AgentFailed` arm — clippy/compiler will point to the exact file:line; add a sensible arm following the neighbours).

- [ ] **Step 11: Commit**

```bash
git add src/core/orchestration/es/event.rs src/core/orchestration/es/state.rs src/core/orchestration/es/bridge.rs
git commit -m "feat(oh1): add AgentFailed event with reducer and RunEvent bridge

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `Action::InvokeParallel` + concurrent `run_loop` handler

**Files:**
- Modify: `Cargo.toml` (promote `futures-util` to a direct dependency)
- Modify: `src/core/orchestration/es/engine.rs:30-42` (add variant + `InvokeSpec`), `:173-201` (add handler arm), `:291-359` (add tests)

**Interfaces:**
- Consumes: `ExecutionEvent::AgentFailed { agent, error }` (Task 1); `append_and_apply(log, run_id, state, event)`; the `EffectRunner::run_invoke(&self, agent: &str, input: &str, state: &ExecutionState) -> anyhow::Result<ExecutionEvent>` trait (unchanged).
- Produces:
  - `Action::InvokeParallel { batch: Vec<InvokeSpec>, max_concurrency: usize }` on the `Action` enum.
  - `pub struct InvokeSpec { pub agent: String, pub input: String }`.

- [ ] **Step 1: Promote `futures-util` to a direct dependency**

`futures-util` is already in `Cargo.lock` (transitive via reqwest/tokio), so this pulls nothing new. Add to `Cargo.toml` under `[dependencies]` (keep it non-optional so the ES engine compiles in every feature mode):

```toml
futures-util = { version = "0.3", default-features = false, features = ["std", "async-await"] }
```

Run `cargo build --no-default-features --features tui` to confirm it resolves.
Expected: builds; `Cargo.lock` unchanged except the dependency graph edge.

- [ ] **Step 2: Write the failing determinism test**

Add to `#[cfg(test)] mod tests` in `src/core/orchestration/es/engine.rs`. This mock decider emits one `InvokeParallel([a, b, c])` before any observation, then `Complete`. The mock runner sleeps **longer for earlier `Vec` positions** so completion order (c, b, a) is the reverse of `Vec` order (a, b, c); it also records completion order into a shared `Vec`. The test asserts the **recorded log** keeps `Vec` order regardless.

```rust
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
                vec![Action::Complete { content: "done".into() }]
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
        order: Vec<String>,               // Vec order a,b,c → delays 30,20,10ms
        completions: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl EffectRunner for OrderedEff {
        async fn run_invoke(
            &self,
            agent: &str,
            _input: &str,
            _s: &ExecutionState,
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
            InvokeSpec { agent: "a".into(), input: "x".into() },
            InvokeSpec { agent: "b".into(), input: "x".into() },
            InvokeSpec { agent: "c".into(), input: "x".into() },
        ];
        let completions = Arc::new(Mutex::new(Vec::new()));
        let decider = ParDecider { batch: batch.clone(), cap: 4 };
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
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --no-default-features --features tui invoke_parallel_records_in_vec_order -- --nocapture`
Expected: FAIL to compile — `no variant named InvokeParallel` / `cannot find type InvokeSpec`.

- [ ] **Step 4: Add the `Action` variant + `InvokeSpec`**

In `src/core/orchestration/es/engine.rs`, extend the `Action` enum (currently ends with `Complete { content: String }` around line 41) and add `InvokeSpec` just after it:

```rust
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
```

```rust
/// One unit of work inside an [`Action::InvokeParallel`] batch. Named
/// distinctly from the `Action::Invoke` variant to avoid confusion.
#[derive(Debug, Clone)]
pub struct InvokeSpec {
    pub agent: String,
    pub input: String,
}
```

- [ ] **Step 5: Add the `run_loop` handler arm**

Add `use futures_util::StreamExt;` to the imports at the top of `engine.rs`. Then add this arm to the `match action` block in `run_loop` (after the `Action::Invoke { .. }` arm, before `Action::Emit`):

```rust
                Action::InvokeParallel { batch, max_concurrency } => {
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
                    //    snapshot of the now-updated state. `buffer_unordered`
                    //    polls the borrowing futures in place (no spawn, no
                    //    'static bound). Nothing is appended during this phase,
                    //    so only shared borrows of `state`/`effects` are live.
                    let snapshot: &ExecutionState = state;
                    let cap = max_concurrency.max(1);
                    let mut outcomes: Vec<(usize, anyhow::Result<ExecutionEvent>)> =
                        futures_util::stream::iter(batch.iter().enumerate())
                            .map(|(i, spec)| async move {
                                (i, effects.run_invoke(&spec.agent, &spec.input, snapshot).await)
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
```

- [ ] **Step 6: Run the determinism test to verify it passes**

Run: `cargo test --no-default-features --features tui invoke_parallel_records_in_vec_order -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Write the failing cap test**

Add to the same test module. Mock runner tracks live concurrency with atomics and records the max observed; a batch of 6 with `max_concurrency: 2` must never exceed 2 in flight.

```rust
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CapEff {
        live: AtomicUsize,
        max_seen: AtomicUsize,
    }
    #[async_trait]
    impl EffectRunner for CapEff {
        async fn run_invoke(
            &self,
            agent: &str,
            _input: &str,
            _s: &ExecutionState,
        ) -> anyhow::Result<E> {
            let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_seen.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            self.live.fetch_sub(1, Ordering::SeqCst);
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
            .map(|i| InvokeSpec { agent: format!("a{i}"), input: "x".into() })
            .collect();
        let decider = ParDecider { batch: batch.clone(), cap: 2 };
        let eff = CapEff {
            live: AtomicUsize::new(0),
            max_seen: AtomicUsize::new(0),
        };
        let mut log = InMemoryLog::default();
        let init = vec![E::RunStarted {
            run_id: "r".into(),
            pattern: "test".into(),
            agents: batch.iter().map(|s| s.agent.clone()).collect(),
            input: "go".into(),
            project: None,
        }];
        run_event_sourced("r", init, &decider, &eff, &mut log)
            .await
            .unwrap();
        assert!(
            eff.max_seen.load(Ordering::SeqCst) <= 2,
            "observed {} concurrent invocations, cap was 2",
            eff.max_seen.load(Ordering::SeqCst)
        );
    }
```

- [ ] **Step 8: Run the cap test**

Run: `cargo test --no-default-features --features tui invoke_parallel_respects_concurrency_cap -- --nocapture`
Expected: PASS (the handler already caps via `buffer_unordered(cap)` — this test locks the behavior in).

- [ ] **Step 9: Write the failing partial-failure test**

The runner `Err`s for agent `b`; assert the log records `AgentObserved a`, `AgentFailed b`, `AgentObserved c` in Vec order, the run still completes, and `b`'s conversation ends with the assistant marker.

```rust
    struct FailBEff;
    #[async_trait]
    impl EffectRunner for FailBEff {
        async fn run_invoke(
            &self,
            agent: &str,
            _input: &str,
            _s: &ExecutionState,
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
            InvokeSpec { agent: "a".into(), input: "x".into() },
            InvokeSpec { agent: "b".into(), input: "x".into() },
            InvokeSpec { agent: "c".into(), input: "x".into() },
        ];
        let decider = ParDecider { batch: batch.clone(), cap: 4 };
        let mut log = InMemoryLog::default();
        let init = vec![E::RunStarted {
            run_id: "r".into(),
            pattern: "test".into(),
            agents: vec!["a".into(), "b".into(), "c".into()],
            input: "go".into(),
            project: None,
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
        assert!(events
            .iter()
            .any(|e| matches!(e, E::AgentFailed { agent, .. } if agent == "b")));

        // b reads as settled: last turn is the assistant failure marker.
        let convo_b = state.conversations.get("b").unwrap();
        assert_eq!(convo_b.last().unwrap().role, "assistant");
        assert_eq!(convo_b.last().unwrap().content, "[Delegation failed: boom]");
    }
```

- [ ] **Step 10: Run the partial-failure test**

Run: `cargo test --no-default-features --features tui invoke_parallel_records_failure_and_continues -- --nocapture`
Expected: PASS.

- [ ] **Step 11: Run the gate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage
```
Expected: all green. If clippy flags the `match action` in `run_loop` or any other `Action` match as non-exhaustive, add the `InvokeParallel` arm where indicated.

- [ ] **Step 12: Commit**

```bash
git add Cargo.toml Cargo.lock src/core/orchestration/es/engine.rs
git commit -m "feat(oh1): add Action::InvokeParallel with bounded concurrent run_loop

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: `OrchestrationConfig::max_concurrency` config surface

**Files:**
- Modify: `src/core/orchestration/mod.rs:154-215` (add field to the struct + accessor in the `impl OrchestrationConfig` block)

**Interfaces:**
- Produces: `OrchestrationConfig::max_concurrency: Option<u32>` field + `pub fn max_concurrency(&self) -> usize` (default 4). Consumed by the hierarchical decider in Lot 2.

- [ ] **Step 1: Write the failing accessor test**

Add to the test module in `src/core/orchestration/mod.rs` (if none exists, create `#[cfg(test)] mod tests { use super::*; ... }` at the end of the file):

```rust
    #[test]
    fn max_concurrency_defaults_to_four() {
        let cfg = OrchestrationConfig::default();
        assert_eq!(cfg.max_concurrency(), 4);
    }

    #[test]
    fn max_concurrency_honors_override() {
        let cfg = OrchestrationConfig {
            max_concurrency: Some(8),
            ..OrchestrationConfig::default()
        };
        assert_eq!(cfg.max_concurrency(), 8);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --no-default-features --features tui max_concurrency -- --nocapture`
Expected: FAIL to compile — `no field max_concurrency` / `no method named max_concurrency`.

- [ ] **Step 3: Add the field**

In `src/core/orchestration/mod.rs`, add to `struct OrchestrationConfig` in the `// ── Shared limits (all patterns) ──` block (next to `max_iterations`), with a serde default so existing `armadai.yaml` files without the key keep working:

```rust
    /// Max number of delegations executed concurrently within a single
    /// parallel fan-out batch (default: 4). Consumed by patterns that opt into
    /// `Action::InvokeParallel` (hierarchical, Lot 2).
    #[serde(default)]
    pub max_concurrency: Option<u32>,
```

- [ ] **Step 4: Add the accessor**

In the `impl OrchestrationConfig` block (next to `max_iterations()`), add:

```rust
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency.unwrap_or(4) as usize
    }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --no-default-features --features tui max_concurrency -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Run the gate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage
```
Expected: all green. If adding the field breaks any struct-literal construction of `OrchestrationConfig` elsewhere (the compiler will name the file:line), add `max_concurrency: None,` there or switch that literal to `..Default::default()`.

- [ ] **Step 7: Commit**

```bash
git add src/core/orchestration/mod.rs
git commit -m "feat(oh1): add OrchestrationConfig.max_concurrency (default 4)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage:**
- `Action::InvokeParallel { batch, max_concurrency }` + `InvokeSpec` → Task 2. ✅
- Deterministic recorded order (AgentInvoked×N then outcomes, Vec order) → Task 2 handler + determinism test. ✅
- `buffer_unordered(max_concurrency)` bounded concurrency → Task 2 handler + cap test. ✅
- `ExecutionEvent::AgentFailed { agent, error }` (event + reducer + bridge) → Task 1. ✅
- Reducer pushes assistant marker so child reads as settled → Task 1 reducer test. ✅
- Budget untouched on failure → Task 1 reducer test. ✅
- Bridge → `RunEvent::AgentEnd` → Task 1 bridge test. ✅
- Collect-and-record (run continues) → Task 2 partial-failure test. ✅
- `OrchestrationConfig::max_concurrency` field + accessor default 4 → Task 3. ✅
- Engine signatures unchanged (cap on the action) → guaranteed by Task 2's variant shape (no signature edits). ✅
- `futures-util` non-optional direct dep → Task 2 Step 1. ✅
- Ring/direct/blackboard untouched → no task modifies them (they never emit `InvokeParallel`). ✅
- Out of scope (blackboard parallelism, #279 rich failure render, #270 timeout, hierarchical opt-in) → not in this lot (Lot 2 / separate features). ✅

**2. Placeholder scan:** No TBD/TODO/"handle edge cases" — every code step carries complete code. ✅

**3. Type consistency:**
- `InvokeSpec { agent: String, input: String }` — used identically in Task 2 variant, tests, and handler. ✅
- `ExecutionEvent::AgentFailed { agent: String, error: String }` — same shape in Task 1 (event/apply/bridge) and Task 2 (handler/tests). ✅
- `delegation_failed_content(error: &str) -> String` — defined in Task 1, consumed by apply + bridge; the literal `"[Delegation failed: boom]"` in every test matches `format!("[Delegation failed: {error}]")`. ✅
- `RunEvent::AgentEnd { agent, tin, tout, cost, content }` — field names match `src/core/events.rs:22-28`. ✅
- `ChatMessage { role, content }` — matches `src/providers/traits.rs:18`. ✅
- `max_concurrency()` accessor name consistent across Task 3 and the Lot 2 consumer note. ✅
