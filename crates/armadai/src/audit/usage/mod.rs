//! Observed usage of native agentic assets, read from Claude Code transcripts.
//!
//! Mirror of `audit::reverse` in the runtime direction: `reverse` reads what a
//! project *declares*, this module reads what it actually *ran*.
pub mod discovery;
pub mod facts;
pub mod scan;

// Not yet consumed by any CLI surface (a later task wires `scan` into
// `armadai audit`) — `dead_code` is already allowed per-item on `scan` and
// `UsageFacts` themselves; re-exports need their own allow since
// `unused_imports` is a distinct lint.
#[allow(unused_imports)]
pub use facts::UsageFacts;
#[allow(unused_imports)]
pub use scan::scan;
