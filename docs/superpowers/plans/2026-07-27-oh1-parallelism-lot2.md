# OH1 Parallelism — Lot 2 (hierarchical opt-in) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the hierarchical orchestration pattern emit `Action::InvokeParallel` for a multi-child delegation fan-out, so a coordinator's independent delegations run concurrently instead of one-by-one, and surface `AgentFailed` in the legacy result.

**Architecture:** The Lot 1 socle already runs an `InvokeParallel { batch, max_concurrency }` action concurrently with deterministic recorded order and collect-and-record failure handling. Lot 2 flips the one place that fans out — `HierarchicalDecider::dispatch_actions` — to build a single `InvokeParallel` (cap = `config.max_concurrency()`) when a delegation round has ≥2 children, keeping the sequential `Action::Invoke` path for 0/1-child rounds, the coordinator kick-off, and synthesis re-invokes. It also extends the legacy result extractor to render `AgentFailed`.

**Tech Stack:** Rust edition 2024, event-sourcing engine under `src/core/orchestration/es/` (`hierarchical.rs`, `bridge.rs`), `tokio`, `ScriptedProvider` test harness.

## Global Constraints

- Target branch: `master` (master-only). Work on `feat/oh1-parallel-2` (branched from `master` at `9f9afd2`, which includes the Lot 1 socle).
- Reference spec: `docs/superpowers/specs/2026-07-27-oh1-parallelism-design.md` (§5 hierarchical opt-in).
- **Determinism (cardinal):** recorded event order = the decider's `Vec` order. The parallel dispatch MUST emit the bookkeeping `Emit(...)` events in step order, then a single `InvokeParallel` whose `batch` is in `plan_from_response` line order. The socle already records `AgentInvoked ×N` then outcomes in `batch` order.
- **Opt-in only:** only `dispatch_actions` changes. The coordinator kick-off (`decide` step 1) and synthesis re-invoke (`synthesis_actions`) stay `Action::Invoke`. Ring/direct/blackboard untouched.
- **Threshold:** emit `InvokeParallel` only for a batch of **≥2** invocations. A 0- or 1-invocation dispatch keeps its current `Action::Invoke` shape (no concurrency needed, zero behavior change, minimal test churn).
- **Cap source:** `self.config.max_concurrency()` (the Lot 1 accessor, default 4).
- **Collect-and-record:** already in the socle — a failed child becomes `AgentFailed` and the run continues. Lot 2 must ensure `to_orchestration_result` surfaces that failure instead of returning empty content.
- Engine signatures unchanged (cap carried in the action).
- Gate per task: `cargo fmt --all` + clippy 3 modes (`--no-default-features --features tui` / `tui,providers-api` / `tui,web,storage`) `-D warnings` + `cargo test --no-default-features --features tui` + `cargo test --no-default-features --features tui,storage`.
- `rust-analyzer` is unreliable here (stale ABI proc-macro, E0308/HRTB false positives) — **verify at the compiler** with `cargo`.
- `cat` is aliased to `bat`; use `command cat`.
- Conventional Commits, single type per commit, trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- One PR for the whole lot + independent review + Dimitri visual/manual validation (this lot changes real runtime behavior — a live hierarchical run in the Workroom).

---

## File Structure

- `src/core/orchestration/es/bridge.rs` — `to_orchestration_result` fallback content extended to consider `AgentFailed` (Task 1).
- `src/core/orchestration/es/hierarchical.rs`:
  - `impl HierarchicalDecider`: extract `invoke_emit_actions` (emits only) from `invoke_actions`; rewrite `dispatch_actions` to build a parallel batch for ≥2 children (Task 2).
  - import `InvokeSpec` from `super::engine` (Task 2).
  - decider unit tests: convert ≥2-child dispatch assertions from `Action::Invoke` to `Action::InvokeParallel { batch }` (Task 2).
  - e2e integration tests: add a concurrent-fan-out partial-failure test (Task 3).

---

## Task 1: `to_orchestration_result` surfaces `AgentFailed`

