//! Custom registry sources configuration and resolution.
//!
//! ArmadAI ships with default skill sources baked into
//! `skills_registry::sync` (`default_sources`). This module lets users and
//! projects declare *additional* registry sources: top-level `skills:`,
//! `starters:`… lists in `~/.config/armadai/registries.yaml` (user; that file
//! has no `registries:` wrapper) and/or a `registries:` section of a legacy
//! project config (`armadai.yaml`, `.armadai/config.yaml`; the new
//! `.armadai/project.yaml` has none, see #479). Consumed by
//! `skills_registry::cache` (skills) and `starters_registry` (starter packs).

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::registries_config_path;

/// Delivery kind of a registry source (transport-agnostic fetch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Git,
    Archive,
}

/// A single custom registry source.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RegistrySource {
    pub url: String,
    /// Delivery kind. Absent = inferred from the URL (see `resolved_kind`).
    #[serde(default)]
    pub kind: Option<SourceKind>,
}

impl RegistrySource {
    /// The effective delivery kind: explicit `kind`, else inferred from the URL.
    pub fn resolved_kind(&self) -> SourceKind {
        if let Some(k) = self.kind {
            return k;
        }
        let u = self.url.to_lowercase();
        // A presigned archive URL ends in `?token=…`: the extension is before it.
        let u = u.split(['?', '#']).next().unwrap_or_default();
        if u.ends_with(".tar.gz") || u.ends_with(".tgz") || u.ends_with(".zip") {
            SourceKind::Archive
        } else {
            SourceKind::Git
        }
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryKind {
    Skills,
}

impl RegistryKind {
    fn sources(self, config: &RegistriesConfig) -> &[RegistrySource] {
        match self {
            RegistryKind::Skills => &config.skills,
        }
    }
}

/// Load the user-level registries config from
/// `~/.config/armadai/registries.yaml`. Returns [`RegistriesConfig::default`]
/// when the file is absent, and also (with a warning) when it does not read.
pub fn load_user_registries() -> RegistriesConfig {
    let path: PathBuf = registries_config_path();
    crate::config::read_user_yaml(&path)
        .unwrap_or_else(|e| {
            crate::config::warn_once(format!("{e:#}; using the built-in registries only"));
            None
        })
        .unwrap_or_default()
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

/// `url` as shown in messages and cache directory names: without credentials.
///
/// The authority's userinfo (`user:pass@`) becomes `***@`, the query `?***`. An `@`
/// after the authority is ambiguous (a path `@`, or an unencoded password holding
/// `/`, `?` or `#`), so everything after the scheme is masked then: the result never
/// shows a secret, nor a host that is not the one contacted.
pub fn redact_url(url: &str) -> String {
    // Without a scheme (`user:tok@host:org/repo.git`, `host/a.tar.gz?sig=…`), the
    // same rules apply to the whole string.
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), url),
    };
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    if tail.contains('@') {
        return format!("{scheme}***");
    }
    let shown = match authority.rfind('@') {
        Some(at) => format!("{scheme}***@{}{tail}", &authority[at + 1..]),
        None => url.to_string(),
    };
    match shown.split_once('?') {
        Some((base, _)) => format!("{base}?***"),
        None => shown,
    }
}

/// `text` with the credentials of `url` masked: `url` itself as [`redact_url`] shows
/// it, then its userinfo and query wherever else they appear (git rewrites URLs).
pub fn mask_url_secrets(text: &str, url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let mut text = text.replace(url, &redact_url(url));
    if let Some(at) = rest.rfind('@') {
        text = text.replace(&rest[..=at], "***@");
    }
    if let Some((_, query)) = rest.split_once('?')
        && !query.is_empty()
    {
        text = text.replace(&format!("?{query}"), "?***");
        // shortcut: a short query (`?a`) is only masked after its `?`, so that common
        // words of the message stay readable; a token is longer than 8 characters.
        if query.len() >= 8 {
            text = text.replace(query, "***");
        }
    }
    text
}

/// Derive a filesystem-safe, collision-resistant cache key for a registry
/// source URL.
///
/// This is the single sanitization scheme shared by every consumer that
/// needs to turn an arbitrary registry source URL into a directory or file
/// name: the starter registry cache (`starters_registry::source_cache_dir`)
/// delegates here. Before this, each maintained its own ad
/// hoc sanitization, neither of which was collision-resistant (see B2 Task 2
/// review, minor finding).
///
/// The key is `<readable-prefix>` (a sanitized, truncated version of the URL as
/// [`redact_url`] shows it, without its query: no credential or token lands in a
/// directory name) followed by a hash of the *whole*, original URL:
/// - a plain URL (see [`is_plain_url`]) keeps the key it always had,
///   `<prefix>-<8 hex chars>`: its prefix alone tells it apart from every other
///   URL, so the short hash only has to be stable;
/// - any other URL gets `<prefix>~<32 hex chars of its SHA-256>`: its prefix may
///   be shared (`https` for every URL masked as `https://***`), and a 32-bit hash
///   can be forged to collide in seconds, which would let one source overwrite
///   another's cache.
///
/// As a consequence:
/// - two different URLs that happen to sanitize to the same prefix (e.g.
///   differing only in characters that both get replaced) still get
///   distinct keys;
/// - a URL that sanitizes to a bare `.` or `..` (which would otherwise be an
///   unsafe, path-traversal-shaped directory name) can never produce a key
///   equal to `.`/`..`, since the hash suffix is always appended.
pub fn cache_key(url: &str) -> String {
    let sanitized: String = redact_url(url)
        .split('?')
        .next()
        .unwrap_or_default()
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

    if is_plain_url(url) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut hasher);
        let hash = hasher.finish() as u32;
        return format!("{prefix}-{hash:08x}");
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}~{hex}")
}

