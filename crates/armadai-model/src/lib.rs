//! Canonical ArmadAI project model. Pure: depends on no other workspace crate.

pub mod types;
pub use types::*;

/// The only accepted `schema_version`.
pub const SCHEMA_VERSION: u32 = 1;
/// Supported link targets (single list; checked against `LinkTarget` in the binary).
pub const TARGETS: [&str; 4] = ["claude", "codex", "copilot", "opencode"];
