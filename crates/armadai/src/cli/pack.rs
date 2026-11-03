//! `armadai pack sync` and `armadai pack status`.

use std::collections::BTreeSet;
use std::path::Path;

use armadai_core::registries::{RegistrySource, cache_key, redact_url};
use clap::Subcommand;

use crate::pack::cache::{self, Available};
use crate::pack::lock::{self, TagState};
use crate::pack::status;
use crate::starters_registry::{effective_starter_sources, sync_warning};

#[derive(Subcommand)]
pub enum PackCommand {
    /// Refresh the pack sources now (network), and the snapshot the project locks
    Sync {
        /// Only this source
        #[arg(long)]
        source: Option<String>,
    },
    /// Show the project's pack, its locked and newest cached versions (no network)
    Status,
}

pub fn execute(what: PackCommand) -> anyhow::Result<()> {
    let root = crate::compile::find_project(&std::env::current_dir()?);
    let registry = armadai_core::config::registry_cache_dir();
    match what {
        PackCommand::Sync { source } => sync(root.ok().as_deref(), &registry, source),
        PackCommand::Status => {
            let config = armadai_core::config::config_dir();
            print!("{}", status::report(&root?, &registry, &config)?);
            Ok(())
        }
    }
}

fn sync(root: Option<&Path>, registry: &Path, only: Option<String>) -> anyhow::Result<()> {
    let declared = root.and_then(status::declared);
    let mut sources = effective_starter_sources();
    // The project's own source is synced too, registered or not.
    let mut unregistered = None;
    if let Some(d) = &declared
        && !sources.iter().any(|s| s.url == d.source)
    {
        unregistered = Some(d);
        sources.push(RegistrySource {
            url: d.source.clone(),
            kind: None,
        });
    }
    if let Some(url) = &only {
        sources.retain(|s| &s.url == url);
        if sources.is_empty() {
            sources.push(RegistrySource {
                url: url.clone(),
                kind: None,
            });
        }
    }
    if let Some(d) = unregistered
        && sources.iter().any(|s| s.url == d.source)
    {
        eprintln!(
            "note: {} comes from .armadai/project.yaml (pack {}), not from your registries.yaml",
            redact_url(&d.source),
            d.name
        );
    }

    let mut failed = 0;
    let mut synced = BTreeSet::new();
    for s in &sources {
        match cache::sync_source_in(registry, s) {
            Ok(v) => {
                let n: usize = v.packs.values().map(|p| p.versions.len()).sum();
                println!(
                    "Synced {}: {} pack(s), {n} version(s)",
                    redact_url(&s.url),
                    v.packs.len()
                );
                synced.insert(s.url.clone());
            }
            Err(e) => {
                eprintln!("{}", sync_warning(&s.url, &e));
                failed += 1;
            }
        }
    }

    if let Some(root) = root
        && declared.is_some()
    {
        failed += check_lock(root, registry, &sources, &synced);
    }
    if let Some(root) = root
        && let Some(n) = status::newer(root, registry)
    {
        eprintln!("{}", status::note(&n));
    }
    if failed > 0 {
        anyhow::bail!("{failed} source(s) or check(s) did not complete: see the warnings above");
    }
    Ok(())
}

/// The locked snapshot (M5c's cure) and the lock's tag (M3, M3b), once its source is fresh.
/// The lock is never written. Returns the number of failures, each already warned.
fn check_lock(
    root: &Path,
    registry: &Path,
    sources: &[RegistrySource],
    synced: &BTreeSet<String>,
) -> usize {
    let lock = match lock::read(root) {
        Ok(Some(l)) => l,
        Ok(None) => return 0,
        Err(e) => {
            eprintln!("  warn: {e}");
            return 1;
        }
    };
    let Some(source) = sources.iter().find(|s| s.url == lock.source) else {
        return 0;
    };
    if !synced.contains(&source.url) {
        return 0;
    }
    let r = &lock.resolved;
    let mut failed = 0;
    let path = (cache::versions_in(registry, &source.url))
        .and_then(|v| v.packs.get(&lock.pack).map(|p| p.path.clone()));
    if let Some(path) = path {
        let at = Available {
            version: r.version.clone(),
            tag: r.tag.clone(),
            commit: r.commit.clone(),
            digest: r.digest.clone(),
        };
        if let Err(e) = cache::ensure_snapshot_in(registry, source, &lock.pack, &path, &at) {
            eprintln!("  warn: {e}");
            failed += 1;
        }
    }
    let clone = registry.join("starters").join(cache_key(&source.url));
    match lock::tag_moved(&clone, &lock.pack, r) {
        Ok(Some(TagState::Moved(now))) => eprintln!(
            "  warn: tag {} of pack {} now points to {}, .armadai/pack.lock says {}: the lock is kept (link uses its commit)",
            r.tag.as_deref().unwrap_or_default(),
            lock.pack,
            status::id12(&now),
            status::id12(&r.id())
        ),
        Ok(Some(TagState::Gone)) => eprintln!(
            "  warn: tag {} of pack {} no longer exists, .armadai/pack.lock keeps {}",
            r.tag.as_deref().unwrap_or_default(),
            lock.pack,
            status::id12(&r.id())
        ),
        Ok(None) => {}
        Err(e) => {
            eprintln!("  warn: {e}");
            failed += 1;
        }
    }
    failed
}
