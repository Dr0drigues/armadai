use std::collections::{HashMap, HashSet};

/// Embedded alias defaults — updated each release.
fn embedded_aliases() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        // Google
        // `gemini-3.0-pro` is not a real Google model — it was a name commonly
        // used by users anticipating Gemini 3. Map it to the portable `latest:pro`
        // tier rather than pinning to a concrete model, so the resolution picks
        // the best stable Gemini at runtime via the model registry.
        ("gemini-3.0-pro", "latest:pro"),
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
    let path = crate::config::config_dir().join("model-aliases.json");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_alias_known() {
        let result = resolve_alias("gemini-3.0-pro");
        assert_eq!(result, Some("latest:pro".to_string()));
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
    fn test_circular_alias_safe() {
        // With only embedded aliases, no circularity exists.
        // But the algorithm protects against it via the `seen` set.
        // We verify the function terminates and returns a valid result.
        let result = resolve_alias("gpt-3.5-turbo");
        assert_eq!(result, Some("gpt-4o-mini".to_string()));
    }
}