**Files:**
- Modify: `src/core/orchestration/es/bridge.rs:284-299` (the `content` fallback chain in `to_orchestration_result`)
- Test: `src/core/orchestration/es/bridge.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `ExecutionEvent::AgentFailed { agent, error }` and `delegation_failed_content` (Lot 1).
- Produces: no new public API — behavior change only (a run whose last observation is a failure yields the failure marker as content instead of empty).

Context: today `to_orchestration_result`'s `content` is: last `Completed` → else last `AgentObserved` → else empty. Once the hierarchical decider emits `AgentFailed` (Task 2), a fan-out that fails entirely and halts (no `Completed`, last event an `AgentFailed`, no `AgentObserved` after it) would yield **empty** content. Extend the fallback so a trailing `AgentFailed` surfaces its marker.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `src/core/orchestration/es/bridge.rs`:

```rust
#[test]
fn to_orchestration_result_falls_back_to_agent_failed_when_no_completed_or_observed() {
    // A halted run whose only outcome was a failed delegation: no Completed,
    // no AgentObserved — the result must surface the failure, not be empty.
    let events = vec![
        ExecutionEvent::AgentInvoked { agent: "b".into(), input: "go".into() },
        ExecutionEvent::AgentFailed { agent: "b".into(), error: "boom".into() },
        ExecutionEvent::Halted { reason: "no_progress".into() },
    ];
    let state = crate::core::orchestration::es::state::fold(&events);
    let result = to_orchestration_result(&state, &events);
    assert_eq!(result.content, "[Delegation failed: boom]");
}

#[test]
fn to_orchestration_result_prefers_completed_over_agent_failed() {
    // A Completed still wins over any earlier AgentFailed.
    let events = vec![
        ExecutionEvent::AgentInvoked { agent: "b".into(), input: "go".into() },
        ExecutionEvent::AgentFailed { agent: "b".into(), error: "boom".into() },
        ExecutionEvent::Completed { content: "final answer".into() },
    ];
    let state = crate::core::orchestration::es::state::fold(&events);
    let result = to_orchestration_result(&state, &events);
    assert_eq!(result.content, "final answer");
}
```

(If `fold` is imported under a different path in this test module, use the module's existing import — grep the test module for how it builds an `ExecutionState` from events; the socle tests use `state::fold`.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --no-default-features --features tui to_orchestration_result_falls_back_to_agent_failed -- --nocapture`
Expected: FAIL — `result.content` is empty (`""`), not `"[Delegation failed: boom]"`.

- [ ] **Step 3: Extend the fallback chain**

In `src/core/orchestration/es/bridge.rs`, change the `content` extraction in `to_orchestration_result` (currently: `Completed` → else last `AgentObserved` → else empty) so the second fallback also matches `AgentFailed`:

```rust
    let content = events
        .iter()
        .rev()
        .find_map(|e| match e {
            ExecutionEvent::Completed { content } => Some(content.clone()),
            _ => None,
        })
        .or_else(|| {
            // Fall back to the last observed OR failed agent turn, so a run
            // that ended on a failed delegation surfaces the failure marker
            // rather than empty content.
            events.iter().rev().find_map(|e| match e {
                ExecutionEvent::AgentObserved { content, .. } => Some(content.clone()),
                ExecutionEvent::AgentFailed { error, .. } => {
                    Some(crate::core::orchestration::es::event::delegation_failed_content(error))
                }
                _ => None,
            })
        })
        .unwrap_or_default();
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --no-default-features --features tui to_orchestration_result -- --nocapture`
Expected: PASS (both new tests).

