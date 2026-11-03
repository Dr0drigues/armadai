//! A pack's source as `.armadai/` records it, and as the user's `registries.yaml` holds it.
//!
//! Nothing secret is ever written under `.armadai/` (versioned) nor to
//! `pack-referents.yaml`: they keep the source in the form `redact_url` shows. The cache
//! is keyed by the whole URL (`cache_key`), so the masked form names another folder:
//! every reader of a recorded source goes through [`real_url`] first.

use armadai_core::registries::{RegistrySource, redact_url};

use crate::starters_registry::effective_starter_sources;

/// Whether `url` carries a secret: a query, a password (`user:pass@`), or any userinfo on
/// an `http(s)` URL (`https://TOKEN@host`). An SSH login (`git@host:org/repo`,
/// `ssh://git@host/…`) is not one. An `@` after the authority is ambiguous: a secret.
fn carries_secret(url: &str) -> bool {
    if url.contains('?') {
        return true;
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => (String::new(), url),
    };
    let (authority, tail) = rest.split_at(rest.find(['/', '#']).unwrap_or(rest.len()));
    if tail.contains('@') {
        return true;
    }
    match authority.rfind('@') {
        Some(at) => matches!(scheme.as_str(), "http" | "https") || authority[..at].contains(':'),
        None => false,
    }
}

/// The source as `.armadai/` and the referent record write it: as it is, unless it carries
/// a secret, then as `redact_url` shows it (the user's `registries.yaml` keeps the real one).
pub fn recorded(url: &str) -> String {
    if carries_secret(url) {
        redact_url(url)
    } else {
        url.to_string()
    }
}

/// A URL `redact_url` has masked (it holds `***`): it cannot be synced as it stands.
fn is_masked(url: &str) -> bool {
    url.contains("***")
}

/// The URL of the user's own entry that `recorded` stands for; a URL with nothing masked
/// stands for itself, and `registries.yaml` is not read.
pub fn real_url(recorded: &str) -> anyhow::Result<String> {
    if !is_masked(recorded) {
        return Ok(recorded.to_string());
    }
    real_url_in(recorded, &effective_starter_sources())
}

/// [`real_url`] against the given entries.
pub fn real_url_in(recorded: &str, known: &[RegistrySource]) -> anyhow::Result<String> {
    if !is_masked(recorded) {
        return Ok(recorded.to_string());
    }
    let hits: Vec<&RegistrySource> = known
        .iter()
        .filter(|s| redact_url(&s.url) == recorded)
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.url.clone()),
        [] => anyhow::bail!(
            "the source {recorded} matches none of your sources (registries.yaml): add it \
             there, with its credentials"
        ),
        many => anyhow::bail!(
            "the source {recorded} matches {} entries of your registries.yaml, which differ \
             only by their credentials: keep one",
            many.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_carries_a_secret_is_masked() {
        for plain in [
            "git@example.com:org/packs.git",
            "ssh://git@example.com/org/packs.git",
            "https://example.com/org/packs.git",
            "https://example.com/packs.tar.gz",
            "file:///tmp/origin",
        ] {
            assert_eq!(recorded(plain), plain);
        }
        for (secret, shown) in [
            (
                "https://TOKEN@github.com/org/p.git",
                "https://***@github.com/org/p.git",
            ),
            (
                "https://user:pass@example.com/p.git",
                "https://***@example.com/p.git",
            ),
            (
                "ssh://user:pass@example.com/p.git",
                "ssh://***@example.com/p.git",
            ),
            (
                "https://example.com/p.tar.gz?sig=abc",
                "https://example.com/p.tar.gz?***",
            ),
            (
                "user:tok@example.com:org/p.git",
                "***@example.com:org/p.git",
            ),
        ] {
            assert_eq!(recorded(secret), shown, "{secret}");
        }
    }
}
