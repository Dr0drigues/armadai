# OH1 Lot 6 — resume/replay CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `armadai run --replay <run_id>` (deterministic re-emission of a finished run to the display, no effects) and `armadai run --resume <run_id>` (reconstruct state from the event log and continue an interrupted run), closing OH1 Lot 6.

**Architecture:** The event log is already the source of truth (`execution_events` table, `SqliteLog`). `replay(run_id, log)` folds it to an `ExecutionState` without effects. Replay = fold + emit each event to the `EventSink`. Resume = seed a fresh loop from the replayed state (skipping re-append of existing events) and continue. A new core primitive `resume_event_sourced` provides the seeded loop; the 4 pattern dispatchers gain a resume entry. `run_id` (today a fresh uuid never shown) becomes visible so users can target it.

**Tech Stack:** Rust edition 2024, `core/orchestration/es/`, `cli/run.rs`, `storage` (rusqlite), clap.

## Global Constraints

- Design source: `docs/superpowers/specs/2026-07-21-oh1-event-sourcing-design.md` §8 (Reprise C7 & audit/replay) + Lot 6. Verbatim semantics:
  - `--resume <run_id>`: load `execution_events[run_id]`, fold → `ExecutionState`, **resume the pattern loop from that state** (only if status is non-terminal). **No effect re-executed** (observations are in the log).
  - `--replay <run_id>`: fold + **replay events to the display sink without executing effects** (deterministic visualization).
- **Persistence is gated `storage`.** Without `storage` (or if the DB is unavailable) the log is `InMemoryLog` (ephemeral) → resume/replay are impossible and MUST fail with a clear error, never silently no-op.
- **Never duplicate log events on resume**: `run_event_sourced` re-appends everything in its `initial` arg (`engine.rs:124` `append_and_apply` in the `for event in initial` loop). Resume MUST seed state via `replay()` (pure fold, no append) and continue — do NOT pass existing events as `initial`.
- `RunStatus` (`es/state.rs:14`): `Running` (resumable) | `Completed` | `Halted` (terminal). Resume of a terminal run is an error.
- Conventional Commits, single type per commit. Trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- rust-analyzer unreliable — verify at the compiler. Gate every task: `cargo fmt --all` + clippy 3 modes (`tui` / `tui,providers-api` / `tui,web,storage`) `-D warnings` + `cargo test --no-default-features --features tui` + `cargo test --no-default-features --features tui,storage`.

---

## File Structure

- `src/cli/mod.rs` — `Run` variant: `agent` becomes optional; add `--resume`/`--replay` flags (clap `ArgGroup`, mutually exclusive, and exclusive with `agent`).
- `src/cli/run.rs` — `execute(...)`/`run_inner(...)`: two new `Option<String>` params (`resume`, `replay`); dispatch to replay/resume paths; surface `run_id`.
- `src/core/events.rs` — surface `run_id`: add a `RunStart.run_id` field (or a new `RunEvent::Meta { run_id }` emitted first). Chosen: add `run_id: String` to `RunStart`.
- `src/core/orchestration/es/engine.rs` — new `resume_event_sourced(...)`; extract the shared loop body so `run_event_sourced` and `resume_event_sourced` share it.
- `src/core/orchestration/es/{direct,blackboard,ring,hierarchical}.rs` — a resume entry per pattern (seed from replayed state, continue).
- `src/storage/queries.rs` — reuse `all_event_log_run_ids`; add a helper to fetch a run's stored pattern (from `orchestration_runs`) so resume picks the right engine.
- `docs/wiki/orchestration-guide.md` — document `--resume`/`--replay`.

---

## Task 1: Surface `run_id` + CLI flag plumbing

**Files:**
- Modify: `src/core/events.rs` (RunStart gains `run_id`)
- Modify: `src/cli/mod.rs` (Run variant + handler)
- Modify: `src/cli/run.rs` (`execute`/`run_inner` params; emit run_id; the 4 dispatch sites pass their generated run_id into the emitted `RunStart`)
- Test: `src/core/events.rs` unit test (RunStart serializes run_id in `--json`)