- [ ] **Step 5: Run the gate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage
```
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/core/orchestration/es/bridge.rs
git commit -m "feat(oh1): surface AgentFailed in to_orchestration_result fallback

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Parallelize `dispatch_actions` (≥2 children → `InvokeParallel`)

**Files:**
- Modify: `src/core/orchestration/es/hierarchical.rs:20` (import `InvokeSpec`)
- Modify: `src/core/orchestration/es/hierarchical.rs:570-592` (extract `invoke_emit_actions` from `invoke_actions`)
- Modify: `src/core/orchestration/es/hierarchical.rs:690-714` (`dispatch_actions` builds a parallel batch)
- Test: `src/core/orchestration/es/hierarchical.rs` decider unit tests (convert `two_delegations_become_two_invokes_in_order` and any other ≥2-child dispatch test; add a new fan-out-parallel test)

**Interfaces:**
- Consumes: `Action::InvokeParallel { batch: Vec<InvokeSpec>, max_concurrency: usize }` and `pub struct InvokeSpec { pub agent: String, pub input: String }` (Lot 1, in `super::engine`); `self.config.max_concurrency() -> usize` (Lot 1).
- Produces: `fn invoke_emit_actions(&self, agent_name: &str, input: &str, state: &ExecutionState, delegation_event: Option<ExecutionEvent>) -> Vec<Action>` — returns the ordered `Emit(...)` actions (ModelRouted?, delegation?, NestedStarted?) WITHOUT the trailing `Invoke`. `invoke_actions` becomes `invoke_emit_actions` + the `Action::Invoke` push.

- [ ] **Step 1: Add the `InvokeSpec` import**

In `src/core/orchestration/es/hierarchical.rs:20`, extend the import:

```rust
use super::engine::{Action, Decider, EffectRunner, InvokeSpec, run_event_sourced};
```

- [ ] **Step 2: Extract `invoke_emit_actions` from `invoke_actions`**

Replace the current `invoke_actions` (lines ~570-592) with a split that keeps `invoke_actions` behavior byte-identical for its existing callers (kick-off, synthesis) while exposing the emit-only part:

```rust
    /// The ordered bookkeeping `Emit(...)` actions that precede an invocation:
    /// an optional `ModelRouted` (if it routes `latest:auto`), an optional
    /// delegation event (`Delegated`/`AskedPeer`/`Escalated`, from
    /// `plan_from_response`), then an optional `NestedStarted` (nested-team
    /// lead). Split out of `invoke_actions` so the parallel dispatch path can
    /// record every child's emits sequentially (in Vec order) before a single
    /// `InvokeParallel`, while the sequential callers keep `Emit + Invoke`.
    fn invoke_emit_actions(
        &self,
        agent_name: &str,
        input: &str,
        state: &ExecutionState,
        delegation_event: Option<ExecutionEvent>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(event) = self.model_routed_event(agent_name, input, state) {
            actions.push(Action::Emit(event));
        }
        if let Some(event) = delegation_event {
            actions.push(Action::Emit(event));
        }
        if let Some(event) = self.nested_started_event(agent_name) {
            actions.push(Action::Emit(event));
        }
        actions
    }

    /// The full sequential batch for one invocation: the bookkeeping emits
    /// (`invoke_emit_actions`) followed by the `Invoke` itself. Used by the
    /// coordinator kick-off and synthesis re-invokes (single-agent paths).
    fn invoke_actions(
        &self,
        agent_name: &str,
        input: &str,
        state: &ExecutionState,
        delegation_event: Option<ExecutionEvent>,
    ) -> Vec<Action> {
        let mut actions = self.invoke_emit_actions(agent_name, input, state, delegation_event);
        actions.push(Action::Invoke {
            agent: agent_name.to_string(),
            input: input.to_string(),
        });
        actions
    }
```

- [ ] **Step 3: Run the suite to confirm the refactor is behavior-neutral**

Run: `cargo test --no-default-features --features tui --lib core::orchestration::es::hierarchical`
Expected: PASS unchanged (pure refactor — `invoke_actions` still produces `emits + Invoke`; no caller changed yet).

- [ ] **Step 4: Write the failing fan-out-parallel unit test**

Add to the decider unit-test module (near `two_delegations_become_two_invokes_in_order`, ~line 1637). This asserts the new shape: emits in step order, then a single `InvokeParallel` with the batch in line order and the default cap:

```rust
        // Fan-out ≥2: coordinator delegates to two siblings → one
        // InvokeParallel (batch in line order, cap = default 4), preceded by
        // both Delegated emits in order.
        #[test]
        fn two_delegations_become_one_invoke_parallel_in_order() {
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-specialist", "concrete-model"),
                    ("qa-specialist", "concrete-model"),
                ],
                base_config(),
                5,
                50,
                None,
                None,
            );
            let events = vec![
                run_started(&["dev-lead"]),
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "build X".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "@core-specialist: implémente X\n@qa-specialist: teste X".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);

            // Exactly one InvokeParallel, batch in line order, default cap 4.
            let parallels: Vec<&(Vec<InvokeSpec>, usize)> = Vec::new(); // placeholder to keep types explicit below
            let _ = parallels;
            let batch_agents: Vec<&str> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::InvokeParallel { batch, max_concurrency } => {
                        assert_eq!(*max_concurrency, 4);
                        Some(batch.iter().map(|s| s.agent.as_str()).collect::<Vec<_>>())
                    }
                    _ => None,
                })
                .flatten()
                .collect();
            assert_eq!(batch_agents, vec!["core-specialist", "qa-specialist"]);
            assert_eq!(
                actions
                    .iter()
                    .filter(|a| matches!(a, Action::InvokeParallel { .. }))
                    .count(),
                1
            );
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Invoke { .. })),
                "fan-out of 2 must not emit any sequential Invoke"
            );

            // Both Delegated emitted in line order, before the InvokeParallel.
            let delegated_targets: Vec<&str> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::Emit(ExecutionEvent::Delegated { to, .. }) => Some(to.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(delegated_targets, vec!["core-specialist", "qa-specialist"]);
            let parallel_pos = actions
                .iter()
                .position(|a| matches!(a, Action::InvokeParallel { .. }))
                .unwrap();
            let last_delegated_pos = actions
                .iter()
                .rposition(|a| matches!(a, Action::Emit(ExecutionEvent::Delegated { .. })))
                .unwrap();
            assert!(last_delegated_pos < parallel_pos);
        }
