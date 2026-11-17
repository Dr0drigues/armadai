//! Pack management: the source cache and its versions.
// Wired by the pack commands (later tasks of the plan).
#![allow(dead_code)]

pub mod cache;
pub mod copy;
pub mod effective;
pub mod lock;
pub mod outputs;
pub mod referents;
pub mod resolve;
pub mod source;
pub mod standards;
pub mod status;
pub mod vars;

use std::path::Path;

use anyhow::{bail, ensure};
use armadai_core::registries::RegistrySource;

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
    /// `pack` or `standards`: what the message calls the thing that was not applied.
    what: &'static str,
    pack: String,
    version: String,
    files: Vec<(std::path::PathBuf, Option<Vec<u8>>)>,
    /// Copy mode: the folders the update created, and the project with the pack's record
    /// as it was in the link manifest.
    dirs: Vec<std::path::PathBuf>,
    record: Option<(std::path::PathBuf, crate::linker::manifest::InstalledPack)>,
}

impl Undo {
    fn new(
        what: &'static str,
        pack: &str,
        version: &str,
        paths: &[std::path::PathBuf],
    ) -> anyhow::Result<Undo> {
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
            what,
            pack: pack.to_string(),
            version: version.to_string(),
            files,
            dirs: Vec::new(),
            record: None,
        })
    }

    /// Best effort: a file that cannot be put back is said, not hidden.
    pub fn restore(&self) {
        for (p, bytes) in &self.files {
            let done = match bytes {
                // a folder emptied by the update was removed with its last file
                Some(b) => p
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(p, b)),
                None => std::fs::remove_file(p),
            };
            if let Err(e) = done
                && !(bytes.is_none() && e.kind() == std::io::ErrorKind::NotFound)
            {
                eprintln!("  warn: could not put {} back: {e}", p.display());
            }
        }
        for d in self.dirs.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
        if let Some((root, old)) = &self.record
            && let Err(e) =
                crate::linker::manifest::write_installed_pack(root, &self.pack, old.clone())
        {
            eprintln!(
                "  warn: could not put the record of pack {} back: {e}",
                self.pack
            );
        }
    }

    pub fn message(&self) -> String {
        if self.record.is_some() {
            return format!(
                "pack {} {} was not applied: link failed, so the files of .armadai/, the record \
                 of the pack and the files link had written are back as they were",
                self.pack, self.version
            );
        }
        format!(
            "{} {} {} was not applied: link failed, so .armadai/pack.lock, the referent \
             record and the files link had written are back as they were",
            self.what, self.pack, self.version
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
    what: &str,
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
        let way_out = match (in_project_yaml, what) {
            (false, _) => String::new(),
            (true, "pack") => ", or change pack.version in .armadai/project.yaml".to_string(),
            (true, _) => {
                format!(", or change the version of {what} {name} in .armadai/project.yaml")
            }
        };
        bail!(
            "no cached version of {what} {name} matches {range_text} (cached: {cached}): run \
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

/// Refuse a project that is invalid already (its diagnostics printed): nothing is changed,
/// and the update is not blamed for errors it did not bring. A directory with no project
/// yet (`pack add` makes one) has nothing to be invalid.
fn refuse_invalid(root: &Path, registry: &Path, env: Env, retry: &str) -> anyhow::Result<()> {
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
    Ok(())
}

/// The project as it would be with `lock`. An already invalid project is refused with its
/// own diagnostics, and so is one the lock would invalidate (M13): nothing is changed, the
/// lock does not move toward a `link` that would fail on diagnostics. (A `link` that fails
/// for another reason, a write, is undone by the caller through [`Undo`].) `retry` is the
/// command to run again. Nor does the `Undo` cover the process being interrupted (Ctrl-C)
/// between the writes and the end of the link.
fn check(
    root: &Path,
    locked: &lock::LockFile,
    adding: Option<&str>,
    registry: &Path,
    env: Env,
    subject: &str,
    retry: &str,
) -> anyhow::Result<()> {
    refuse_invalid(root, registry, env, retry)?;
    let vars = effective::Vars::Resolve(env);
    let view = match adding {
        None => effective::open_with_lock_in(root, locked, vars, registry)?,
        Some(name) => effective::open_adding_standards_in(root, locked, name, vars, registry)?,
    };
    if let Err(ds) = view.load_checked() {
        for d in &ds {
            eprintln!("{}", d.render());
        }
        bail!("{subject} would make the project invalid; nothing was changed");
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

/// M31: `version` must be inside the range of `project.yaml`.
fn in_range(d: &status::Declared, version: &str) -> anyhow::Result<()> {
    let range = resolve::parse_range(&d.version).map_err(anyhow::Error::msg)?;
    let one = Available {
        version: version.trim().to_string(),
        tag: None,
        commit: None,
        digest: None,
    };
    ensure!(
        resolve::best(&range, std::slice::from_ref(&one)).is_some(),
        "pack {} {} is outside the range {} of .armadai/project.yaml; nothing was changed: \
         change pack.version there to move to it",
        d.name,
        one.version,
        d.version
    );
    Ok(())
}

/// `pack update`: a project that references a pack (`pack:`) or standards packs
/// (`standards:`) is moved on its lock, one that installed a pack by copy goes through
/// [`copy::update_in`] (and its standards, if it has any, move after it, under one link).
pub fn update_in(
    root: &Path,
    config: &Path,
    registry: &Path,
    known: &[RegistrySource],
    version: Option<&str>,
    env: Env,
) -> anyhow::Result<Done> {
    let team = status::declared(root);
    let stds = effective::standards_locked_refs(root);
    if team.is_none() && effective::only_unfinished_standards(root) {
        bail!("{}", effective::unfinished_standards("update"));
    }
    // nothing referenced, but the lock still holds standards: the update drops them
    let leftover = team.is_none()
        && stds.is_empty()
        && lock::read_committed(root)
            .ok()
            .flatten()
            .is_some_and(|l| !l.standards.is_empty());
    if team.is_none() && stds.is_empty() && !leftover {
        return copy::update_in(root, registry, known, version, env);
    }
    // before any network access
    if let Some(pin) = lock::read_pin(root)? {
        anyhow::bail!("{}", lock::pinned_refusal(&pin));
    }
    let copied = team.is_none() && !crate::linker::manifest::installed_packs(root).is_empty();
    if version.is_some() && team.is_none() && !copied {
        bail!(
            "pack update <version> moves the pack: of .armadai/project.yaml, and this project \
             has none; armadai pack update alone moves its standards"
        );
    }
    let copy_done = if copied {
        Some(copy::update_in(root, registry, known, version, env)?)
    } else {
        None
    };
    let moved = update_locks(
        root,
        config,
        registry,
        known,
        version.filter(|_| team.is_some()),
        env,
        team.as_ref(),
        &stds,
    );
    let Some(mut done) = copy_done else {
        return moved;
    };
    // the copy is applied already: what follows it failing puts it back
    let moved = match moved {
        Ok(m) => m,
        Err(e) => {
            if let Some(u) = &done.undo {
                u.restore();
            }
            return Err(e);
        }
    };
    done.lines.extend(moved.lines);
    done.undo = match (done.undo.take(), moved.undo) {
        (Some(mut copy), Some(locks)) => {
            copy.files.extend(locks.files);
            Some(copy)
        }
        (copy, locks) => copy.or(locks),
    };
    Ok(done)
}

/// The `pack:` entry (when declared) and each `standards:` entry moved to the best cached
/// version of its range, in one lock, one check and one `Undo`.
#[allow(clippy::too_many_arguments)]
fn update_locks(
    root: &Path,
    config: &Path,
    registry: &Path,
    known: &[RegistrySource],
    version: Option<&str>,
    env: Env,
    team: Option<&status::Declared>,
    stds: &[(String, String, String)],
) -> anyhow::Result<Done> {
    let mut synced = std::collections::BTreeSet::new();
    let mut sync_once = |source: &RegistrySource| -> anyhow::Result<()> {
        if synced.insert(source.url.clone()) {
            sync_or_stop(registry, source)?;
        }
        Ok(())
    };
    let old_file = lock::read_committed(root)?;
    let mut file = old_file.clone().unwrap_or_default();
    // a standards pack the project no longer references leaves the lock
    let (kept, dropped): (Vec<_>, Vec<_>) = std::mem::take(&mut file.standards)
        .into_iter()
        .partition(|e| stds.iter().any(|(n, ..)| *n == e.pack));
    file.standards = kept;
    let mut lines = Vec::new();
    let mut changed = Vec::new();
    // what the first moved entry is called by the message of an undone update
    let mut first: Option<(&'static str, String, String)> = None;
    let mut team_new = None;
    let mut team_lines = Vec::new();
    if let Some(d) = team {
        let source = source_of(known, &source::real_url_in(&d.source, known)?);
        sync_once(&source)?;
        // The range comes first: an exact version outside it is refused cached or not.
        if let Some(v) = version.filter(|v| semver::Version::parse(v.trim()).is_ok()) {
            in_range(d, v)?;
        }
        let (best, path) = resolve_in(
            registry,
            &source,
            "pack",
            &d.name,
            version.unwrap_or(&d.version),
            true,
        )?;
        if version.is_some() {
            in_range(d, &best.version)?;
        }
        cache::ensure_snapshot_in(registry, &source, &d.name, &path, &best)?;
        let new = lock_of(&d.name, &d.source, &best);
        let id12 = status::id12(&best.id()).to_string();
        let old = file.team.take();
        if old.as_ref().is_some_and(|l| {
            l.pack == new.pack && l.source == new.source && l.resolved.id() == best.id()
        }) {
            lines.push(format!(
                "Pack {} is up to date: {} ({id12})",
                d.name, best.version
            ));
        } else {
            let was = old.map_or("(none)".to_string(), |l| l.resolved.version);
            lines.push(format!(
                "Pack {}: {was} -> {} ({id12}), .armadai/pack.lock updated",
                d.name, best.version
            ));
            changed.push(format!("pack {} {}", d.name, best.version));
            first = Some(("pack", d.name.clone(), best.version.clone()));
            team_lines = others_lines(config, root, &new)?;
        }
        file.team = Some(new.clone());
        team_new = Some(new);
    }
    for (name, src, range) in stds {
        let source = source_of(known, &source::real_url_in(src, known)?);
        sync_once(&source)?;
        let (best, path) = resolve_in(registry, &source, "standards", name, range, true)?;
        cache::ensure_snapshot_in(registry, &source, name, &path, &best)?;
        let new = lock_of(name, src, &best);
        let id12 = status::id12(&best.id()).to_string();
        let old = file.standards.iter().position(|e| e.pack == *name);
        let same = old.is_some_and(|i| {
            let l = &file.standards[i];
            l.source == new.source && l.resolved.id() == best.id()
        });
        if same {
            lines.push(format!(
                "Standards {name} is up to date: {} ({id12})",
                best.version
            ));
            continue;
        }
        let was = old.map_or("(none)".to_string(), |i| {
            file.standards[i].resolved.version.clone()
        });
        lines.push(format!(
            "Standards {name}: {was} -> {} ({id12}), .armadai/pack.lock updated",
            best.version
        ));
        changed.push(format!("standards {name} {}", best.version));
        first.get_or_insert(("standards", name.clone(), best.version.clone()));
        match old {
            Some(i) => file.standards[i] = new,
            None => file.standards.push(new),
        }
    }
    // an entry that only leaves the lock still rewrites it (it prints nothing, as it
    // moves no version)
    if first.is_none()
        && let Some(d) = dropped.first()
    {
        first = Some(("standards", d.pack.clone(), d.resolved.version.clone()));
        changed.push(format!("standards {} {}", d.pack, d.resolved.version));
    }
    let Some((what, name, v)) = first else {
        if let Some(t) = &team_new {
            record(config, root, t)?;
        }
        return Ok(Done {
            relink: true,
            lines,
            undo: None,
        });
    };
    check(
        root,
        &file,
        None,
        registry,
        env,
        &changed.join(" and "),
        "armadai pack update",
    )?;
    let undo = Undo::new(
        what,
        &name,
        &v,
        &[root.join(lock::LOCK), referents::path_in(config)],
    )?;
    lock::write(root, &file)?;
    if let Some(t) = &team_new
        && let Err(e) = record(config, root, t)
    {
        undo.restore();
        return Err(e);
    }
    lines.extend(team_lines);
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

/// `project.yaml` with an entry added to `standards:`: after the last line of the key's block
/// list (the user's comments stay where they are), or as a new block at the end of the text
/// when there is no such key. `None` for a `standards:` of any other form (inline list,
/// empty, scalar): it is never rewritten.
fn with_standards(text: Option<&str>, name: &str, source: &str, range: &str) -> Option<String> {
    let entry = |ind: &str| {
        format!(
            "{ind}- name: {name}\n{ind}  source: {}\n{ind}  version: {}\n",
            single_quoted(source),
            single_quoted(range)
        )
    };
    let Some(t) = text else {
        return Some(format!("schema_version: 1\nstandards:\n{}", entry("  ")));
    };
    let has_key = armadai_model::yaml::from_str_manifest(t)
        .ok()?
        .get("standards")
        .is_some();
    if !has_key {
        let mut out = if t.is_empty() || t.ends_with('\n') {
            t.to_string()
        } else {
            format!("{t}\n")
        };
        out.push_str(&format!("standards:\n{}", entry("  ")));
        return Some(out);
    }
    let lines: Vec<&str> = t.split_inclusive('\n').collect();
    let key = lines.iter().position(|l| {
        l.trim_end()
            .strip_prefix("standards:")
            .is_some_and(|rest| rest.trim().is_empty() || rest.trim_start().starts_with('#'))
    })?;
    let inert = |l: &str| l.trim().is_empty() || l.trim_start().starts_with('#');
    let mut indent = None;
    let mut last = None;
    for (i, l) in lines.iter().enumerate().skip(key + 1) {
        if inert(l) {
            continue;
        }
        let ws = &l[..l.len() - l.trim_start().len()];
        let item = l.trim_start().starts_with("- ");
        if indent.is_none() {
            // the first line after the key must open a block list
            if !item {
                return None;
            }
            indent = Some(ws.to_string());
        } else if ws.is_empty() && !item {
            break;
        }
        last = Some(i);
    }
    let (indent, last) = (indent?, last?);
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out.push_str(l);
        if i == last {
            if !l.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&entry(&indent));
        }
    }
    Some(out)
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
/// With `standards`, the pack is added to `standards:` instead of being the project's `pack:`.
#[allow(clippy::too_many_arguments)]
pub fn add_in(
    root: &Path,
    config: &Path,
    registry: &Path,
    known: &[RegistrySource],
    spec: &str,
    only: Option<&str>,
    standards: bool,
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
    // before any network access, as in `update_in`
    if let Some(pin) = lock::read_pin(root)? {
        bail!("{}", lock::pinned_refusal(&pin));
    }
    let file = root.join(".armadai/project.yaml");
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => bail!("{}: {e}", file.display()),
    };
    if let Some(t) = &text {
        let v = armadai_model::yaml::from_str_manifest(t)
            .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        if standards {
            let listed = v.get("standards").and_then(|s| s.as_array());
            let there = (listed.into_iter().flatten())
                .any(|e| e.get("name").and_then(|n| n.as_str()) == Some(name));
            if there {
                bail!(
                    ".armadai/project.yaml already references standards {name}: use `armadai pack update`"
                );
            }
        } else if let Some(p) = v.get("pack") {
            let there = p.get("name").and_then(|n| n.as_str()).unwrap_or(name);
            bail!(
                ".armadai/project.yaml already references pack {there}: use `armadai pack update`"
            );
        }
    }
    if standards && with_standards(text.as_deref(), name, "", "").is_none() {
        bail!(
            ".armadai/project.yaml has a standards: key that pack add cannot extend (not a block \
             list): add the entry by hand, then run `armadai pack update`"
        );
    }
    // the lock as it is now, read once: an unreadable one stops the command
    let mut locked = lock::read_committed(root)?.unwrap_or_default();

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
                        .map(|s| source::recorded(&s.url))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
    };

    let what = if standards { "standards" } else { "pack" };
    let (best, path) = resolve_in(
        registry,
        &source,
        what,
        name,
        given.unwrap_or("latest"),
        false,
    )?;
    let range = given.map_or_else(|| format!("^{}", best.version), str::to_string);
    cache::ensure_snapshot_in(registry, &source, name, &path, &best)?;
    // what `.armadai/` and the referent record keep: the source without its secrets
    let shown = source::recorded(&source.url);
    let new = lock_of(name, &shown, &best);
    // a standards pack the project no longer references leaves the lock
    let declared = effective::standards_locked_refs(root);
    locked
        .standards
        .retain(|e| e.pack != name && declared.iter().any(|(n, ..)| *n == e.pack));
    if standards {
        locked.standards.push(new.clone());
    } else {
        locked.team = Some(new.clone());
    }
    check(
        root,
        &locked,
        standards.then_some(name),
        registry,
        env,
        &format!("{what} {name} {}", best.version),
        &format!(
            "armadai pack add {name}{}",
            if standards { " --standards" } else { "" }
        ),
    )?;
    let undo = Undo::new(
        what,
        name,
        &best.version,
        &[
            root.join(lock::LOCK),
            file.clone(),
            referents::path_in(config),
        ],
    )?;
    // the lock first: a lock without the key is ignored, the key without a lock is not
    lock::write(root, &locked)?;
    let written = if standards {
        with_standards(text.as_deref(), name, &shown, &range).expect("checked above")
    } else {
        with_pack(text.as_deref(), name, &shown, &range)
    };
    write_project_yaml(root, &written)?;
    if !standards && let Err(e) = record(config, root, &new) {
        undo.restore();
        return Err(e);
    }
    Ok(Done {
        relink: true,
        lines: vec![format!(
            "Added {what} {name} {} ({}): .armadai/project.yaml (version: {range}) and .armadai/pack.lock",
            best.version,
            status::id12(&best.id())
        )],
        undo: Some(undo),
    })
}
