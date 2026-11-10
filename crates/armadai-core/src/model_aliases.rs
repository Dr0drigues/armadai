use std::collections::{HashMap, HashSet};

/// Embedded alias defaults — updated each release.
fn embedded_aliases() -> HashMap<&'static str, &'static str> {
    crate::model_table::model_table()
        .aliases
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
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
        let _iso = crate::test_support::IsolatedConfigDir::enter();
        assert_eq!(resolve_alias("gpt-4o"), Some("gpt-6-luna".to_string()));
        assert_eq!(
            resolve_alias("gemini-2.5-pro"),
            Some("gemini-3.8-flash".to_string())
        );
    }

    #[test]
    fn test_alias_unknown_returns_none() {
        let _iso = crate::test_support::IsolatedConfigDir::enter();
        assert_eq!(resolve_alias("claude-sonnet-5-5"), None);
    }

    /// The embedded table has no chain, so a local file supplies one:
    /// `a -> b -> c` resolves to `c`.
    #[test]
    fn test_transitive_resolution() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        std::fs::write(
            iso.config_dir().join("model-aliases.json"),
            r#"{"gpt-6-luna": "luna-x", "luna-x": "luna-y"}"#,
        )
        .unwrap();
        // gpt-4o -> gpt-6-luna (embedded) -> luna-x -> luna-y (local)
        assert_eq!(resolve_alias("gpt-4o"), Some("luna-y".to_string()));
    }

    #[test]
    fn test_circular_alias_safe() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        std::fs::write(
            iso.config_dir().join("model-aliases.json"),
            r#"{"cyc-a": "cyc-b", "cyc-b": "cyc-a"}"#,
        )
        .unwrap();
        // Terminates on the `seen` set; the walk stops before looping back.
        assert_eq!(resolve_alias("cyc-a"), Some("cyc-b".to_string()));
    }

    #[test]
    fn every_alias_points_somewhere_else() {
        let _iso = crate::test_support::IsolatedConfigDir::enter();
        let table = &crate::model_table::model_table().aliases;
        assert!(!table.is_empty());
        for (from, to) in table {
            assert_ne!(from, to, "{from} aliases itself");
            assert!(!to.contains("latest"), "{from} -> {to} is a placeholder");
            let end = resolve_alias(from).unwrap_or_else(|| panic!("{from} resolves to nothing"));
            assert_ne!(&end, from, "{from} loops back to itself");
            assert!(!end.contains("latest"), "{from} ends at {end}");
            // `resolve_alias` stops quietly on a cycle: walk the table itself.
            let mut at = to;
            for _ in 0..table.len() {
                match table.get(at) {
                    Some(next) => at = next,
                    None => break,
                }
                assert_ne!(at, from, "{from} is on an alias cycle");
            }
        }
    }
}
