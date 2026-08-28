//! Shell configuration — parsed from the `shell:` section of armadai.yaml.
//!
//! The data model (`ShellConfig` and friends) lives in
//! `armadai_core::project` (it's part of the `armadai.yaml` schema); this
//! module re-exports it and adds the model-resolution helpers below, which
//! reach into `crate::linker` and so cannot live in core.

use crate::linker::model_resolution::parse_latest_placeholder;
use armadai_core::model_resolution::{
    ModelTier, fallback_model_for_tier, model_catalog_provider, resolve_routed_tier,
};
pub use armadai_core::project::{PipelineStep, ShellConfig, ShellProviderEntry};

// ── Model resolution for shell providers ────────────────────────

/// Resolve a model string (which may be a `latest:*` placeholder) for a shell provider.
///
/// The tool-name → vendor-catalog mapping this used to carry privately
/// (`shell_provider_to_linker`) now lives in
/// [`armadai_core::model_resolution::model_catalog_provider`], shared with
/// `armadai run` and `armadai link`. It was the only correct copy of it in
/// the tree, which is how `armadai shell` and `armadai run` came to resolve
/// the same agent file's `latest:pro` to two different vendors' models
/// (#398 review, F1).
pub fn resolve_shell_model(provider: &str, model: &str) -> String {
    match parse_latest_placeholder(model) {
        Some(tier) => resolve_routed_tier(provider, tier),
        None => model.to_string(),
    }
}

/// Get the default model for a provider (Pro tier).
pub fn default_model_for_provider(provider: &str) -> String {
    let catalog = model_catalog_provider(provider).unwrap_or(provider);
    fallback_model_for_tier(catalog, ModelTier::Pro).to_string()
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
        // Hermetic: force an empty, private `ARMADAI_CONFIG_DIR` so this test
        // never sees the ambient/machine-local models.dev cache. Without this,
        // `resolve_shell_model` → `resolve_model_for_tier` reads the shared,
        // mutable cache file via `load_models_cached`; under the parallel test
        // suite other tests populate/clear/refresh that cache between the two
        // calls below, so the two resolutions could observe different cache
        // states and diverge — flaky (passes in isolation / on a clean CI
        // runner, fails under `--test-threads>1`). With no cache reachable,
        // resolution always takes the deterministic hardcoded-fallback path
        // (`fallback_model_for_tier`), while still exercising the real
        // low/fast → Fast tier and high/max → Max tier collapsing invariant
        // this test is meant to guard. Mirrors
        // `armadai_core::model_resolution::test_preview_resolution_with_latest`.
        let _guard = armadai_core::test_support::env_lock();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: env mutation is serialised via `env_lock()` for the duration
        // of this test, and the original value is restored before returning.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", tmp.path());
        }

        let low = resolve_shell_model("claude", "latest:low");
        let fast = resolve_shell_model("claude", "latest:fast");
        let high = resolve_shell_model("claude", "latest:high");
        let max = resolve_shell_model("claude", "latest:max");

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }

        assert_eq!(low, fast);
        assert_eq!(high, max);
    }

    /// `armadai shell` and `armadai run` must resolve one agent file's
    /// placeholder to one model.
    ///
    /// They did not: shell carried the tool → vendor table (privately) and
    /// run did not, so `provider: gemini` + `latest:pro` gave
    /// `gemini-2.5-pro` under `shell` and `claude-sonnet-4-5-20250929` under
    /// `run` (#398 review, F1). Both now read
    /// `model_catalog_provider`; this pins the two together so a future
    /// second table cannot re-open the gap silently.
    #[test]
    fn shell_and_run_resolve_a_placeholder_to_the_same_model() {
        let _iso = armadai_core::test_support::IsolatedConfigDir::enter();
        for provider in ["claude", "gemini", "aider", "codex", "gpt", "anthropic"] {
            for placeholder in ["latest", "latest:fast", "latest:pro", "latest:max"] {
                let via_run =
                    armadai_core::model_resolution::resolve_tier_placeholder(placeholder, provider)
                        .unwrap_or_else(|| panic!("{provider} should name a vendor"));
                assert_eq!(
                    resolve_shell_model(provider, placeholder),
                    via_run,
                    "{provider} + {placeholder}: shell and run disagree"
                );
            }
        }
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
