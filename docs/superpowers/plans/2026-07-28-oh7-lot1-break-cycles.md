# OH7 Lot 1 — Break the dependency cycles — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `core` and `providers` cycle-free (as `use crate::` module graphs) by relocating mislocated code, so they can later be extracted into workspace crates — without introducing the workspace yet.

**Architecture:** Five independent, pure in-crate refactors (module moves + import-path updates). No behavior change, no new types, no workspace. The existing test suite is the safety net: each task must leave the full gate green. One PR per task.

**Tech Stack:** Rust edition 2024, single binary crate `armadai`. No new dependencies.

## Global Constraints

- Target branch: `master` (master-only). Each task is its own PR off `master`.
- Reference spec: `docs/superpowers/specs/2026-07-28-oh7-workspace-design.md`.
- **Pure refactors:** no behavior change, no new/renamed public behavior. Move code and update `use` paths only. The existing tests must pass **unchanged** (only their own `use` paths may change if they reference a moved item).
- **No compatibility shims:** update every import site to the new path; do NOT leave `pub use old::path` re-export stubs behind (the point is clean boundaries). The one exception: a crate/module's own `mod.rs` may re-export moved items at the new location if that matches existing style.
- Preserve all `#[cfg(feature = "...")]` gates when moving gated code (e.g. the `providers-api` gate on API providers, the `storage` gate on `SqliteLog`).
- Gate per task: `cargo fmt --all` + clippy 3 modes (`--no-default-features --features tui` / `tui,providers-api` / `tui,web,storage`) `-D warnings` + `cargo test --no-default-features --features tui` + `cargo test --no-default-features --features tui,storage`.
- `rust-analyzer` is unreliable here (stale ABI proc-macro, E0308/E0605 false positives) — **verify at the compiler** with `cargo`.
- `cat` is aliased to `bat`; use `command cat`.
- Conventional Commits (`refactor` type), trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

## Invariant checked at end of Lot 1

The inter-module dependency matrix (grep `use crate::<module>` per top-level
module) has **no cycle involving `core` or `providers`**. Concretely, after all
five tasks:
- `core` no longer contains `use crate::linker`, `use crate::parser`, `use crate::providers`, or `use crate::storage`.
- `providers` no longer contains `use crate::shell`.

---

## Task 1a — Relocate model resolution into `core`

**Rationale:** `linker::model_resolution` + `linker::model_aliases` are model-tier
routing/aliasing logic (a core/routing concern), mislocated in `linker`. Moving
them to `core` breaks the `core → linker` edge. `linker` (and other consumers)
will import them from `core`.

**Files:**
- Move: `src/linker/model_resolution.rs` → `src/core/model_resolution.rs`
- Move: `src/linker/model_aliases.rs` → `src/core/model_aliases.rs`
- Modify: `src/linker/mod.rs` (drop `mod model_resolution; mod model_aliases;` declarations + any `pub use`)
- Modify: `src/core/mod.rs` (add `pub mod model_resolution; pub mod model_aliases;`)
- Update imports (`crate::linker::model_resolution` → `crate::core::model_resolution`, same for `model_aliases`) in:
  `src/core/model_updater.rs`, `src/core/routing.rs`,
  `src/core/orchestration/es/{blackboard,hierarchical,ring,direct}.rs`,
  `src/linker/mod.rs`, `src/tui/views/agent_detail.rs`,
  `src/shell/config.rs`, `src/shell/wizard.rs`, `src/web/api.rs`,
  `src/cli/link.rs`, `src/cli/run.rs`, `src/audit/proposal.rs`,
  `src/audit/rules/models.rs`.

- [ ] **Step 1: Confirm green baseline**

Run: `cargo test --no-default-features --features tui,storage 2>&1 | tail -3`
Expected: all pass (this is the safety net for the refactor).

- [ ] **Step 2: Move the two files**

```bash
git mv src/linker/model_resolution.rs src/core/model_resolution.rs
git mv src/linker/model_aliases.rs src/core/model_aliases.rs
```

- [ ] **Step 3: Update module declarations**

In `src/core/mod.rs`, add near the other `pub mod` lines:
```rust
pub mod model_aliases;
pub mod model_resolution;
```
In `src/linker/mod.rs`, remove the `mod model_resolution;` / `mod model_aliases;` (and any `pub use model_resolution::...` / `model_aliases::...`) declarations.

- [ ] **Step 4: Update all import sites**