**Interfaces:**
- Produces: `execute(..., resume: Option<String>, replay: Option<String>)` and `run_inner(..., resume: Option<String>, replay: Option<String>)`. `RunStart { run_id, v, agents, prov, model, in_chars }`.

- [ ] **Step 1: Add `run_id` to `RunStart`**

In `src/core/events.rs`, add `run_id: String` as the first field of the `RunStart` variant. Update every constructor/emitter of `RunStart` (grep `RunStart {`) — the 4 dispatch sites in `run.rs` already own a `run_id` local; pass it. For any non-orchestrated/simple path that emits `RunStart`, generate/propagate a run_id.

- [ ] **Step 2: Test run_id in JSON**

Add a unit test asserting a `RunStart { run_id: "abc", .. }` serializes with `"run_id":"abc"` (serde tag `t`). Run it; expect PASS.

- [ ] **Step 3: Human-surface the run_id**

On the human path (non-json), print one muted line at run start via `anstream` + `crate::cli::style::muted()`: `run <run_id>` (so users can copy it for `--resume`). Not on `--json`/`--quiet` (json carries it in `RunStart`).

- [ ] **Step 4: Make `agent` optional + add flags (clap)**

In `src/cli/mod.rs` `Run` variant: change `agent: String` → `agent: Option<String>`; add `#[arg(long, value_name="RUN_ID")] resume: Option<String>` and `replay: Option<String>`. Add a clap `ArgGroup` (e.g. `group(ArgGroup::new("mode").args(["agent","resume","replay"]).required(true))`) so exactly one of agent/resume/replay is given and resume⊕replay are mutually exclusive. Update the `Command::Run { .. }` handler to pass `resume`/`replay` into `run::execute`.

- [ ] **Step 5: Thread params + validate**

In `run.rs::execute` and `run_inner`, add `resume`/`replay` params. At the top of `execute`, branch: if `replay.is_some()` → call the replay path (Task 2, stub returning `anyhow::bail!("--replay not yet wired")` for now); if `resume.is_some()` → resume path (Task 3 stub); else the existing agent path (unwrap `agent`, which the ArgGroup guarantees is present). Compile-time: `agent_name` is now `Option<String>` — unwrap only in the normal branch.

- [ ] **Step 6: Gate**

Run the full gate. Expect green. Commit: `feat(run): surface run_id and add --resume/--replay flag scaffolding (OH1 Lot 6)`.

---

## Task 2: `--replay <run_id>` (deterministic re-emission)

**Files:**
- Modify: `src/cli/run.rs` (replay path)
- Create: `src/cli/run_replay.rs` (or a fn in run.rs) — `async fn replay_run(run_id, sink, human_output) -> Result<()>`
- Test: integration test — run a pattern to a log, then replay reproduces the same `RunEvent` sequence.

**Interfaces:**
- Consumes: `crate::core::orchestration::es::replay`, `SqliteLog`, the existing event→`RunEvent` projection used by the live path.

- [ ] **Step 1: Failing test**

In an ES integration test (mirror `es/blackboard.rs:2137` harness), build a `SqliteLog` on a temp DB, run a short pattern (run_id `r1`), collect the live `RunEvent`s into a `Vec` via a capturing sink; then call the new replay path for `r1` with a second capturing sink and assert the replayed `RunEvent` sequence equals the live one (modulo timing fields). Expect FAIL (replay path unimplemented).

- [ ] **Step 2: Implement `replay_run`**

