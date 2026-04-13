use super::LinkAgent;

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

// ── Model tiers ──────────────────────────────────────────────────

/// Performance tier for model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    /// Cheap and fast (haiku, flash, gpt-4o-mini).
    Fast,
    /// Balanced performance (sonnet, pro, gpt-4o).
    Pro,
    /// Maximum capability (opus, ultra, o3-pro).
    Max,
}

/// Parse a `latest` placeholder into a tier.
///
/// Returns `Some(tier)` if the model string is a `latest` placeholder,
/// `None` if it is a concrete model name.
///
/// Syntax: `latest` (defaults to Pro), `latest:fast`, `latest:pro`, `latest:max`.
pub fn parse_latest_placeholder(model: &str) -> Option<ModelTier> {
    match model.trim() {
        "latest" | "latest:pro" | "latest:medium" => Some(ModelTier::Pro),
        "latest:fast" | "latest:low" => Some(ModelTier::Fast),
        "latest:max" | "latest:high" => Some(ModelTier::Max),
        _ => None,
    }
}

/// Check whether a model string is a `latest:*` placeholder.
pub fn is_latest_placeholder(model: &str) -> bool {
    parse_latest_placeholder(model).is_some()
}

/// Classify a model ID into a tier based on its name.
///
/// Returns `None` for non-chat models (embeddings, TTS, image, etc.)
/// or unrecognised naming patterns.
fn classify_model_tier(id: &str, provider: &str) -> Option<ModelTier> {
    // Filter out non-chat models
    if id.contains("embedding")
        || id.contains("-tts")
        || id.contains("-live")
        || id.contains("image")
        || id.contains("deep-research")
        || id.contains("realtime")
    {
        return None;
    }

    match provider {
        "anthropic" => {
            if id.contains("haiku") {
                Some(ModelTier::Fast)
            } else if id.contains("opus") {
                Some(ModelTier::Max)
            } else if id.contains("sonnet") {
                Some(ModelTier::Pro)
            } else {
                None
            }
        }
        "google" => {
            if id.contains("flash") {
                Some(ModelTier::Fast)
            } else if id.contains("pro") {
                Some(ModelTier::Pro)
            } else {
                None
            }
        }
        "openai" => {
            if id.contains("mini") || id.contains("nano") {
                Some(ModelTier::Fast)
            } else if id.ends_with("-pro") {
                Some(ModelTier::Max)
            } else if id.starts_with("gpt-") || id.starts_with("o") {
                Some(ModelTier::Pro)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Hardcoded fallback model for a given provider and tier.
///
/// Used when the model registry cache is unavailable.
pub fn fallback_model_for_tier(provider: &str, tier: ModelTier) -> &'static str {
    match (provider, tier) {
        ("anthropic", ModelTier::Fast) => "claude-haiku-4-5-20251001",
        ("anthropic", ModelTier::Pro) => "claude-sonnet-4-5-20250929",
        ("anthropic", ModelTier::Max) => "claude-opus-4-6",
        ("google", ModelTier::Fast) => "gemini-2.5-flash",
        ("google", ModelTier::Pro) => "gemini-2.5-pro",
        ("google", ModelTier::Max) => "gemini-2.5-pro",
        ("openai", ModelTier::Fast) => "gpt-4o-mini",
        ("openai", ModelTier::Pro) => "gpt-4o",
        ("openai", ModelTier::Max) => "o3-pro",
        (_, ModelTier::Fast) => "claude-haiku-4-5-20251001",
        (_, ModelTier::Pro) => "claude-sonnet-4-5-20250929",
        (_, ModelTier::Max) => "claude-opus-4-6",
    }
}

/// Hardcoded fallback model for a given provider (defaults to Pro tier).
#[allow(dead_code)]
pub fn fallback_model(provider: &str) -> &'static str {
    fallback_model_for_tier(provider, ModelTier::Pro)
}

/// Resolve the best model for a provider and tier from the cached registry.
///
/// Strategy:
/// 1. Filter cached models by tier (using `classify_model_tier`).
/// 2. Exclude dated variants (IDs containing `-20` date suffixes).
/// 3. Exclude preview models.
/// 4. Among remaining, pick the one that sorts last alphabetically (highest version).
/// 5. If no candidate survives filtering, fall back to hardcoded defaults.
pub fn resolve_model_for_tier(provider: &str, tier: ModelTier) -> String {
    if let Some(entries) = crate::model_registry::fetch::load_models_cached(provider) {
        let candidates: Vec<&str> = entries
            .iter()
            .filter(|e| classify_model_tier(&e.id, provider) == Some(tier))
            .map(|e| e.id.as_str())
            .collect();

        // Prefer non-dated, non-preview variants
        let clean: Vec<&&str> = candidates
            .iter()
            .filter(|id| !id.contains("-20") && !id.contains("preview"))
            .collect();

        if let Some(best) = clean.iter().max() {
            return (**best).to_string();
        }

        // Fallback: any candidate, pick highest
        if let Some(best) = candidates.iter().max() {
            return best.to_string();
        }
    }
    fallback_model_for_tier(provider, tier).to_string()
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
    let tier = agent_model.and_then(parse_latest_placeholder);
    let targets = ["claude", "codex", "gemini", "copilot", "opencode"];
    targets
        .iter()
        .map(|&target| {
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

    if let Some(entries) = crate::model_registry::fetch::load_models_online(provider).await
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

    if let Some(entries) = crate::model_registry::fetch::load_models(provider)
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

/// Warn if the model is not found in the cached models.dev registry.
///
/// Skips the warning for `latest:*` placeholders (they are resolved at link time).
pub fn warn_unknown_model(model: &str, provider: &str) {
    if is_latest_placeholder(model) {
        return;
    }
    if let Some(entries) = crate::model_registry::fetch::load_models_cached(provider)
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

    #[test]
    fn test_fallback_models_per_tier() {
        assert_eq!(
            fallback_model_for_tier("anthropic", ModelTier::Fast),
            "claude-haiku-4-5-20251001"
        );
        assert_eq!(
            fallback_model_for_tier("anthropic", ModelTier::Pro),
            "claude-sonnet-4-5-20250929"
        );
        assert_eq!(
            fallback_model_for_tier("anthropic", ModelTier::Max),
            "claude-opus-4-6"
        );
        assert_eq!(
            fallback_model_for_tier("google", ModelTier::Fast),
            "gemini-2.5-flash"
        );
        assert_eq!(
            fallback_model_for_tier("google", ModelTier::Pro),
            "gemini-2.5-pro"
        );
        assert_eq!(
            fallback_model_for_tier("openai", ModelTier::Fast),
            "gpt-4o-mini"
        );
        assert_eq!(fallback_model_for_tier("openai", ModelTier::Max), "o3-pro");
    }

    // ── Latest placeholder parsing ───────────────────────────────

    #[test]
    fn test_parse_latest_placeholder() {
        assert_eq!(parse_latest_placeholder("latest"), Some(ModelTier::Pro));
        assert_eq!(
            parse_latest_placeholder("latest:fast"),
            Some(ModelTier::Fast)
        );
        assert_eq!(parse_latest_placeholder("latest:pro"), Some(ModelTier::Pro));
        assert_eq!(parse_latest_placeholder("latest:max"), Some(ModelTier::Max));
        assert_eq!(parse_latest_placeholder("claude-sonnet-4-5-20250929"), None);
        assert_eq!(parse_latest_placeholder("gemini-2.5-pro"), None);
        assert_eq!(parse_latest_placeholder(""), None);
    }

    #[test]
    fn test_is_latest_placeholder() {
        assert!(is_latest_placeholder("latest"));
        assert!(is_latest_placeholder("latest:fast"));
        assert!(!is_latest_placeholder("claude-sonnet-4-5-20250929"));
    }

    // ── Model tier classification ────────────────────────────────

    #[test]
    fn test_classify_anthropic_tiers() {
        assert_eq!(
            classify_model_tier("claude-haiku-4-5", "anthropic"),
            Some(ModelTier::Fast)
        );
        assert_eq!(
            classify_model_tier("claude-3-5-haiku-20241022", "anthropic"),
            Some(ModelTier::Fast)
        );
        assert_eq!(
            classify_model_tier("claude-sonnet-4-5", "anthropic"),
            Some(ModelTier::Pro)
        );
        assert_eq!(
            classify_model_tier("claude-sonnet-4-5-20250929", "anthropic"),
            Some(ModelTier::Pro)
        );
        assert_eq!(
            classify_model_tier("claude-opus-4-6", "anthropic"),
            Some(ModelTier::Max)
        );
    }

    #[test]
    fn test_classify_google_tiers() {
        assert_eq!(
            classify_model_tier("gemini-2.5-flash", "google"),
            Some(ModelTier::Fast)
        );
        assert_eq!(
            classify_model_tier("gemini-1.5-flash", "google"),
            Some(ModelTier::Fast)
        );
        assert_eq!(
            classify_model_tier("gemini-2.5-pro", "google"),
            Some(ModelTier::Pro)
        );
        assert_eq!(
            classify_model_tier("gemini-1.5-pro", "google"),
            Some(ModelTier::Pro)
        );
    }

    #[test]
    fn test_classify_openai_tiers() {
        assert_eq!(
            classify_model_tier("gpt-4o-mini", "openai"),
            Some(ModelTier::Fast)
        );
        assert_eq!(
            classify_model_tier("gpt-4o", "openai"),
            Some(ModelTier::Pro)
        );
        assert_eq!(classify_model_tier("o3", "openai"), Some(ModelTier::Pro));
        assert_eq!(
            classify_model_tier("o3-pro", "openai"),
            Some(ModelTier::Max)
        );
    }

    #[test]
    fn test_classify_filters_non_chat() {
        assert_eq!(classify_model_tier("text-embedding-3", "openai"), None);
        assert_eq!(classify_model_tier("tts-1", "openai"), None);
    }

    // ── Tier resolution (without cache) ──────────────────────────

    #[test]
    fn test_resolve_model_for_tier_fallback() {
        // Without cache, falls back to hardcoded values
        let fast = resolve_model_for_tier("anthropic", ModelTier::Fast);
        let pro = resolve_model_for_tier("anthropic", ModelTier::Pro);
        let max = resolve_model_for_tier("anthropic", ModelTier::Max);

        // Should be one of: cached model or fallback
        assert!(fast.contains("haiku") || fast == "claude-haiku-4-5-20251001");
        assert!(pro.contains("sonnet") || pro == "claude-sonnet-4-5-20250929");
        assert!(max.contains("opus") || max == "claude-opus-4-6");
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
        let result = preview_model_resolution(Some("latest:fast"));
        for (_target, model) in &result {
            // All targets should resolve to a concrete model, not "latest:fast"
            assert!(!model.contains("latest"));
        }
    }
}
