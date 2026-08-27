use super::LinkAgent;
use armadai_core::model_resolution::{ModelTier, fallback_model_for_tier, resolve_model_for_tier};

/// Classification of link targets.
pub enum TargetKind {
    /// Target is a standalone LLM editor that speaks a specific provider's API.
    LlmEditor { provider: &'static str },
    /// Target is an orchestrator that can use any model (needs explicit --model).
    Orchestrator,
}

/// Classify a link target name into its kind.
pub fn classify_target(target: &str) -> TargetKind {
    match target {
        "claude" => TargetKind::LlmEditor {
            provider: "anthropic",
        },
        "gemini" => TargetKind::LlmEditor { provider: "google" },
        "codex" => TargetKind::LlmEditor { provider: "openai" },
        // copilot, opencode, etc.
        _ => TargetKind::Orchestrator,
    }
}

/// Parse a `latest` placeholder into a tier.
///
/// Re-exported from core, where it moved in #376 so the `armadai run` path
/// resolves the exact same tier table this linker does — an alias that
/// `link` honours and `run` sends verbatim to the provider was that issue.
pub use armadai_core::model_resolution::parse_latest_placeholder;

/// Check whether a model string is a `latest:*` placeholder.
pub fn is_latest_placeholder(model: &str) -> bool {
    parse_latest_placeholder(model).is_some()
}

/// The portable placeholder string for a tier (inverse of `parse_latest_placeholder`).
pub(crate) fn tier_placeholder(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "latest:fast",
        ModelTier::Pro => "latest:pro",
        ModelTier::Max => "latest:max",
    }
}

/// Hardcoded fallback model for a given provider (defaults to Pro tier).
#[allow(dead_code)]
pub fn fallback_model(provider: &str) -> &'static str {
    fallback_model_for_tier(provider, ModelTier::Pro)
}

/// Resolve the best model for a provider (defaults to Pro tier).
#[allow(dead_code)]
pub fn resolve_best_model_cached(provider: &str) -> String {
    resolve_model_for_tier(provider, ModelTier::Pro)
}

// ── Remap functions ──────────────────────────────────────────────

/// Remap all agents' models for an LLM editor target.
///
/// Agents with `latest:*` placeholders get tier-specific resolution.
/// Agents with concrete models get remapped to the target provider's Pro tier.
#[cfg(feature = "providers-api")]
pub async fn remap_models_for_llm_editor(agents: &mut [LinkAgent], provider: &str) {
    for agent in agents.iter_mut() {
        let tier = agent
            .model
            .as_deref()
            .and_then(parse_latest_placeholder)
            .unwrap_or(ModelTier::Pro);
        agent.model = Some(resolve_model_for_tier(provider, tier));
    }
}

/// Remap all agents' models for an LLM editor target (sync/cache-only).
#[cfg(not(feature = "providers-api"))]
pub fn remap_models_for_llm_editor(agents: &mut [LinkAgent], provider: &str) {
    for agent in agents.iter_mut() {
        let tier = agent
            .model
            .as_deref()
            .and_then(parse_latest_placeholder)
            .unwrap_or(ModelTier::Pro);
        agent.model = Some(resolve_model_for_tier(provider, tier));
    }
}

/// Remap all agents' models to a specific model (for orchestrator targets).
pub fn remap_models_for_orchestrator(agents: &mut [LinkAgent], model: &str) {
    for agent in agents.iter_mut() {
        agent.model = Some(model.to_string());
    }
}

/// Resolve `latest:*` placeholders in agents using each agent's own provider.
///
/// Used for orchestrator targets where no single provider is imposed.
/// Agents without a `latest:*` placeholder are left unchanged.
pub fn resolve_latest_placeholders(agents: &mut [LinkAgent]) {
    for agent in agents.iter_mut() {
        if let Some(ref model) = agent.model
            && let Some(tier) = parse_latest_placeholder(model)
        {
            let provider = agent.provider.as_deref().unwrap_or("anthropic");
            agent.model = Some(resolve_model_for_tier(provider, tier));
        }
    }
}

