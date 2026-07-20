//! Custom registry sources configuration and resolution.
//!
//! ArmadAI ships with default registry URLs baked into `registry::sync`
//! (`DEFAULT_REGISTRY_URL`), `skills_registry::sync` (`default_skill_sources`),
//! and `model_registry::fetch` (`MODELS_DEV_URL`). This module is the socle
//! (B2 Lot A / Task 1) that lets users and projects declare *additional*
//! registry sources via a `registries:` section in
//! `~/.config/armadai/registries.yaml` (user) and/or `armadai.yaml`
//! (project). Wiring this into the actual registries (Task 2) and exposing
//! it via the CLI (Task 3) are handled separately.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::core::config::registries_config_path;

/// A single custom registry source.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RegistrySource {
    pub url: String,
}

/// Custom registry sources declared by the user or a project, grouped by
/// registry kind. Absent keys deserialize to empty vecs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct RegistriesConfig {
    #[serde(default)]
    pub agents: Vec<RegistrySource>,
    #[serde(default)]
    pub skills: Vec<RegistrySource>,
    #[serde(default)]
    pub models: Vec<RegistrySource>,
    #[serde(default)]
    pub starters: Vec<RegistrySource>,
}

/// Which registry kind is being resolved.
///
/// `starters` intentionally has no variant here yet: starter registry
/// resolution is Lot B and out of scope for this task, even though the
/// `starters` field already exists on [`RegistriesConfig`] for forward
/// compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryKind {
    Agents,
    Skills,
    /// Only constructed when the `providers-api` feature is enabled:
    /// `model_registry::fetch` is the sole consumer, since resolving model
    /// catalog sources is only useful alongside the HTTP fetch that reads
    /// them. Harmless to keep visible in `tui`-only builds, where model
    /// registry fetching is unavailable anyway.
    #[allow(dead_code)]
    Models,
}

impl RegistryKind {
    fn sources(self, config: &RegistriesConfig) -> &[RegistrySource] {
        match self {
            RegistryKind::Agents => &config.agents,
            RegistryKind::Skills => &config.skills,
            RegistryKind::Models => &config.models,
        }
    }
}

/// Load the user-level registries config from
/// `~/.config/armadai/registries.yaml`. Returns [`RegistriesConfig::default`]
/// when the file is absent, unreadable, or fails to parse.
pub fn load_user_registries() -> RegistriesConfig {
    let path: PathBuf = registries_config_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_yaml_ng::from_str(&content).unwrap_or_default(),
        Err(_) => RegistriesConfig::default(),
    }
}

/// Resolve the final ordered list of registry source URLs for `kind`.
///
/// Order: built-in `defaults` first, then user-level custom sources, then
/// project-level custom sources (if any). The result is deduplicated while
/// preserving the position of each URL's first occurrence, so defaults
/// always win the front of the list.
pub fn resolved_sources(
    kind: RegistryKind,
    defaults: &[&str],
    user: &RegistriesConfig,
    project: Option<&RegistriesConfig>,
) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();

    let mut extend = |urls: &mut dyn Iterator<Item = String>| {
        for url in urls {
            if seen.insert(url.clone()) {
                out.push(url);
            }
        }
    };

    extend(&mut defaults.iter().map(|s| (*s).to_string()));
    extend(&mut kind.sources(user).iter().map(|s| s.url.clone()));
    if let Some(project) = project {
        extend(&mut kind.sources(project).iter().map(|s| s.url.clone()));
    }

    out
}