```

Remove the stray `parallels` placeholder line if the implementer prefers — it is only there to avoid an unused-import lint if `InvokeSpec` is otherwise unreferenced; the `batch.iter()` closure already references `InvokeSpec` fields, so it can be deleted.

- [ ] **Step 5: Run the new test to verify it fails**

Run: `cargo test --no-default-features --features tui two_delegations_become_one_invoke_parallel -- --nocapture`
Expected: FAIL — `dispatch_actions` still emits two `Action::Invoke`, no `InvokeParallel`.

- [ ] **Step 6: Rewrite `dispatch_actions` to fan out in parallel for ≥2 children**

Replace the `plan_from_response(...).flat_map(...)` tail of `dispatch_actions` (lines ~706-714, keep the `max_depth` guard above it unchanged) with:

```rust
        // Collect the invoke-steps (a `Complete` cannot occur here — only
        // agents with pending directives reach dispatch — and is dropped
        // defensively).
        let invoke_steps: Vec<(String, String, ExecutionEvent)> =
            plan_from_response(&latest, agent, &self.config, depth)
                .into_iter()
                .filter_map(|step| match step {
                    PlannedStep::Invoke { agent, task, event } => Some((agent, task, event)),
                    PlannedStep::Complete { .. } => None,
                })
                .collect();

        // 0 or 1 child: keep the sequential `Emit(s) + Invoke` shape (no
        // concurrency needed, byte-identical to before this lot).
        if invoke_steps.len() <= 1 {
            return invoke_steps
                .into_iter()
                .flat_map(|(child, task, event)| {
                    self.invoke_actions(&child, &task, state, Some(event))
                })
                .collect();
        }

        // ≥2 children: record every child's bookkeeping emits in line order,
        // then a single `InvokeParallel` whose batch is in line order. The
        // socle records `AgentInvoked ×N` then outcomes in batch order, so
        // replay stays deterministic.
        let mut actions = Vec::new();
        let mut batch = Vec::new();
        for (child, task, event) in invoke_steps {
            actions.extend(self.invoke_emit_actions(&child, &task, state, Some(event)));
            batch.push(InvokeSpec { agent: child, input: task });
        }
        actions.push(Action::InvokeParallel {
            batch,
            max_concurrency: self.config.max_concurrency(),
        });
        actions
```

- [ ] **Step 7: Run the new test to verify it passes**

Run: `cargo test --no-default-features --features tui two_delegations_become_one_invoke_parallel -- --nocapture`
Expected: PASS.

- [ ] **Step 8: Convert the superseded exemplar decider test**

The old `two_delegations_become_two_invokes_in_order` (line ~1637) now fails (it asserts two `Action::Invoke`). It is superseded by the new parallel test from Step 4 — **delete** `two_delegations_become_two_invokes_in_order` entirely (do not keep a duplicate asserting the old shape).

- [ ] **Step 9: Find and convert any other ≥2-child dispatch decider test**

Run the hierarchical decider unit tests and fix any remaining failure caused by the shape change:

Run: `cargo test --no-default-features --features tui --lib core::orchestration::es::hierarchical 2>&1 | grep -E "FAILED|test result"`

For each failing decider test whose failure is because it matched `Action::Invoke { agent, .. }` on a **dispatch of ≥2 children** (e.g. a test seeding an `AgentObserved` whose content is `@core-a: …\n@core-b: …`, around line ~2399): apply the same transformation as Step 4 — replace the `Action::Invoke` extraction with an `Action::InvokeParallel { batch, .. }` extraction reading `batch.iter().map(|s| s.agent.as_str())`, and adjust any "Delegated precedes Invoke" assertion to "Delegated precedes the InvokeParallel". Do NOT touch tests that assert `Action::Invoke` for the **coordinator kick-off** (state with empty conversations) or **single-child** dispatch or **synthesis** re-invoke — those legitimately still emit `Action::Invoke` and must keep passing. If a test's intent is genuinely a single delegation (batch size 1), it is unaffected by design.

- [ ] **Step 10: Run the gate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage
```
Expected: all green.

