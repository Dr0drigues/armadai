//! Read-only access to the local models.dev catalogue cache
//! (`~/.config/armadai/models-cache.json`). Nothing here reaches the network:
//! refreshing the catalogue is not part of ArmadAI any more.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use armadai_core::config::config_dir;

use super::ModelEntry;

const CACHE_FILE: &str = armadai_core::config::MODELS_CACHE_FILE;

/// Cached registry: provider_id → Vec<ModelEntry>. Other keys in the file
/// (e.g. `fetched_at`, written by earlier versions) are ignored.
#[derive(serde::Deserialize)]
struct CachedRegistry {
    providers: HashMap<String, Vec<ModelEntry>>,
}

fn cache_path() -> PathBuf {
    config_dir().join(CACHE_FILE)
}

/// Load models for a given provider from cache only (sync).
/// Returns None only if the cache is missing/unreadable (ignores cache age —
/// a stale catalog is still shown; see [`read_cache`]).
pub fn load_models(provider: &str) -> Option<Vec<ModelEntry>> {
    let cached = read_cache(&cache_path())?;
    cached.providers.get(provider).cloned()
}

/// Load all providers from cache (sync).
pub fn load_all_providers_cached() -> Option<HashMap<String, Vec<ModelEntry>>> {
    let cached = read_cache(&cache_path())?;
    Some(cached.providers)
}

/// Read and parse the cache file, **ignoring age**: the model catalog changes
/// slowly, so showing a stale catalog is far better than showing nothing.
fn read_cache(path: &Path) -> Option<CachedRegistry> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cache file as earlier versions wrote it, `fetched_at` included.
    const CACHE_JSON: &str = r#"{
        "fetched_at": 0,
        "providers": {
            "anthropic": [{"id": "claude-sonnet-4-5", "name": "Claude Sonnet 4.5",
                           "cost": {"input": 3.0, "output": 15.0},
                           "limit": {"context": 200000, "output": 8192}}],
            "google": [{"id": "gemini-2.5-pro", "name": "Gemini 2.5 Pro"}]
        }
    }"#;

    #[test]
    fn stale_cache_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE);
        std::fs::write(&path, CACHE_JSON).unwrap();

        let loaded = read_cache(&path).expect("an epoch-old cache must still load");
        assert_eq!(loaded.providers.len(), 2);
        let anthropic = &loaded.providers["anthropic"];
        assert_eq!(anthropic[0].id, "claude-sonnet-4-5");
        assert_eq!(anthropic[0].limit.as_ref().unwrap().context, Some(200_000));
        assert_eq!(loaded.providers["google"][0].id, "gemini-2.5-pro");
    }

    #[test]
    fn cache_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_cache(&dir.path().join("nonexistent.json")).is_none());
    }

    #[test]
    fn cache_unparsable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE);
        std::fs::write(&path, "not json").unwrap();
        assert!(read_cache(&path).is_none());
    }

    #[test]
    fn loaders_read_the_cache_in_the_config_dir() {
        let iso = armadai_core::test_support::IsolatedConfigDir::enter();
        std::fs::write(iso.config_dir().join(CACHE_FILE), CACHE_JSON).unwrap();

        let google = load_models("google").expect("google is cached");
        assert_eq!(google.len(), 1);
        assert_eq!(google[0].id, "gemini-2.5-pro");
        assert!(load_models("unknown").is_none());

        let all = load_all_providers_cached().expect("cache present");
        assert!(all.contains_key("anthropic") && all.contains_key("google"));
    }
}