/// Derive a filesystem-safe, collision-resistant cache key for a registry
/// source URL.
///
/// This is the single sanitization scheme shared by every consumer that
/// needs to turn an arbitrary registry source URL into a directory or file
/// name: `registry::sync::source_key` (per-source clone directories) and
/// `model_registry::fetch::source_cache_path` (per-source model catalog
/// cache files) both delegate here. Before this, each maintained its own ad
/// hoc sanitization, neither of which was collision-resistant (see B2 Task 2
/// review, minor finding).
///
/// The key is `<readable-prefix>-<8 hex chars of a hash of the full URL>`.
/// The prefix (a sanitized, truncated version of the URL) exists only for
/// human readability in cache directory listings; the hash suffix — derived
/// from the *whole*, un-sanitized URL — is what actually guarantees
/// uniqueness:
/// - two different URLs that happen to sanitize to the same prefix (e.g.
///   differing only in characters that both get replaced) still get
///   distinct keys;
/// - a URL that sanitizes to a bare `.` or `..` (which would otherwise be an
///   unsafe, path-traversal-shaped directory name) can never produce a key
///   equal to `.`/`..`, since the hash suffix is always appended.
pub fn cache_key(url: &str) -> String {
    let sanitized: String = url
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();

    let prefix = sanitized.trim_matches(|c| c == '.' || c == '_');
    let prefix = if prefix.is_empty() { "src" } else { prefix };

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let hash = hasher.finish() as u32;

    format!("{prefix}-{hash:08x}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrapper {
        registries: RegistriesConfig,
    }

    #[test]
    fn deserializes_full_registries_yaml() {
        let yaml = r#"
registries:
  agents:
    - url: https://example.com/agents.git
  skills:
    - url: https://example.com/skills.git
  models:
    - url: https://example.com/models.json
"#;
        let wrapper: Wrapper = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            wrapper.registries.agents,
            vec![RegistrySource {
                url: "https://example.com/agents.git".to_string()
            }]
        );
        assert_eq!(
            wrapper.registries.skills,
            vec![RegistrySource {
                url: "https://example.com/skills.git".to_string()
            }]
        );
        assert_eq!(
            wrapper.registries.models,
            vec![RegistrySource {
                url: "https://example.com/models.json".to_string()
            }]
        );
        assert!(wrapper.registries.starters.is_empty());
    }

    #[test]
    fn missing_keys_default_to_empty_vecs() {
        let yaml = r#"
registries:
  agents:
    - url: https://example.com/agents.git
"#;
        let wrapper: Wrapper = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(wrapper.registries.agents.len(), 1);
        assert!(wrapper.registries.skills.is_empty());
        assert!(wrapper.registries.models.is_empty());
        assert!(wrapper.registries.starters.is_empty());
    }

    #[test]
    fn default_is_all_empty() {
        let cfg = RegistriesConfig::default();
        assert!(cfg.agents.is_empty());
        assert!(cfg.skills.is_empty());
        assert!(cfg.models.is_empty());
        assert!(cfg.starters.is_empty());
    }

    #[test]
    fn resolved_sources_defaults_only_when_user_and_project_empty() {
        let user = RegistriesConfig::default();
        let result = resolved_sources(
            RegistryKind::Agents,
            &["https://default.example/a"],
            &user,
            None,
        );
        assert_eq!(result, vec!["https://default.example/a".to_string()]);
    }

    #[test]
    fn resolved_sources_merges_and_dedupes_preserving_order() {
        let user = RegistriesConfig {
            agents: vec![
                RegistrySource {
                    url: "https://default.example/a".to_string(),
                }, // duplicate of a default
                RegistrySource {
                    url: "https://user.example/b".to_string(),
                },
            ],
            ..Default::default()
        };
        let project = RegistriesConfig {
            agents: vec![
                RegistrySource {
                    url: "https://user.example/b".to_string(),
                }, // duplicate of a user source
                RegistrySource {
                    url: "https://project.example/c".to_string(),
                },
            ],
            ..Default::default()
        };

        let result = resolved_sources(
            RegistryKind::Agents,
            &["https://default.example/a"],
            &user,
            Some(&project),
        );

        assert_eq!(
            result,
            vec![
                "https://default.example/a".to_string(),
                "https://user.example/b".to_string(),
                "https://project.example/c".to_string(),
            ]
        );
    }

    #[test]
    fn resolved_sources_does_not_cross_contaminate_kinds() {
        let user = RegistriesConfig {
            skills: vec![RegistrySource {
                url: "https://user.example/skills".to_string(),
            }],
            ..Default::default()
        };
        let result = resolved_sources(
            RegistryKind::Agents,
            &["https://default.example/a"],
            &user,
            None,
        );
        assert_eq!(result, vec!["https://default.example/a".to_string()]);
    }

    #[test]
    fn cache_key_is_deterministic() {
        let url = "https://github.com/github/awesome-copilot.git";
        assert_eq!(cache_key(url), cache_key(url));
    }

    #[test]
    fn cache_key_has_no_path_separators() {
        let key = cache_key("https://github.com/github/awesome-copilot.git");
        assert!(!key.contains('/'));
        assert!(!key.contains('\\'));
    }

    #[test]
    fn cache_key_distinguishes_similar_urls() {
        let a = cache_key("https://github.com/acme/registry");
        let b = cache_key("https://github.com/acme/registry-fork");
        let c = cache_key("http://github.com/acme/registry"); // http vs https
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn cache_key_guards_against_dot_dot_traversal() {
        assert_ne!(cache_key(".."), "..");
        assert_ne!(cache_key("."), ".");
        assert_ne!(cache_key("...."), "..");
    }

    #[test]
    fn cache_key_readable_prefix_preserved_for_normal_urls() {
        let key = cache_key("https://github.com/github/awesome-copilot.git");
        assert!(key.contains("github"));
        assert!(key.contains("awesome-copilot"));
    }

    #[test]
    fn load_user_registries_missing_file_returns_default() {
        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var(
                "ARMADAI_CONFIG_DIR",
                "/tmp/armadai-registries-test-nonexistent-dir-xyz",
            );
        }

        let cfg = load_user_registries();
        assert!(cfg.agents.is_empty());
        assert!(cfg.skills.is_empty());
        assert!(cfg.models.is_empty());

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }
}