- [ ] **Step 11: Commit**

```bash
git add src/core/orchestration/es/hierarchical.rs
git commit -m "feat(oh1): hierarchical fan-out emits InvokeParallel for >=2 children

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: e2e integration — concurrent fan-out with a partial failure

**Files:**
- Test: `src/core/orchestration/es/hierarchical.rs` e2e integration test module (the one with `ScriptedProvider`, `run_hierarchical_es`, `es_test_agent`, `es_flat_config`, `final_content` — around lines 3215-3760; model on `es_multiple_delegations_synthesize` at ~3697)

**Interfaces:**
- Consumes: `run_hierarchical_es(run_id, coordinator, input, config, agents, providers, routing_rules, &mut log)`, `es_test_agent`, `es_flat_config`, `final_content`, `ScriptedProvider`, and the Lot 1/Lot 2 machinery. This task adds no production code — it proves the whole path end-to-end.

Context: `es_multiple_delegations_synthesize` (~3697) already exercises a healthy 2-child fan-out through `run_hierarchical_es`; after Task 2 it now runs the two children via `InvokeParallel` and still passes (it asserts trace + non-empty content, not `Action` shapes). This task adds the **partial-failure** e2e: one child's provider errors, and the run must still complete on the surviving child's result (collect-and-record), with the failure recorded.

- [ ] **Step 1: Add a minimal always-failing provider helper**

In the e2e test module (same module as `ScriptedProvider`, ~line 3263), add a provider whose calls always error, modeled EXACTLY on `ScriptedProvider`'s `impl Provider` (same method signatures, same `metadata()` body — copy them), but each async method body is `anyhow::bail!("simulated provider failure")`:

```rust
    /// A provider whose every call fails — to exercise the collect-and-record
    /// path (`run_invoke` `Err` → `AgentFailed`, run continues).
    struct FailingProvider;

    #[async_trait::async_trait]
    impl Provider for FailingProvider {
        async fn complete(
            &self,
            _request: crate::providers::traits::CompletionRequest,
        ) -> anyhow::Result<crate::providers::traits::CompletionResponse> {
            anyhow::bail!("simulated provider failure")
        }
        async fn stream(
            &self,
            _request: crate::providers::traits::CompletionRequest,
        ) -> anyhow::Result<crate::providers::traits::TokenStream> {
            anyhow::bail!("simulated provider failure")
        }
        fn metadata(&self) -> crate::providers::traits::ProviderMetadata {
            // Mirror ScriptedProvider::metadata() in this module (copy its body
            // verbatim — same struct fields/values).
            <REPLACE WITH THE EXACT BODY OF ScriptedProvider::metadata() FROM THIS MODULE>
        }
    }
