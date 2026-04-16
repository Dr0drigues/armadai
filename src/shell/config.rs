//! Shell configuration — parsed from the `shell:` section of armadai.yaml.

use serde::Deserialize;

use crate::linker::model_resolution::{
    ModelTier, fallback_model_for_tier, parse_latest_placeholder, resolve_model_for_tier,
};

/// Shell configuration section from armadai.yaml.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ShellConfig {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub timeout: Option<u64>,
    pub max_history: Option<usize>,
    pub auto_save: Option<bool>,
    pub tandem: Vec<ShellProviderEntry>,
    pub pipeline: Option<ShellPipelineConfig>,
}

/// A single entry in tandem or pipeline stages.
///
/// Two usage modes:
/// - **Provider mode**: `provider:` + optional `model:` — invokes the CLI directly
///   with a system prompt defined at the step level
/// - **Agent mode**: `agent:` — loads a project agent (from `agents:` list), uses its
///   system prompt, metadata provider/model, and adds the step's `prompt:` as extra context
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ShellProviderEntry {
    #[serde(default)]
    pub provider: String,
    pub model: Option<String>,
    /// Agent name (from the project's `agents:` list) — when set, loads the agent's
    /// system prompt and metadata. Takes precedence over `provider`/`model` for config
    /// (the agent's metadata defines the actual provider used).
    pub agent: Option<String>,
}

/// Pipeline configuration with named, ordered steps.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ShellPipelineConfig {
    pub steps: Vec<PipelineStep>,
}

/// A single pipeline step with a name, role prompt, and providers.
#[derive(Debug, Clone, Deserialize)]
pub struct PipelineStep {
    pub name: String,
    pub prompt: Option<String>,
    pub providers: Vec<ShellProviderEntry>,
}

impl ShellConfig {
    pub fn effective_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout.unwrap_or(120))
    }

    pub fn effective_max_history(&self) -> usize {
        self.max_history.unwrap_or(5)
    }

    pub fn effective_auto_save(&self) -> bool {
        self.auto_save.unwrap_or(true)
    }
}

// ── Model resolution for shell providers ────────────────────────

/// Map a shell provider name to the linker provider identifier.
fn shell_provider_to_linker(provider: &str) -> &str {
    match provider {
        "gemini" => "google",
        "claude" => "anthropic",
        "aider" | "codex" => "openai",
        _ => "anthropic",
    }
}

/// Resolve a model string (which may be a `latest:*` placeholder) for a shell provider.
pub fn resolve_shell_model(provider: &str, model: &str) -> String {
    let linker_provider = shell_provider_to_linker(provider);
    if let Some(tier) = parse_latest_placeholder(model) {
        resolve_model_for_tier(linker_provider, tier)
    } else {
        model.to_string()
    }
}

/// Get the default model for a provider (Pro tier).
pub fn default_model_for_provider(provider: &str) -> String {
    let linker_provider = shell_provider_to_linker(provider);
    fallback_model_for_tier(linker_provider, ModelTier::Pro).to_string()
}

/// Build CLI model flags for a provider, if the CLI supports it.
/// Returns additional args to insert before the prompt.
pub fn model_cli_args(provider: &str, model: &str) -> Vec<String> {
    match provider {
        "claude" => vec!["--model".to_string(), model.to_string()],
        "aider" => vec!["--model".to_string(), model.to_string()],
        // gemini: model selection via env var or settings.json, not CLI flag
        // codex: similar
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_shell_model_tier() {
        let model = resolve_shell_model("gemini", "latest:fast");
        assert!(!model.is_empty());
        assert!(!model.starts_with("latest"));
    }

    #[test]
    fn test_resolve_shell_model_concrete() {
        let model = resolve_shell_model("gemini", "gemini-2.5-flash");
        assert_eq!(model, "gemini-2.5-flash");
    }

    #[test]
    fn test_resolve_shell_model_aliases() {
        let low = resolve_shell_model("claude", "latest:low");
        let fast = resolve_shell_model("claude", "latest:fast");
        assert_eq!(low, fast);

        let high = resolve_shell_model("claude", "latest:high");
        let max = resolve_shell_model("claude", "latest:max");
        assert_eq!(high, max);
    }

    #[test]
    fn test_model_cli_args_claude() {
        let args = model_cli_args("claude", "claude-sonnet-4-5");
        assert_eq!(args, vec!["--model", "claude-sonnet-4-5"]);
    }

    #[test]
    fn test_model_cli_args_gemini_empty() {
        let args = model_cli_args("gemini", "gemini-2.5-flash");
        assert!(args.is_empty());
    }

    #[test]
    fn test_default_model_for_provider() {
        let model = default_model_for_provider("gemini");
        assert!(!model.is_empty());
    }

    #[test]
    fn test_shell_config_defaults() {
        let config = ShellConfig::default();
        assert_eq!(config.effective_timeout().as_secs(), 120);
        assert_eq!(config.effective_max_history(), 5);
        assert!(config.effective_auto_save());
    }

    #[test]
    fn test_pipeline_step_with_agent() {
        let yaml = r#"
pipeline:
  steps:
    - name: plan
      prompt: "Context"
      providers:
        - agent: architect
    - name: review
      providers:
        - agent: reviewer
"#;
        let config: ShellConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let pipeline = config.pipeline.unwrap();
        assert_eq!(pipeline.steps.len(), 2);
        assert_eq!(
            pipeline.steps[0].providers[0].agent,
            Some("architect".to_string())
        );
        assert_eq!(pipeline.steps[0].providers[0].provider, "");
        assert_eq!(
            pipeline.steps[1].providers[0].agent,
            Some("reviewer".to_string())
        );
    }

    #[test]
    fn test_shell_config_deserialize() {
        let yaml = r#"
default_provider: gemini
default_model: latest:pro
timeout: 60
max_history: 20
tandem:
  - provider: gemini
    model: latest:fast
  - provider: claude
    model: latest:pro
pipeline:
  steps:
    - name: analyze
      prompt: "Analyze this"
      providers:
        - provider: gemini
          model: latest:fast
    - name: generate
      prompt: "Generate a solution"
      providers:
        - provider: claude
          model: latest:max
"#;
        let config: ShellConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.default_provider, Some("gemini".to_string()));
        assert_eq!(config.default_model, Some("latest:pro".to_string()));
        assert_eq!(config.effective_timeout().as_secs(), 60);
        assert_eq!(config.effective_max_history(), 20);
        assert_eq!(config.tandem.len(), 2);
        let pipeline = config.pipeline.unwrap();
        assert_eq!(pipeline.steps.len(), 2);
        assert_eq!(pipeline.steps[0].name, "analyze");
        assert_eq!(
            pipeline.steps[1].providers[0].model,
            Some("latest:max".to_string())
        );
    }
}
