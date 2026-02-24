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

/// Hardcoded fallback model for a given provider.
pub fn fallback_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-4-5-20250929",
        "google" => "gemini-2.5-pro",
        "openai" => "o3-mini",
        _ => "claude-sonnet-4-5-20250929",
    }
}

/// Resolve the best model for a provider from the model registry.
/// Falls back to a hardcoded default if the registry is unavailable.
#[cfg(feature = "providers-api")]
pub async fn resolve_best_model(provider: &str) -> String {
    if let Some(entries) = crate::model_registry::fetch::load_models_online(provider).await
        && !entries.is_empty()
    {
        // First model in registry is typically the best/newest
        return entries[0].id.clone();
    }
    fallback_model(provider).to_string()
}

/// Resolve the best model for a provider from cache only (sync).
#[cfg(not(feature = "providers-api"))]
pub fn resolve_best_model(provider: &str) -> String {
    if let Some(entries) = crate::model_registry::fetch::load_models(provider)
        && !entries.is_empty()
    {
        return entries[0].id.clone();
    }
    fallback_model(provider).to_string()
}

/// Remap all agents' models to the best model for the given LLM provider.
#[cfg(feature = "providers-api")]
pub async fn remap_models_for_llm_editor(agents: &mut [LinkAgent], provider: &str) {
    let best = resolve_best_model(provider).await;
    for agent in agents.iter_mut() {
        agent.model = Some(best.clone());
    }
}

/// Remap all agents' models to the best model for the given LLM provider (sync/cache-only).
#[cfg(not(feature = "providers-api"))]
pub fn remap_models_for_llm_editor(agents: &mut [LinkAgent], provider: &str) {
    let best = resolve_best_model(provider);
    for agent in agents.iter_mut() {
        agent.model = Some(best.clone());
    }
}

/// Remap all agents' models to a specific model (for orchestrator targets).
pub fn remap_models_for_orchestrator(agents: &mut [LinkAgent], model: &str) {
    for agent in agents.iter_mut() {
        agent.model = Some(model.to_string());
    }
}

/// Preview model resolution for all known link targets (sync, always available).
///
/// Returns a list of (target_name, resolved_model) tuples showing what model
/// would be used when linking to each target.
pub fn preview_model_resolution(agent_model: Option<&str>) -> Vec<(&'static str, String)> {
    let targets = ["claude", "codex", "gemini", "copilot", "opencode"];
    targets
        .iter()
        .map(|&target| {
            let resolved = match classify_target(target) {
                TargetKind::LlmEditor { provider } => {
                    crate::model_registry::fetch::load_models_cached(provider)
                        .and_then(|e| e.first().map(|m| m.id.clone()))
                        .unwrap_or_else(|| fallback_model(provider).to_string())
                }
                TargetKind::Orchestrator => agent_model.unwrap_or("(requires --model)").to_string(),
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
        }
    }

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

    #[test]
    fn test_fallback_models() {
        assert_eq!(fallback_model("anthropic"), "claude-sonnet-4-5-20250929");
        assert_eq!(fallback_model("google"), "gemini-2.5-pro");
        assert_eq!(fallback_model("openai"), "o3-mini");
        // Unknown provider falls back to anthropic default
        assert_eq!(fallback_model("unknown"), "claude-sonnet-4-5-20250929");
    }

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
    fn test_preview_resolution_fallbacks() {
        // Without cache, preview should return fallback models for LLM editors
        // and the agent model (or placeholder) for orchestrators.
        let result = preview_model_resolution(Some("my-model"));
        assert_eq!(result.len(), 5);

        // Check targets exist
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
}
