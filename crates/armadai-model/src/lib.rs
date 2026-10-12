//! Canonical ArmadAI project model. Pure: depends on no other workspace crate.

pub mod check;
pub mod diag;
pub mod load;
pub mod types;
pub use check::{check, has_errors, validate};
pub use diag::{Diagnostic, Severity, locate, locate_key};
pub use load::*;
pub use types::*;

/// The only accepted `schema_version`.
pub const SCHEMA_VERSION: u32 = 1;
/// Supported link targets (single list; checked against `LinkTarget` in the binary).
pub const TARGETS: [&str; 4] = ["claude", "codex", "copilot", "opencode"];