/// Preview model resolution for all known link targets (sync, always available).
///
/// Returns a list of (target_name, resolved_model) tuples showing what model
/// would be used when linking to each target.
pub fn preview_model_resolution(agent_model: Option<&str>) -> Vec<(&'static str, String)> {
    use clap::ValueEnum;
    let tier = agent_model.and_then(parse_latest_placeholder);
    // Derived from the enum rather than restated: a preview that silently
    // omits a link target is a UI that lies about what `link` will do.
    super::LinkTarget::value_variants()
        .iter()
        .map(|t| t.as_str())
        .map(|target| {
            let resolved = match classify_target(target) {
                TargetKind::LlmEditor { provider } => {
                    resolve_model_for_tier(provider, tier.unwrap_or(ModelTier::Pro))
                }
                TargetKind::Orchestrator => {
                    if let Some(t) = tier {
                        // Resolve against anthropic as default for preview
                        resolve_model_for_tier("anthropic", t)
                    } else {
                        agent_model.unwrap_or("(requires --model)").to_string()
                    }
                }
            };
            (target, resolved)
        })
        .collect()
}

/// Prompt the user interactively to pick a provider and model.
///
/// Used for orchestrator targets (copilot, opencode) when no `--model` flag is given.
#[cfg(feature = "providers-api")]
pub async fn prompt_model_interactive() -> anyhow::Result<String> {
    use dialoguer::Select;

    let providers = &["anthropic", "google", "openai"];
    let idx = Select::new()
        .with_prompt("Provider for model selection")
        .items(providers)
        .default(0)
        .interact()?;
    let provider = providers[idx];

    if let Some(entries) =
        armadai_providers::model_registry::fetch::load_models_online(provider).await
        && !entries.is_empty()
    {
        let labels: Vec<String> = entries.iter().map(|e| e.display_label()).collect();
        let mut items = labels;
        items.push("(custom)".to_string());

        let model_idx = Select::new()
            .with_prompt("Model")
            .items(&items)
            .default(0)
            .interact()?;

        if model_idx == items.len() - 1 {
            let model: String = dialoguer::Input::new()
                .with_prompt("Custom model name")
                .interact_text()?;
            return Ok(model);
        }
        return Ok(entries[model_idx].id.clone());
    }

    let model: String = dialoguer::Input::new()
        .with_prompt("Model name")
        .interact_text()?;
    Ok(model)
}

/// Prompt the user interactively to pick a provider and model (cache-only, sync).
#[cfg(not(feature = "providers-api"))]
pub fn prompt_model_interactive() -> anyhow::Result<String> {
    use dialoguer::Select;

    let providers = &["anthropic", "google", "openai"];
    let idx = Select::new()
        .with_prompt("Provider for model selection")
        .items(providers)
        .default(0)
        .interact()?;
    let provider = providers[idx];

    if let Some(entries) = armadai_providers::model_registry::fetch::load_models(provider)
        && !entries.is_empty()
    {
        let labels: Vec<String> = entries.iter().map(|e| e.display_label()).collect();
        let mut items = labels;
        items.push("(custom)".to_string());

        let model_idx = Select::new()
            .with_prompt("Model")
            .items(&items)
            .default(0)
            .interact()?;

        if model_idx == items.len() - 1 {
            let model: String = dialoguer::Input::new()
                .with_prompt("Custom model name")
                .interact_text()?;
            return Ok(model);
        }
        return Ok(entries[model_idx].id.clone());
    }

    let model: String = dialoguer::Input::new()
        .with_prompt("Model name")
        .interact_text()?;
    Ok(model)
}

/// Whether `warn_unknown_model` should skip its unknown-model warning for `model`.
///
/// True for `latest:*` placeholders (resolved at link time, see
/// [`is_latest_placeholder`]) and for the `latest:auto` routing placeholder used by
/// the OH4 router (deliberately NOT recognized by [`parse_latest_placeholder`],
/// since it is resolved by tier-routing logic upstream rather than by the
/// `latest:*` tier parser — see `run_single_agent` step 5).
fn should_skip_unknown_model_warning(model: &str) -> bool {
    is_latest_placeholder(model) || model == "latest:auto"
}

