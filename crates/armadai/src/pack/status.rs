//! What `pack status` shows and what the "newer version" note reads: the lock (or the
//! `packs:` section of the link manifest) against `versions.yaml`. No network, no process.

use std::path::Path;

use anyhow::anyhow;

use super::cache::{self, Available};
use super::resolve::{self, Range};
use super::{lock, referents, source};
use crate::linker::manifest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Reference,
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Newer {
    pub pack: String,
    pub current: String,
    pub available: String,
    pub mode: Mode,
}

/// The `pack:` key of the project's `project.yaml`.
#[derive(Debug, Clone)]
pub struct Declared {
    pub name: String,
    pub source: String,
    pub version: String,
}

/// `None` when there is no `pack:` key or the file is not readable (the loader says why).
pub fn declared(root: &Path) -> Option<Declared> {
    let text = std::fs::read_to_string(root.join(".armadai/project.yaml")).ok()?;
    let v = armadai_model::yaml::from_str_manifest(&text).ok()?;
    let p = v.get("pack")?;
    let field = |k: &str| p.get(k)?.as_str().map(str::to_string);
    Some(Declared {
        name: field("name")?,
        source: field("source")?,
        version: field("version")?,
    })
}

fn cached(registry: &Path, source: &str, pack: &str) -> Vec<Available> {
    (cache::versions_in(registry, source))
        .and_then(|v| v.packs.get(pack).map(|p| p.versions.clone()))
        .unwrap_or_default()
}

/// The version newer than the project's, read from `versions.yaml`: the best of the range
/// when it differs from the lock (reference mode), or the newest one when it differs from
/// the installed version (copy mode, a registry source). `None` otherwise, and for a pack
/// from a folder or the embedded ones (they have no `versions.yaml`).
pub fn newer(root: &Path, registry: &Path) -> Option<Newer> {
    if let Some(d) = declared(root) {
        // the pin is not read: `pack update`, which this note advises, is refused under it
        let lock = lock::read_committed(root).ok()??;
        let range = resolve::parse_range(&d.version).ok()?;
        let all = cached(registry, &source::real_url(&d.source).ok()?, &d.name);
        let best = resolve::best(&range, &all)?;
        return (best.version != lock.resolved.version).then(|| Newer {
            pack: d.name,
            current: lock.resolved.version,
            available: best.version.clone(),
            mode: Mode::Reference,
        });
    }
    manifest::installed_packs(root)
        .into_iter()
        .find_map(|(name, installed)| {
            let current = installed.version?;
            let all = cached(registry, &installed.source, &name);
            let newest = resolve::best(&Range::Latest, &all)?;
            (newest.version != current).then(|| Newer {
                pack: name,
                current,
                available: newest.version.clone(),
                mode: Mode::Copy,
            })
        })
}

/// M4.
pub fn note(n: &Newer) -> String {
    let what = match n.mode {
        Mode::Reference => "locked",
        Mode::Copy => "installed",
    };
    format!(
        "note: pack {} {} is in the cache ({what}: {}): run `armadai pack update`",
        n.pack, n.available, n.current
    )
}

/// First 12 characters of a commit, or of a digest without its `sha256:` prefix.
pub fn id12(id: &str) -> &str {
    let id = (id
        .strip_prefix("sha256:")
        .or_else(|| id.strip_prefix("sha256-")))
    .unwrap_or(id);
    &id[..id.len().min(12)]
}

fn at_of(r: &lock::Resolved) -> String {
    match &r.tag {
        Some(t) => format!("tag {t}, {}", id12(&r.id())),
        None => id12(&r.id()).to_string(),
    }
}

/// M17: the text of `pack status`.
pub fn report(root: &Path, registry: &Path, config: &Path) -> anyhow::Result<String> {
    let mut blocks = Vec::new();
    if let Some(d) = declared(root) {
        let Some(lock) = lock::read_committed(root)? else {
            return Err(anyhow!(super::effective::lock_missing(&d.name)));
        };
        let all = cached(registry, &source::real_url(&d.source)?, &d.name);
        let newest = resolve::best(&Range::Latest, &all);
        let best = resolve::parse_range(&d.version)
            .ok()
            .and_then(|r| resolve::best(&r, &all).map(|a| a.version.clone()));
        let in_cache = match (best, newest) {
            (Some(b), Some(n)) => format!("{b} matches the range (newest: {})", n.version),
            (None, Some(n)) => format!("none matches the range (newest: {})", n.version),
            (_, None) => "nothing (run `armadai pack sync`)".to_string(),
        };
        let r = &lock.resolved;
        let at = at_of(r);
        let pinned = match lock::read_pin(root)? {
            None => String::new(),
            Some(pin) => {
                let r = &pin.resolved;
                let at = at_of(r);
                let gone = match super::effective::snapshot_path(root, &pin, registry) {
                    Ok((dir, _)) if dir.is_dir() => "",
                    _ => " (not in cache: run armadai pack sync)",
                };
                format!("  pinned (worktree): {} ({at}){gone}\n", r.version)
            }
        };
        let mut b = format!(
            "Pack {} (reference)\n  source:    {}\n  range:     {}\n  locked:    {} ({at})\n{pinned}  in cache:  {in_cache}\n",
            d.name,
            source::recorded(&d.source),
            d.version,
            r.version
        );
        let others = referents::others(config, &d.name, &d.source, root)?;
        if !others.is_empty() {
            b.push_str("Projects referencing it:\n");
            for (path, standing) in others {
                let tail = match standing {
                    referents::Standing::Present => String::new(),
                    referents::Standing::Missing => " (missing)".to_string(),
                    referents::Standing::NoLongerReferences => {
                        format!(" (no longer references {})", d.name)
                    }
                };
                b.push_str(&format!("  {}{tail}\n", path.display()));
            }
        }
        blocks.push(b);
    }
    for (name, p) in manifest::installed_packs(root) {
        let installed = match (&p.version, &p.id) {
            (Some(v), Some(id)) => format!("{v} ({})", id12(id)),
            (Some(v), None) => v.clone(),
            (None, _) => "(no version)".to_string(),
        };
        let mut b = format!(
            "Pack {name} (copy)\n  source:    {}\n  installed: {installed}\n",
            source::recorded(&p.source)
        );
        if let Some(n) = resolve::best(&Range::Latest, &cached(registry, &p.source, &name)) {
            b.push_str(&format!("  in cache:  {}\n", n.version));
        }
        blocks.push(b);
    }
    if blocks.is_empty() {
        return Ok(format!("{}\n", super::copy::unknown_text(root)));
    }
    Ok(blocks.join("\n"))
}
