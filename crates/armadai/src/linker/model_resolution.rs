use super::LinkAgent;
use armadai_core::model_resolution::{
    ModelTier, fallback_model_for_tier, resolve_model_for_tier, resolve_routed_tier,
};

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
            // `resolve_routed_tier`, not `resolve_model_for_tier`: the value
            // here is an agent's own `provider:` — a *tool* name (`gemini`,
            // `aider`, …), which the vendor-keyed catalog does not know. Fed
            // in raw it missed the cache and fell through to the Anthropic
            // catch-all, so a `provider: gemini` agent was written into a
            // native config with a Claude model id (#398 review, F1).
            let provider = agent.provider.as_deref().unwrap_or("anthropic");
            agent.model = Some(resolve_routed_tier(provider, tier));
        }
    }
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

    /// An agent's `provider:` may be a *tool* name, which the vendor-keyed
    /// catalog does not know: `link` wrote a Claude model id into a native
    /// config for a `provider: gemini` agent (#398 review, F1).
    ///
    /// Hermetic for the same reason `test_preview_resolution_with_latest`
    /// is: with no models.dev cache reachable, resolution is the hardcoded
    /// fallback table and the expectation is derived from it rather than
    /// restated.
    #[test]
    fn resolve_latest_placeholders_uses_each_agents_own_vendor() {
        let _iso = armadai_core::test_support::IsolatedConfigDir::enter();
        let mut agents = vec![
            make_agent_with_provider("A", Some("latest:pro"), Some("gemini")),
            make_agent_with_provider("B", Some("latest:fast"), Some("aider")),
            make_agent_with_provider("C", Some("latest:max"), Some("claude")),
        ];

        resolve_latest_placeholders(&mut agents);

        for (agent, vendor, tier) in [
            (&agents[0], "google", ModelTier::Pro),
            (&agents[1], "openai", ModelTier::Fast),
            (&agents[2], "anthropic", ModelTier::Max),
        ] {
            assert_eq!(
                agent.model.as_deref(),
                Some(fallback_model_for_tier(vendor, tier)),
                "{} should resolve against {vendor}",
                agent.name
            );
        }
    }
}