/// Warn if the model is not found in the cached models.dev registry.
///
/// Skips the warning for `latest:*` placeholders (they are resolved at link time)
/// and for `latest:auto` (resolved by the router before the provider call).
pub fn warn_unknown_model(model: &str, provider: &str) {
    if should_skip_unknown_model_warning(model) {
        return;
    }
    if let Some(entries) = armadai_providers::model_registry::fetch::load_models_cached(provider)
        && !entries.iter().any(|e| e.id == model)
    {
        tracing::warn!(
            "Model '{model}' not found in {provider} registry — \
             it may be unavailable. Consider adding model_fallback entries."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agent(name: &str, model: Option<&str>) -> LinkAgent {
        LinkAgent {
            name: name.to_string(),
            system_prompt: "You are a test agent.".to_string(),
            instructions: None,
            output_format: None,
            context: None,
            description: Some("A test agent.".to_string()),
            tags: vec![],
            stacks: vec![],
            scope: vec![],
            model: model.map(String::from),
            model_fallback: vec![],
            temperature: 0.7,
            provider: None,
        }
    }

    fn make_agent_with_provider(
        name: &str,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> LinkAgent {
        let mut a = make_agent(name, model);
        a.provider = provider.map(String::from);
        a
    }

    // ── Target classification ────────────────────────────────────

    #[test]
    fn test_classify_claude() {
        assert!(matches!(
            classify_target("claude"),
            TargetKind::LlmEditor {
                provider: "anthropic"
            }
        ));
    }

    #[test]
    fn test_classify_gemini() {
        assert!(matches!(
            classify_target("gemini"),
            TargetKind::LlmEditor { provider: "google" }
        ));
    }

    #[test]
    fn test_classify_codex() {
        assert!(matches!(
            classify_target("codex"),
            TargetKind::LlmEditor { provider: "openai" }
        ));
    }

    #[test]
    fn test_classify_copilot_is_orchestrator() {
        assert!(matches!(
            classify_target("copilot"),
            TargetKind::Orchestrator
        ));
    }

    #[test]
    fn test_classify_opencode_is_orchestrator() {
        assert!(matches!(
            classify_target("opencode"),
            TargetKind::Orchestrator
        ));
    }

    #[test]
    fn test_classify_unknown_is_orchestrator() {
        assert!(matches!(
            classify_target("some-tool"),
            TargetKind::Orchestrator
        ));
    }

    // ── Fallback models ──────────────────────────────────────────

    #[test]
    fn test_fallback_models() {
        assert_eq!(fallback_model("anthropic"), "claude-sonnet-4-5-20250929");
        assert_eq!(fallback_model("google"), "gemini-2.5-pro");
        assert_eq!(fallback_model("openai"), "gpt-4o");
        assert_eq!(fallback_model("unknown"), "claude-sonnet-4-5-20250929");
    }

    // ── Latest placeholder parsing ───────────────────────────────

    #[test]
    fn test_is_latest_placeholder() {
        assert!(is_latest_placeholder("latest"));
        assert!(is_latest_placeholder("latest:fast"));
        assert!(!is_latest_placeholder("claude-sonnet-4-5-20250929"));
    }

    // ── Remap functions ──────────────────────────────────────────

    #[test]
    fn test_remap_orchestrator() {
        let mut agents = vec![
            make_agent("Agent A", Some("claude-sonnet-4-5-20250929")),
            make_agent("Agent B", None),
            make_agent("Agent C", Some("gpt-4o")),
        ];

        remap_models_for_orchestrator(&mut agents, "gemini-2.5-pro");

        for agent in &agents {
            assert_eq!(agent.model.as_deref(), Some("gemini-2.5-pro"));
        }
    }

    #[test]
    fn test_remap_orchestrator_empty() {
        let mut agents: Vec<LinkAgent> = vec![];
        remap_models_for_orchestrator(&mut agents, "some-model");
        assert!(agents.is_empty());
    }

    #[test]
    fn test_resolve_latest_placeholders() {
        let mut agents = vec![
            make_agent_with_provider("A", Some("latest:fast"), Some("anthropic")),
            make_agent_with_provider("B", Some("latest:max"), Some("google")),
            make_agent_with_provider("C", Some("claude-sonnet-4-5-20250929"), Some("anthropic")),
            make_agent_with_provider("D", Some("latest"), None),
        ];

        resolve_latest_placeholders(&mut agents);

        // A: fast anthropic → haiku variant
        assert!(agents[0].model.as_ref().unwrap().contains("haiku"));
        // B: max google → pro variant (no ultra)
        assert!(agents[1].model.as_ref().unwrap().contains("pro"));
        // C: concrete model → unchanged
        assert_eq!(
            agents[2].model.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        // D: latest without provider → defaults to anthropic pro
        assert!(agents[3].model.as_ref().unwrap().contains("sonnet"));
    }

    #[test]
    fn test_preview_resolution_fallbacks() {
        // Without cache, preview should return fallback models for LLM editors
        // and the agent model (or placeholder) for orchestrators.
        let result = preview_model_resolution(Some("my-model"));
        assert_eq!(result.len(), 5);

        let targets: Vec<&str> = result.iter().map(|(t, _)| *t).collect();
        assert!(targets.contains(&"claude"));
        assert!(targets.contains(&"codex"));
        assert!(targets.contains(&"gemini"));
        assert!(targets.contains(&"copilot"));
        assert!(targets.contains(&"opencode"));

        // Orchestrator targets use agent model
        for (target, model) in &result {
            if matches!(classify_target(target), TargetKind::Orchestrator) {
                assert_eq!(model, "my-model");
            }
        }

        // Without agent model, orchestrators show placeholder
        let result_no_model = preview_model_resolution(None);
        for (target, model) in &result_no_model {
            if matches!(classify_target(target), TargetKind::Orchestrator) {
                assert_eq!(model, "(requires --model)");
            }
        }
    }

    #[test]
    fn test_preview_resolution_with_latest() {
        // Hermetic: force an empty, private `ARMADAI_CONFIG_DIR` so this test
        // never sees the ambient/machine-local models.dev cache (which may
        // be present, absent, or contain `-latest` alias ids depending on
        // machine + parallel test runs — see resolve_model_for_tier's
        // doc-comment). With no cache reachable, resolution always takes the
        // hardcoded-fallback path, making the assertion deterministic.
        let _guard = armadai_core::test_support::env_lock();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: env mutation is serialised via `env_lock()` for the duration
        // of this test, and the original value is restored before returning.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", tmp.path());
        }

        let result = preview_model_resolution(Some("latest:fast"));

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }

        for (_target, model) in &result {
            // All targets should resolve to a concrete model, not "latest:fast"
            assert!(!model.contains("latest"));
        }
    }

    // ── warn_unknown_model guard ──────────────────────────────────

    #[test]
    fn test_should_skip_unknown_model_warning_for_latest_auto() {
        // Regression test: `latest:auto` must be treated as a placeholder to
        // skip, even though `parse_latest_placeholder`/`is_latest_placeholder`
        // deliberately do NOT recognize it (it's resolved by router tier
        // logic, not the `latest:*` tier parser).
        assert!(should_skip_unknown_model_warning("latest:auto"));
    }

    #[test]
    fn test_should_skip_unknown_model_warning_for_latest_placeholders() {
        assert!(should_skip_unknown_model_warning("latest"));
        assert!(should_skip_unknown_model_warning("latest:pro"));
        assert!(should_skip_unknown_model_warning("latest:fast"));
        assert!(should_skip_unknown_model_warning("latest:max"));
    }

    #[test]
    fn test_should_warn_for_concrete_models() {
        // Concrete/`latest:pro`-resolved models must still get the normal
        // unknown-model warning path (routing behavior must not change).
        assert!(!should_skip_unknown_model_warning(
            "claude-sonnet-4-5-20250929"
        ));
        assert!(!should_skip_unknown_model_warning("some-unknown-model"));
        assert!(!should_skip_unknown_model_warning("latest:autopilot"));
    }
}
