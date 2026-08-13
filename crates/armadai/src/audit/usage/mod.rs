//! Observed usage of native agentic assets, read from Claude Code transcripts.
//!
//! Mirror of `audit::reverse` in the runtime direction: `reverse` reads what a
//! project *declares*, this module reads what it actually *ran*.
pub mod discovery;
pub mod facts;
pub mod scan;

pub use facts::UsageFacts;
pub use scan::scan;