In each file listed under **Files → Update imports**, replace
`crate::linker::model_resolution` → `crate::core::model_resolution` and
`crate::linker::model_aliases` → `crate::core::model_aliases`. If a moved file
referenced its sibling as `super::` or `crate::linker::...`, fix those too
(inside the moved files themselves, `super::` now means `core`).

Find every remaining stale path:
```bash
grep -rn 'linker::model_resolution\|linker::model_aliases' src
```
Expected after edits: no matches.

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

- [ ] **Step 6: Verify the cycle edge is gone**

Run: `grep -rn 'use crate::linker' src/core || echo "core no longer imports linker (for these) — OK"`
Expected: no `model_resolution`/`model_aliases` imports from linker remain in core. (Other `core → linker` edges are addressed by their own tasks if any; per the matrix, model_resolution/aliases were the only ones.)

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(oh7): move model_resolution/aliases from linker to core (#252)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 1b — Relocate `json_runner` into `providers`

**Rationale:** `shell::json_runner` parses provider CLIs' `stream-json` output — a
provider concern, mislocated in `shell`. Moving it to `providers` breaks the
`providers → shell` edge. `shell` (which also parses relayed CLI output) will
import it from `providers`.

**Files:**
- Move: `src/shell/json_runner.rs` → `src/providers/json_runner.rs`
- Modify: `src/shell/mod.rs` (drop `mod json_runner;` / `pub use json_runner::...`)
- Modify: `src/providers/mod.rs` (add `pub mod json_runner;`)
- Update imports (`crate::shell::json_runner` → `crate::providers::json_runner`) in:
  `src/bin/fake-claude.rs`, `src/shell/detect.rs`, `src/shell/app.rs`,
  `src/providers/factory.rs`, `src/providers/cli.rs`, and `src/shell/mod.rs`.

- [ ] **Step 1: Move the file**

```bash
git mv src/shell/json_runner.rs src/providers/json_runner.rs
```

- [ ] **Step 2: Update module declarations**

`src/providers/mod.rs`: add `pub mod json_runner;`.
`src/shell/mod.rs`: remove `mod json_runner;` and repoint any `pub use json_runner::...` to `pub use crate::providers::json_runner::...` if such a re-export exists and is used; otherwise drop it.

- [ ] **Step 3: Update all import sites**

Replace `crate::shell::json_runner` → `crate::providers::json_runner` in each listed file. Inside the moved file, fix any `super::`/`crate::shell::` self-references.

Find stragglers:
```bash
grep -rn 'shell::json_runner' src
```
Expected after edits: no matches.

- [ ] **Step 4: Run the gate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage
```
Expected: all green. (Note: `json_runner` must stay non-feature-gated — `cli.rs` and `factory.rs` use it in every mode; confirm the `tui`-only clippy build passes.)

- [ ] **Step 5: Verify the edge is gone**

Run: `grep -rn 'use crate::shell' src/providers || echo "providers no longer imports shell — OK"`
Expected: no matches.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(oh7): move json_runner from shell to providers (#252)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 1c — Fold `parser` into `core::parser`

**Rationale:** Agent-file parsing is a domain concern; `core ↔ parser` is a
bidirectional cycle today. Moving `parser` under `core::parser` makes it an
intra-`core` submodule (cycle becomes internal to one module tree, fine for a
single crate and, later, a single `armadai-core` crate).

**Files:**
- Move: `src/parser/` → `src/core/parser/` (`frontmatter.rs`, `markdown.rs`, `metadata.rs`, `mod.rs`)
- Modify: `src/main.rs` or crate root (remove top-level `mod parser;`)
- Modify: `src/core/mod.rs` (add `pub mod parser;`)
- Update imports (`crate::parser` → `crate::core::parser`) in all 19 consumers:
  `src/core/{pack_validation,skill,model_updater,prompt,agent}.rs`,
  `src/tui/app.rs`, `src/web/api.rs`, `src/shell/{app,wizard}.rs`,
  `src/cli/{list,link,inspect,run,new,unlink}.rs`, `src/audit/proposal.rs`,
  `src/skills_registry/cache.rs`, `src/audit/reverse/claude.rs`,
  `src/registry/convert.rs`.

- [ ] **Step 1: Move the directory**

```bash
git mv src/parser src/core/parser
```

- [ ] **Step 2: Update module declarations**

Remove the crate-root `mod parser;` (in `src/main.rs` — grep to confirm where the top-level module is declared: `grep -rn '^mod parser;\|^pub mod parser;' src/main.rs`). Add `pub mod parser;` to `src/core/mod.rs`.

- [ ] **Step 3: Update all import sites**

Replace `crate::parser` → `crate::core::parser` in the 19 consumers. Inside the moved parser files, any `crate::core::...` references stay valid; any `super::` self-references within the parser tree stay valid (still a self-contained tree, now under `core`).

Find stragglers:
```bash
grep -rn 'crate::parser\b' src
```
Expected after edits: no matches (all now `crate::core::parser`).

- [ ] **Step 4: Run the gate** (same 6 commands as Task 1a Step 5) — all green.

- [ ] **Step 5: Verify**

Run: `grep -rn 'use crate::parser\b' src/core || echo "core parser is now intra-core — OK"`
Expected: `core` references `crate::core::parser`, no top-level `crate::parser`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(oh7): fold parser into core::parser (#252)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 1d — Move the `Provider` trait + request/response types into `core`

**Rationale:** Dependency inversion. The `Provider` trait and its request/response
types are the contract the orchestration engine (core) calls; `providers` should
IMPLEMENT them and depend on core, not the reverse. Moving `providers::traits`
into `core` breaks the `core → providers` edge (core owns the trait; providers
implements it).

**Files:**
- Move: `src/providers/traits.rs` → `src/core/provider.rs` (contains `TokenStream`, `CompletionRequest`, `ChatMessage`, `CompletionResponse`, `ProviderMetadata`, `Provider`)
- Modify: `src/providers/mod.rs` (remove `mod traits;` / `pub use traits::...`)
- Modify: `src/core/mod.rs` (add `pub mod provider;`)
- Update imports (`crate::providers::traits::X` → `crate::core::provider::X`) in:
  `src/core/orchestration/es/{hierarchical,direct,blackboard,ring,state}.rs`,
  `src/core/orchestration/test_helpers.rs`,
  `src/providers/{proxy,cli,rate_limiter,factory}.rs`,
  `src/providers/api/{openai,google,anthropic}.rs`,
  `src/cli/{audit,run}.rs`.

**Note on `ChatMessage`:** it currently lives in `providers::traits` and is
already used by `core::orchestration::es::state`. Moving it to `core::provider`
resolves that latent cross-dependency too.

- [ ] **Step 1: Move the file**

```bash
git mv src/providers/traits.rs src/core/provider.rs
```

- [ ] **Step 2: Update module declarations**

`src/core/mod.rs`: add `pub mod provider;`.
`src/providers/mod.rs`: remove `mod traits;` and any `pub use traits::...`. If callers used `crate::providers::traits::Provider` via a `providers`-level re-export, they are updated in Step 3 to the core path.

- [ ] **Step 3: Update all import sites**

Replace `crate::providers::traits::` → `crate::core::provider::` in every listed file (and any `use crate::providers::traits;` module import → `use crate::core::provider;`). Inside `core/provider.rs` itself, ensure it references only `core`/external crates (it should already — it's a leaf contract).

Find stragglers:
```bash
grep -rn 'providers::traits' src
```
Expected after edits: no matches.

- [ ] **Step 4: Run the gate** (6 commands) — all green. (The API providers under `src/providers/api/*` are `providers-api`-gated; confirm the `tui,providers-api` clippy build passes.)

- [ ] **Step 5: Verify the edge is gone**

Run: `grep -rn 'use crate::providers' src/core || echo "core no longer imports providers — OK"`
Expected: no matches (core owns the trait; the ES engine calls `dyn core::provider::Provider`).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(oh7): move Provider trait into core (dependency inversion) (#252)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 1e — Move `SqliteLog` out of `core` into the bin

**Rationale:** `EventLog` is a storage-agnostic trait and `InMemoryLog` is
always-on — both stay in `core`. `SqliteLog` is the single piece coupling `core`
to `rusqlite`/`crate::storage`; moving it to the bin (implementing
`core::EventLog`) makes `core` a true storage-free leaf.

**Files:**
- Modify: `src/core/orchestration/es/log.rs` (remove the `#[cfg(feature = "storage")] SqliteLog` struct + its `EventLog` impl + the `#[cfg(feature="storage")]` test; keep `EventLog` trait + `InMemoryLog` + its tests)
- Create: a bin-side module for `SqliteLog`, e.g. `src/storage/es_log.rs` (the bin's `storage` module is a natural home — it already owns `rusqlite` and `crate::storage::Database`/`schema`). Add `#[cfg(feature = "storage")] pub mod es_log;` to `src/storage/mod.rs`.
- Update imports of `SqliteLog` (`crate::core::orchestration::es::log::SqliteLog` or the `es::mod` re-export → `crate::storage::es_log::SqliteLog`) in:
  `src/core/orchestration/es/mod.rs` (remove its `pub use ...SqliteLog` re-export),
  `src/cli/{run_replay,run,projections,run_es_record}.rs`.

**`SqliteLog` body to relocate** (from `es/log.rs`, currently ~lines 55-100, all under `#[cfg(feature = "storage")]`):
```rust
pub struct SqliteLog {
    db: crate::storage::Database,
}
impl SqliteLog {
    pub fn new(db: crate::storage::Database) -> Self { Self { db } }
}
impl crate::core::orchestration::es::log::EventLog for SqliteLog {
    // append(...) / events(...) bodies moved verbatim (rusqlite::params!, schema v3)
}
```
(Move the exact existing bodies; only the containing module and the `EventLog`
path change — `EventLog` is now referenced via its `core` path.)

- [ ] **Step 1: Read the current `SqliteLog` block**

Run: `command cat src/core/orchestration/es/log.rs`
Note the exact `SqliteLog` struct + `impl EventLog for SqliteLog` bodies and the `#[cfg(feature = "storage")]` test, to move them verbatim.

- [ ] **Step 2: Create the bin-side module**

Create `src/storage/es_log.rs` with the relocated `SqliteLog` (struct + `EventLog` impl, referencing `crate::core::orchestration::es::log::EventLog`), preserving the schema-v3 SQL verbatim. Move the `#[cfg(feature="storage")]` SqliteLog test here too.
Add to `src/storage/mod.rs`: `#[cfg(feature = "storage")] pub mod es_log;`.

- [ ] **Step 3: Remove `SqliteLog` from core**

In `src/core/orchestration/es/log.rs`, delete the `SqliteLog` struct, its `impl`, and its test — keep the `EventLog` trait, `InMemoryLog`, and the `InMemoryLog` tests. Remove now-unused `#[cfg(feature = "storage")]` blocks and any `use crate::storage::...` in this file.
In `src/core/orchestration/es/mod.rs`, remove the `pub use ...log::SqliteLog` re-export.

- [ ] **Step 4: Update `SqliteLog` import sites**

In `src/cli/{run_replay,run,projections,run_es_record}.rs`, repoint `SqliteLog` imports to `crate::storage::es_log::SqliteLog`.

Find stragglers:
```bash
grep -rn 'SqliteLog' src/core
```
Expected: no matches (SqliteLog no longer in core).

- [ ] **Step 5: Verify core is storage-free**

Run: `grep -rn 'rusqlite\|use crate::storage' src/core || echo "core is storage-free — OK"`
Expected: no matches in `src/core`.

- [ ] **Step 6: Run the gate** (6 commands) — all green. **Critical:** the
`tui,storage` test mode must pass (it exercises `run --resume/--replay` through
`SqliteLog`, OH1 Lot 6 e2e) — confirm those tests still pass after relocation.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(oh7): move SqliteLog out of core into the storage-backed bin (#252)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage:** The five relocations in the spec's Lot 1 table map 1:1 to
Tasks 1a (model_resolution→core), 1b (json_runner→providers), 1c (parser→core),
1d (Provider trait→core), 1e (SqliteLog→bin). ✅ The end-of-Lot-1 invariant
(core/providers cycle-free) is verified per task (Steps "Verify the edge is
gone") and holistically restated at the top. ✅

**2. Placeholder scan:** No TBD/TODO. Each task lists exact file moves + the full
import-site list to update + the straggler-grep to confirm completeness. Import
sites are enumerated from the actual `grep` sweep. ✅

**3. Consistency:** All tasks share the identical 6-command gate and the
`no-shims` / preserve-cfg constraints. `ChatMessage`'s move (1d) is flagged
because `es::state` already consumes it. `json_runner` non-gated note (1b) and
`storage` test-mode criticality (1e) are called out. ✅

**4. Ambiguity:** "No compatibility shims" is explicit (update sites, don't leave
re-export stubs). The bin-side home for `SqliteLog` is pinned to
`src/storage/es_log.rs`. Task order (1a→1e) is independent moves; any order
works, but the listed order matches the spec. ✅
