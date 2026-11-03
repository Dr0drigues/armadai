//! `armadai pack sync`, `status`, `add` and `update`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use armadai_core::registries::{RegistrySource, cache_key};
use clap::Subcommand;

use crate::cli::new_wizard::{PackAnswers, ask_pack, pack_answers, plan_pack};
use crate::pack::cache::{self, Available};
use crate::pack::lock::{self, TagState};
use crate::pack::outputs::Outputs;
use crate::pack::{source, status};
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
    /// Create a native pack: guided, minimal (--empty) or from this project (--from-project)
    New {
        name: String,
        /// A minimal pack, no questions
        #[arg(long, conflicts_with = "from_project")]
        empty: bool,
        /// Capture this project's .armadai/ (not the link manifest, state or variables)
        #[arg(long)]
        from_project: bool,
        #[arg(long)]
        description: Option<String>,
        /// Parent folder (default: the current directory)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

pub fn execute(what: PackCommand) -> anyhow::Result<()> {
    if let PackCommand::New {
        name,
        empty,
        from_project,
        description,
        dir,
    } = what
    {
        let parent = match dir {
            Some(d) => d,
            None => std::env::current_dir()?,
        };
        let line = if from_project {
            let cwd = std::env::current_dir()?;
            new_from_project(
                &crate::compile::find_project(&cwd)?,
                &parent,
                &name,
                description,
            )?
        } else {
            let answers = if empty {
                pack_answers(&name, description.as_deref())
            } else {
                ask_pack(&name, description.as_deref())?
            };
            new_minimal(&parent, &answers)?
        };
        println!("{line}");
        return Ok(());
    }
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
            finish(&root, done)
        }
        PackCommand::Update { version } => {
            let env = |n: &str| std::env::var(n).ok();
            let root = root?;
            let done = crate::pack::update_in(
                &root,
                &armadai_core::config::config_dir(),
                &registry,
                &effective_starter_sources(),
                version.as_deref(),
                &env,
            )?;
            finish(&root, done)
        }
        PackCommand::New { .. } => unreachable!("handled above"),
        PackCommand::Status => {
            let config = armadai_core::config::config_dir();
            print!("{}", status::report(&root?, &registry, &config)?);
            Ok(())
        }
    }
}

/// Link the project on its new lock, then say what changed.
fn finish(root: &Path, done: crate::pack::Done) -> anyhow::Result<()> {
    if done.relink {
        // only a command that changed something has anything to put back; what link may
        // write is copied first (its own list of paths, never a folder)
        let outputs = match &done.undo {
            None => None,
            Some(undo) => match crate::cli::link::touched_paths()
                .and_then(|paths| Ok(Outputs::take(root, &paths)?))
            {
                Ok(o) => Some(o),
                Err(e) => {
                    undo.restore();
                    eprintln!("{}", undo.message());
                    return Err(e);
                }
            },
        };
        if let Err(e) = crate::cli::link::execute(vec![], false, false, None, None, false, false) {
            if let Some(undo) = &done.undo {
                // the files first: the record of the pack is read from the link manifest
                if let Some(o) = &outputs {
                    o.restore();
                }
                undo.restore();
                eprintln!("{}", undo.message());
            }
            return Err(e);
        }
    }
    for l in done.lines {
        println!("{l}");
    }
    Ok(())
}

