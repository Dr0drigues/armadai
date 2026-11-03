//! Pack management: the source cache and its versions.
// Wired by the pack commands (later tasks of the plan).
#![allow(dead_code)]

pub mod cache;
pub mod effective;
pub mod lock;
pub mod referents;
pub mod resolve;
pub mod source;
pub mod status;
pub mod vars;

use std::path::Path;

use anyhow::{bail, ensure};
use armadai_core::registries::{RegistrySource, redact_url};

use self::cache::{Available, valid_name};
use self::lock::{PackLock, Resolved};
use self::referents::Referent;
use crate::starters_registry::sync_warning;

/// What `pack add` and `pack update` leave to the caller: relink the project, then print
/// `lines` (stdout) once the link has reported.
pub struct Done {
    pub relink: bool,
    pub lines: Vec<String>,
    /// What to put back if the link fails (the lock was moved).
    pub undo: Option<Undo>,
}

/// The files a command is about to write, as they are now: put back when the link that
/// follows fails, so that a failed `pack add` or `pack update` changes nothing.
pub struct Undo {
    pack: String,
    version: String,
    files: Vec<(std::path::PathBuf, Option<Vec<u8>>)>,
}

impl Undo {
    fn new(pack: &str, version: &str, paths: &[std::path::PathBuf]) -> anyhow::Result<Undo> {
        let mut files = Vec::new();
        for p in paths {
            let bytes = match std::fs::read(p) {
                Ok(b) => Some(b),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => bail!("{}: {e}", p.display()),
            };
            files.push((p.clone(), bytes));
        }
        Ok(Undo {
            pack: pack.to_string(),
            version: version.to_string(),
            files,
        })
    }

    /// Best effort: a file that cannot be put back is said, not hidden.
    pub fn restore(&self) {
        for (p, bytes) in &self.files {
            let done = match bytes {
                Some(b) => std::fs::write(p, b),
                None => std::fs::remove_file(p),
            };
            if let Err(e) = done
                && !(bytes.is_none() && e.kind() == std::io::ErrorKind::NotFound)
            {
                eprintln!("  warn: could not put {} back: {e}", p.display());
            }
        }
    }

    pub fn message(&self) -> String {
        format!(
            "pack {} {} was not applied: link failed, so .armadai/pack.lock and the referent \
             record are back as they were",
            self.pack, self.version
        )
    }
}

type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

fn source_of(known: &[RegistrySource], url: &str) -> RegistrySource {
    known
        .iter()
        .find(|s| s.url == url)
        .cloned()
        .unwrap_or(RegistrySource {
            url: url.to_string(),
            kind: None,
        })
}

/// A source that does not sync stops the command (its warning line is the message).
fn sync_or_stop(registry: &Path, source: &RegistrySource) -> anyhow::Result<()> {
    cache::sync_source_in(registry, source)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("{}", sync_warning(&source.url, &e).trim_start()))
}

