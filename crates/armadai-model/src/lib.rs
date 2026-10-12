//! Canonical ArmadAI project model. Pure: depends on no other workspace crate.

pub mod check;
pub mod diag;
pub mod load;
pub mod schema;
pub mod types;
pub mod write;
pub use check::{check, has_errors, is_valid_id, load_checked, validate};
pub use diag::{Diagnostic, Severity, locate, locate_key};
pub use load::*;
pub use schema::schemas;
pub use types::*;
pub use write::*;

/// The only accepted `schema_version`.
pub const SCHEMA_VERSION: u32 = 1;
/// Supported link targets (single list; checked against `LinkTarget` in the binary).
pub const TARGETS: [&str; 4] = ["claude", "codex", "copilot", "opencode"];
