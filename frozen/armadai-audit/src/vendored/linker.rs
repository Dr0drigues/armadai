//! FROZEN copies of the three linker items the audit engine used, from
//! `crates/armadai/src/linker/mod.rs` and
//! `crates/armadai/src/linker/model_resolution.rs` at commit c486959. Not kept
//! in sync. (`linker::slugify` was already a re-export of
//! `armadai_core::agent::slugify`, which the audit now calls directly.)

use armadai_core::agent::Agent;
use armadai_core::model_resolution::{ModelTier, parse_latest_placeholder};

/// Check whether a model string is a `latest:*` placeholder.
///
/// From `linker/model_resolution.rs::is_latest_placeholder`.
pub fn is_latest_placeholder(model: &str) -> bool {
    parse_latest_placeholder(model).is_some()
}

/// The portable placeholder string for a tier (inverse of `parse_latest_placeholder`).
///
/// From `linker/model_resolution.rs::tier_placeholder`.
pub fn tier_placeholder(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "latest:fast",
        ModelTier::Pro => "latest:pro",
        ModelTier::Max => "latest:max",
    }
}

/// The description `link` gives an agent: the first non-blank line of its
/// system prompt, trimmed.
///
/// The `description` field of `impl From<&Agent> for LinkAgent` in
/// `linker/mod.rs`, which is the only part of `LinkAgent` the audit read.
pub fn link_description(agent: &Agent) -> Option<String> {
    agent
        .system_prompt
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
}