Load events: `#[cfg(feature="storage")]` open `SqliteLog` via `crate::storage::init_db()`; `let events = log.events(run_id)?;`. If empty → `anyhow::bail!("no run found for id {run_id}")`. Map each `ExecutionEvent` to the display `RunEvent` using the SAME projection the live engine uses (find it — grep where `ExecutionEvent` → `RunEvent`/`sink.emit` mapping lives; reuse it, do not fork). Emit to `sink`. Execute NO effects (no provider calls). `#[cfg(not(feature="storage"))]` → `anyhow::bail!("--replay requires the 'storage' feature (event log persistence)")`.

- [ ] **Step 3: Wire + error cases**

Replace the Task 1 replay stub with `replay_run`. Add tests: unknown run_id → error; (storage-off compile path returns the storage error — covered by the `tui` (no storage) test mode compiling the bail arm).

- [ ] **Step 4: Gate + commit**

`feat(run): --replay reconstructs a finished run to the display without effects (OH1 Lot 6)`.

---

## Task 3: `--resume <run_id>` (continue an interrupted run)

**Files:**
- Modify: `src/core/orchestration/es/engine.rs` (extract shared loop; add `resume_event_sourced`)
- Modify: `src/core/orchestration/es/{direct,blackboard,ring,hierarchical}.rs` (resume entry per pattern)
- Modify: `src/cli/run.rs` (resume path: pick pattern, guard status, dispatch)
- Modify: `src/storage/queries.rs` (helper: fetch stored pattern for a run_id from `orchestration_runs`)
- Test: integration — a run interrupted before completion resumes to completion without re-invoking already-observed agents.

**Interfaces:**
- Produces: `pub async fn resume_event_sourced<D,R,L>(run_id: &str, decider: &D, effects: &R, log: &mut L) -> Result<ExecutionState>` — seeds `state = replay(run_id, log)?` (NO re-append), then runs the shared loop.

- [ ] **Step 1: Extract the shared loop**

In `engine.rs`, factor the `while state.status == RunStatus::Running { … }` body (lines ~128-185) into `async fn run_loop<D,R,L>(run_id, state: &mut ExecutionState, decider, effects, log) -> Result<()>`. `run_event_sourced` becomes: default state → append+apply `initial` → `run_loop`. Verify existing engine tests still pass (behavior identical).

- [ ] **Step 2: Add `resume_event_sourced`**

```rust
pub async fn resume_event_sourced<D, R, L>(run_id: &str, decider: &D, effects: &R, log: &mut L) -> anyhow::Result<ExecutionState>
where D: Decider, R: EffectRunner, L: EventLog {
    let mut state = replay(run_id, log)?;      // pure fold, NO re-append
    if state.status != RunStatus::Running {
        anyhow::bail!("run {run_id} is not resumable (status: {:?})", state.status);
    }
    run_loop(run_id, &mut state, decider, effects, log).await?;
    Ok(state)
}
```
Export it from `es/mod.rs` next to `run_event_sourced`/`replay`.

- [ ] **Step 3: Failing test (core)**

In `engine.rs` (or per-pattern) tests: build a log, run a decider that would take N steps but stop the process after step 1 by using a log pre-seeded with only the first events (simulate a crash: append `RunStarted` + one `AgentInvoked`/`AgentObserved` manually, leaving status `Running`). Call `resume_event_sourced` with the real decider/effects; assert it completes (`status == Completed`) and that the already-observed agent is NOT re-invoked (effects runner records invocations; assert the pre-observed one absent). Expect FAIL until wired. Also assert resuming a `Completed` log bails.

- [ ] **Step 4: Per-pattern resume entry**

Each `run_*_es` builds its decider+effects from config then calls `run_event_sourced`. Add a sibling `resume_*_es(run_id, <config/agents>, log, sink)` that builds the SAME decider+effects and calls `resume_event_sourced`. The config needed to rebuild decider/effects comes from the `ConfigSnapshot` event already in the log (Lot 5b) — reconstruct config from the replayed state/events, so resume needs no external args beyond run_id. Verify `ConfigSnapshot` carries what each pattern's decider/effects need; if a pattern needs more, note it.

