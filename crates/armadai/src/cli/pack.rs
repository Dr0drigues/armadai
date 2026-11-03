//! `armadai pack sync`, `status`, `add` and `update`.

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
    /// Build the project on a pack of a source, by reference (cache and lock)
    Add {
        /// `<name>` or `<name>@<range>` (`@latest`, `@1.2.3`, `@^1.3`)
        pack: String,
        /// The source holding the pack, when more than one of yours does
        #[arg(long)]
        source: Option<String>,
    },
    /// Move the pack to the best cached version of its range (or the one given), then link
    Update {
        /// An exact version instead of the range's best
        version: Option<String>,
    },
}

pub fn execute(what: PackCommand) -> anyhow::Result<()> {
    let root = crate::compile::find_project(&std::env::current_dir()?);
    let registry = armadai_core::config::registry_cache_dir();
    match what {
        PackCommand::Sync { source } => sync(root.ok().as_deref(), &registry, source),
        PackCommand::Add { pack, source } => {
            let cwd = std::env::current_dir()?;
            let root = match crate::compile::find_project(&cwd) {
                Ok(r) => r,
                Err(e) if e.to_string().contains("legacy format") => return Err(e),
                Err(_) => cwd,
            };
            let env = |n: &str| std::env::var(n).ok();
            let done = crate::pack::add_in(
                &root,
                &armadai_core::config::config_dir(),
                &registry,
                &effective_starter_sources(),
                &pack,
                source.as_deref(),
                &env,
            )?;
            finish(done)
        }
        PackCommand::Update { version } => {
            let env = |n: &str| std::env::var(n).ok();
            let done = crate::pack::update_in(
                &root?,
                &armadai_core::config::config_dir(),
                &registry,
                &effective_starter_sources(),
                version.as_deref(),
                &env,
            )?;
            finish(done)
        }
        PackCommand::Status => {
            let config = armadai_core::config::config_dir();
            print!("{}", status::report(&root?, &registry, &config)?);
            Ok(())
        }
    }
}

/// Link the project on its new lock, then say what changed.
fn finish(done: crate::pack::Done) -> anyhow::Result<()> {
    if done.relink {
        crate::cli::link::execute(vec![], false, false, None, None, false, false)?;
    }
    for l in done.lines {
        println!("{l}");
    }
    Ok(())
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
    let mut unsynced = BTreeSet::new();
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
                unsynced.insert(s.url.clone());
            }
        }
    }

    if let Some(root) = root
        && declared.is_some()
    {
        failed += check_lock(root, registry, &sources, &synced);
    }
    // a source that failed keeps its old `versions.yaml` (already warned about): no
    // signal read from it, one from a source that just synced stays
    if let Some(root) = root
        && !declared
            .as_ref()
            .is_some_and(|d| unsynced.contains(&d.source))
        && !crate::linker::manifest::installed_packs(root)
            .values()
            .any(|p| unsynced.contains(&p.source))
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
