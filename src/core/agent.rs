use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// An agent loaded from a Markdown definition file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Agent name (from H1 heading)
    pub name: String,
    /// Source file path
    pub source: PathBuf,
    /// Technical configuration
    pub metadata: AgentMetadata,
    /// System prompt sent to the model
    pub system_prompt: String,
    /// Execution instructions
    pub instructions: Option<String>,
    /// Expected output format
    pub output_format: Option<String>,
    /// Pipeline configuration
    pub pipeline: Option<PipelineConfig>,
    /// Additional context to inject
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    /// Provider name: anthropic, openai, google, cli, proxy
    pub provider: String,
    /// Model identifier (for API providers)
    pub model: Option<String>,
    /// CLI command (for cli provider)
    pub command: Option<String>,
    /// CLI arguments (for cli provider)
    pub args: Option<Vec<String>>,
    /// Sampling temperature
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Max output tokens
    pub max_tokens: Option<u32>,
    /// Execution timeout in seconds
    pub timeout: Option<u64>,
    /// Tags for filtering
    #[serde(default)]
    pub tags: Vec<String>,
    /// Supported tech stacks
    #[serde(default)]
    pub stacks: Vec<String>,
    /// Cost limit per execution (USD)
    pub cost_limit: Option<f64>,
    /// Rate limit (e.g. "10/min")
    pub rate_limit: Option<String>,
    /// Context window size override
    pub context_window: Option<u32>,
}

fn default_temperature() -> f32 {
    0.7
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Agents to chain after this one
    pub next: Vec<String>,
}

impl Agent {
    /// Load all agents from the given directory (recursively).
    pub fn load_all(agents_dir: &std::path::Path) -> anyhow::Result<Vec<Agent>> {
        let mut agents = Vec::new();
        Self::load_from_dir(agents_dir, &mut agents)?;
        agents.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(agents)
    }

    fn load_from_dir(dir: &std::path::Path, agents: &mut Vec<Agent>) -> anyhow::Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::load_from_dir(&path, agents)?;
            } else if path.extension().is_some_and(|ext| ext == "md") {
                match crate::parser::parse_agent_file(&path) {
                    Ok(agent) => agents.push(agent),
                    Err(e) => {
                        tracing::warn!("Failed to parse {}: {e}", path.display());
                    }
                }
            }
        }
        Ok(())
    }

    /// Display string for the model/command column.
    pub fn model_display(&self) -> String {
        if let Some(ref model) = self.metadata.model {
            model.clone()
        } else if let Some(ref command) = self.metadata.command {
            format!("$ {command}")
        } else {
            "-".to_string()
        }
    }

    /// Filter agents by tags (all tags must match).
    pub fn matches_tags(&self, tags: &[String]) -> bool {
        tags.iter()
            .all(|t| self.metadata.tags.iter().any(|at| at == t))
    }

    /// Filter agents by stack.
    pub fn matches_stack(&self, stack: &str) -> bool {
        self.metadata
            .stacks
            .iter()
            .any(|s| s.eq_ignore_ascii_case(stack))
    }
}
