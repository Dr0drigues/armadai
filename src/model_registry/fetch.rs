use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::core::config::config_dir;

use super::ModelEntry;

const CACHE_FILE: &str = "models-cache.json";
const CACHE_TTL_SECS: u64 = 86400; // 24h
#[cfg(feature = "providers-api")]
const MODELS_DEV_URL: &str = "https://models.dev/api.json";

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

#[cfg(feature = "providers-api")]
async fn fetch_and_cache() -> anyhow::Result<CachedRegistry> {
    let body: serde_json::Value = reqwest::get(MODELS_DEV_URL).await?.json().await?;
    let providers = parse_registry(&body);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let registry = CachedRegistry {
        fetched_at: now,
        providers,
    };
    save_cache_to(&cache_path(), &registry);
    Ok(registry)
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
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(registry) {
        let _ = std::fs::write(path, json);
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
}