/// The best cached version of `name` in `range` (M14 when none), and the pack's path.
fn resolve_in(
    registry: &Path,
    source: &RegistrySource,
    name: &str,
    range_text: &str,
    in_project_yaml: bool,
) -> anyhow::Result<(Available, String)> {
    let range = resolve::parse_range(range_text).map_err(anyhow::Error::msg)?;
    let pack = cache::versions_in(registry, &source.url).and_then(|v| v.packs.get(name).cloned());
    let all = pack.as_ref().map(|p| p.versions.as_slice()).unwrap_or(&[]);
    let Some(best) = resolve::best(&range, all) else {
        let cached = if all.is_empty() {
            "none".to_string()
        } else {
            all.iter()
                .map(|a| a.version.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        // `pack add` has no `pack.version` to change yet
        let way_out = if in_project_yaml {
            ", or change pack.version in .armadai/project.yaml"
        } else {
            ""
        };
        bail!(
            "no cached version of pack {name} matches {range_text} (cached: {cached}): run \
             `armadai pack sync`{way_out}"
        );
    };
    let path = pack.as_ref().map(|p| p.path.clone()).unwrap_or_default();
    Ok((best.clone(), path))
}

fn lock_of(name: &str, source: &str, a: &Available) -> PackLock {
    PackLock {
        format: 1,
        pack: name.to_string(),
        source: source.to_string(),
        resolved: Resolved {
            version: a.version.clone(),
            tag: a.tag.clone(),
            commit: a.commit.clone(),
            digest: a.digest.clone(),
        },
    }
}

/// The diagnostics of the project as it is now (none when its view does not open).
fn errors_now(root: &Path, registry: &Path, env: Env) -> Vec<armadai_model::Diagnostic> {
    let Ok(view) = effective::open_in(root, effective::Vars::Resolve(env), registry) else {
        return Vec::new();
    };
    view.load_checked().err().unwrap_or_default()
}

/// The project as it would be with `lock`. An already invalid project is refused with its
/// own diagnostics, and so is one the lock would invalidate (M13): nothing is changed, the
/// lock does not move toward a `link` that would fail on diagnostics. (A `link` that fails
/// for another reason, a write, is undone by the caller through [`Undo`].) `retry` is the
/// command to run again.
fn check(
    root: &Path,
    lock: &PackLock,
    registry: &Path,
    env: Env,
    version: &str,
    retry: &str,
) -> anyhow::Result<()> {
    // a directory with no project yet (`pack add` makes one) has nothing to be invalid
    let now = if root.join(".armadai/project.yaml").is_file() {
        errors_now(root, registry, env)
    } else {
        Vec::new()
    };
    if !now.is_empty() {
        for d in &now {
            eprintln!("{}", d.render());
        }
        bail!(
            "the project is already invalid: nothing was changed; fix the errors above, then run \
             `{retry}` again"
        );
    }
    let view = effective::open_with_lock_in(root, lock, effective::Vars::Resolve(env), registry)?;
    if let Err(ds) = view.load_checked() {
        for d in &ds {
            eprintln!("{}", d.render());
        }
        bail!(
            "pack {} {version} would make the project invalid; nothing was changed",
            lock.pack
        );
    }
    Ok(())
}

fn record(config: &Path, root: &Path, lock: &PackLock) -> anyhow::Result<()> {
    referents::record_in(
        config,
        Referent {
            pack: lock.pack.clone(),
            source: lock.source.clone(),
            project: root.to_path_buf(),
        },
    )
}

/// M12, the other projects on the same pack (listed, never touched).
fn others_lines(config: &Path, root: &Path, lock: &PackLock) -> anyhow::Result<Vec<String>> {
    let others = referents::others(config, &lock.pack, &lock.source, root)?;
    if others.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines = vec![format!(
        "Other projects referencing pack {} (not changed; run `armadai pack update` in each):",
        lock.pack
    )];
    for (path, standing) in others {
        let tail = match standing {
            referents::Standing::Present => String::new(),
            referents::Standing::Missing => " (missing)".to_string(),
            referents::Standing::NoLongerReferences => {
                format!(" (no longer references {})", lock.pack)
            }
        };
        lines.push(format!("  {}{tail}", path.display()));
    }
    Ok(lines)
}

/// `pack update` of a project that references a pack (copy mode: a later task).
pub fn update_in(
    root: &Path,
    config: &Path,
    registry: &Path,
    known: &[RegistrySource],
    version: Option<&str>,
    env: Env,
) -> anyhow::Result<Done> {
    let Some(d) = status::declared(root) else {
        if !crate::linker::manifest::installed_packs(root).is_empty() {
            bail!("pack update of a pack installed by `armadai init --pack` is not available yet");
        }
        bail!(
            "this project references no pack and records no installed pack: use \
             `armadai pack add <name>` or `armadai init --pack <name>`"
        );
    };
    let source = source_of(known, &source::real_url_in(&d.source, known)?);
    sync_or_stop(registry, &source)?;
    let (best, path) = resolve_in(
        registry,
        &source,
        &d.name,
        version.unwrap_or(&d.version),
        true,
    )?;
    if version.is_some() {
        let range = resolve::parse_range(&d.version).map_err(anyhow::Error::msg)?;
        ensure!(
            resolve::best(&range, std::slice::from_ref(&best)).is_some(),
            "pack {} {} is outside the range {} of .armadai/project.yaml; nothing was changed: \
             change pack.version there to move to it",
            d.name,
            best.version,
            d.version
        );
    }
    cache::ensure_snapshot_in(registry, &source, &d.name, &path, &best)?;
    let new = lock_of(&d.name, &d.source, &best);
    let old = lock::read(root)?;
    let id12 = status::id12(&best.id()).to_string();
    if old.as_ref().is_some_and(|l| {
        l.pack == new.pack && l.source == new.source && l.resolved.id() == best.id()
    }) {
        record(config, root, &new)?;
        return Ok(Done {
            relink: true,
            lines: vec![format!(
                "Pack {} is up to date: {} ({id12})",
                d.name, best.version
            )],
            undo: None,
        });
    }
    check(
        root,
        &new,
        registry,
        env,
        &best.version,
        "armadai pack update",
    )?;
    let undo = Undo::new(
        &d.name,
        &best.version,
        &[root.join(lock::LOCK), referents::path_in(config)],
    )?;
    lock::write(root, &new)?;
    record(config, root, &new)?;
    let was = old.map_or("(none)".to_string(), |l| l.resolved.version);
    let mut lines = vec![format!(
        "Pack {}: {was} -> {} ({id12}), .armadai/pack.lock updated",
        d.name, best.version
    )];
    lines.extend(others_lines(config, root, &new)?);
    Ok(Done {
        relink: true,
        lines,
        undo: Some(undo),
    })
}

fn single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `project.yaml` with the `pack:` block added at the end of the text (the user's
/// comments stay where they are), or a new one.
fn with_pack(text: Option<&str>, name: &str, source: &str, range: &str) -> String {
    let mut out = match text {
        Some(t) if t.is_empty() || t.ends_with('\n') => t.to_string(),
        Some(t) => format!("{t}\n"),
        None => "schema_version: 1\n".to_string(),
    };
    out.push_str(&format!(
        "pack:\n  name: {name}\n  source: {}\n  version: {}\n",
        single_quoted(source),
        single_quoted(range)
    ));
    out
}

fn write_project_yaml(root: &Path, text: &str) -> anyhow::Result<()> {
    let p = root.join(".armadai/project.yaml");
    let tmp = p.with_extension("yaml.tmp");
    for f in [&p, &tmp] {
        ensure!(
            !std::fs::symlink_metadata(f).is_ok_and(|m| m.file_type().is_symlink()),
            "{} is a symbolic link; it is not written through",
            f.display()
        );
    }
    std::fs::create_dir_all(root.join(".armadai"))?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// `pack add <name>[@<range>]`: `root` is the project, or the directory to make one in.
pub fn add_in(
    root: &Path,
    config: &Path,
    registry: &Path,
    known: &[RegistrySource],
    spec: &str,
    only: Option<&str>,
    env: Env,
) -> anyhow::Result<Done> {
    let (name, given) = match spec.split_once('@') {
        Some((n, r)) => (n, Some(r.trim())),
        None => (spec, None),
    };
    ensure!(
        valid_name(name),
        "pack {name} has an invalid name or version id: {name}"
    );
    let file = root.join(".armadai/project.yaml");
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => bail!("{}: {e}", file.display()),
    };
    if let Some(t) = &text {
        let v = armadai_model::yaml::from_str_manifest(t)
            .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        if let Some(p) = v.get("pack") {
            let there = p.get("name").and_then(|n| n.as_str()).unwrap_or(name);
            bail!(
                ".armadai/project.yaml already references pack {there}: use `armadai pack update`"
            );
        }
    }

    let source = match only {
        Some(url) => {
            let s = source_of(known, &source::real_url_in(url, known)?);
            sync_or_stop(registry, &s)?;
            s
        }
        None => {
            for s in known {
                if let Err(e) = cache::sync_source_in(registry, s) {
                    eprintln!("{}", sync_warning(&s.url, &e));
                }
            }
            let mut holders: Vec<&RegistrySource> = known
                .iter()
                .filter(|s| {
                    cache::versions_in(registry, &s.url).is_some_and(|v| v.packs.contains_key(name))
                })
                .collect();
            match holders.len() {
                0 => bail!(
                    "pack {name} is in none of your sources (registries.yaml): add its source \
                     there, or pass --source <url>"
                ),
                1 => holders.remove(0).clone(),
                _ => bail!(
                    "pack {name} is in several of your sources ({}): pass --source <url>",
                    holders
                        .iter()
                        .map(|s| redact_url(&s.url))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
    };

    let (best, path) = resolve_in(registry, &source, name, given.unwrap_or("latest"), false)?;
    let range = given.map_or_else(|| format!("^{}", best.version), str::to_string);
    cache::ensure_snapshot_in(registry, &source, name, &path, &best)?;
    // what `.armadai/` and the referent record keep: the source without its secrets
    let shown = redact_url(&source.url);
    let new = lock_of(name, &shown, &best);
    check(
        root,
        &new,
        registry,
        env,
        &best.version,
        &format!("armadai pack add {name}"),
    )?;
    let undo = Undo::new(
        name,
        &best.version,
        &[
            root.join(lock::LOCK),
            file.clone(),
            referents::path_in(config),
        ],
    )?;
    // the lock first: a lock without the key is ignored, the key without a lock is not
    lock::write(root, &new)?;
    write_project_yaml(root, &with_pack(text.as_deref(), name, &shown, &range))?;
    record(config, root, &new)?;
    Ok(Done {
        relink: true,
        lines: vec![format!(
            "Added pack {name} {} ({}): .armadai/project.yaml (version: {range}) and .armadai/pack.lock",
            best.version,
            status::id12(&best.id())
        )],
        undo: Some(undo),
    })
}
