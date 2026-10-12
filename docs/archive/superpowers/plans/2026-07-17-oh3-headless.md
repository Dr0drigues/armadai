# OH3 — Mode headless CI-first — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rendre `armadai run` utilisable en CI : non-interactif (`--headless`), sortie structurée JSONL (`--json`) à clés courtes, exit codes exploitables, avec économie de tokens (sortie compacte + budget).

**Architecture:** Un `enum RunEvent` (serde) et un `trait EventSink` (`NullSink` par défaut = zéro coût, `JsonlSink` = écrit stdout) dans `core/events.rs`. `run::execute` construit le sink selon les flags et le passe (`Arc<dyn EventSink>`) à l'exécution agent simple, `--pipe`, et l'orchestration, qui émettent aux points agent. Aucun changement de comportement hors flags.

**Tech Stack:** Rust edition 2024, clap, serde + serde_json (déjà en deps), tokio.

## Global Constraints

- Rust edition 2024. `serde`/`serde_json` déjà présents (Cargo.toml:43-44).
- Clippy DOIT passer dans **les deux modes** : `cargo clippy --no-default-features --features tui -- -D warnings` ET `--features tui,providers-api -- -D warnings`.
- Le module `shell` est gaté `tui` ; `run`/`core` ne le sont pas — le nouveau code de `core/events.rs` et `cli/run.rs` doit compiler dans les deux modes (ne dépend d'aucune feature optionnelle).
- Conventional Commits. Branche `feat/oh3-headless` depuis `origin/release/1.0.0` ; PR vers `release/1.0.0` (jamais de push direct).
- **Clés JSONL exactes** (verbatim) : `t`, `v`, `agents`, `prov`, `model`, `in_chars`, `agent`, `tin`, `tout`, `cost`, `content`, `code`, `from`, `to`, `msg`.
- Sortie humaine → **stderr** ; flux JSONL → **stdout**. Résultat final texte (mode non-json) reste sur stdout comme aujourd'hui (`run.rs:85`).
- `record_run`/storage restent gatés `#[cfg(feature = "storage")]`.

---

### Task 1: Module `core/events.rs` — RunEvent + EventSink

**Files:**
- Create: `src/core/events.rs`
- Modify: `src/core/mod.rs` (ajouter `pub mod events;` après `pub mod embedded;`)
- Test: dans `src/core/events.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `enum RunEvent` (serde `Serialize`, tag interne `t`)
  - `trait EventSink: Send + Sync { fn emit(&self, ev: &RunEvent); }`
  - `struct NullSink;` (impl no-op)
  - `struct JsonlSink { out: std::sync::Mutex<Box<dyn std::io::Write + Send>> }` avec `JsonlSink::stdout() -> Self`
  - Constructeur helper `pub fn make_sink(json: bool) -> std::sync::Arc<dyn EventSink>`

- [ ] **Step 1: Write the failing test**

```rust
// src/core/events.rs (bas du fichier)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_start_serializes_with_short_keys() {
        let ev = RunEvent::RunStart {
            v: 1,
            agents: vec!["dev-lead".into()],
            prov: "claude".into(),
            model: "claude-x".into(),
            in_chars: 412,
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"run_start","v":1,"agents":["dev-lead"],"prov":"claude","model":"claude-x","in_chars":412}"#
        );
    }

    #[test]
    fn agent_end_serializes_with_short_keys() {
        let ev = RunEvent::AgentEnd {
            agent: "a".into(),
            tin: 10,
            tout: 20,
            cost: 0.001,
            content: "hi".into(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            s,
            r#"{"t":"agent_end","agent":"a","tin":10,"tout":20,"cost":0.001,"content":"hi"}"#
        );
    }

    #[test]
    fn jsonl_sink_writes_one_line_per_event() {
        use std::sync::{Arc, Mutex};
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = JsonlSink {
            out: Mutex::new(Box::new(SharedBuf(buf.clone()))),
        };
        sink.emit(&RunEvent::Error { code: "x".into(), msg: "y".into() });
        sink.emit(&RunEvent::Error { code: "z".into(), msg: "w".into() });
        let s = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert!(s.lines().all(|l| serde_json::from_str::<serde_json::Value>(l).is_ok()));
    }

    // Test helper: a Write that appends to a shared buffer.
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for SharedBuf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api events::`
Expected: FAIL — `RunEvent` / `JsonlSink` not found.

- [ ] **Step 3: Write minimal implementation**

```rust
// src/core/events.rs (haut du fichier)
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Structured run events emitted in headless/JSON mode. Short keys for token economy.
#[derive(Debug, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RunEvent {
    RunStart {
        v: u32,
        agents: Vec<String>,
        prov: String,
        model: String,
        in_chars: usize,
    },
    AgentStart {
        agent: String,
        prov: String,
        model: String,
    },
    AgentEnd {
        agent: String,
        tin: u32,
        tout: u32,
        cost: f64,
        content: String,
    },
    Warning {
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
    Result {
        content: String,
        tin: u32,
        tout: u32,
        cost: f64,
        agents: usize,
    },
    Error {
        code: String,
        msg: String,
    },
}

/// Sink for run events. `NullSink` is a zero-cost no-op; `JsonlSink` writes JSONL to a writer.
pub trait EventSink: Send + Sync {
    fn emit(&self, ev: &RunEvent);
}

pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _ev: &RunEvent) {}
}

