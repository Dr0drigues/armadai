use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::core::config::config_dir;

use super::ModelEntry;

const CACHE_FILE: &str = "models-cache.json";
const CACHE_TTL_SECS: u64 = 86400; // 24h
pub(crate) const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// Cached registry: provider_id → Vec<ModelEntry>
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CachedRegistry {
    fetched_at: u64,
    providers: HashMap<String, Vec<ModelEntry>>,
}

fn cache_path() -> PathBuf {
    config_dir().join(CACHE_FILE)
}

/// Load models for a given provider from cache only (sync).
/// Returns None if cache is missing or stale.
#[cfg(not(feature = "providers-api"))]
pub fn load_models(provider: &str) -> Option<Vec<ModelEntry>> {
    let cached = load_cache_from(&cache_path())?;
    cached.providers.get(provider).cloned()
}

/// Load models, fetching from remote if cache is stale or missing.
#[cfg(feature = "providers-api")]
pub async fn load_models_online(provider: &str) -> Option<Vec<ModelEntry>> {
    // Try fresh cache first
    if let Some(cached) = load_cache_from(&cache_path())
        && let Some(models) = cached.providers.get(provider)
    {
        return Some(models.clone());
    }
    // Fetch and cache
    if let Ok(registry) = fetch_and_cache().await {
        return registry.providers.get(provider).cloned();
    }
    None
}

/// Resolve the effective list of model registry sources: the built-in
/// `models.dev` catalog, plus any user-level
/// (`~/.config/armadai/registries.yaml`) and project-level (`armadai.yaml` /
/// `.armadai/config.yaml`) custom sources.
///
/// Project config is looked up via `core::project::find_project_config`,
/// which walks up from the current working directory. Without a
/// `registries:` section anywhere, this returns exactly `[MODELS_DEV_URL]`.
#[cfg(feature = "providers-api")]
fn resolved_model_sources() -> Vec<String> {
    let user = crate::core::registries::load_user_registries();
    let project = crate::core::project::find_project_config().map(|(_, cfg)| cfg);
    let project_registries = project.as_ref().and_then(|cfg| cfg.registries.as_ref());
    crate::core::registries::resolved_sources(
        crate::core::registries::RegistryKind::Models,
        &[MODELS_DEV_URL],
        &user,
        project_registries,
    )
}

/// Directory holding one cache file per model registry source, keyed by a
/// sanitized version of the source URL. Kept separate from the merged
/// `models-cache.json` (see `cache_path`) so a fetch failure on one source
/// doesn't lose previously-fetched data for the others.
#[cfg(feature = "providers-api")]
fn source_cache_dir() -> PathBuf {
    config_dir().join("models-sources")
}

/// Derive a filesystem-safe cache file path for a given source URL.
///
/// Delegates to `core::registries::cache_key`, the sanitization scheme
/// shared with `registry::sync::source_key` (previously each had its own,
/// independently-maintained and collision-prone sanitization — see B2 Task 2
/// review).
#[cfg(feature = "providers-api")]
fn source_cache_path(url: &str) -> PathBuf {
    let key = crate::core::registries::cache_key(url);
    source_cache_dir().join(format!("{key}.json"))
}

/// Fetch and parse a single source's catalog (models.dev format).
#[cfg(feature = "providers-api")]
async fn fetch_source(url: &str) -> anyhow::Result<HashMap<String, Vec<ModelEntry>>> {
    let body: serde_json::Value = reqwest::get(url).await?.json().await?;
    Ok(parse_registry(&body))
}