- [ ] **Step 5: CLI resume path**

In `run.rs`, the resume branch: require storage (else bail as in Task 2). Open `SqliteLog`; fetch the run's pattern via the new `queries` helper (from `orchestration_runs.pattern`); `replay` to check it exists + is `Running` (else clear error); dispatch to the matching `resume_*_es`; drive the same sink/TUI as a normal orchestrated run.

- [ ] **Step 6: Gate + commit**

`feat(run): --resume continues an interrupted event-sourced run from the log (OH1 Lot 6)`.

---

## Task 4: End-to-end resume test + docs

**Files:**
- Create/modify: an integration test simulating crash→resume at the CLI level (bespoke, since the declarative `tests/e2e` harness can't inject a mid-run crash — see spec note).
- Modify: `docs/wiki/orchestration-guide.md`

**Interfaces:** consumes Tasks 1-3.

- [ ] **Step 1: Crash→resume integration test**

Bespoke Rust integration test (not the declarative yaml harness): using a temp DB + `fake-claude`, drive a hierarchical/blackboard run whose fake spec makes the process stop after partial progress (e.g. a decider/effects seam that returns early once), leaving the log with status `Running`; then invoke the resume path on the same run_id and assert it reaches `Completed` with the full expected event set, and that no already-observed agent was re-invoked. Reuse the `es/*.rs` test harness style (temp `SqliteLog`).

- [ ] **Step 2: Replay determinism test**

Assert `--replay` of a completed run emits a `RunEvent` sequence byte-identical (modulo timestamps) to the original live run (already drafted in Task 2 Step 1 — consolidate here if needed).

- [ ] **Step 3: Docs**

In `docs/wiki/orchestration-guide.md`, add a "Resume & replay" subsection: how run_id is shown, `armadai run --resume <run_id>` / `--replay <run_id>`, the `storage`-required caveat, and that no LLM effect is re-run.

- [ ] **Step 4: Gate + commit**

`test(run): crash→resume and replay-determinism integration tests + docs (OH1 Lot 6)`.

---

## Self-Review

**Spec coverage (§8 + Lot 6):** `--resume` (fold→state→continue, no effects, terminal-guard) → Task 3. `--replay` (fold→sink, no effects) → Task 2. Storage-gated with clear failure when absent → Global Constraints + Task 2/3 bail arms. run_id surfacing (prereq, not in spec but required for the feature to be usable) → Task 1. Crash→resume e2e (Lot 6 "tests bout-en-bout") → Task 4. ✅

**No-duplication invariant** (the subtle one): called out in Global Constraints and enforced by `resume_event_sourced` seeding via `replay` (pure fold) — Task 3 Step 2. The shared-loop extraction (Step 1) is behavior-preserving for the normal path (verified by existing engine tests).

**Placeholder scan:** Task 1 intentionally stubs the replay/resume branches with explicit `bail!` that Tasks 2/3 replace — each task still ends green and independently reviewable. Two items require the implementer to *locate then reuse* an existing thing rather than invent: (a) the `ExecutionEvent`→`RunEvent` display projection (Task 2 Step 2) — reuse the live one; (b) whether `ConfigSnapshot` carries enough to rebuild each pattern's decider/effects (Task 3 Step 4) — verify, and if a pattern needs more, surface it as a blocker rather than guessing. These are "find the real API" directives, not vague placeholders.

**Type consistency:** `resume`/`replay: Option<String>` threaded identically through `execute`→`run_inner`. `resume_event_sourced` mirrors `run_event_sourced`'s generics `<D,R,L>`. `RunStart.run_id: String` added once and populated at every emit site.

**Open risk flagged for execution:** Task 3 Step 4 hinges on `ConfigSnapshot` sufficiency — if a pattern's decider/effects need runtime config not in the snapshot, that pattern's resume needs the snapshot extended (a Lot-5b touch-up) before it can resume; the implementer must report this rather than silently narrow scope.
