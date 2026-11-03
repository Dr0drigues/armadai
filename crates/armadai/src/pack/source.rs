//! A pack's source as `.armadai/` records it, and as the user's `registries.yaml` holds it.
//!
//! Nothing secret is ever written under `.armadai/` (versioned) nor to
//! `pack-referents.yaml`: they keep the source in the form `redact_url` shows. The cache
//! is keyed by the whole URL (`cache_key`), so the masked form names another folder:
//! every reader of a recorded source goes through [`real_url`] first.

use armadai_core::registries::{RegistrySource, redact_url};

use crate::starters_registry::effective_starter_sources;

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