```

Read `ScriptedProvider`'s `impl Provider` block (~line 3286) and copy the exact `complete`/`stream` type paths and the exact `metadata()` body — the placeholders above must be replaced with the real signatures/body used in this module (they may already be imported unqualified, e.g. `CompletionRequest` rather than the full path — match the module's existing style).

- [ ] **Step 2: Write the failing partial-failure e2e test**

Add next to `es_multiple_delegations_synthesize` (~3756):

```rust
    // Scenario: coordinator delegates to two siblings concurrently; one child's
    // provider fails. Collect-and-record: the run still completes on the
    // surviving child, and the failure is recorded (AgentFailed) rather than
    // aborting the run.
    #[tokio::test]
    async fn es_parallel_fanout_survives_one_failed_child() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        agents.insert(
            "core-specialist".to_string(),
            es_test_agent("core-specialist", "concrete-model"),
        );
        agents.insert(
            "qa-specialist".to_string(),
            es_test_agent("qa-specialist", "concrete-model"),
        );
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "@core-specialist: implémente X\n@qa-specialist: teste X",
                "Synthèse : livré malgré un échec.",
            ])),
        );
        // core-specialist fails; qa-specialist succeeds.
        providers.insert("core-specialist".to_string(), Arc::new(FailingProvider));
        providers.insert(
            "qa-specialist".to_string(),
            Arc::new(ScriptedProvider::new(&["X est testé, RAS."])),
        );

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-partial-fail",
            "dev-lead",
            "build X",
            es_flat_config("dev-lead", &["core-specialist", "qa-specialist"]),
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        // Run completed despite the failure (collect-and-record, not abort).
        assert_eq!(st.status, RunStatus::Completed);

        let events = log.events("run-partial-fail").unwrap();

        // The failed child is recorded as AgentFailed, in Vec order (core
        // before qa), and qa was still invoked and observed.
        assert!(
            events.iter().any(|e| matches!(
                e,
                ExecutionEvent::AgentFailed { agent, .. } if agent == "core-specialist"
            )),
            "expected AgentFailed for core-specialist"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                ExecutionEvent::AgentObserved { agent, .. } if agent == "qa-specialist"
            )),
            "expected qa-specialist to still be observed"
        );

        // Deterministic recorded order: both AgentInvoked in batch order.
        let invoked: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                ExecutionEvent::AgentInvoked { agent, .. }
                    if agent == "core-specialist" || agent == "qa-specialist" =>
                {
                    Some(agent.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(invoked, vec!["core-specialist", "qa-specialist"]);

        // Final content is the coordinator's synthesis (non-empty).
        assert!(
            !final_content(&log, "run-partial-fail").trim().is_empty(),
            "expected non-empty final content after synthesizing partial results"
        );
    }
```

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo test --no-default-features --features tui es_parallel_fanout_survives_one_failed_child -- --nocapture`
Expected: PASS. (If it FAILS because the coordinator's synthesis does not settle when a child is `AgentFailed`, that is a real finding — the child must read as "settled" via the Lot 1 marker; capture it and report before forcing the test green.)

- [ ] **Step 4: Confirm the healthy fan-out e2e still passes through the parallel path**

Run: `cargo test --no-default-features --features tui es_multiple_delegations_synthesize -- --nocapture`
Expected: PASS (unchanged assertions; it now runs its two children via `InvokeParallel`).

- [ ] **Step 5: Run the gate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage
```
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/core/orchestration/es/hierarchical.rs
git commit -m "test(oh1): e2e concurrent fan-out survives a failed child

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage (§5 + final-review Lot-2 item):**
- Hierarchical `dispatch_actions` opts in via `InvokeParallel` (≥2 children), emits `Delegated` sequentially then one `InvokeParallel` → Task 2. ✅
- Cap = `config.max_concurrency()` → Task 2 Step 6. ✅
- Determinism (emits in step order, batch in line order) → Task 2 test Step 4 + socle guarantee. ✅
- Sequential paths (kick-off, synthesis, 0/1-child) unchanged → Task 2 threshold + Step 9 guard. ✅
- `to_orchestration_result` surfaces `AgentFailed` (the final-review Lot-2 acceptance criterion) → Task 1. ✅
- e2e concurrent fan-out + one child fails → run completes on partial → Task 3. ✅
- Ring/direct/blackboard untouched → no task modifies them. ✅

**2. Placeholder scan:** One intentional placeholder in Task 3 Step 1 (`FailingProvider::metadata()` body) — it must be filled by copying `ScriptedProvider::metadata()` from the same module, which the step instructs explicitly, because the exact `ProviderMetadata` field values are module-local and not reproduced here to avoid drift. Every other step carries complete code. The stray `parallels` placeholder line in Task 2 Step 4 is flagged for deletion in the same step.

**3. Type consistency:**
- `InvokeSpec { agent, input }` and `Action::InvokeParallel { batch, max_concurrency }` — same shapes as Lot 1 (verified against `es/engine.rs`). ✅
- `invoke_emit_actions` / `invoke_actions` signatures identical except the trailing `Invoke` — callers of `invoke_actions` (kick-off line 754, synthesis) unchanged. ✅
- `plan_from_response` returns `Vec<PlannedStep>` with `PlannedStep::Invoke { agent, task, event }` / `Complete { content }` — matched in Task 2 Step 6. ✅
- `run_hierarchical_es` / `es_test_agent` / `es_flat_config` / `final_content` / `ScriptedProvider` signatures taken from `es_multiple_delegations_synthesize` (same module) — Task 3 mirrors them. ✅
- `delegation_failed_content(error)` path — same as Lot 1 Task 1. ✅
