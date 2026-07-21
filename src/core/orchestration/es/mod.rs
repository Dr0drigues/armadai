//! Event-sourcing socle for orchestration runs (OH1 Lot 1).
//!
//! The event log (`ExecutionEvent`) is the source of truth; `ExecutionState`
//! is a pure projection folded from it via `apply`/`fold`. No existing
//! orchestration engine is wired to this module yet — it is pure domain
//! plumbing for later lots to build on.

pub mod event;
pub mod state;

// Not yet consumed by any engine (this lot only lays the socle down) — the
// parent `orchestration` module already allows `dead_code` for the same
// reason, but re-exports need their own allow since `unused_imports` is a
// distinct lint.
#[allow(unused_imports)]
pub use event::ExecutionEvent;
#[allow(unused_imports)]
pub use state::{
    BoardEntryRec, BoardState, ContribRec, ExecutionState, HierState, RingState, RunStatus,
    VoteRec, apply, fold,
};
