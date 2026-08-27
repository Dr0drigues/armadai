// ── Model tiers ──────────────────────────────────────────────────

/// Performance tier for model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelTier {
    /// Cheap and fast (haiku, flash, gpt-4o-mini).
    Fast,
    /// Balanced performance (sonnet, pro, gpt-4o).
    Pro,
    /// Maximum capability (opus, ultra, o3-pro).
    Max,
}

/// Classify a model ID into a tier based on its name.
///
/// Returns `None` for non-chat models (embeddings, TTS, image, etc.)
/// or unrecognised naming patterns.
pub fn classify_model_tier(id: &str, provider: &str) -> Option<ModelTier> {
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

/// Parse a **static** `latest` tier placeholder into its tier.
///
/// Syntax: `latest` (defaults to Pro), `latest:fast`/`latest:low`,
/// `latest:pro`/`latest:medium`, `latest:max`/`latest:high`.
///
/// Returns `None` for a concrete model id — and, deliberately, for
/// `latest:auto`: that placeholder's tier is not knowable statically. It
/// depends on the run's own input and is resolved per call by
/// [`crate::routing::route`], which is why every caller here treats it
/// separately rather than folding it into this table.
///
/// Lives in core (rather than beside the linker, which was its only user
/// until #376) because the run path needs the exact same table: a tier
/// placeholder that `armadai link` resolves but `armadai run` sends
/// verbatim to the provider is the defect #376 closed, and two tables would
/// have let that divergence come back.
pub fn parse_latest_placeholder(model: &str) -> Option<ModelTier> {
    match model.trim() {
        "latest" | "latest:pro" | "latest:medium" => Some(ModelTier::Pro),
        "latest:fast" | "latest:low" => Some(ModelTier::Fast),
        "latest:max" | "latest:high" => Some(ModelTier::Max),
        _ => None,
    }
}

/// Resolve a static `latest:*` tier placeholder into a concrete model id for
/// `provider`.
///
/// Returns `None` when `model` is not a static placeholder — a concrete
/// model id, or `latest:auto` — so a caller can keep its own handling for
/// those two cases (`.unwrap_or(raw_model)` for the former, the router for
/// the latter).
///
/// This is the last gate before a model string reaches a provider: every
/// site that builds a `CompletionRequest` from an agent's declared model
/// goes through it, so no `latest:*` placeholder other than `latest:auto`
/// can be sent over the wire as a model name (#376).
pub fn resolve_tier_placeholder(model: &str, provider: &str) -> Option<String> {
    parse_latest_placeholder(model).map(|tier| resolve_model_for_tier(provider, tier))
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

/// Read just the model IDs for `provider` from the shared, on-disk
/// models.dev cache (`<config_dir>/models-cache.json`), ignoring cache age.
///
/// This is a minimal, dependency-free mirror of the bin-side
/// `model_registry::fetch::load_models_cached` (which also exposes cost/
/// context-window metadata for display purposes). Core only ever needs the
/// bare IDs to classify tiers, so it reads the same cache file directly
/// instead of depending on the `model_registry` module — which stays in the
/// bin (it owns the online refresh path, gated behind `providers-api`, and
/// richer `ModelEntry` metadata used by the TUI/Web/CLI listings). Returns
/// `None` if the cache is missing/unreadable; callers fall back to
/// [`fallback_model_for_tier`].
fn load_cached_model_ids(provider: &str) -> Option<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct CachedEntry {
        id: String,
    }
    #[derive(serde::Deserialize, Default)]
    struct CachedRegistry {
        #[serde(default)]
        providers: std::collections::HashMap<String, Vec<CachedEntry>>,
    }

    let path = crate::config::config_dir().join(crate::config::MODELS_CACHE_FILE);
    let content = std::fs::read_to_string(path).ok()?;
    let cached: CachedRegistry = serde_json::from_str(&content).ok()?;
    let entries = cached.providers.get(provider)?;
    Some(entries.iter().map(|e| e.id.clone()).collect())
}

/// Resolve the best model for a provider and tier from the cached registry.
///
/// Strategy:
/// 1. Filter cached models by tier (using `classify_model_tier`).
/// 2. Exclude `latest`-alias IDs (e.g. `claude-3-5-sonnet-latest`) — real
///    provider catalogs (models.dev) can list these alongside dated/bare
///    variants, and since `"latest"` sorts alphabetically after any digit,
///    an unfiltered `max()` would prefer them over a concrete pinned
///    version. Callers (e.g. `latest:*` placeholder resolution) require a
///    genuinely concrete model id — never one containing the literal
///    substring `latest` — so these are excluded at every stage below, not
///    just from the "clean" preference pass.
/// 3. Exclude dated variants (IDs containing `-20` date suffixes) and
///    preview models from the preferred ("clean") candidate set.
/// 4. Among remaining, pick the one that sorts last alphabetically (highest version).
/// 5. If no "clean" candidate survives, fall back to any non-`latest` candidate.
/// 6. If no candidate survives filtering at all, fall back to hardcoded defaults.
///
/// This guarantees `resolve_model_for_tier` never returns a string
/// containing `"latest"`, regardless of what the ambient model registry
/// cache does or doesn't contain.
pub fn resolve_model_for_tier(provider: &str, tier: ModelTier) -> String {
    if let Some(ids) = load_cached_model_ids(provider) {
        let candidates: Vec<&str> = ids
            .iter()
            .filter(|id| classify_model_tier(id, provider) == Some(tier))
            .filter(|id| !id.contains("latest"))
            .map(|id| id.as_str())
            .collect();

        // Prefer non-dated, non-preview variants
        let clean: Vec<&&str> = candidates
            .iter()
            .filter(|id| !id.contains("-20") && !id.contains("preview"))
            .collect();

        if let Some(best) = clean.iter().max() {
            return (**best).to_string();
        }

        // Fallback: any candidate (already excludes `latest`-alias ids), pick highest
        if let Some(best) = candidates.iter().max() {
            return best.to_string();
        }
    }
    fallback_model_for_tier(provider, tier).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Fallback models ──────────────────────────────────────────

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

    // ── `latest:*` placeholders ──────────────────────────────────

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

    // `latest:auto` is NOT a static placeholder: its tier depends on the
    // run's input, so it must fall through to the caller's router rather
    // than silently resolving to Pro here.
    #[test]
    fn latest_auto_is_not_a_static_placeholder() {
        assert_eq!(parse_latest_placeholder("latest:auto"), None);
        assert_eq!(
            resolve_tier_placeholder("latest:auto", "test-only-uncached-provider"),
            None
        );
    }

    #[test]
    fn resolve_tier_placeholder_maps_each_tier_and_passes_concrete_ids_through() {
        // Uncached provider name so the hardcoded `fallback_model_for_tier`
        // table is the deterministic answer, whatever the machine's
        // models.dev cache holds.
        let prov = "test-only-uncached-provider";
        assert_eq!(
            resolve_tier_placeholder("latest:fast", prov).as_deref(),
            Some(fallback_model_for_tier(prov, ModelTier::Fast))
        );
        assert_eq!(
            resolve_tier_placeholder("latest", prov).as_deref(),
            Some(fallback_model_for_tier(prov, ModelTier::Pro))
        );
        assert_eq!(
            resolve_tier_placeholder("latest:max", prov).as_deref(),
            Some(fallback_model_for_tier(prov, ModelTier::Max))
        );
        // A concrete id is not a placeholder: `None` tells the caller to
        // keep the string it already has.
        assert_eq!(resolve_tier_placeholder("gpt-4o-mini", prov), None);
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

    #[test]
    fn model_tier_is_ordered_fast_pro_max() {
        use ModelTier::*;
        assert!(Fast < Pro && Pro < Max);
        assert_eq!([Pro, Fast, Max].iter().copied().max().unwrap(), Max);
    }
}