/// A URL that [`cache_key`]'s readable prefix maps one-to-one: a lowercase
/// `scheme://`, then only `[A-Za-z0-9.-]` and `/` (each `/` becomes `_`, nothing else
/// changes), starting and ending with a character the prefix does not trim, and short
/// enough not to be truncated.
fn is_plain_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    !scheme.is_empty()
        && scheme.chars().all(|c| c.is_ascii_lowercase())
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '/'))
        && rest.starts_with(|c: char| c.is_ascii_alphanumeric())
        && rest.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '-')
        && url.len() <= 60
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
        let wrapper: Wrapper = armadai_model::yaml::from_str(yaml).unwrap();
        assert_eq!(
            wrapper.registries.agents,
            vec![RegistrySource {
                url: "https://example.com/agents.git".to_string(),
                kind: None
            }]
        );
        assert_eq!(
            wrapper.registries.skills,
            vec![RegistrySource {
                url: "https://example.com/skills.git".to_string(),
                kind: None
            }]
        );
        assert_eq!(
            wrapper.registries.models,
            vec![RegistrySource {
                url: "https://example.com/models.json".to_string(),
                kind: None
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
        let wrapper: Wrapper = armadai_model::yaml::from_str(yaml).unwrap();
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
            RegistryKind::Skills,
            &["https://default.example/a"],
            &user,
            None,
        );
        assert_eq!(result, vec!["https://default.example/a".to_string()]);
    }

    #[test]
    fn resolved_sources_merges_and_dedupes_preserving_order() {
        let user = RegistriesConfig {
            skills: vec![
                RegistrySource {
                    url: "https://default.example/a".to_string(),
                    kind: None,
                }, // duplicate of a default
                RegistrySource {
                    url: "https://user.example/b".to_string(),
                    kind: None,
                },
            ],
            ..Default::default()
        };
        let project = RegistriesConfig {
            skills: vec![
                RegistrySource {
                    url: "https://user.example/b".to_string(),
                    kind: None,
                }, // duplicate of a user source
                RegistrySource {
                    url: "https://project.example/c".to_string(),
                    kind: None,
                },
            ],
            ..Default::default()
        };

        let result = resolved_sources(
            RegistryKind::Skills,
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
    fn cache_key_never_holds_credentials_and_keeps_urls_apart() {
        for (a, b) in [
            (
                "https://example.com/acme/starters.tar.gz",
                "https://u:s3cret@example.com/acme/starters.tar.gz",
            ),
            (
                "https://cdn.example/acme/starters.tar.gz",
                "https://evil.example/x@cdn.example/acme/starters.tar.gz",
            ),
            (
                "https://example.com/a.tar.gz?token=s3cret",
                "https://example.com/a.tar.gz?token=other",
            ),
            (
                "https://u:pa/ss@example.com/a.tar.gz",
                "https://u:pa/zz@example.com/a.tar.gz",
            ),
        ] {
            assert_ne!(cache_key(a), cache_key(b), "{a} and {b} share a cache dir");
            for key in [cache_key(a), cache_key(b)] {
                for secret in ["s3cret", "other", "pa_ss", "pa_zz", "evil"] {
                    assert!(!key.contains(secret), "{key} holds {secret}");
                }
            }
        }
    }

    #[test]
    fn cache_key_keeps_forged_collisions_apart_by_construction() {
        // A 32-bit collision found by brute force in review: same short key before.
        let npm = "https://registry.npmjs.org/@acme/starters/-/starters-1.0.0.tgz";
        let evil = "https://evil.example/x@y.tgz?n=7220038856";
        assert_ne!(cache_key(npm), cache_key(evil));
        let suffix = |key: &str| key.rsplit_once('~').map(|(_, h)| h.len());
        for tainted in [
            npm,
            evil,
            "https://u:p@example.com/a.tar.gz",
            "https://example.com/a.tar.gz?x=1",
            "https://example.com/a#x",
            "https://example.com/a_b",
            "https://example.com/a/",
            "https://example.com/a.",
            "https:///example.com/a",
            "HTTPS://example.com/a",
            "/srv/git/starters",
            "git@github.com:org/repo.git",
            "https://example.com/a-very-long-path/that-goes/past-sixty-characters",
        ] {
            assert_eq!(suffix(&cache_key(tainted)), Some(32), "{tainted}");
        }
        // Plain URLs keep their `-<8 hex>` key, so existing caches stay valid.
        for plain in [
            "https://github.com/github/awesome-copilot.git",
            "https://example.com/a-b/c.tar.gz",
        ] {
            let key = cache_key(plain);
            assert!(!key.contains('~'), "{key}");
            assert_eq!(key.rsplit_once('-').map(|(_, h)| h.len()), Some(8), "{key}");
        }
    }

    #[test]
    fn redact_url_hides_credentials_and_never_shows_a_wrong_host() {
        for (url, shown) in [
            (
                "https://example.com/a.tar.gz",
                "https://example.com/a.tar.gz",
            ),
            ("git@github.com:org/repo.git", "***@github.com:org/repo.git"),
            ("https://TOKEN@example.com/a", "https://***@example.com/a"),
            ("https://u:p@ss@example.com/a", "https://***@example.com/a"),
            (
                "https://example.com/a?token=s3cret",
                "https://example.com/a?***",
            ),
            // an `@` after the authority: path or unencoded password, all masked
            ("https://evil.example/x@cdn.example/a", "https://***"),
            ("https://u:pa/ss@example.com/a", "https://***"),
            ("https://host/a@b", "https://***"),
            // without a scheme
            ("u:tok@h:o/r.git", "***@h:o/r.git"),
            ("127.0.0.1/v.tar.gz?sig=t0kenF", "127.0.0.1/v.tar.gz?***"),
            ("/srv/git/starters", "/srv/git/starters"),
        ] {
            assert_eq!(redact_url(url), shown, "{url}");
        }
    }

    #[test]
    fn mask_url_secrets_masks_rewritten_urls_too() {
        let url = "https://u:s3cret@example.com/x.git?token=t0ken";
        // git shows the URL rewritten: without its query, and the query on its own.
        let text =
            format!("clone {url}: fatal: 'https://u:s3cret@example.com/x.git/' (token=t0ken)");
        let masked = mask_url_secrets(&text, url);
        assert!(
            !masked.contains("s3cret") && !masked.contains("t0ken"),
            "{masked}"
        );
        assert!(
            masked.starts_with("clone https://***@example.com/x.git?***: "),
            "{masked}"
        );
    }

    #[test]
    fn mask_url_secrets_leaves_a_short_query_alone_in_prose() {
        assert_eq!(
            mask_url_secrets("fatal: a?a is a bad url", "https://h/x?a"),
            "fatal: a?*** is a bad url"
        );
    }

    #[test]
    fn resolved_kind_ignores_the_query() {
        let source = |url: &str| RegistrySource {
            url: url.to_string(),
            kind: None,
        };
        assert_eq!(
            source("https://e.com/i.tar.gz?token=x").resolved_kind(),
            SourceKind::Archive
        );
        assert_eq!(
            source("https://e.com/i.zip#top").resolved_kind(),
            SourceKind::Archive
        );
        assert_eq!(
            source("https://e.com/r.git?x=y").resolved_kind(),
            SourceKind::Git
        );
    }

    /// A plain URL keeps the cache directory earlier versions used: no new sync.
    #[test]
    fn cache_key_of_a_plain_url_is_unchanged() {
        assert_eq!(
            cache_key("https://github.com/Dr0drigues/armadai-starters"),
            "https___github.com_Dr0drigues_armadai-starters-7d70fef7"
        );
    }

    #[test]
    fn cache_key_readable_prefix_preserved_for_normal_urls() {
        let key = cache_key("https://github.com/github/awesome-copilot.git");
        assert!(key.contains("github"));
        assert!(key.contains("awesome-copilot"));
    }

    #[test]
    fn load_user_registries_missing_file_returns_default() {
        let _guard = crate::test_support::env_lock();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: serialised via `env_lock()`; restored at end of test.
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

    #[test]
    fn test_source_kind_deserialize_and_infer() {
        let yaml = r#"
starters:
  - url: "https://github.com/me/starters.git"
  - url: "https://x.com/p.tar.gz"
  - url: "https://interne/p"
    kind: archive
"#;
        let c: RegistriesConfig = armadai_model::yaml::from_str(yaml).unwrap();
        assert_eq!(c.starters[0].resolved_kind(), SourceKind::Git); // .git → git
        assert_eq!(c.starters[1].resolved_kind(), SourceKind::Archive); // .tar.gz → archive
        assert_eq!(c.starters[2].resolved_kind(), SourceKind::Archive); // explicit override
        assert_eq!(c.starters[2].kind, Some(SourceKind::Archive));
    }

    #[test]
    fn test_infer_defaults_to_git() {
        let s = RegistrySource {
            url: "https://host/repo".to_string(),
            kind: None,
        };
        assert_eq!(s.resolved_kind(), SourceKind::Git);
    }
}
