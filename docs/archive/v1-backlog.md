# ArmadAI — v1.0.0 milestone backlog

Living backlog driving toward the **1.0.0** release. Scope decision (2026-07-21):
the 1.0.0 perimeter was deliberately reopened — every remaining candidate is
**in scope for a "real v1"**, delivered as progressive lots (feature branch →
PR → independent review → visual validation when user-facing). Each large bet
starts with a HARD-GATE brainstorm before any code.

Branch model: **master-only** (no `develop`). Release line: `release/1.0.0`.

## Legend
- **P0** — must ship / finishes started work · **P1** — defines the v1 product ·
  **P2** — extension, evaluate/spike.
- ✅ done (on `release/1.0.0`) · 🔲 open · 🧪 needs spike/brainstorm first.

---

## Already shipped on `release/1.0.0`
- ✅ Event-sourcing OH1 Lots 1–5 (#214–224): core, 4 patterns + nested C9, `run`
  on ES, persisted log + derived projections, deterministic e2e; legacy engines
  removed (#221).
- ✅ Provider-agnostic Workroom on the core `RunEvent` stream (#244).
- ✅ Web: full Svelte rewrite (#231→236).
- ✅ Design System: TUI (#237,239,240–243) + CLI (#246,247).
- ✅ Orchestration: C8 declarative routing, C9 pattern mixing.
- ✅ Audit engine, conversational shell, remote starters (B2), History/Costs.

---

## Epic A — Orchestration core maturity
*Finish event-sourcing and turn the engine into a real reusable core.*

| Item | Detail | Depends on | Prio | State |
|------|--------|-----------|------|-------|
| **OH1 Lot 6** | `run --resume/--replay <run_id>` (fold the log; config already in `ConfigSnapshot`) | OH1 L5 ✅ | P0 | 🔲 |
| **OH1 parallelism** | Concurrent execution with recorded order (currently sequential) | OH1 L5 ✅ | P0 | 🔲 |
| **C4** | Native tool calling (Claude `--agents`) + bidirectional stream-json | — | P1 | 🧪 |
| **OH7** | Extract reusable orchestration core as a lib (⟂ TUI/Web/CLI) + **Cargo workspace modularity** | — (prereq for OH2) | P1 | 🧪 |

## Epic B — Declarative engine *(headline feature, paradigm shift)*
*One formalism: workflows (orchestration+routing) + config-as-source-of-truth,
from which agents / `.md` / native configs become generated projections.*

| Item | Detail | Depends on | Prio | State |
|------|--------|-----------|------|-------|
| **Declarative A** | Declarative workflows unifying orchestration + routing | brainstorm | P1 | 🧪 |
| **Declarative B** | Templated config as source of truth (agents generated) | brainstorm | P1 | 🧪 |

Key questions to settle at brainstorm: AGENTS.md as input vs output only;
config format & boundary with `armadai.yaml`; workflow engine wraps vs replaces
patterns; config-explosion ergonomics (inheritance/templates/defaults).

## Epic C — Reach & integrations *(where/how agents run)*

| Item | Detail | Depends on | Prio | State |
|------|--------|-----------|------|-------|
| **Claude Code plugin** | Plugin/hooks as an adapter over the core (#227) | OH7 ideally | P1 | 🧪 |
| **OH2** | Local/remote portability (`Local`/`Remote` API) | **OH7** | P2 | 🔲 |
| **OH5** | Optional Docker sandbox runtime (feature-flag) | — | P2 | 🔲 |
| **OH6** | IDE integration via ACP protocol | ACP-maturity spike first | P2 | 🧪 |

## Epic D — Finish & docs *(public-v1 quality)*

| Item | Detail | Prio | State |
|------|--------|------|-------|
| **DS-Docs** | README + wiki + mdBook site (identity); logo/wordmark asset extracted; mdBook DS theme + GitHub Pages | P0 | 🔲 (brainstorm done) |
| **Debts** | Hermeticize flaky `test_resolve_shell_model_aliases`; zip-slip validation for remote-starter archives; UX arbitration (detail-view scroll, Costs conventions, unified quit-keys) | P1 | 🔲 |
| **Release closeout** | Refresh v0→v1 migration guide; final CHANGELOG; packaging/install (crates.io?); bump + tag `v1.0.0`; publish to `master`; finalize master-only (default branch, delete `develop`) | P0 | 🔲 |

---

## Recommended sequence
1. **Phase 1 — Finish & polish** (fast, low-risk): DS-Docs · OH1 Lot 6 · OH1 parallelism · P1 debts.
2. **Phase 2 — Core as a product** (structural): OH7 + Cargo workspace · C4.
3. **Phase 3 — Declarative engine** (biggest bet, highest risk): brainstorm A+B → spec → phased build.
4. **Phase 4 — Reach**: Claude Code plugin · OH2 (post-OH7) · OH5 · OH6 (spike).
5. **Closeout**: migration/CHANGELOG/packaging → **release 1.0.0** (master + tag + master-only finalized).

## References
- Scope decision: memory `project_scope_unfrozen_v1`.
- Design-system rollout: memory `project_design_system_rollout`.
- OpenHands study items (OH1/2/5/6/7): `docs/proposals/etude-openhands.md`, memory `project_openhands_backlog`.
- Declarative vision: memory `project_vision_declarative`.
- Workspace modularity: memory `project_modularity_workspace`.
