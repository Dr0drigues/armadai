//! Event-sourcing socle for orchestration runs (OH1 Lot 1).
//!
//! The event log (`ExecutionEvent`) is the source of truth; `ExecutionState`
//! is a pure projection folded from it via `apply`/`fold`. No existing
//! orchestration engine is wired to this module yet — it is pure domain
//! plumbing for later lots to build on.

pub mod blackboard;
pub mod bridge;
pub mod direct;
pub mod engine;
pub mod event;
pub mod hierarchical;
pub mod log;
pub mod ring;
pub mod state;

// Not yet consumed by any engine (this lot only lays the socle down) — the
// parent `orchestration` module already allows `dead_code` for the same
// reason, but re-exports need their own allow since `unused_imports` is a
// distinct lint.
#[allow(unused_imports)]
pub use bridge::{SinkProjectingLog, map_execution_to_run_events, to_orchestration_result};
#[allow(unused_imports)]
pub use engine::{
    Action, Decider, EffectRunner, config_snapshot, replay, resume_event_sourced,
    run_event_sourced, run_started_roster_and_input,
};
#[allow(unused_imports)]
pub use event::ExecutionEvent;
#[cfg(feature = "storage")]
#[allow(unused_imports)]
pub use log::SqliteLog;
#[allow(unused_imports)]
pub use log::{EventLog, InMemoryLog};
#[allow(unused_imports)]
pub use state::{
    BoardEntryRec, BoardState, ContribRec, ExecutionState, HierState, RingState, RunStatus,
    VoteRec, apply, fold,
};