pub struct JsonlSink {
    pub out: Mutex<Box<dyn std::io::Write + Send>>,
}

impl JsonlSink {
    pub fn stdout() -> Self {
        JsonlSink {
            out: Mutex::new(Box::new(std::io::stdout())),
        }
    }
}

impl EventSink for JsonlSink {
    fn emit(&self, ev: &RunEvent) {
        if let Ok(line) = serde_json::to_string(ev) {
            let mut w = self.out.lock().unwrap();
            let _ = writeln!(w, "{line}");
            let _ = w.flush();
        }
    }
}

/// Build the sink for a run: JSONL to stdout when `json`, otherwise a no-op.
pub fn make_sink(json: bool) -> Arc<dyn EventSink> {
    if json {
        Arc::new(JsonlSink::stdout())
    } else {
        Arc::new(NullSink)
    }
}
```

Add to `src/core/mod.rs` (after `pub(crate) mod embedded;`, keep alphabetical-ish order used in the file):
```rust
pub mod events;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --no-default-features --features tui,providers-api events::`
Expected: PASS (3 tests).

- [ ] **Step 5: Clippy both modes + commit**

```bash
cargo clippy --no-default-features --features tui -- -D warnings
cargo clippy --no-default-features --features tui,providers-api -- -D warnings
git add src/core/events.rs src/core/mod.rs
git commit -m "feat(core): add RunEvent + EventSink for headless JSON output"
```

---

### Task 2: CLI flags on `run`

**Files:**
- Modify: `src/cli/mod.rs:59-71` (variant `Run` — add flags) and `:435-441` (dispatch)
- Modify: `src/cli/run.rs:19-24` (signature `execute`)

**Interfaces:**
- Produces: `run::execute(agent_name, input, pipe, orchestrate, headless: bool, json: bool, quiet: bool, max_content: Option<usize>)`

- [ ] **Step 1: Add flags to the `Run` clap variant** (`src/cli/mod.rs`, inside `Run { ... }` after `orchestrate`)

```rust
        /// Non-interactive mode for CI: no prompts, CI exit codes
        #[arg(long)]
        headless: bool,
        /// Emit a JSONL event stream on stdout (implies non-interactive)
        #[arg(long)]
        json: bool,
        /// With --json: emit only the final `result` event
        #[arg(long)]
        quiet: bool,
        /// With --json: truncate `content` of intermediate events to N chars
        #[arg(long, value_name = "N")]
        max_content: Option<usize>,
```

- [ ] **Step 2: Update the dispatch** (`src/cli/mod.rs:435-441`)

```rust
        Command::Run {
            agent,
            input,
            pipe,
            orchestrate,
            headless,
            json,
            quiet,
            max_content,
        } => run::execute(agent, input, pipe, orchestrate, headless, json, quiet, max_content).await,
