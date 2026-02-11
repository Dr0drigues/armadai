mod markdown;
mod metadata;

use std::path::Path;

use crate::core::agent::Agent;

pub use markdown::parse_agent_file;

/// Validate an agent definition without loading providers.
#[allow(dead_code)]
pub fn validate_agent(path: &Path) -> anyhow::Result<Agent> {
    let agent = parse_agent_file(path)?;
    metadata::validate_metadata(&agent.metadata)?;
    Ok(agent)
}
