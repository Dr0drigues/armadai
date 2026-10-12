# Finding: gaveldrop's runner cannot accept a consumer-provided adapter

**From:** armadai (first real consumer of gaveldrop), 2026-07-29
**Context:** the "put armadai's e2e suite on gaveldrop" task (`gaveldrop/docs/superpowers/briefing-armadai-integration.md`).
**Severity:** blocking for the task as briefed (implement `gaveldrop::Adapter` for `Armadai`, run the 9 cases through it). Non-blocking for building + conformance-testing the armadai side.

## The defect

The task's whole premise is a **custom adapter** (`Armadai: gaveldrop::Adapter`) that claims cases carrying `setup.pattern` and invokes the fleet. gaveldrop's public runner cannot use it.

```rust
// crates/gaveldrop/src/adapters.rs
pub fn registry() -> Vec<Box<dyn Adapter>> {
    vec![Box::new(Web), Box::new(Shell), Box::new(Process)]   // hardcoded, no extension point
}

// crates/gaveldrop/src/runner.rs
pub fn run_all(config, root, fake_binary, sink) -> Result<Report, ConfigError>
pub fn run_all_selected(config, root, fake_binary, sink, shard, only) -> Result<Report, ConfigError> {
    ...
    let adapters = adapters::registry();          // ← both public entries hardcode this
    ...
    run_one(&case, fake_binary, config, root, &adapters)   // run_one takes &adapters but is PRIVATE
}
fn run_one(..., adapters: &[Box<dyn Adapter>]) -> ...   // not pub
```

- `run_all` / `run_all_selected` are the only public runner entries; both hardcode `adapters::registry()` (Process/Shell/Web).
- `run_one`, the only function that takes `&adapters`, is private.
- No `run_*_with_adapters`, no `register_adapter`, no extensible registry (exhaustive grep: nothing).
- The **conformance** kit *does* take an adapter (`gaveldrop_conformance::run_with(&Armadai, fake, factory)`), so the adapter is provable in isolation — but the **suite runner** that actually executes cases and produces a `Report` cannot use it.

**Result:** a project with its own adapter — the exact scenario this task exists to test — cannot run its own cases through gaveldrop. The two escape hatches both defeat the task:
- Rewrite cases to `run:` + a `fake.render` hook → the briefing explicitly forbids this (kills case readability, never exercises the adapter API).
- Reimplement `run_one`'s loop (`Isolation::prepare_with` → `adapter.invoke` → `verdict::evaluate` → `Report`) from the public pieces → this is precisely the "reimplementation of what gaveldrop already does" the briefing says to flag, and it re-derives discovery/selection/evaluation/report aggregation.

## The fix (to be made in gaveldrop, with its own test + invariant)

Expose adapter injection on the public runner. Minimal, additive, no behavior change for existing callers.

```rust
// crates/gaveldrop/src/runner.rs

/// Like `run_all_selected`, but the caller supplies the adapters. A project
/// with its own `Adapter` (its cases carry keys no built-in claims) runs its
/// suite through this, passing e.g. `&[Box::new(MyAdapter), ..registry()]` —
/// order is selection order, so a consumer adapter that claims a case wins
/// over a built-in that also would.
pub fn run_all_with_adapters(
    config: &Config,
    root: &Path,
    fake_binary: &Path,
    sink: &mut dyn Sink,
    shard: Option<Shard>,
    only: Option<&str>,
    adapters: &[Box<dyn Adapter>],
) -> Result<Report, ConfigError> {
    let paths = crate::config::select(config.discover(root)?, shard, only)?;
    // ... the current body of run_all_selected, using the passed `adapters`
    //     instead of `let adapters = adapters::registry();`
}

// run_all_selected becomes a thin delegate — preserves every existing caller:
pub fn run_all_selected(
    config: &Config, root: &Path, fake_binary: &Path,
    sink: &mut dyn Sink, shard: Option<Shard>, only: Option<&str>,
) -> Result<Report, ConfigError> {
    run_all_with_adapters(config, root, fake_binary, sink, shard, only, &adapters::registry())
}
```

Equivalent alternatives, less preferred: make `run_one` `pub` (forces consumers to re-derive discovery/selection/aggregation — most of run_all), or a global registration hook (mutable global state, un-Rust-like). The additive `run_all_with_adapters` is the smallest change that keeps the CLI path (`registry()`) untouched.

Consumers still get the CLI's built-in adapters by including `registry()` in the slice; the CLI (`gaveldrop` binary, `locate_fake()` → `gaveldrop-fake`) is unchanged. A consumer with a custom adapter runs its suite from a **Rust test** calling `run_all_with_adapters` with its own `fake_binary` (e.g. armadai's `fake-claude`) — which matches the "cargo test --workspace" model the briefing already assumes.

### gaveldrop's own test for the new API (its invariant)

Mirror the `shell.rs` guard: a trivial test adapter proves the injected adapter is the one selected + invoked.

```rust
// a test adapter that claims cases with an "echo" key and echoes a marker to stdout
struct Echo;
impl Adapter for Echo {
    fn claims(&self, case: &Case) -> bool { case.setup.extra.contains_key("echo") }
    fn invoke(&self, case: &Case, _iso: &Isolation) -> Result<Observations, AdapterError> {
        Ok(Observations { stdout: "ECHO-ADAPTER-RAN".into(), ..Default::default() })
    }
}

#[test]
fn a_consumer_adapter_is_used_by_the_runner() {
    // a case carrying `echo:` that no built-in claims
    // Config with cases pointing at it
    // run_all_with_adapters(..., &[Box::new(Echo)]) — NOT registry()
    // assert the Report reflects Echo's invoke (its stdout marker), proving the
    // injected adapter claimed + ran, and that a built-in did not.
}
```

Invariant this locks: *a consumer-provided adapter that claims a case is the one that invokes it, through the public runner.* Absent today; the reason armadai (and any future custom-adapter consumer) is blocked.

## What armadai builds regardless (ready to plug in the moment this lands)

- `Armadai` adapter (`impl gaveldrop::Adapter`) + its `gaveldrop_conformance::run_with` test (works today — conformance already takes an adapter).
- `fake-claude` rebuilt on `gaveldrop-fake` as a library (engine selects/counts/journals; the binary renders Claude Code's `stream-json` bytes).
- `gaveldrop.yaml` (`cases`, `events.type_field: t`, the four named invariants, `fake.bins: [claude]`) + the 9 cases (already near-identical to gaveldrop's format).
- The suite-run + deletion of the 1655-line harness (deliverables #4/#5) unblock once `run_all_with_adapters` exists.