```

- [ ] **Step 3: Update `execute` signature** (`src/cli/run.rs:19`)

```rust
pub async fn execute(
    agent_name: String,
    input: Option<String>,
    pipe: Option<Vec<String>>,
    orchestrate: Option<String>,
    headless: bool,
    json: bool,
    quiet: bool,
    max_content: Option<usize>,
) -> anyhow::Result<()> {
    // headless is implied by json (machine output cannot be interrupted by a prompt)
    let headless = headless || json;
    let sink = crate::core::events::make_sink(json);
    // ... existing body continues (sink/quiet/max_content wired in Tasks 3-6)
```

- [ ] **Step 4: Build to verify signature wiring compiles**

Run: `cargo build --no-default-features --features tui,providers-api`
Expected: compiles (flags parsed, `sink`/`quiet`/`max_content` may be unused yet — allow with `let _ = (quiet, max_content, &sink);` temporarily at end of the added block, removed in Task 4).

- [ ] **Step 5: Commit**

```bash
git add src/cli/mod.rs src/cli/run.rs
git commit -m "feat(cli): add --headless/--json/--quiet/--max-content flags to run"
```

---

### Task 3: Non-interactivity in headless

**Files:**
- Modify: `src/cli/run.rs:321-333` (`resolve_agents_dir`) and its call site (`:25`)

**Interfaces:**
- Consumes: `headless: bool` from Task 2.
- Produces: `resolve_agents_dir(headless: bool)` — passes `interactive = false` to `auto_check_and_prompt` when headless.

- [ ] **Step 1: Thread `headless` into `resolve_agents_dir`** (`src/cli/run.rs:321`)

Current call (line 332): `crate::core::model_updater::auto_check_and_prompt(&root, !atty_is_pipe());`
Change the function to accept `headless` and compute interactivity:

```rust
fn resolve_agents_dir(headless: bool) -> AgentResolution {
    // ... existing logic to compute `root` ...
    let interactive = !headless && !atty_is_pipe();
    crate::core::model_updater::auto_check_and_prompt(&root, interactive);
    // ... rest unchanged ...
}
```

Update the call site (`src/cli/run.rs:25`): `let resolution = resolve_agents_dir(headless);`

- [ ] **Step 2: Build + manual check**

Run: `cargo build --no-default-features --features tui,providers-api`
Expected: compiles. `armadai run <agent> "x" --headless` runs without any interactive model-deprecation prompt.

- [ ] **Step 3: Commit**

```bash
git add src/cli/run.rs
git commit -m "feat(run): skip interactive model prompt in headless mode"
```

---

### Task 4: Instrument single-agent + `--pipe`

**Files:**
- Modify: `src/cli/run.rs` — `execute` (`:66-88`, sequential path) and `run_single_agent` (`:127-245`)

**Interfaces:**
- Consumes: `Arc<dyn EventSink>` (Task 1), `RunMetrics` (`run.rs:248`), `quiet`, `max_content`.
- Produces: emits `RunStart`, `AgentStart`, `AgentEnd`, `Result` around the sequential chain.

- [ ] **Step 1: Emit events in the sequential path** (`src/cli/run.rs:66-88`)

Replace the sequential block so it emits events and gates stdout on `!json`:

```rust
    use crate::core::events::RunEvent;

    // run_start (resolve provider/model of the first agent for reporting)
    sink.emit(&RunEvent::RunStart {
        v: 1,
        agents: chain.clone(),
        prov: String::new(), // filled per-agent in agent_start; kept minimal here
        model: String::new(),
        in_chars: current_input.chars().count(),
    });

    let mut current_input = current_input;
    let mut agg_tin = 0u32;
    let mut agg_tout = 0u32;
    let mut agg_cost = 0.0f64;
    let project_defaults = match &resolution {
        AgentResolution::Project { config, .. } => Some(&config.defaults),
        _ => None,
    };

    for (i, name) in chain.iter().enumerate() {
        if chain.len() > 1 && !json {
            eprintln!("--- [{}/{} {}] ---", i + 1, chain.len(), name);
        }
        let agent_path = resolve_agent_path(&resolution, name)?;
        let (output, metrics) =
            run_single_agent(&agent_path, name, &current_input, project_defaults, &sink, quiet, max_content).await?;
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
```

- [ ] **Step 2: Emit `AgentStart`/`AgentEnd` inside `run_single_agent`** (`src/cli/run.rs:127`)

Extend the signature and emit around execution:

```rust
async fn run_single_agent(
    agent_path: &Path,
    agent_name: &str,
    input: &str,
    project_defaults: Option<&ProjectDefaults>,
    sink: &std::sync::Arc<dyn crate::core::events::EventSink>,
    quiet: bool,
    max_content: Option<usize>,
) -> anyhow::Result<(String, RunMetrics)> {
    use crate::core::events::RunEvent;
    // ... existing steps 1..5 (parse, aliases, provider, request) unchanged ...

    // deprecated-model warning as an event (replaces silent alias resolution reporting)
    // (emit right after resolve_model_deprecations detects a change; see note below)

    sink.emit(&RunEvent::AgentStart {
        agent: agent_name.to_string(),
        prov: agent.metadata.provider.clone(),
        model: agent.metadata.model.clone().unwrap_or_default(),
    });

    // ... existing step 6 (execute with fallback) unchanged, producing `response` ...

    let content_out = match max_content {
        Some(n) if !quiet => response.content.chars().take(n).collect::<String>(),
        _ => response.content.clone(),
    };
    if !quiet {
        sink.emit(&RunEvent::AgentEnd {
            agent: agent_name.to_string(),
            tin: response.tokens_in,
            tout: response.tokens_out,
            cost: response.cost,
            content: content_out,
        });
    }

    // existing stderr summary: gate behind non-JSON (keep for humans)
    // wrap the eprintln!("...summary...") block in `if !sink_is_json { ... }` —
    // simplest: always keep on stderr (does not pollute stdout JSONL). Leave as-is.

    // ... build `metrics`, record_run (unchanged) ...
    Ok((response.content, metrics))
}
```

Note (deprecated-model warning): in `resolve_model_deprecations` the alias is resolved in place. To emit a `Warning`, compare `agent.metadata.model` before/after the call in `run_single_agent` and, if changed, `sink.emit(&RunEvent::Warning { code: "deprecated_model".into(), from: Some(old), to: Some(new) })`.

- [ ] **Step 3: Write an integration-style test with a mock provider** — deferred to Task 7 (needs the mock harness). Here just build.

Run: `cargo build --no-default-features --features tui,providers-api`
Expected: compiles; remove the temporary `let _ = ...` from Task 2.

- [ ] **Step 4: Manual smoke**

Run: `cargo run --no-default-features --features tui,providers-api -- run <agent> "hello" --json`
Expected on stdout: `run_start`, `agent_start`, `agent_end`, `result` JSONL lines; no plain text on stdout.

- [ ] **Step 5: Clippy both modes + commit**

```bash
cargo clippy --no-default-features --features tui -- -D warnings
cargo clippy --no-default-features --features tui,providers-api -- -D warnings
git add src/cli/run.rs
git commit -m "feat(run): emit JSONL events for single-agent and --pipe"
```

---

### Task 5: Instrument orchestration (agent-level)

**Files:**
- Modify: `src/cli/run.rs` — `run_orchestrated` (`:344-541`) + its call sites (`:41`, `:62`)

**Interfaces:**
- Consumes: `Arc<dyn EventSink>`.
- Produces: `run_orchestrated(resolution, agent_names, input, pattern, sink)` emitting `RunStart`, one `AgentStart` per agent, and a final `Result`. Per-agent `AgentEnd` token detail is OUT of scope (documented — needs engine instrumentation).

- [ ] **Step 1: Thread `sink` into `run_orchestrated`** (signature + both call sites)

```rust
async fn run_orchestrated(
    resolution: &AgentResolution,
    agent_names: &[String],
    input: &str,
    pattern: &str,
    sink: &std::sync::Arc<dyn crate::core::events::EventSink>,
) -> anyhow::Result<()> {
```
Call sites: `run.rs:41` → `return run_orchestrated(&resolution, &chain, &current_input, &pattern, &sink).await;` and `run.rs:62` similarly with `&sink`.

- [ ] **Step 2: Emit run_start + agent_start during agent loading** (inside the `for name in agent_names` loop, `run.rs:374-384`)

```rust
    use crate::core::events::RunEvent;
    sink.emit(&RunEvent::RunStart {
        v: 1,
        agents: agent_names.to_vec(),
        prov: String::new(),
        model: pattern.to_string(),
        in_chars: input.chars().count(),
    });
    for name in agent_names {
        let agent_path = resolve_agent_path(resolution, name)?;
        let mut agent = crate::parser::parse_agent_file(&agent_path)?;
        crate::linker::model_aliases::resolve_model_deprecations(
            &mut agent.metadata.model,
            &mut agent.metadata.model_fallback,
        );
        sink.emit(&RunEvent::AgentStart {
            agent: name.clone(),
            prov: agent.metadata.provider.clone(),
            model: agent.metadata.model.clone().unwrap_or_default(),
        });
        let provider = create_provider(&agent)?;
        providers.push(Arc::from(provider));
        agents.push(agent);
    }
```

- [ ] **Step 3: Emit final `Result`** — after the pattern match produces its outcome string (blackboard/ring each print an outcome). At the end of `run_orchestrated`, before `Ok(())`, capture the outcome text used for the existing stdout print and emit:

```rust
    sink.emit(&RunEvent::Result {
        content: outcome_text.clone(), // the same string currently printed for the final answer
        tin: 0, // per-agent token aggregation is future work (engine instrumentation)
        tout: 0,
        cost: 0.0,
        agents: agent_names.len(),
    });
```
Add a code comment: `// NOTE: token/cost aggregation for orchestration requires engine-level instrumentation (out of scope for beta.3).`
Gate the existing human `println!`/`eprintln!` final-answer output behind `!json` is NOT needed here because orchestration prints progress to stderr; ensure the FINAL answer currently sent to stdout is guarded: wrap that single `println!` with `if !json { println!(...) }` (pass `json` down or derive from sink type — simplest: add a `json: bool` param alongside `sink`).

- [ ] **Step 4: Build + manual smoke on a 2-agent blackboard**

Run: `cargo run --no-default-features --features tui,providers-api -- run a --pipe b --orchestrate blackboard "task" --json`
Expected: `run_start`, two `agent_start`, `result` on stdout.

- [ ] **Step 5: Clippy both modes + commit**

```bash
cargo clippy --no-default-features --features tui -- -D warnings
cargo clippy --no-default-features --features tui,providers-api -- -D warnings
git add src/cli/run.rs
git commit -m "feat(run): emit agent-level JSONL events for orchestration"
```

---

### Task 6: Exit codes + budget

**Files:**
- Create: helper in `src/cli/run.rs` — `fn exit_code_for(err: &anyhow::Error) -> i32`
- Modify: `src/cli/run.rs` `execute` (headless error path), `src/main.rs:25-37`

**Interfaces:**
- Produces: CI exit codes `0/1/2/3/4`; `budget_exceeded` → `3`, `provider_unavailable` → `4`.

- [ ] **Step 1: Write the failing test for the mapping**

```rust
// src/cli/run.rs #[cfg(test)] mod tests
#[test]
fn exit_code_mapping() {
    assert_eq!(exit_code_for(&anyhow::anyhow!("token budget exceeded")), 3);
    assert_eq!(exit_code_for(&anyhow::anyhow!("provider 'x' not available")), 4);
    assert_eq!(exit_code_for(&anyhow::anyhow!("boom")), 1);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api run::tests::exit_code_mapping`
Expected: FAIL — `exit_code_for` not defined.

- [ ] **Step 3: Implement the mapping + headless error handling**

```rust
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
```
In `execute`, wrap the run so that in headless mode an error is emitted as an `Error` event and mapped to an exit code:
```rust
    // at the points that return Err in headless mode, instead:
    if let Err(e) = result {
        if headless {
            sink.emit(&crate::core::events::RunEvent::Error {
                code: match exit_code_for(&e) { 3 => "budget_exceeded", 4 => "provider_unavailable", _ => "agent_failed" }.into(),
                msg: e.to_string(),
            });
            std::process::exit(exit_code_for(&e));
        }
        return Err(e);
    }
```
(Wrap the sequential/orchestrated calls into a `let result = ...;` so this single handler covers both.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --no-default-features --features tui,providers-api run::tests::exit_code_mapping`
Expected: PASS.

- [ ] **Step 5: Clippy both modes + commit**

```bash
cargo clippy --no-default-features --features tui -- -D warnings
cargo clippy --no-default-features --features tui,providers-api -- -D warnings
git add src/cli/run.rs src/main.rs
git commit -m "feat(run): CI exit codes and error events in headless mode"
```

---

### Task 7: Integration tests (mock provider) + non-regression

**Files:**
- Create: `tests/headless_json.rs`
- (Reuse) any existing test provider/maker; if none, drive via a stub agent whose provider is the `cli` echo path or a `MockProvider` behind `create_provider`.

**Interfaces:**
- Consumes: the CLI binary (`assert_cmd`-style) or the `execute` fn directly with a mock sink capturing events.

- [ ] **Step 1: Write the failing test — capturing sink**

```rust
// tests/headless_json.rs
// Strategy: unit-drive the sink contract by parsing emitted JSONL from a child process,
// OR (preferred, no network) test the event ordering via a CapturingSink in a small
// in-crate test. Here we assert the JSONL contract shape from a captured buffer.
#[test]
fn result_event_present_and_last() {
    // Arrange a buffer-backed JsonlSink, emit a representative sequence,
    // assert the last line is a `result` and every line parses as JSON.
    use armadai::core::events::{EventSink, JsonlSink, RunEvent};
    use std::sync::{Arc, Mutex};
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.0.lock().unwrap().extend_from_slice(b); Ok(b.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let buf = Arc::new(Mutex::new(Vec::new()));
    let sink = JsonlSink { out: Mutex::new(Box::new(Buf(buf.clone()))) };
    sink.emit(&RunEvent::RunStart { v: 1, agents: vec!["a".into()], prov: "p".into(), model: "m".into(), in_chars: 3 });
    sink.emit(&RunEvent::AgentStart { agent: "a".into(), prov: "p".into(), model: "m".into() });
    sink.emit(&RunEvent::AgentEnd { agent: "a".into(), tin: 1, tout: 2, cost: 0.0, content: "x".into() });
    sink.emit(&RunEvent::Result { content: "x".into(), tin: 1, tout: 2, cost: 0.0, agents: 1 });
    let s = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    let lines: Vec<_> = s.lines().collect();
    assert!(lines.iter().all(|l| serde_json::from_str::<serde_json::Value>(l).is_ok()));
    let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert_eq!(last["t"], "result");
}
```
(Requires `events` items to be `pub` and reachable as `armadai::core::events` — confirm the crate exposes `core` publicly; if the binary crate isn't importable, move this test into `src/core/events.rs` `#[cfg(test)]` instead.)

- [ ] **Step 2: Run to verify it fails, then passes once wired**

Run: `cargo test --no-default-features --features tui,providers-api result_event_present_and_last`
Expected: FAIL then PASS.

- [ ] **Step 3: Non-regression check**

Run: `cargo test --no-default-features --features tui,providers-api` and `cargo test --no-default-features --features tui`
Expected: all pass; existing `run` behavior (no flags) unchanged — plain output still on stdout.

- [ ] **Step 4: Clippy both modes + commit**

```bash
cargo clippy --no-default-features --features tui -- -D warnings
cargo clippy --no-default-features --features tui,providers-api -- -D warnings
git add tests/headless_json.rs
git commit -m "test(run): headless JSONL contract and non-regression"
```

---

## Notes for the implementer

- If the binary crate cannot be imported by an external `tests/` file (`armadai::core::events`), keep all event tests inside `src/core/events.rs` under `#[cfg(test)]` — do NOT invent a `pub` re-export that doesn't fit the crate layout.
- The stderr human summary in `run_single_agent` (`run.rs:220-228`) may stay as-is: it goes to stderr and never pollutes the stdout JSONL. Only the FINAL answer `println!` (stdout) must be gated behind `!json`.
- Orchestration token/cost per agent is intentionally `0` in `result` for beta.3 (documented). OH4 and later fine-grained instrumentation will fill these.
