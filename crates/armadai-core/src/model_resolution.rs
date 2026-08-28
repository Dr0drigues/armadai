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
            if id.ends_with("-pro") {
                // The `-pro` line is the Max tier, `o3-pro` included — this
                // enum's own doc-comment names it as the example.
                Some(ModelTier::Max)
            } else if is_openai_reasoning_id(id) {
                // The rest of the o-series (`o1`, `o3`, `o3-mini`,
                // `o4-mini`) is a *reasoning* line, not a rung on the chat
                // price ladder these three tiers describe — so it claims no
                // tier at all rather than an ill-fitting one.
                //
                // `mini` in an o-series id names a smaller *reasoning*
                // model, not a cheap chat one: `o4-mini` costs $1.10/$4.40
                // per Mtok against `gpt-4o-mini`'s $0.15/$0.60 (models.dev,
                // read 2026-08-28). Filed under Fast, it was 7x the price of
                // the tier's own example model, and `latest:fast` answered
                // with it (issue #404).
                //
                // `is_openai_reasoning_id` also replaces `starts_with("o")`,
                // which matched every id beginning with that letter,
                // moderation and omni models included.
                None
            } else if id.contains("mini") || id.contains("nano") {
                Some(ModelTier::Fast)
            } else if id.starts_with("gpt-") {
                Some(ModelTier::Pro)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Is `id` one of OpenAI's o-series reasoning models (`o1`, `o3`, `o4-mini`,
/// `o3-pro`, …)?
///
/// The shape is the letter `o` followed immediately by a digit. Deliberately
/// narrower than the `id.starts_with("o")` it replaces, which classified any
/// id beginning with that letter as a chat model.
fn is_openai_reasoning_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next() == Some('o') && chars.next().is_some_and(|c| c.is_ascii_digit())
}

/// The model generation an id names, as the sequence of numbers it contains:
/// `gpt-4.1` → `[4, 1]`, `gpt-4o` → `[4]`, `claude-opus-4-8` → `[4, 8]`.
///
/// Compared as a vector, i.e. element-wise and numerically. That is what
/// [`resolve_model_for_tier`]'s doc always claimed ("the highest version")
/// and what `candidates.iter().max()` never did: the string order puts `o3`
/// above `gpt-5.6` because `o` follows `g`, and `gpt-9` above `gpt-10`
/// because `9` follows `1` (issue #404).
///
/// A dated suffix is *not* stripped: two dated snapshots of one model
/// (`…-4-5-20250929` vs `…-4-5-20260101`) then order by date, which is the
/// answer you want, and a dated id never competes with its undated alias
/// anyway — [`resolve_model_for_tier`] prefers the undated set outright.
///
/// An id with no digits yields an empty vector, which sorts below every
/// other generation. Deterministic rather than arbitrary: such an id makes
/// no version claim at all.
fn generation_key(id: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut current: Option<u64> = None;
    for c in id.chars() {
        match c.to_digit(10) {
            Some(d) => current = Some(current.unwrap_or(0).saturating_mul(10) + u64::from(d)),
            None => {
                if let Some(n) = current.take() {
                    out.push(n);
                }
            }
        }
    }
    out.extend(current);
    out
}

