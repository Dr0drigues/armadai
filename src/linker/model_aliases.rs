use std::collections::{HashMap, HashSet};

/// Embedded alias defaults — updated each release.
fn embedded_aliases() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        // Google
        ("gemini-3.0-pro", "gemini-2.5-pro"),
        ("gemini-1.5-flash", "gemini-2.5-flash"),
        ("gemini-1.5-pro", "gemini-2.0-pro"),
        ("gemini-1.0-pro", "gemini-2.0-pro"),
        // OpenAI
        ("gpt-4-turbo", "gpt-4o"),
        ("gpt-3.5-turbo", "gpt-4o-mini"),
    ])
}

/// Load local overrides from `~/.config/armadai/model-aliases.json`.
fn load_local_aliases() -> Option<HashMap<String, String>> {
    let path = crate::core::config::config_dir().join("model-aliases.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Resolve a model alias. Returns the replacement if deprecated, `None` otherwise.
/// Local overrides take priority over embedded defaults.
/// Resolves transitively (A→B→C yields A→C).
pub fn resolve_alias(model: &str) -> Option<String> {
    let local = load_local_aliases();
    let embedded = embedded_aliases();

    let mut current = model.to_string();
    let mut seen = HashSet::new();

    loop {
        seen.insert(current.clone());
        let next = local
            .as_ref()
            .and_then(|l| l.get(&current).cloned())
            .or_else(|| embedded.get(current.as_str()).map(|s| s.to_string()));

        match next {
            Some(replacement) if !seen.contains(&replacement) => current = replacement,
            _ => break,
        }
    }

    if current != model {
        Some(current)
    } else {
        None
    }
}

/// Apply alias resolution to model + fallbacks, logging warnings.
pub fn resolve_model_deprecations(model: &mut Option<String>, fallbacks: &mut [String]) {
    if let Some(current) = model.as_deref()
        && let Some(replacement) = resolve_alias(current)
    {
        tracing::warn!("Model '{current}' is deprecated, using '{replacement}' instead");
        *model = Some(replacement);
    }
    for fb in fallbacks.iter_mut() {
        if let Some(replacement) = resolve_alias(fb) {
            tracing::warn!("Fallback model '{fb}' is deprecated, using '{replacement}' instead");
            *fb = replacement;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_alias_known() {
        let result = resolve_alias("gemini-3.0-pro");
        assert_eq!(result, Some("gemini-2.5-pro".to_string()));
    }

    #[test]
    fn test_alias_unknown_returns_none() {
        let result = resolve_alias("claude-sonnet-4-5");
        assert_eq!(result, None);
    }

    #[test]
    fn test_transitive_resolution() {
        // gemini-1.0-pro → gemini-2.0-pro (direct embedded)
        // If we had a chain, it would resolve transitively.
        // Test with embedded: gemini-1.0-pro → gemini-2.0-pro
        let result = resolve_alias("gemini-1.0-pro");
        assert_eq!(result, Some("gemini-2.0-pro".to_string()));
    }

    #[test]
    fn test_resolve_deprecations_mutates() {
        let mut model = Some("gpt-4-turbo".to_string());
        let mut fallbacks = vec![
            "gemini-3.0-pro".to_string(),
            "claude-sonnet-4-5".to_string(),
        ];

        resolve_model_deprecations(&mut model, &mut fallbacks);

        assert_eq!(model, Some("gpt-4o".to_string()));
        assert_eq!(fallbacks[0], "gemini-2.5-pro");
        // Unknown model stays unchanged
        assert_eq!(fallbacks[1], "claude-sonnet-4-5");
    }

    #[test]
    fn test_circular_alias_safe() {
        // With only embedded aliases, no circularity exists.
        // But the algorithm protects against it via the `seen` set.
        // We verify the function terminates and returns a valid result.
        let result = resolve_alias("gpt-3.5-turbo");
        assert_eq!(result, Some("gpt-4o-mini".to_string()));
    }

    #[test]
    fn test_no_mutation_when_no_alias() {
        let mut model = Some("claude-sonnet-4-5".to_string());
        let mut fallbacks = vec!["o3-mini".to_string()];

        resolve_model_deprecations(&mut model, &mut fallbacks);

        assert_eq!(model, Some("claude-sonnet-4-5".to_string()));
        assert_eq!(fallbacks[0], "o3-mini");
    }
}
