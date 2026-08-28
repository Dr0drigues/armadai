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

/// The model catalog whose ids name `provider`'s models, or `None` when
/// nothing here knows.
///
/// An agent's `provider:` is a *tool* name (`gemini`, `aider`, `claude`, …)
/// while the models.dev catalog — and [`fallback_model_for_tier`] — are
/// keyed by *vendor* (`google`, `openai`, `anthropic`). Handing the tool
/// name straight to either lookup misses the cache and then falls through
/// the vendor table's catch-all, which answers with an **Anthropic** model:
/// `provider: gemini` + `model: latest:pro` used to send
/// `claude-sonnet-4-5-20250929` to `generativelanguage.googleapis.com`
/// (#398 review, F1). `armadai shell` already carried this table privately
/// (`shell::config::shell_provider_to_linker`), which is exactly how two
/// subcommands of one binary came to disagree on one agent file.
///
/// Deliberately NOT `armadai_providers::factory::api_backend_for_tool`,
/// which answers a different question — "if this CLI is missing, which API
/// can I call instead?". `codex` has no such backend (issue #369) yet its
/// models are named by OpenAI, so the two mappings differ on purpose. (Core
/// could not depend on `armadai-providers` anyway: the dependency runs the
/// other way.)
///
/// `None` — for `cli`, `proxy`, `copilot`, `opencode` and anything unknown
/// — means "no vendor catalog names these models", not "use Anthropic's".
/// See [`resolve_tier_placeholder`] for what callers do with it.
pub fn model_catalog_provider(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" | "claude" => Some("anthropic"),
        "google" | "gemini" => Some("google"),
        "openai" | "gpt" | "aider" | "codex" => Some("openai"),
        _ => None,
    }
}

/// Resolve a static `latest:*` tier placeholder into a concrete model id for
/// `provider`.
///
/// Returns `None` when the string is not a static placeholder — a concrete
/// model id, or `latest:auto` — so a caller can keep its own handling for
/// those two cases (`.unwrap_or(raw_model)` for the former, the router for
/// the latter).
///
/// Also returns `None` when [`model_catalog_provider`] does not name a
/// vendor for `provider`. That covers `provider: cli` and the CLI-only tools
/// (whose relay ignores `request.model` outright) and `provider: proxy`,
/// where the placeholder is the more useful string of the two: a gateway
/// administrator can route `latest:max` through a house alias, whereas a
/// concrete `claude-opus-4-6` picked here is a vendor this side of the wire
/// chose on its own, with no opt-out (#398 review, F1).
///
/// For every provider that *does* name a vendor, this is the last gate
/// before a model string reaches the wire: every site that builds a
/// `CompletionRequest` from an agent's declared model goes through it, so no
/// `latest:*` placeholder other than `latest:auto` can be sent to an API as
/// a model name (#376).
pub fn resolve_tier_placeholder(model: &str, provider: &str) -> Option<String> {
    let catalog = model_catalog_provider(provider)?;
    parse_latest_placeholder(model).map(|tier| resolve_model_for_tier(catalog, tier))
}

/// Resolve `tier` to a concrete model id for `provider`, naming its vendor
/// catalog first.
///
/// The counterpart of [`resolve_tier_placeholder`] for `latest:auto`, whose
/// tier the router picks per call: there is no placeholder left to pass
/// through by then, so an unnamed vendor keeps the old behaviour (the
/// provider name is handed to the catalog lookup as-is, which misses and
/// lands on [`fallback_model_for_tier`]'s catch-all) rather than answering
/// `None`.
pub fn resolve_routed_tier(provider: &str, tier: ModelTier) -> String {
    resolve_model_for_tier(model_catalog_provider(provider).unwrap_or(provider), tier)
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
        // A named vendor, so the `None` can only come from the parse — not
        // from `model_catalog_provider` declining to name one.
        assert_eq!(resolve_tier_placeholder("latest:auto", "anthropic"), None);
    }

    #[test]
    fn resolve_tier_placeholder_maps_each_tier_and_passes_concrete_ids_through() {
        // Hermetic: an empty `ARMADAI_CONFIG_DIR` means no models.dev cache
        // is reachable, so `fallback_model_for_tier`'s hardcoded table is
        // the deterministic answer whatever the machine holds.
        let _iso = crate::test_support::IsolatedConfigDir::enter();
        let prov = "anthropic";
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

    // ── Which vendor names a provider's models (#398 review, F1) ─────

    /// The defect this closes, measured at the real binary before the fix:
    /// an agent declaring `provider: gemini` + `model: latest:pro` sent
    /// `POST /v1beta/models/claude-sonnet-4-5-20250929:generateContent` to
    /// Google. The tool name missed the vendor-keyed lookup and fell through
    /// `fallback_model_for_tier`'s catch-all, which answers with Anthropic.
    #[test]
    fn a_tool_name_resolves_against_its_own_vendor_not_anthropic() {
        let _iso = crate::test_support::IsolatedConfigDir::enter();
        for (tool, vendor) in [
            ("gemini", "google"),
            ("claude", "anthropic"),
            ("aider", "openai"),
            ("codex", "openai"),
            ("gpt", "openai"),
        ] {
            for tier in [ModelTier::Fast, ModelTier::Pro, ModelTier::Max] {
                let placeholder = match tier {
                    ModelTier::Fast => "latest:fast",
                    ModelTier::Pro => "latest:pro",
                    ModelTier::Max => "latest:max",
                };
                assert_eq!(
                    resolve_tier_placeholder(placeholder, tool).as_deref(),
                    Some(fallback_model_for_tier(vendor, tier)),
                    "{tool} + {placeholder} must resolve against {vendor}"
                );
                assert_eq!(
                    resolve_routed_tier(tool, tier),
                    fallback_model_for_tier(vendor, tier),
                    "{tool} + routed {tier:?} must resolve against {vendor}"
                );
            }
        }
    }

    /// A provider no vendor names keeps its placeholder rather than being
    /// handed a model some other vendor sells. `cli` and the CLI-only tools
    /// ignore `request.model` outright; a `proxy` gateway can route
    /// `latest:max` through a house alias, which a concrete
    /// `claude-opus-4-6` chosen here would silently override.
    #[test]
    fn a_provider_with_no_named_vendor_keeps_the_placeholder() {
        let _iso = crate::test_support::IsolatedConfigDir::enter();
        for prov in ["cli", "proxy", "copilot", "opencode", "some-local-thing"] {
            assert_eq!(model_catalog_provider(prov), None, "{prov}");
            for placeholder in ["latest", "latest:fast", "latest:pro", "latest:max"] {
                assert_eq!(
                    resolve_tier_placeholder(placeholder, prov),
                    None,
                    "{prov} + {placeholder} must be left to the caller"
                );
            }
        }
    }

    /// `latest:auto` has no placeholder left to pass through once the router
    /// has named a tier, so `resolve_routed_tier` always answers — including
    /// for a provider with no named vendor, where it keeps the pre-existing
    /// catch-all rather than leaking `latest:auto` to a server.
    #[test]
    fn a_routed_tier_always_answers_even_with_no_named_vendor() {
        let _iso = crate::test_support::IsolatedConfigDir::enter();
        for prov in ["cli", "proxy", "some-local-thing"] {
            let got = resolve_routed_tier(prov, ModelTier::Pro);
            assert!(!got.contains("latest"), "{prov} got {got}");
            assert_eq!(got, fallback_model_for_tier(prov, ModelTier::Pro));
        }
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
