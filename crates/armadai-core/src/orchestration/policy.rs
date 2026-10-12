//! Delegation policy mode of the legacy `orchestration:` section, read by
//! `armadai migrate` (it becomes `policy.delegation` of the canonical model).
//! The gate itself reads the canonical model (`claude_adapter::policy_gate`).

use serde::{Deserialize, Serialize};

/// How strictly the declared topology is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolicyMode {
    /// Declared topology is advisory. Default.
    #[default]
    Off,
    /// Anything not declared is refused.
    Strict,
}