fn sync(root: Option<&Path>, registry: &Path, only: Option<String>) -> anyhow::Result<()> {
    let declared = root.and_then(status::declared);
    // the project records its source masked: the entry of the user's it stands for
    let mut failed = 0;
    let declared_real = declared
        .as_ref()
        .and_then(|d| match source::real_url(&d.source) {
            Ok(u) => Some(u),
            Err(e) => {
                eprintln!("  warn: {e}");
                failed += 1;
                None
            }
        });
    let mut sources = effective_starter_sources();
    // The project's own source is synced too, registered or not.
    let mut unregistered = None;
    if let (Some(d), Some(real)) = (&declared, &declared_real)
        && !sources.iter().any(|s| &s.url == real)
    {
        unregistered = Some(d);
        sources.push(RegistrySource {
            url: real.clone(),
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
    if let (Some(d), Some(real)) = (unregistered, &declared_real)
        && sources.iter().any(|s| &s.url == real)
    {
        eprintln!(
            "note: {} comes from .armadai/project.yaml (pack {}), not from your registries.yaml",
            source::recorded(&d.source),
            d.name
        );
    }

    let mut synced = BTreeSet::new();
    let mut unsynced = BTreeSet::new();
    for s in &sources {
        match cache::sync_source_in(registry, s) {
            Ok(v) => {
                let n: usize = v.packs.values().map(|p| p.versions.len()).sum();
                println!(
                    "Synced {}: {} pack(s), {n} version(s)",
                    source::recorded(&s.url),
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
        && declared_real.is_some()
    {
        failed += check_lock(root, registry, &sources, &synced);
    }
    // a source that failed keeps its old `versions.yaml` (already warned about): no
    // signal read from it, one from a source that just synced stays
    if let Some(root) = root
        && !declared
            .as_ref()
            .is_some_and(|_| declared_real.as_ref().is_some_and(|r| unsynced.contains(r)))
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
    let Ok(real) = source::real_url(&lock.source) else {
        return 0;
    };
    let Some(source) = sources.iter().find(|s| s.url == real) else {
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

/// `pack new --empty` and the guide: the plan of `plan_pack`, written.
fn new_minimal(parent: &Path, a: &PackAnswers) -> anyhow::Result<String> {
    crate::cli::new_target::check_id(&a.name)?;
    let files = plan_pack(a)
        .into_iter()
        .map(|(r, b)| (r, b, false))
        .collect();
    let dir = write_pack(parent, &parent.join(&a.name), &a.name, files)?;
    Ok(format!(
        "Created pack {} in {}: pack.yaml, project.yaml, agents/, skills/, flows/, automations/",
        a.name,
        dir.display()
    ))
}

/// `pack new --from-project`: the white list of the project's `.armadai/`, plus `pack.yaml`.
fn new_from_project(
    root: &Path,
    parent: &Path,
    name: &str,
    description: Option<String>,
) -> anyhow::Result<String> {
    crate::cli::new_target::check_id(name)?;
    if let Some(d) = status::declared(root) {
        anyhow::bail!(
            "this project references pack {}: its .armadai/ is an overlay, not a pack; \
             create the pack from its source instead",
            d.name
        );
    }
    let mut files = crate::cli::native_pack::read_listed(&root.join(".armadai"), true)?;
    let pack_yaml = plan_pack(&pack_answers(name, description.as_deref()))
        .into_iter()
        .find(|(r, _)| r == Path::new("pack.yaml"))
        .map(|(r, b)| (r, b, false));
    files.extend(pack_yaml);
    let count = |dir: &str| {
        (files.iter())
            .filter(|(r, ..)| r.starts_with(dir))
            .filter(|(r, ..)| dir != "skills" || r.ends_with("SKILL.md"))
            .count()
    };
    let (a, s, f, u, t, c) = (
        count("agents"),
        count("skills"),
        count("flows"),
        count("automations"),
        count("templates"),
        count("commands"),
    );
    let dir = write_pack(parent, &root.join(".armadai"), name, files)?;
    Ok(format!(
        "Created pack {name} in {} from .armadai/: {a} agent(s), {s} skill(s), {f} flow(s), \
         {u} automation(s), {t} template(s), {c} command(s) \
         (link-manifest.yaml, state/, pack.lock and the vars files left out)",
        dir.display()
    ))
}

/// Validate the files as a pack (diagnostics placed on `source`, where they come from),
/// then write them in a new folder `parent/name`. Nothing is written when the pack is
/// invalid; a failed write removes what this call created (the folder and the parents it
/// had to make).
fn write_pack(
    parent: &Path,
    source: &Path,
    name: &str,
    files: Vec<(PathBuf, Vec<u8>, bool)>,
) -> anyhow::Result<PathBuf> {
    let dir = parent.join(name);
    if std::fs::symlink_metadata(&dir).is_ok() {
        anyhow::bail!(
            "{} already exists: choose another name, or --dir",
            dir.display()
        );
    }
    let pack = crate::cli::native_pack::PackFiles {
        files,
        ignored: Vec::new(),
    };
    let (_tmp, ds) = crate::cli::native_pack::stage(source, &pack)?;
    if armadai_model::has_errors(&ds) {
        for d in &ds {
            eprintln!("{}", d.render());
        }
        anyhow::bail!("pack '{name}' is invalid; nothing was written");
    }
    write_all(&dir, &pack.files)?;
    Ok(dir)
}

/// Write `files` under `dir`; on failure remove what this call created: `dir` and the
/// parents it had to make, never a folder that was there before.
fn write_all(dir: &Path, files: &[(PathBuf, Vec<u8>, bool)]) -> anyhow::Result<()> {
    use anyhow::Context;
    // the outermost folder this call creates
    let top = dir
        .ancestors()
        .take_while(|a| std::fs::symlink_metadata(a).is_err())
        .last()
        .unwrap_or(dir)
        .to_path_buf();
    let built = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        for (rel, bytes, exec) in files {
            let at = dir.join(rel);
            std::fs::create_dir_all(at.parent().context("a file has a parent")?)?;
            std::fs::write(&at, bytes).with_context(|| format!("writing {}", at.display()))?;
            #[cfg(unix)]
            if *exec {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&at)?.permissions().mode();
                std::fs::set_permissions(&at, std::fs::Permissions::from_mode(mode | 0o111))?;
            }
            #[cfg(not(unix))]
            let _ = exec;
        }
        Ok(())
    })();
    if let Err(e) = built {
        let _ = std::fs::remove_dir_all(&top);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_write_removes_the_folders_it_created_and_only_those() {
        let t = tempfile::tempdir().unwrap();
        let kept = t.path().join("kept");
        std::fs::create_dir(&kept).unwrap();
        let dir = kept.join("out/x/pack");
        // `a` is a file, so writing `a/b` fails after the folders and `a` exist
        let files = vec![
            (PathBuf::from("a"), b"1".to_vec(), false),
            (PathBuf::from("a/b"), b"2".to_vec(), false),
        ];
        assert!(write_all(&dir, &files).is_err());
        assert!(kept.is_dir(), "a folder that was there stays");
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 0);
    }
}
