//! Flow status: what Claude Code hooks record about sub-agents, and (later
//! tasks) where each declared flow stands.

pub mod record;
// shortcut: no caller until `armadai flow status` (task 4) wires it; drop then.
#[allow(dead_code)]
pub mod runs;