/// Fetch every configured model registry source and merge them into a
/// single provider → models map.
///
/// Merge semantics: **union by provider, last source wins** — sources are
/// processed in `resolved_model_sources()` order (built-in default first,
/// then user, then project custom sources), and for a given provider id the
/// last source that supplied it overwrites earlier ones.
///
/// Each source is cached independently (see `source_cache_path`). If a
/// source's fetch fails, we fall back to that source's last successful
/// cache (regardless of its TTL) so one flaky/unreachable custom registry
/// doesn't wipe out previously known data — only if a source has *never*
/// succeeded does it contribute nothing. If every source fails, the first
/// error encountered is returned (preserving the pre-multi-source behavior
/// of propagating a fetch error when there is only the single default
/// source).
///
/// The aggregate cache's `fetched_at` is the oldest timestamp among the
/// sources that actually contributed data this run (see
/// [`aggregate_fetched_at`]) rather than unconditionally `now()`: if any
/// source had to fall back to its own stale cache, stamping the merged
/// result as freshly-fetched would hide up to 24h (the per-source TTL) of
/// degradation from consumers that only check the aggregate's freshness.
#[cfg(feature = "providers-api")]
async fn fetch_and_cache() -> anyhow::Result<CachedRegistry> {
    let sources = resolved_model_sources();
    let mut merged: HashMap<String, Vec<ModelEntry>> = HashMap::new();
    let mut first_err: Option<anyhow::Error> = None;
    let mut any_data = false;
    let mut contributing_timestamps: Vec<u64> = Vec::new();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for source in &sources {
        let source_path = source_cache_path(source);
        match fetch_source(source).await {
            Ok(providers) => {
                any_data = true;
                contributing_timestamps.push(now);
                save_cache_to(
                    &source_path,
                    &CachedRegistry {
                        fetched_at: now,
                        providers: providers.clone(),
                    },
                );
                for (provider, entries) in providers {
                    merged.insert(provider, entries);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch model registry source '{source}': {e:?}");
                // Fall back to whatever this source last produced, however
                // stale, rather than losing it entirely.
                if let Ok(content) = std::fs::read_to_string(&source_path)
                    && let Ok(cached) = serde_json::from_str::<CachedRegistry>(&content)
                {
                    any_data = true;
                    contributing_timestamps.push(cached.fetched_at);
                    for (provider, entries) in cached.providers {
                        merged.insert(provider, entries);
                    }
                } else if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }

    if !any_data {
        return Err(
            first_err.unwrap_or_else(|| anyhow::anyhow!("no model registry source available"))
        );
    }

    let fetched_at = aggregate_fetched_at(&contributing_timestamps, now);
    if fetched_at < now {
        tracing::warn!(
            "Model registry cache freshness degraded: at least one source fell back to a stale cache (aggregate fetched_at={fetched_at}, now={now})"
        );
    }

    let registry = CachedRegistry {
        fetched_at,
        providers: merged,
    };
    save_cache_to(&cache_path(), &registry);
    Ok(registry)
}

/// Compute the aggregate cache's `fetched_at` from the per-source
/// timestamps that contributed data this run. Any source that fell back to
/// a stale cache pulls the aggregate down to that source's own `fetched_at`,
/// so a merged cache that includes stale fallback data never claims to be
/// fresher than its stalest ingredient. Pure function, kept separate from
/// [`fetch_and_cache`] so this logic is unit-testable without a network
/// call.
#[cfg(any(feature = "providers-api", test))]
fn aggregate_fetched_at(contributing_timestamps: &[u64], now: u64) -> u64 {
    contributing_timestamps.iter().copied().min().unwrap_or(now)
}

/// Parse the models.dev JSON structure into a provider → models map.
#[cfg(any(feature = "providers-api", test))]
fn parse_registry(body: &serde_json::Value) -> HashMap<String, Vec<ModelEntry>> {
    let mut providers = HashMap::new();
    let Some(obj) = body.as_object() else {
        return providers;
    };
    for (provider_id, provider_val) in obj {
        let Some(models_obj) = provider_val.get("models").and_then(|m| m.as_object()) else {
            continue;
        };
        let entries: Vec<ModelEntry> = models_obj
            .iter()
            .map(|(model_id, val)| ModelEntry {
                id: model_id.clone(),
                name: val.get("name").and_then(|n| n.as_str()).map(String::from),
                cost: val
                    .get("cost")
                    .and_then(|c| serde_json::from_value(c.clone()).ok()),
                limit: val
                    .get("limit")
                    .and_then(|l| serde_json::from_value(l.clone()).ok()),
            })
            .collect();
        providers.insert(provider_id.clone(), entries);
    }
    providers
}

/// Force-refresh the model registry from models.dev, ignoring cache TTL.
/// Returns the number of providers fetched, or an error.
#[cfg(feature = "providers-api")]
pub async fn refresh_registry() -> anyhow::Result<usize> {
    let registry = fetch_and_cache().await?;
    Ok(registry.providers.len())
}

/// Load models for a provider from cache (sync). Always available, no feature gate.
pub fn load_models_cached(provider: &str) -> Option<Vec<ModelEntry>> {
    let cached = load_cache_from(&cache_path())?;
    cached.providers.get(provider).cloned()
}

/// Load all providers from cache (sync). Always available, no feature gate.
pub fn load_all_providers_cached() -> Option<HashMap<String, Vec<ModelEntry>>> {
    let cached = load_cache_from(&cache_path())?;
    Some(cached.providers)
}

fn load_cache_from(path: &Path) -> Option<CachedRegistry> {
    let content = std::fs::read_to_string(path).ok()?;
    let cached: CachedRegistry = serde_json::from_str(&content).ok()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if now - cached.fetched_at < CACHE_TTL_SECS {
        Some(cached)
    } else {
        None
    }
}

#[cfg(any(feature = "providers-api", test))]
fn save_cache_to(path: &Path, registry: &CachedRegistry) {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!("Failed to create model registry cache directory: {:?}", e);
    }
    if let Ok(json) = serde_json::to_string(registry)
        && let Err(e) = std::fs::write(path, json)
    {
        tracing::warn!("Failed to write model registry cache: {:?}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_registry::{ModelCost, ModelLimits};

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE);

        let registry = CachedRegistry {
            fetched_at: now_secs(),
            providers: HashMap::from([(
                "anthropic".to_string(),
                vec![ModelEntry {
                    id: "claude-sonnet-4-5".to_string(),
                    name: Some("Claude Sonnet 4.5".to_string()),
                    cost: Some(ModelCost {
                        input: Some(3.0),
                        output: Some(15.0),
                    }),
                    limit: Some(ModelLimits {
                        context: Some(200_000),
                        output: Some(8192),
                    }),
                }],
            )]),
        };

        save_cache_to(&path, &registry);
        let loaded = load_cache_from(&path).expect("cache should load");
        assert_eq!(loaded.providers.len(), 1);
        let models = loaded.providers.get("anthropic").unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "claude-sonnet-4-5");
    }

    #[test]
    fn cache_expired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE);

        let registry = CachedRegistry {
            fetched_at: 0, // epoch — definitely expired
            providers: HashMap::from([("openai".to_string(), vec![])]),
        };

        save_cache_to(&path, &registry);
        assert!(
            load_cache_from(&path).is_none(),
            "expired cache should return None"
        );
    }

    #[test]
    fn cache_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        assert!(load_cache_from(&path).is_none());
    }

    #[test]
    fn parse_registry_json() {
        let json = serde_json::json!({
            "anthropic": {
                "name": "Anthropic",
                "models": {
                    "claude-sonnet-4-5-20250929": {
                        "name": "Claude Sonnet 4.5",
                        "cost": { "input": 3.0, "output": 15.0 },
                        "limit": { "context": 200000, "output": 8192 }
                    },
                    "claude-haiku-4-5-20251001": {
                        "name": "Claude Haiku 4.5",
                        "cost": { "input": 0.8, "output": 4.0 },
                        "limit": { "context": 200000 }
                    }
                }
            },
            "openai": {
                "name": "OpenAI",
                "models": {
                    "gpt-4o": {
                        "name": "GPT-4o"
                    }
                }
            }
        });

        let providers = parse_registry(&json);
        assert_eq!(providers.len(), 2);

        let anthropic = providers.get("anthropic").unwrap();
        assert_eq!(anthropic.len(), 2);
        let sonnet = anthropic
            .iter()
            .find(|m| m.id == "claude-sonnet-4-5-20250929")
            .unwrap();
        assert_eq!(sonnet.name.as_deref(), Some("Claude Sonnet 4.5"));
        assert_eq!(sonnet.cost.as_ref().unwrap().input, Some(3.0));
        assert_eq!(sonnet.limit.as_ref().unwrap().context, Some(200_000));

        let openai = providers.get("openai").unwrap();
        assert_eq!(openai.len(), 1);
        assert_eq!(openai[0].name.as_deref(), Some("GPT-4o"));
        assert!(openai[0].cost.is_none());
    }

    #[test]
    fn parse_empty_registry() {
        let json = serde_json::json!({});
        let providers = parse_registry(&json);
        assert!(providers.is_empty());
    }

    #[test]
    #[cfg(feature = "providers-api")]
    fn resolved_sources_defaults_only_without_custom_config() {
        use crate::core::registries::{RegistriesConfig, RegistryKind, resolved_sources};

        let user = RegistriesConfig::default();
        let result = resolved_sources(RegistryKind::Models, &[MODELS_DEV_URL], &user, None);
        assert_eq!(result, vec![MODELS_DEV_URL.to_string()]);
    }

    #[test]
    #[cfg(feature = "providers-api")]
    fn resolved_sources_includes_default_and_custom_model_source() {
        use crate::core::registries::{
            RegistriesConfig, RegistryKind, RegistrySource, resolved_sources,
        };

        let user = RegistriesConfig {
            models: vec![RegistrySource {
                url: "https://example.com/custom-models.json".to_string(),
                kind: None,
            }],
            ..Default::default()
        };

        let result = resolved_sources(RegistryKind::Models, &[MODELS_DEV_URL], &user, None);
        assert_eq!(
            result,
            vec![
                MODELS_DEV_URL.to_string(),
                "https://example.com/custom-models.json".to_string(),
            ]
        );
    }

    #[test]
    #[cfg(feature = "providers-api")]
    fn source_cache_path_is_stable_and_distinct_per_source() {
        let a = source_cache_path("https://models.dev/api.json");
        let b = source_cache_path("https://example.com/custom-models.json");
        assert_ne!(a, b);
        // Deterministic: same URL always maps to the same path.
        assert_eq!(a, source_cache_path("https://models.dev/api.json"));
    }

    #[test]
    #[cfg(feature = "providers-api")]
    fn resolved_model_sources_glue_includes_user_level_custom_source() {
        // Exercises the real `resolved_model_sources()` glue (not just the
        // pure `resolved_sources` helper) with a `RegistriesConfig` loaded
        // from disk, per the B2 Task 2 review follow-up.
        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
        }

        std::fs::write(
            dir.path().join("registries.yaml"),
            "models:\n  - url: https://example.com/custom-glue-models.json\n",
        )
        .unwrap();

        let sources = resolved_model_sources();

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }

        assert!(sources.contains(&MODELS_DEV_URL.to_string()));
        assert!(sources.contains(&"https://example.com/custom-glue-models.json".to_string()));
    }

    #[test]
    fn aggregate_fetched_at_is_now_when_all_sources_fresh() {
        let now = 1_700_000_000;
        assert_eq!(aggregate_fetched_at(&[now, now], now), now);
    }

    #[test]
    fn aggregate_fetched_at_uses_oldest_when_a_source_fell_back_to_stale_cache() {
        let now = 1_700_000_000;
        let stale = now - 3600; // one source fell back to a 1h-old cache
        assert_eq!(aggregate_fetched_at(&[now, stale], now), stale);
    }

    #[test]
    fn aggregate_fetched_at_defaults_to_now_when_no_contributions() {
        // Defensive default; `fetch_and_cache` never actually reaches this
        // with an empty slice (it bails out via `any_data` first), but the
        // pure function should still behave sanely.
        assert_eq!(aggregate_fetched_at(&[], 42), 42);
    }

    #[test]
    fn test_load_all_providers_cached_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE);

        let registry = CachedRegistry {
            fetched_at: now_secs(),
            providers: HashMap::from([
                (
                    "anthropic".to_string(),
                    vec![ModelEntry {
                        id: "claude-sonnet-4-5".to_string(),
                        name: Some("Claude Sonnet 4.5".to_string()),
                        cost: None,
                        limit: None,
                    }],
                ),
                (
                    "openai".to_string(),
                    vec![ModelEntry {
                        id: "gpt-4o".to_string(),
                        name: Some("GPT-4o".to_string()),
                        cost: None,
                        limit: None,
                    }],
                ),
            ]),
        };

        save_cache_to(&path, &registry);

        // Temporarily override cache path by loading directly
        let loaded = load_cache_from(&path).expect("cache should load");
        assert_eq!(loaded.providers.len(), 2);
        assert!(loaded.providers.contains_key("anthropic"));
        assert!(loaded.providers.contains_key("openai"));
    }

    #[test]
    fn test_load_models_cached_specific_provider() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE);

        let registry = CachedRegistry {
            fetched_at: now_secs(),
            providers: HashMap::from([(
                "google".to_string(),
                vec![ModelEntry {
                    id: "gemini-2.5-pro".to_string(),
                    name: Some("Gemini 2.5 Pro".to_string()),
                    cost: None,
                    limit: None,
                }],
            )]),
        };

        save_cache_to(&path, &registry);
        let loaded = load_cache_from(&path).expect("cache should load");
        let google = loaded.providers.get("google").unwrap();
        assert_eq!(google.len(), 1);
        assert_eq!(google[0].id, "gemini-2.5-pro");
        // Non-existent provider
        assert!(!loaded.providers.contains_key("unknown"));
    }
}