/// The tier a vendor actually distinguishes, for a requested `tier`.
///
/// Today this collapses exactly one case: **Google publishes no line above
/// `pro`**, so `latest:max` on Google is the Pro tier. That was already the
/// behaviour, but by accident — [`classify_model_tier`] simply never returned
/// `Max` for `google`, so the request found no candidate and fell through to
/// [`fallback_model_for_tier`]'s hardcoded `gemini-2.5-pro`. The accident had
/// a cost: `google` + Pro tracked whatever the catalog listed while `google`
/// + Max stayed frozen on that one id forever (issue #404).
///
/// Stating it here gives the decision one home: a vendor whose Max is its Pro
/// resolves Max *through the Pro tier*, catalog included, and the day Google
/// ships a line above `pro` this arm is what gets deleted.
pub fn effective_tier(vendor: &str, tier: ModelTier) -> ModelTier {
    match (vendor, tier) {
        ("google", ModelTier::Max) => ModelTier::Pro,
        _ => tier,
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
///
/// Goes through [`effective_tier`] first, so a vendor that does not
/// distinguish a tier answers here exactly what it answers from the catalog:
/// `("google", Max)` has no row of its own because Google's Max *is* its Pro,
/// stated once rather than twice.
pub fn fallback_model_for_tier(provider: &str, tier: ModelTier) -> &'static str {
    match (provider, effective_tier(provider, tier)) {
        ("anthropic", ModelTier::Fast) => "claude-haiku-4-5-20251001",
        ("anthropic", ModelTier::Pro) => "claude-sonnet-4-5-20250929",
        ("anthropic", ModelTier::Max) => "claude-opus-4-6",
        ("google", ModelTier::Fast) => "gemini-2.5-flash",
        ("google", ModelTier::Pro) => "gemini-2.5-pro",
        ("openai", ModelTier::Fast) => "gpt-4o-mini",
        ("openai", ModelTier::Pro) => "gpt-4o",
        ("openai", ModelTier::Max) => "o3-pro",
        (_, ModelTier::Fast) => "claude-haiku-4-5-20251001",
        (_, ModelTier::Pro) => "claude-sonnet-4-5-20250929",
        (_, ModelTier::Max) => "claude-opus-4-6",
    }
}

/// One model as this module needs it: its id, and the per-Mtok price the
/// catalog quotes for it when it quotes one.
struct CachedModel {
    id: String,
    /// `(input, output)` per million tokens, or `None` when the catalog
    /// carries no price — which happens for real entries
    /// (`gemma-4-26b-a4b-it`, `chatgpt-image-latest`, …), so it is a case to
    /// order, not a case to reject.
    cost: Option<(f64, f64)>,
}

/// Read the model ids and prices for `provider` from the shared, on-disk
/// models.dev cache (`<config_dir>/models-cache.json`), ignoring cache age.
///
/// This is a minimal, dependency-free mirror of the bin-side
/// `model_registry::fetch::load_models_cached` (which also exposes the
/// context window and the display name). Core reads the same cache file
/// directly instead of depending on the `model_registry` module — which
/// stays in the bin (it owns the online refresh path, gated behind
/// `providers-api`, and the richer `ModelEntry` used by the TUI/Web/CLI
/// listings). Returns `None` if the cache is missing/unreadable; callers
/// fall back to [`fallback_model_for_tier`].
///
/// The price is read and the context window is not, on purpose: a tier is a
/// price class, whereas the context window is not monotone in the tier —
/// `gpt-4.1-nano` carries 1M tokens against `gpt-4o`'s 128K, so ordering a
/// tier by context window would rank the cheap line above the balanced one.
fn load_cached_models(provider: &str) -> Option<Vec<CachedModel>> {
    #[derive(serde::Deserialize)]
    struct CachedCost {
        input: Option<f64>,
        output: Option<f64>,
    }
    #[derive(serde::Deserialize)]
    struct CachedEntry {
        id: String,
        #[serde(default)]
        cost: Option<CachedCost>,
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
    Some(
        entries
            .iter()
            .map(|e| CachedModel {
                id: e.id.clone(),
                // A half-quoted price is no price: an entry missing either
                // side cannot be compared against a fully quoted one
                // without inventing the missing half. Both fields are
                // optional in the catalog's schema; measured on the live
                // catalog (5694 entries, 2026-08-28) 399 carry no cost at
                // all and none carries only half of one, so this arm guards
                // the schema rather than a case seen in the wild.
                cost: e.cost.as_ref().and_then(|c| Some((c.input?, c.output?))),
            })
            .collect(),
    )
}

/// Order two candidates of one tier, worst first, so `max_by` yields the one
/// to use.
///
/// Three keys, in order:
///
/// 1. **Generation** ([`generation_key`]) — `latest:pro` asks for the latest
///    model of the Pro tier, and the old `max()` read "latest" off the
///    alphabet: `o3` beat `gpt-5.6` because `o` follows `g` (issue #404).
///    Generation comes first because price is not monotone in capability
///    across generations — measured on the live catalog, "cheapest of the
///    Pro tier" is `gpt-3.5-turbo` and "dearest" is `gpt-4`, both worse
///    answers than the `o3` being replaced. Price only separates models of
///    one generation cleanly.
/// 2. **Priced beats unpriced** — a model the catalog does not price cannot
///    be checked against the tier's price promise, so it never wins over one
///    that is priced. Reading a missing price as `+∞` would have handed the
///    `Max` tier (dearest wins) to every unpriced model.
/// 3. **Price**, in the direction the tier's own doc-comment states: `Fast`
///    ("cheap and fast") and `Pro` ("balanced performance") take the
///    cheapest of the generation, `Max` ("maximum capability") the dearest.
///    Input price decides, output price breaks its ties.
///
/// The id is the final tie-break, ascending — the same order the previous
/// implementation used, kept so a catalog where every key above ties resolves
/// exactly as it did before.
fn compare_candidates(a: &CachedModel, b: &CachedModel, tier: ModelTier) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let generation = generation_key(&a.id).cmp(&generation_key(&b.id));
    if generation != Ordering::Equal {
        return generation;
    }

    let priced = a.cost.is_some().cmp(&b.cost.is_some());
    if priced != Ordering::Equal {
        return priced;
    }

    if let (Some((ai, ao)), Some((bi, bo))) = (a.cost, b.cost) {
        let dearer = ai.total_cmp(&bi).then_with(|| ao.total_cmp(&bo));
        let price = match tier {
            ModelTier::Max => dearer,
            ModelTier::Fast | ModelTier::Pro => dearer.reverse(),
        };
        if price != Ordering::Equal {
            return price;
        }
    }

    a.id.cmp(&b.id)
}

/// Resolve the best model for a provider and tier from the cached registry.
///
/// Strategy:
/// 1. Collapse the tier onto the one this vendor distinguishes
///    ([`effective_tier`]) — Google's Max is its Pro.
/// 2. Filter cached models by tier (using [`classify_model_tier`]).
/// 3. Exclude `latest`-alias IDs (e.g. `claude-3-5-sonnet-latest`) — real
///    provider catalogs (models.dev) can list these alongside dated/bare
///    variants. Callers (e.g. `latest:*` placeholder resolution) require a
///    genuinely concrete model id — never one containing the literal
///    substring `latest` — so these are excluded at every stage below, not
///    just from the "clean" preference pass.
/// 4. Exclude dated variants (IDs containing `-20` date suffixes) and
///    preview models from the preferred ("clean") candidate set.
/// 5. Among the remaining, pick the best by [`compare_candidates`]: newest
///    generation first, then price in the direction the tier promises.
/// 6. If no "clean" candidate survives, fall back to any non-`latest`
///    candidate, ordered the same way.
/// 7. If no candidate survives filtering at all, fall back to hardcoded
///    defaults.
///
/// This guarantees `resolve_model_for_tier` never returns a string
/// containing `"latest"`, regardless of what the ambient model registry
/// cache does or doesn't contain.
pub fn resolve_model_for_tier(provider: &str, tier: ModelTier) -> String {
    let tier = effective_tier(provider, tier);
    if let Some(models) = load_cached_models(provider) {
        let candidates: Vec<&CachedModel> = models
            .iter()
            .filter(|m| classify_model_tier(&m.id, provider) == Some(tier))
            .filter(|m| !m.id.contains("latest"))
            .collect();

        // Prefer non-dated, non-preview variants
        let clean: Vec<&&CachedModel> = candidates
            .iter()
            .filter(|m| !m.id.contains("-20") && !m.id.contains("preview"))
            .collect();

        if let Some(best) = clean.iter().max_by(|a, b| compare_candidates(a, b, tier)) {
            return best.id.clone();
        }

        // Fallback: any candidate (already excludes `latest`-alias ids)
        if let Some(best) = candidates
            .iter()
            .max_by(|a, b| compare_candidates(a, b, tier))
        {
            return best.id.clone();
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
        // `o3` used to be Pro on the strength of its first letter. See
        // `the_openai_reasoning_line_is_not_a_chat_tier` (#404).
        assert_eq!(classify_model_tier("o3", "openai"), None);
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

    // ── Tier selection is a price class, not an alphabet (issue #404) ─

    /// Plant a `models-cache.json` naming `models` under `provider` in the
    /// isolated config dir, in exactly the shape
    /// `model_registry::fetch` writes.
    ///
    /// `None` for a cost is the real thing, not a contrivance: the live
    /// models.dev catalog answers `"cost": null` for several entries
    /// (`gemma-4-26b-a4b-it`, `chatgpt-image-latest`, …).
    fn seed_cache(
        iso: &crate::test_support::IsolatedConfigDir,
        provider: &str,
        models: &[(&str, Option<(f64, f64)>)],
    ) {
        let entries: Vec<serde_json::Value> = models
            .iter()
            .map(|(id, cost)| {
                serde_json::json!({
                    "id": id,
                    "name": id,
                    "cost": cost.map(|(i, o)| serde_json::json!({"input": i, "output": o})),
                    "limit": {"context": 128000, "output": 4096},
                })
            })
            .collect();
        let doc = serde_json::json!({
            "fetched_at": "2026-08-28T00:00:00Z",
            "providers": { provider: entries },
        });
        std::fs::write(
            iso.config_dir().join(crate::config::MODELS_CACHE_FILE),
            serde_json::to_string(&doc).unwrap(),
        )
        .expect("write models cache");
    }

    /// The measurement issue #404 opened on, replayed at the unit level: the
    /// exact catalog it seeded, and the three answers it recorded.
    ///
    /// Before: `latest:fast` → `o4-mini` ($1.10/$4.40 per Mtok, a reasoning
    /// model) and `latest:pro` → `o3`, purely because `o` sorts after `g`.
    /// The user who wrote "cheap and fast" — the tier's own words — was
    /// billed at 7x the tier's own example model.
    ///
    /// `o1-pro` is added to the issue's list so the Max tier has two
    /// candidates rather than one: it is the dearest model in the catalog
    /// ($150/$600), so a Max tier ordered on price alone would answer with
    /// it instead of `o3-pro`.
    #[test]
    fn the_reported_catalog_resolves_to_the_chat_line_not_the_reasoning_line() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "openai",
            &[
                ("gpt-4o", Some((2.5, 10.0))),
                ("gpt-4o-mini", Some((0.15, 0.6))),
                ("o3", Some((2.0, 8.0))),
                ("o4-mini", Some((1.1, 4.4))),
                ("o3-pro", Some((20.0, 80.0))),
                ("o1-pro", Some((150.0, 600.0))),
            ],
        );
        assert_eq!(
            resolve_model_for_tier("openai", ModelTier::Fast),
            "gpt-4o-mini"
        );
        assert_eq!(resolve_model_for_tier("openai", ModelTier::Pro), "gpt-4o");
        assert_eq!(resolve_model_for_tier("openai", ModelTier::Max), "o3-pro");
    }

    /// The generation key, isolated, on the case that shows it is numeric:
    /// generation 10 is above generation 9, while the *string* `"gpt-10"` is
    /// below `"gpt-9"`.
    ///
    /// The fixture also prices the newer model higher, so it separates the
    /// new order from **both** the old alphabetical one and a naive
    /// "cheapest wins" reading of the issue's suggestion — each of which
    /// would answer `gpt-9`.
    #[test]
    fn a_newer_generation_wins_over_one_that_sorts_later_and_costs_less() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "openai",
            &[("gpt-9", Some((1.0, 5.0))), ("gpt-10", Some((5.0, 30.0)))],
        );
        assert_eq!(resolve_model_for_tier("openai", ModelTier::Pro), "gpt-10");
    }

    /// The classification half, at the resolver rather than at
    /// `classify_model_tier`: a *newer* reasoning model must not capture the
    /// Fast tier. This is the defect's next occurrence rather than its last
    /// one — the day OpenAI ships an `o5-mini`, the old rules would have
    /// handed `latest:fast` to it on generation alone, exactly as they
    /// handed it to `o4-mini` on the alphabet.
    #[test]
    fn a_newer_reasoning_model_does_not_capture_the_fast_tier() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "openai",
            &[
                ("gpt-4o-mini", Some((0.15, 0.6))),
                ("o5-mini", Some((1.5, 6.0))),
            ],
        );
        assert_eq!(
            resolve_model_for_tier("openai", ModelTier::Fast),
            "gpt-4o-mini"
        );
    }

    /// The cost key, isolated: four models of one generation, where the
    /// alphabetically last (`…-terra`, $2.50) is not the cheapest
    /// (`…-luna`, $1.00). Real ids and real models.dev prices, so the
    /// fixture is not built to make the point.
    #[test]
    fn within_one_generation_the_cheapest_wins_not_the_last_alphabetically() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "openai",
            &[
                ("gpt-5.6", Some((5.0, 30.0))),
                ("gpt-5.6-luna", Some((1.0, 6.0))),
                ("gpt-5.6-sol", Some((5.0, 30.0))),
                ("gpt-5.6-terra", Some((2.5, 15.0))),
            ],
        );
        assert_eq!(
            resolve_model_for_tier("openai", ModelTier::Pro),
            "gpt-5.6-luna"
        );
    }

    /// `Max` reads its own doc-comment ("maximum capability") and takes the
    /// dearest of its generation, where `Fast`/`Pro` take the cheapest. The
    /// fixture puts the dearest first alphabetically so neither the old
    /// order nor a uniform "cheapest" rule can pass it.
    #[test]
    fn the_max_tier_takes_the_dearest_of_its_generation() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "openai",
            &[
                ("gpt-5.6-alpha-pro", Some((30.0, 180.0))),
                ("gpt-5.6-omega-pro", Some((5.0, 25.0))),
            ],
        );
        assert_eq!(
            resolve_model_for_tier("openai", ModelTier::Max),
            "gpt-5.6-alpha-pro"
        );
    }

    /// A model the catalog does not price cannot be checked against the
    /// tier's price promise, so it never beats one that is priced — in
    /// **either** direction. The `Max` half is the one that matters: with a
    /// missing cost read as "infinitely expensive", the unpriced model would
    /// win the dearest-wins comparison outright.
    #[test]
    fn an_unpriced_model_never_beats_a_priced_one() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "openai",
            &[
                ("gpt-5.6-alpha", Some((9.0, 40.0))),
                ("gpt-5.6-zeta", None),
                ("gpt-5.6-alpha-pro", Some((9.0, 40.0))),
                ("gpt-5.6-zeta-pro", None),
            ],
        );
        assert_eq!(
            resolve_model_for_tier("openai", ModelTier::Pro),
            "gpt-5.6-alpha",
            "an unpriced model must not win the cheapest-wins comparison"
        );
        assert_eq!(
            resolve_model_for_tier("openai", ModelTier::Max),
            "gpt-5.6-alpha-pro",
            "an unpriced model must not win the dearest-wins comparison either"
        );
    }

    /// Half a price is no price. The catalog's schema makes each side of
    /// `cost` optional, and an entry quoting only its input price cannot be
    /// compared against a fully quoted one without inventing the other half
    /// — so it ranks with the unpriced.
    ///
    /// Written as raw JSON rather than through `seed_cache`, whose
    /// `Option<(f64, f64)>` cannot express the half.
    #[test]
    fn a_half_quoted_price_counts_as_no_price() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        std::fs::write(
            iso.config_dir().join(crate::config::MODELS_CACHE_FILE),
            r#"{"providers":{"openai":[
                 {"id":"gpt-5.6-alpha","cost":{"input":9.0,"output":40.0}},
                 {"id":"gpt-5.6-zeta","cost":{"input":0.01,"output":null}}]}}"#,
        )
        .expect("write models cache");
        assert_eq!(
            resolve_model_for_tier("openai", ModelTier::Pro),
            "gpt-5.6-alpha",
            "a half-quoted price must not win the cheapest-wins comparison"
        );
    }

    /// Two unpriced models still have to resolve to *something* stable:
    /// the id order is the last tie-break, and it is total.
    #[test]
    fn unpriced_models_still_resolve_deterministically() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "openai",
            &[("gpt-5.6-alpha", None), ("gpt-5.6-zeta", None)],
        );
        assert_eq!(
            resolve_model_for_tier("openai", ModelTier::Pro),
            "gpt-5.6-zeta"
        );
    }

    /// OpenAI's o-series is a reasoning line, not a cheaper variant of the
    /// chat line: `o4-mini` costs $1.10/$4.40 against `gpt-4o-mini`'s
    /// $0.15/$0.60 (models.dev, read 2026-08-28) — 7x the tier whose own
    /// example model it displaced. It claims no chat tier now, except for
    /// the `-pro` line this enum's doc-comment already calls Max.
    /// `starts_with("o")` also swept up anything else beginning with that
    /// letter.
    #[test]
    fn the_openai_reasoning_line_is_not_a_chat_tier() {
        assert_eq!(classify_model_tier("o4-mini", "openai"), None);
        assert_eq!(classify_model_tier("o3-mini", "openai"), None);
        assert_eq!(classify_model_tier("o3", "openai"), None);
        assert_eq!(classify_model_tier("o1", "openai"), None);
        // …except the `-pro` line, which is Max on both sides.
        assert_eq!(
            classify_model_tier("o3-pro", "openai"),
            Some(ModelTier::Max)
        );
        assert_eq!(
            classify_model_tier("o1-pro", "openai"),
            Some(ModelTier::Max)
        );
        // Not the o-series, just a name that starts with the same letter.
        assert_eq!(classify_model_tier("omni-moderation", "openai"), None);
        // Same letter, and a `mini` the Fast tier would want: the shape has
        // to be `o` + digit, or the narrowing swallows this too. Illustrative
        // of the shape rather than of a shipped id — no model in today's
        // catalog starts with `o` and carries a `mini`/`nano` without being
        // o-series, which is exactly why the predicate is pinned below as
        // well as here.
        assert_eq!(
            classify_model_tier("omni-mini", "openai"),
            Some(ModelTier::Fast)
        );
        // The chat line is untouched.
        assert_eq!(
            classify_model_tier("gpt-4o-mini", "openai"),
            Some(ModelTier::Fast)
        );
        assert_eq!(
            classify_model_tier("gpt-5.6", "openai"),
            Some(ModelTier::Pro)
        );
        assert_eq!(
            classify_model_tier("gpt-5.5-pro", "openai"),
            Some(ModelTier::Max)
        );
    }

    /// The o-series is a shape — the letter `o` followed by a digit — not a
    /// first letter. Pinned on the predicate itself because
    /// `classify_model_tier` answers `None` for most `o…` ids either way,
    /// so the classification alone cannot show the difference.
    #[test]
    fn the_o_series_is_a_letter_followed_by_a_digit() {
        for id in ["o1", "o3", "o3-mini", "o4-mini", "o3-pro", "o1-pro"] {
            assert!(is_openai_reasoning_id(id), "{id} is o-series");
        }
        for id in ["omni-moderation", "openai-thing", "gpt-4o", "o", "", "0-3"] {
            assert!(!is_openai_reasoning_id(id), "{id} is not o-series");
        }
    }

    /// A generation is a sequence of numbers compared numerically, which is
    /// what "highest version" meant all along — `max()` on the id string
    /// only ever approximated it.
    #[test]
    fn a_generation_is_read_as_numbers_not_as_text() {
        assert_eq!(generation_key("gpt-4.1"), vec![4, 1]);
        assert_eq!(generation_key("gpt-4o"), vec![4]);
        assert_eq!(generation_key("gpt-5.6-luna"), vec![5, 6]);
        assert_eq!(generation_key("o3"), vec![3]);
        assert_eq!(generation_key("claude-opus-4-8"), vec![4, 8]);
        assert_eq!(generation_key("gemini-2.5-flash"), vec![2, 5]);
        assert_eq!(generation_key("nothing-numeric"), Vec::<u64>::new());
        // Numerically, not textually: "10" is above "9".
        assert!(generation_key("gpt-10") > generation_key("gpt-9"));
        // …which the old string order got exactly backwards.
        assert!("gpt-10" < "gpt-9");
    }

    /// Google publishes no line above `pro`, so `latest:max` on Google is
    /// deliberately the Pro tier — stated once, in [`effective_tier`],
    /// rather than left to happen because `classify_model_tier` never
    /// returns `Max` for that vendor.
    ///
    /// The difference is observable: before, `google` + `Max` had no
    /// candidate and fell through to the hardcoded `gemini-2.5-pro`, so it
    /// stayed frozen on that id no matter what the catalog listed. It now
    /// tracks the Pro tier's own answer.
    #[test]
    fn google_max_is_the_pro_tier_and_tracks_the_catalog() {
        assert_eq!(effective_tier("google", ModelTier::Max), ModelTier::Pro);
        assert_eq!(effective_tier("google", ModelTier::Fast), ModelTier::Fast);
        assert_eq!(effective_tier("openai", ModelTier::Max), ModelTier::Max);
        assert_eq!(effective_tier("anthropic", ModelTier::Max), ModelTier::Max);

        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "google",
            &[
                ("gemini-9-pro", Some((2.0, 12.0))),
                ("gemini-9-flash", Some((0.5, 3.0))),
            ],
        );
        let pro = resolve_model_for_tier("google", ModelTier::Pro);
        assert_eq!(pro, "gemini-9-pro");
        assert_eq!(
            resolve_model_for_tier("google", ModelTier::Max),
            pro,
            "Google's Max must be its Pro, read from the catalog like any other tier"
        );
    }

    /// Anthropic and Google are untouched by the openai reclassification —
    /// the vendors whose own naming already states the tier.
    #[test]
    fn the_other_vendors_keep_resolving_as_before() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "anthropic",
            &[
                ("claude-haiku-4-5", Some((1.0, 5.0))),
                ("claude-sonnet-4-5", Some((3.0, 15.0))),
                ("claude-sonnet-4-6", Some((3.0, 15.0))),
                ("claude-sonnet-5", Some((2.0, 10.0))),
                ("claude-opus-4-6", Some((5.0, 25.0))),
                ("claude-opus-4-8", Some((5.0, 25.0))),
            ],
        );
        assert_eq!(
            resolve_model_for_tier("anthropic", ModelTier::Fast),
            "claude-haiku-4-5"
        );
        assert_eq!(
            resolve_model_for_tier("anthropic", ModelTier::Pro),
            "claude-sonnet-5"
        );
        assert_eq!(
            resolve_model_for_tier("anthropic", ModelTier::Max),
            "claude-opus-4-8"
        );
    }

    /// The guarantee the whole filter chain exists for: no `latest`-alias id
    /// ever comes back, whatever the catalog holds — including now that the
    /// order is no longer "the string that sorts last".
    #[test]
    fn a_latest_alias_id_is_never_returned() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        seed_cache(
            &iso,
            "openai",
            &[
                ("gpt-9-chat-latest", Some((0.01, 0.01))),
                ("gpt-4o", Some((2.5, 10.0))),
            ],
        );
        assert_eq!(resolve_model_for_tier("openai", ModelTier::Pro), "gpt-4o");
    }
}
