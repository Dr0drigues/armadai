//! The effective project of a directory: its `.armadai/` as the commands must read it.
//!
//! With a `pack:` key, the locked snapshot of the pack overlaid by the project's own
//! files; with `standards:`, the skills of each locked standards pack in between; with
//! variables to resolve, `{{var.name}}` rendered. The result is a scratch copy that is
//! dropped with the [`Effective`]; `.armadai/` and the cache are never written. A project
//! with none of these is read in place, nothing copied.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_core::starter::{PackKind, StarterPack};
use armadai_model::pack::{Layer, OverlayError};
use armadai_model::{Diagnostic, Origins, Project};

use super::cache::{Available, valid_id, valid_name};
use super::lock::{self, LockFile, PackLock};
use super::standards;
use crate::cli::native_pack;

/// What to do with `{{var.name}}`.
pub enum Vars<'a> {
    /// Leave them as written (`validate`, and the commands that only read).
    Literal,
    /// Render them from the three layers, the environment read through the closure.
    Resolve(&'a dyn Fn(&str) -> Option<String>),
}

pub struct LockedPack {
    pub name: String,
    pub lock: PackLock,
    pub snapshot: PathBuf,
}

/// Which key of `project.yaml` references a pack; only the wording of messages differs.
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Pack,
    Standards,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Pack => "pack",
            Kind::Standards => "standards",
        }
    }
}

pub struct Effective {
    /// The directory holding the `.armadai/` to read: `root`, or a scratch copy.
    pub sources: PathBuf,
    pub pack: Option<LockedPack>,
    /// The standards packs read, in the order of `standards:` (an entry `validate` reports
    /// as empty or badly named is not read); `Layer::Standards(i)` is `standards[i]`.
    pub standards: Vec<LockedPack>,
    /// Variables no layer sets (`Vars::Resolve` only): rendered empty.
    pub missing_vars: BTreeSet<String>,
    root: PathBuf,
    /// path relative to `.armadai/` -> (layer, the real file)
    origins: BTreeMap<PathBuf, (Layer, PathBuf)>,
    /// Diagnostics of the layering itself (clash, foreign file, wrong kind), on real files.
    extra: Vec<Diagnostic>,
    _scratch: Option<tempfile::TempDir>,
}

fn id12(id: &str) -> &str {
    let id = id.strip_prefix("sha256-").unwrap_or(id);
    &id[..id.len().min(12)]
}

fn manifest_value(root: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(root.join(".armadai/project.yaml")).ok()?;
    armadai_model::yaml::from_str_manifest(&text).ok()
}

/// The `pack:` key of the project's `project.yaml` as (name, source); `None` when
/// there is none, or the file is not readable (the loader reports that, with a position).
pub(crate) fn pack_ref(root: &Path) -> Option<(String, String)> {
    let v = manifest_value(root)?;
    let p = v.get("pack")?;
    Some((
        p.get("name")?.as_str()?.to_string(),
        p.get("source")?.as_str()?.to_string(),
    ))
}

/// The `standards:` entries of the project's `project.yaml` as (name, source, version), as
/// written; an entry that is not a mapping of three strings is left out (the loader reports it).
pub(crate) fn standards_refs(root: &Path) -> Vec<(String, String, String)> {
    let Some(v) = manifest_value(root) else {
        return Vec::new();
    };
    let list = v.get("standards").and_then(|s| s.as_array());
    (list.into_iter().flatten())
        .filter_map(|e| {
            let s = |k: &str| e.get(k)?.as_str().map(str::to_string);
            Some((s("name")?, s("source")?, s("version")?))
        })
        .collect()
}

/// Whether the project references a pack, `pack:` or `standards:`, so that it has a lock
/// (and a pin) to keep.
pub(crate) fn references_packs(root: &Path) -> bool {
    pack_ref(root).is_some() || !standards_refs(root).is_empty()
}

/// The entries of `standards:` to look for at the lock: a name that is not an id, an empty
/// source or an empty version is `validate`'s to report, with its position.
fn standards_declared(root: &Path) -> Vec<(String, String)> {
    (standards_refs(root).into_iter())
        .filter(|(n, s, v)| valid_name(n) && !s.is_empty() && !v.is_empty())
        .map(|(n, s, _)| (n, s))
        .collect()
}

pub fn open(root: &Path, vars: Vars) -> anyhow::Result<Effective> {
    open_in(root, vars, &armadai_core::config::registry_cache_dir())
}

/// [`open`] with the registry directory given.
pub fn open_in(root: &Path, vars: Vars, registry: &Path) -> anyhow::Result<Effective> {
    if pack_ref(root).is_none() && standards_declared(root).is_empty() {
        return build(root, None, &[], vars, registry);
    }
    let lock = lock::read(root)?;
    let team = match pack_ref(root) {
        None => None,
        Some((name, _)) => match lock.as_ref().and_then(|l| l.team.as_ref()) {
            Some(t) => Some(t),
            None => bail!("{}", lock_missing(&name)),
        },
    };
    open_locked(root, team, lock.as_ref(), None, vars, registry)
}

/// E1 (no lock at all) and E2 (a lock without the entry): a `standards:` entry that is not locked.
pub(crate) fn standards_missing(name: &str, lock_exists: bool) -> String {
    let what = if lock_exists {
        "has no entry for it"
    } else {
        "is missing"
    };
    format!(
        ".armadai/project.yaml references standards {name} but .armadai/pack.lock {what}: run `armadai pack update`"
    )
}

/// M5a.
pub(crate) fn lock_missing(name: &str) -> String {
    format!(
        ".armadai/project.yaml references pack {name} but .armadai/pack.lock is missing: \
         run `armadai pack update`"
    )
}

/// Same, with the lock given (`pack add` and `pack update` check a lock before writing it).
pub fn open_with_lock(root: &Path, lock: &LockFile, vars: Vars) -> anyhow::Result<Effective> {
    open_with_lock_in(
        root,
        lock,
        vars,
        &armadai_core::config::registry_cache_dir(),
    )
}

pub fn open_with_lock_in(
    root: &Path,
    lock: &LockFile,
    vars: Vars,
    registry: &Path,
) -> anyhow::Result<Effective> {
    // the lock given is the team's even before `pack add` has written the `pack:` key
    open_locked(root, lock.team.as_ref(), Some(lock), None, vars, registry)
}

/// [`open_with_lock_in`] for a `pack add --standards`: the entry `adding` is not in
/// `project.yaml` yet, and is looked for in `lock` like the declared ones.
pub fn open_adding_standards_in(
    root: &Path,
    lock: &LockFile,
    adding: &str,
    vars: Vars,
    registry: &Path,
) -> anyhow::Result<Effective> {
    open_locked(
        root,
        lock.team.as_ref(),
        Some(lock),
        Some(adding),
        vars,
        registry,
    )
}

/// `team` as given, and the standards entries `root` references found in `lock` (E1 to
/// E3, E7).
fn open_locked(
    root: &Path,
    team: Option<&PackLock>,
    lock: Option<&LockFile>,
    adding: Option<&str>,
    vars: Vars,
    registry: &Path,
) -> anyhow::Result<Effective> {
    let mut stds = Vec::new();
    let mut seen = BTreeSet::new();
    let declared = standards_declared(root).into_iter().map(|(n, _)| n);
    for name in declared.chain(adding.map(str::to_string)) {
        if !seen.insert(name.clone()) {
            bail!(".armadai/project.yaml references standards {name} twice: keep one entry");
        }
        let entry = match lock {
            None => bail!("{}", standards_missing(&name, false)),
            Some(l) => l.standards_entry(&name),
        };
        match entry {
            Some(e) => stds.push(e.clone()),
            None if lock::read_pin(root)?.is_some() => bail!(
                ".armadai/project.yaml references standards {name} but the pack pin of this \
                 checkout has no entry for it: run armadai pack unpin, then armadai pack update"
            ),
            None => bail!("{}", standards_missing(&name, true)),
        }
    }
    build(root, team, &stds, vars, registry)
}

fn snapshot(root: &Path, lock: &PackLock, kind: Kind, registry: &Path) -> anyhow::Result<PathBuf> {
    let (dir, id) = snapshot_path_of(root, lock, kind, registry)?;
    if !dir.is_dir() {
        bail!(
            "{} {} {} ({}) is not in the local cache: run `armadai pack sync`",
            kind.word(),
            lock.pack,
            lock.resolved.version,
            id12(&id)
        );
    }
    Ok(dir)
}

/// Where the locked snapshot of the team pack is (or would be) in the cache, and its id;
/// the folder itself may be absent.
pub(crate) fn snapshot_path(
    root: &Path,
    lock: &PackLock,
    registry: &Path,
) -> anyhow::Result<(PathBuf, String)> {
    snapshot_path_of(root, lock, Kind::Pack, registry)
}

pub(crate) fn snapshot_path_of(
    root: &Path,
    lock: &PackLock,
    kind: Kind,
    registry: &Path,
) -> anyhow::Result<(PathBuf, String)> {
    let word = kind.word();
    let referenced = match kind {
        Kind::Pack => pack_ref(root),
        Kind::Standards => (standards_declared(root).into_iter()).find(|(n, _)| *n == lock.pack),
    };
    if let Some((name, source)) = referenced
        && (name != lock.pack || source != lock.source)
    {
        let pin = lock::read_pin(root)?;
        let pinned = match kind {
            Kind::Pack => pin.and_then(|p| p.team).as_ref() == Some(lock),
            Kind::Standards => pin.is_some_and(|p| p.standards_entry(&lock.pack) == Some(lock)),
        };
        if pinned {
            bail!(
                "the pack pin of this checkout is for {word} {} from {}, but .armadai/project.yaml \
                 references {name} from {source}: run armadai pack unpin",
                lock.pack,
                lock.source
            );
        }
        bail!(
            ".armadai/pack.lock is for {word} {} from {}, but .armadai/project.yaml references {name} \
             from {source}: run `armadai pack update`",
            lock.pack,
            lock.source
        );
    }
    let available = Available {
        version: lock.resolved.version.clone(),
        tag: None,
        commit: lock.resolved.commit.clone(),
        digest: lock.resolved.digest.clone(),
    };
    let id = available.id();
    if id.is_empty() {
        bail!(
            "{word} {} has a version without commit or digest",
            lock.pack
        );
    }
    for (bad, ok) in [(&lock.pack, valid_name(&lock.pack)), (&id, valid_id(&id))] {
        if !ok {
            bail!(
                "{word} {} has an invalid name or version id: {bad}",
                lock.pack
            );
        }
    }
    let source = super::source::real_url(&lock.source)?;
    Ok((
        super::cache::snapshot_dir_in(registry, &source, &lock.pack, &id),
        id,
    ))
}

fn build(
    root: &Path,
    team: Option<&PackLock>,
    stds: &[PackLock],
    vars: Vars,
    registry: &Path,
) -> anyhow::Result<Effective> {
    let in_place = || Effective {
        sources: root.to_path_buf(),
        pack: None,
        standards: Vec::new(),
        missing_vars: BTreeSet::new(),
        root: root.to_path_buf(),
        origins: BTreeMap::new(),
        extra: Vec::new(),
        _scratch: None,
    };
    let plain = team.is_none() && stds.is_empty();
    if plain && matches!(vars, Vars::Literal) {
        return Ok(in_place());
    }
    let base = root.join(".armadai");
    // A plain project is only copied when a variable must be rendered: until then a link
    // is no reason to refuse it (the loader reads it as before).
    if plain {
        let looked: BTreeMap<PathBuf, Vec<u8>> = native_pack::read_listed(&base, false)?
            .into_iter()
            .map(|(p, b, _)| (p, b))
            .collect();
        if !matches!(armadai_model::pack_vars::files_names(&looked), Ok(n) if !n.is_empty()) {
            return Ok(in_place());
        }
    }
    let project = native_pack::read_listed(&base, true)?;
    let exec_project: BTreeSet<PathBuf> = project
        .iter()
        .filter(|f| f.2)
        .map(|f| f.0.clone())
        .collect();
    let mut exec_pack = BTreeSet::new();
    let project: BTreeMap<PathBuf, Vec<u8>> = project.into_iter().map(|(p, b, _)| (p, b)).collect();

    let mut locked = None;
    let mut extra = Vec::new();
    let mut files: BTreeMap<PathBuf, (Layer, Vec<u8>)> = BTreeMap::new();
    let mut standards = Vec::new();
    let mut exec_std: Vec<BTreeSet<PathBuf>> = Vec::new();
    if !plain {
        let mut team_files = None;
        if let Some(lock) = team {
            let snap = snapshot(root, lock, Kind::Pack, registry)?;
            let pack = native_pack::files(&snap)?;
            exec_pack.extend(pack.files.iter().filter(|f| f.2).map(|f| f.0.clone()));
            team_files = Some(
                pack.files
                    .into_iter()
                    .map(|(p, b, _)| (p, b))
                    .collect::<BTreeMap<PathBuf, Vec<u8>>>(),
            );
            locked = Some(LockedPack {
                name: lock.pack.clone(),
                lock: lock.clone(),
                snapshot: snap,
            });
        }
        let mut layers: Vec<BTreeMap<PathBuf, Vec<u8>>> = Vec::new();
        for lock in stds {
            let snap = snapshot(root, lock, Kind::Standards, registry)?;
            let pack = native_pack::files(&snap)?;
            let kind = StarterPack::load(&snap)
                .with_context(|| format!("standards {}", lock.pack))?
                .kind;
            if kind != PackKind::Standards {
                extra.push(standards::wrong_kind(&lock.pack, &snap));
            }
            exec_std.push(
                pack.files
                    .iter()
                    .filter(|f| f.2)
                    .map(|f| f.0.clone())
                    .collect(),
            );
            layers.push(pack.files.into_iter().map(|(p, b, _)| (p, b)).collect());
            standards.push(LockedPack {
                name: lock.pack.clone(),
                lock: lock.clone(),
                snapshot: snap,
            });
        }
        let refs: Vec<&BTreeMap<PathBuf, Vec<u8>>> = layers.iter().collect();
        let team_name = team.map_or("", |t| t.pack.as_str());
        let stacked = armadai_model::pack::overlay_standards(team_files.as_ref(), &refs, &project)
            .map_err(|e| match e {
                OverlayError::NestedPack => anyhow::anyhow!(
                    "pack {team_name}: its own project.yaml references a pack (pack:), which is not supported"
                ),
                OverlayError::NestedStandards => anyhow::anyhow!(
                    "pack {team_name}: its own project.yaml references standards packs (standards:), which is not supported"
                ),
                OverlayError::InvalidYaml { layer, message } => {
                    let file = match layer {
                        Layer::Pack => locked
                            .as_ref()
                            .map_or_else(PathBuf::new, |l| l.snapshot.clone())
                            .join("project.yaml"),
                        Layer::Standards(i) => standards[i].snapshot.join("project.yaml"),
                        _ => base.join("project.yaml"),
                    };
                    anyhow::anyhow!("{}: {message}", file.display())
                }
            })?;
        files = stacked.files;
        let label = |first: Option<usize>| match first {
            None => format!("pack {team_name}"),
            Some(j) => format!("standards {}", standards[j].name),
        };
        for c in &stacked.clashes {
            let second = &standards[c.second];
            extra.push(standards::duplicate(
                &c.skill,
                &label(c.first),
                &label(Some(c.second)),
                second
                    .snapshot
                    .join("skills")
                    .join(&c.skill)
                    .join("SKILL.md"),
            ));
        }
        for (i, s) in standards.iter().enumerate() {
            let rels: Vec<PathBuf> = (stacked.foreign.iter())
                .filter(|(l, _)| *l == i)
                .map(|(_, p)| p.clone())
                .collect();
            extra.extend(standards::foreign(&s.snapshot, &rels));
        }
    } else {
        files.extend(project.into_iter().map(|(p, b)| (p, (Layer::Project, b))));
    }

    let mut missing_vars = BTreeSet::new();
    let mut rendered = false;
    if let Vars::Resolve(env) = vars {
        let mut bytes: BTreeMap<PathBuf, Vec<u8>> = files
            .iter()
            .map(|(p, (_, b))| (p.clone(), b.clone()))
            .collect();
        match armadai_model::pack_vars::files_names(&bytes) {
            Ok(names) if !names.is_empty() => {
                let values = super::vars::values(root, &names, env)?;
                let done = armadai_model::pack_vars::render_files(&mut bytes, &values)
                    .map_err(anyhow::Error::msg)?;
                missing_vars = done.missing;
                rendered = true;
                for (p, b) in bytes {
                    if let Some(f) = files.get_mut(&p) {
                        f.1 = b;
                    }
                }
            }
            Ok(_) => {}
            // an unreadable file is the loader's to report, with its position
            Err(_) if plain => return Ok(in_place()),
            Err(e) => bail!(e),
        }
    }
    if plain && !rendered {
        return Ok(in_place());
    }

    let scratch = tempfile::tempdir()?;
    let dest = scratch.path().join(".armadai");
    std::fs::create_dir_all(&dest)?;
    let mut origins = BTreeMap::new();
    for (rel, (layer, bytes)) in &files {
        let at = dest.join(rel);
        std::fs::create_dir_all(at.parent().context("a file has a parent")?)?;
        std::fs::write(&at, bytes).with_context(|| format!("writing {}", at.display()))?;
        #[cfg(unix)]
        if match layer {
            Layer::Pack => exec_pack.contains(rel),
            Layer::Standards(i) => exec_std[*i].contains(rel),
            Layer::Project => exec_project.contains(rel),
            Layer::Merged => false,
        } {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o755))?;
        }
        let real = match (layer, &locked) {
            (Layer::Pack, Some(l)) => l.snapshot.join(rel),
            (Layer::Standards(i), _) => standards[*i].snapshot.join(rel),
            (Layer::Merged, _) => base.join("project.yaml"),
            _ => base.join(rel),
        };
        origins.insert(rel.clone(), (*layer, real));
    }
    Ok(Effective {
        sources: scratch.path().to_path_buf(),
        pack: locked,
        standards,
        missing_vars,
        root: root.to_path_buf(),
        origins,
        extra,
        _scratch: Some(scratch),
    })
}

impl Effective {
    /// Diagnostics on the real files; one on a merged `project.yaml` points at the
    /// project's, without line nor column.
    pub fn remap(&self, mut ds: Vec<Diagnostic>) -> Vec<Diagnostic> {
        if self.sources == self.root {
            return ds;
        }
        for d in &mut ds {
            if let Some((layer, real)) = self.real(&d.file) {
                d.file = real;
                if layer == Layer::Merged {
                    d.line = None;
                    d.column = None;
                }
            }
        }
        ds
    }

    fn real(&self, file: &Path) -> Option<(Layer, PathBuf)> {
        let rel = file.strip_prefix(self.sources.join(".armadai")).ok()?;
        Some(
            self.origins
                .get(rel)
                .cloned()
                .unwrap_or_else(|| (Layer::Project, self.root.join(".armadai").join(rel))),
        )
    }

    pub fn remap_origins(&self, o: &mut Origins) {
        for (file, _) in o.files.values_mut() {
            if let Some((_, real)) = self.real(file) {
                *file = real;
            }
        }
    }

    /// The diagnostics of the layering, then `rest` on the real files.
    fn with_extra(&self, rest: Vec<Diagnostic>) -> Vec<Diagnostic> {
        let mut ds = self.extra.clone();
        ds.extend(self.remap(rest));
        ds
    }

    /// `armadai_model::load_checked` of the view; diagnostics and origins on the real files.
    /// A layering error fails it, with those diagnostics first.
    pub fn load_checked(&self) -> Result<(Project, Origins), Vec<Diagnostic>> {
        match armadai_model::load_checked(&self.sources) {
            Ok(_) if armadai_model::has_errors(&self.extra) => Err(self.extra.clone()),
            Ok((p, mut o)) => {
                self.remap_origins(&mut o);
                Ok((p, o))
            }
            Err(ds) => Err(self.with_extra(ds)),
        }
    }

    /// `armadai_model::load_project` of the view, as far as it loads; diagnostics and
    /// origins on the real files.
    pub fn load_project(&self) -> armadai_model::Loaded {
        let mut l = armadai_model::load_project(&self.sources);
        l.diagnostics = self.with_extra(std::mem::take(&mut l.diagnostics));
        self.remap_origins(&mut l.origins);
        l
    }

    pub fn validate(&self) -> Vec<Diagnostic> {
        self.with_extra(armadai_model::validate(&self.sources))
    }

    /// Skill id -> name of the standards pack its `SKILL.md` comes from; skills of the
    /// team pack and of the project are not in it.
    pub fn skill_packs(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (rel, (layer, _)) in &self.origins {
            let Layer::Standards(i) = layer else { continue };
            let c: Vec<_> = rel.iter().filter_map(|s| s.to_str()).collect();
            if let ["skills", id, "SKILL.md"] = c.as_slice() {
                out.insert(id.to_string(), self.standards[*i].name.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::lock::Resolved;

    const URL: &str = "https://example.com/packs.git";
    const C1: &str = "1111111111111111111111111111111111111111";
    const C2: &str = "2222222222222222222222222222222222222222";
    const MANIFEST: &str = "schema_version: 1\ntargets: [claude]\n";
    const REF: &str =
        "pack: { name: demo, source: 'https://example.com/packs.git', version: latest }\n";

    fn put(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn lock(commit: &str) -> PackLock {
        PackLock {
            format: 1,
            pack: "demo".into(),
            source: URL.into(),
            resolved: Resolved {
                version: "1.0.0".into(),
                tag: None,
                commit: Some(commit.into()),
                digest: None,
            },
        }
    }

    struct Fx {
        registry: tempfile::TempDir,
        root: tempfile::TempDir,
    }

    impl Fx {
        /// A project referencing `demo`, a registry with no snapshot yet.
        fn new() -> Fx {
            let fx = Fx {
                registry: tempfile::tempdir().unwrap(),
                root: tempfile::tempdir().unwrap(),
            };
            put(fx.root.path(), ".armadai/project.yaml", REF);
            lock::write(fx.root.path(), &lock(C1).into()).unwrap();
            fx
        }
        fn snapshot(&self, commit: &str, files: &[(&str, &str)]) -> PathBuf {
            let dir =
                crate::pack::cache::snapshot_dir_in(self.registry.path(), URL, "demo", commit);
            for (rel, text) in files {
                put(&dir, rel, text);
            }
            dir
        }
        fn open(&self, vars: Vars) -> anyhow::Result<Effective> {
            open_in(self.root.path(), vars, self.registry.path())
        }
        fn err(&self) -> String {
            self.open(Vars::Literal)
                .err()
                .expect("an error")
                .to_string()
        }
    }

    const AGENT_A: &str = "---\ndescription: from the pack\n---\nA\n";
    const SKILL: &str = "---\nname: s\ndescription: d\n---\nS\n";

    fn agents(e: &Effective) -> Vec<String> {
        let (p, _) = e.load_checked().unwrap();
        let mut ids: Vec<String> = p.agents.keys().cloned().collect();
        ids.sort();
        ids
    }

    #[test]
    fn a_plain_project_is_read_in_place() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), ".armadai/project.yaml", MANIFEST);
        let e = open_in(root.path(), Vars::Literal, root.path()).unwrap();
        assert_eq!(e.sources, root.path());
        let none = |_: &str| None;
        let e = open_in(root.path(), Vars::Resolve(&none), root.path()).unwrap();
        assert_eq!(e.sources, root.path());
        assert!(e._scratch.is_none());
    }

    #[test]
    fn a_pinned_checkout_reads_the_pinned_snapshot_while_the_lock_moves() {
        let fx = Fx::new();
        std::fs::create_dir_all(fx.root.path().join(".git")).unwrap();
        fx.snapshot(
            C1,
            &[("project.yaml", MANIFEST), ("agents/one.md", AGENT_A)],
        );
        fx.snapshot(
            C2,
            &[("project.yaml", MANIFEST), ("agents/two.md", AGENT_A)],
        );
        lock::pin(fx.root.path()).unwrap();
        // what a `pack update` of another checkout does to the committed lock
        std::fs::write(
            fx.root.path().join(lock::LOCK),
            armadai_model::yaml::to_string(&lock(C2)).unwrap(),
        )
        .unwrap();
        assert_eq!(agents(&fx.open(Vars::Literal).unwrap()), ["one"]);
        lock::unpin(fx.root.path()).unwrap();
        assert_eq!(agents(&fx.open(Vars::Literal).unwrap()), ["two"]);
    }

    #[test]
    fn a_pin_for_another_pack_names_the_pin_and_a_missing_snapshot_the_cache() {
        let fx = Fx::new();
        std::fs::create_dir_all(fx.root.path().join(".git")).unwrap();
        lock::pin(fx.root.path()).unwrap();
        // the pinned snapshot is not in the cache
        assert_eq!(
            fx.err(),
            "pack demo 1.0.0 (111111111111) is not in the local cache: run `armadai pack sync`"
        );
        put(
            fx.root.path(),
            ".armadai/project.yaml",
            "pack: { name: other, source: 'https://example.com/more.git', version: latest }\n",
        );
        assert_eq!(
            fx.err(),
            "the pack pin of this checkout is for pack demo from https://example.com/packs.git, but .armadai/project.yaml references other from https://example.com/more.git: run armadai pack unpin"
        );
        // without the pin the committed lock is the one named
        lock::unpin(fx.root.path()).unwrap();
        assert_eq!(
            fx.err(),
            ".armadai/pack.lock is for pack demo from https://example.com/packs.git, but .armadai/project.yaml references other from https://example.com/more.git: run `armadai pack update`"
        );
    }

    #[test]
    fn reference_mode_reads_the_locked_snapshot() {
        let fx = Fx::new();
        fx.snapshot(
            C1,
            &[("project.yaml", MANIFEST), ("agents/one.md", AGENT_A)],
        );
        fx.snapshot(
            C2,
            &[("project.yaml", MANIFEST), ("agents/two.md", AGENT_A)],
        );
        assert_eq!(agents(&fx.open(Vars::Literal).unwrap()), ["one"]);
        lock::write(fx.root.path(), &lock(C2).into()).unwrap();
        assert_eq!(agents(&fx.open(Vars::Literal).unwrap()), ["two"]);
        // a lock given by the caller wins over the file
        let e = open_with_lock_in(
            fx.root.path(),
            &lock(C1).into(),
            Vars::Literal,
            fx.registry.path(),
        )
        .unwrap();
        assert_eq!(agents(&e), ["one"]);
    }

    #[test]
    fn the_overlay_never_writes_the_cache() {
        let fx = Fx::new();
        let snap = fx.snapshot(C1, &[("project.yaml", MANIFEST), ("agents/a.md", AGENT_A)]);
        put(
            fx.root.path(),
            ".armadai/agents/a.md",
            "---\ndescription: mine\n---\nM\n",
        );
        put(fx.root.path(), ".armadai/agents/b.md", AGENT_A);
        let before = crate::pack::cache::tree_digest(&snap).unwrap();
        let e = fx.open(Vars::Literal).unwrap();
        let (p, _) = e.load_checked().unwrap();
        assert_eq!(p.agents["a"].front.description, "mine");
        assert_eq!(agents(&e), ["a", "b"]);
        assert_eq!(crate::pack::cache::tree_digest(&snap).unwrap(), before);
    }

    #[test]
    fn missing_lock_and_missing_snapshot_are_named() {
        let fx = Fx::new();
        assert_eq!(
            fx.err(),
            "pack demo 1.0.0 (111111111111) is not in the local cache: run `armadai pack sync`"
        );
        let mut other = lock(C1);
        other.pack = "other".into();
        other.source = "https://example.com/else.git".into();
        lock::write(fx.root.path(), &other.into()).unwrap();
        assert_eq!(
            fx.err(),
            ".armadai/pack.lock is for pack other from https://example.com/else.git, but \
             .armadai/project.yaml references demo from https://example.com/packs.git: \
             run `armadai pack update`"
        );
        std::fs::remove_file(fx.root.path().join(".armadai/pack.lock")).unwrap();
        assert_eq!(
            fx.err(),
            ".armadai/project.yaml references pack demo but .armadai/pack.lock is missing: \
             run `armadai pack update`"
        );
        let mut bad = lock("../../etc");
        bad.pack = "demo".into();
        lock::write(fx.root.path(), &bad.into()).unwrap();
        assert_eq!(
            fx.err(),
            "pack demo has an invalid name or version id: ../../etc"
        );
    }

    #[test]
    fn a_nested_pack_is_refused() {
        let fx = Fx::new();
        fx.snapshot(C1, &[("project.yaml", &format!("{MANIFEST}{REF}"))]);
        assert_eq!(
            fx.err(),
            "pack demo: its own project.yaml references a pack (pack:), which is not supported"
        );
    }

    #[test]
    fn diagnostics_point_at_their_files() {
        let fx = Fx::new();
        let snap = fx.snapshot(
            C1,
            &[
                ("project.yaml", MANIFEST),
                (
                    "agents/bad-pack.md",
                    "---\ndescription: d\nskills: [nope-pack]\n---\nB\n",
                ),
            ],
        );
        put(
            fx.root.path(),
            ".armadai/agents/bad-mine.md",
            "---\ndescription: d\nskills: [nope-mine]\n---\nB\n",
        );
        put(
            fx.root.path(),
            ".armadai/project.yaml",
            &format!("{REF}coordinator: ghost\n"),
        );
        let e = fx.open(Vars::Literal).unwrap();
        let ds = e.validate();
        let at = |code: &str, needle: &str| {
            ds.iter()
                .find(|d| d.code == code && d.message.contains(needle))
                .unwrap_or_else(|| panic!("no {code} {needle}: {ds:?}"))
        };
        assert_eq!(
            at("unknown-skill", "nope-pack").file,
            snap.join("agents/bad-pack.md")
        );
        assert_eq!(
            at("unknown-skill", "nope-mine").file,
            fx.root.path().join(".armadai/agents/bad-mine.md")
        );
        let merged = ds
            .iter()
            .find(|d| d.file.ends_with("project.yaml"))
            .expect("a manifest diagnostic");
        assert_eq!(merged.file, fx.root.path().join(".armadai/project.yaml"));
        assert_eq!((merged.line, merged.column), (None, None));
        assert!(ds.iter().all(|d| !d.file.starts_with(&e.sources)), "{ds:?}");
        assert!(e.load_checked().is_err());
        let Err(ds) = e.load_checked() else {
            unreachable!()
        };
        assert!(ds.iter().all(|d| !d.file.starts_with(&e.sources)));
    }

    #[test]
    fn a_valid_reference_gives_origins_on_the_real_files() {
        let fx = Fx::new();
        let snap = fx.snapshot(
            C1,
            &[
                ("project.yaml", MANIFEST),
                ("agents/a.md", AGENT_A),
                ("skills/s/SKILL.md", SKILL),
            ],
        );
        let e = fx.open(Vars::Literal).unwrap();
        let (_, o) = e.load_checked().unwrap();
        let files: Vec<&PathBuf> = o.files.values().map(|(f, _)| f).collect();
        assert!(files.contains(&&snap.join("agents/a.md")), "{files:?}");
        assert!(
            files.iter().all(|f| !f.starts_with(&e.sources)),
            "{files:?}"
        );
    }

    #[test]
    fn literal_mode_keeps_the_variables() {
        let fx = Fx::new();
        fx.snapshot(
            C1,
            &[
                (
                    "project.yaml",
                    &format!("{MANIFEST}native:\n  claude:\n    home: '{{{{var.home}}}}'\n"),
                ),
                ("agents/a.md", "---\ndescription: d\n---\nIn {{var.home}}\n"),
            ],
        );
        let read = |e: &Effective| {
            (
                std::fs::read_to_string(e.sources.join(".armadai/agents/a.md")).unwrap(),
                std::fs::read_to_string(e.sources.join(".armadai/project.yaml")).unwrap(),
            )
        };
        let (agent, manifest) = read(&fx.open(Vars::Literal).unwrap());
        assert!(agent.ends_with("In {{var.home}}\n"), "{agent}");
        assert!(manifest.contains("{{var.home}}"), "{manifest}");

        let env = |k: &str| (k == "ARMADAI_VAR_HOME").then(|| "/h".to_string());
        let e = fx.open(Vars::Resolve(&env)).unwrap();
        let (agent, manifest) = read(&e);
        assert!(agent.ends_with("In /h\n"), "{agent}");
        assert!(
            manifest.contains("/h") && !manifest.contains("{{"),
            "{manifest}"
        );
        assert!(e.missing_vars.is_empty());

        let none = |_: &str| None;
        let e = fx.open(Vars::Resolve(&none)).unwrap();
        assert_eq!(e.missing_vars, ["home".to_string()].into());
        assert!(read(&e).0.ends_with("In \n"));
    }

    #[test]
    fn a_plain_project_with_a_variable_is_rendered_not_rewritten() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), ".armadai/project.yaml", MANIFEST);
        let agent = "---\ndescription: d\n---\nIn {{var.home}}\n";
        put(root.path(), ".armadai/agents/a.md", agent);
        put(root.path(), ".armadai/vars.yaml", "home: /shared\n");
        put(root.path(), ".armadai/state/x", "x");
        let none = |_: &str| None;
        let e = open_in(root.path(), Vars::Resolve(&none), root.path()).unwrap();
        assert_ne!(e.sources, root.path());
        assert_eq!(
            std::fs::read_to_string(e.sources.join(".armadai/agents/a.md")).unwrap(),
            "---\ndescription: d\n---\nIn /shared\n"
        );
        // only the white list is copied
        assert!(!e.sources.join(".armadai/vars.yaml").exists());
        assert!(!e.sources.join(".armadai/state").exists());
        let scratch = e.sources.clone();
        drop(e);
        assert!(
            !scratch.exists(),
            "the scratch copy is dropped with the view"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join(".armadai/agents/a.md")).unwrap(),
            agent
        );
    }

    #[test]
    fn an_instance_keeps_its_diagnostic_lines() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), ".armadai/project.yaml", MANIFEST);
        put(
            root.path(),
            ".armadai/templates/t.md",
            "---\ndescription: d\n---\nIn {{a}}\n",
        );
        let agent = ".armadai/agents/r.md";
        // `vars:` spans 3 lines and holds a pack variable; the bad skill is on line 6
        put(
            root.path(),
            agent,
            "---\nfrom: t\nvars:\n  a: \"{{var.d}}\"\n  b: x\nskills: [nope]\n---\nbody\n",
        );
        let env = |k: &str| (k == "ARMADAI_VAR_D").then(|| "/srv/d".to_string());
        let e = open_in(root.path(), Vars::Resolve(&env), root.path()).unwrap();
        let ds = e.validate();
        let d = ds
            .iter()
            .find(|d| d.code == "unknown-skill")
            .unwrap_or_else(|| panic!("no unknown-skill: {ds:?}"));
        assert_eq!(d.file, root.path().join(agent));
        assert_eq!(d.line, Some(6), "{d:?}");
    }

    #[test]
    fn native_numbers_read_as_in_a_plain_project() {
        let native = "native:\n  claude:\n    big: 123456789012345678901234567890\n    dec: 1.10\n    int: 42\n    txt: '{{var.x}}'\n";
        let direct = tempfile::tempdir().unwrap();
        put(
            direct.path(),
            ".armadai/project.yaml",
            &format!("{MANIFEST}{native}"),
        );
        let want = |e: &Effective| {
            let (p, _) = e.load_checked().unwrap();
            serde_json::to_string(&p.manifest.native).unwrap()
        };
        let plain = open_in(direct.path(), Vars::Literal, direct.path()).unwrap();
        let want = want(&plain);
        assert!(
            want.contains("42") && want.contains("123456789012345678901234567890"),
            "{want}"
        );

        let fx = Fx::new();
        fx.snapshot(C1, &[("project.yaml", MANIFEST)]);
        put(
            fx.root.path(),
            ".armadai/project.yaml",
            &format!("{REF}{native}"),
        );
        let got = |e: &Effective| {
            let (p, _) = e.load_checked().unwrap();
            serde_json::to_string(&p.manifest.native).unwrap()
        };
        assert_eq!(got(&fx.open(Vars::Literal).unwrap()), want);
        let env = |_: &str| Some("v".to_string());
        // the same once the variable is rendered: only the text differs
        assert_eq!(
            got(&fx.open(Vars::Resolve(&env)).unwrap()),
            want.replace("{{var.x}}", "v")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_executable_bit_is_kept_and_links_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fx::new();
        let snap = fx.snapshot(
            C1,
            &[
                ("project.yaml", MANIFEST),
                ("skills/s/SKILL.md", SKILL),
                ("skills/s/run.sh", "#!/bin/sh\n"),
            ],
        );
        std::fs::set_permissions(
            snap.join("skills/s/run.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let e = fx.open(Vars::Literal).unwrap();
        let mode = |f: &str| {
            std::fs::metadata(e.sources.join(f))
                .unwrap()
                .permissions()
                .mode()
                & 0o111
        };
        assert_ne!(mode(".armadai/skills/s/run.sh"), 0);
        assert_eq!(mode(".armadai/skills/s/SKILL.md"), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_matters_only_where_a_file_is_read() {
        let fx = Fx::new();
        fx.snapshot(
            C1,
            &[("project.yaml", MANIFEST), ("skills/s/SKILL.md", SKILL)],
        );
        let notes = fx.root.path().join(".armadai/agents/notes.txt");
        put(fx.root.path(), ".armadai/agents/real.txt", "x");
        std::os::unix::fs::symlink("real.txt", &notes).unwrap();
        // a link to a file outside the list does not block, and is not copied
        let e = fx.open(Vars::Literal).unwrap();
        assert!(!e.sources.join(".armadai/agents/notes.txt").exists());
        drop(e);
        // a link standing for a folder of the list is refused
        let skill = fx.root.path().join(".armadai/skills/s");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(fx.registry.path(), &skill).unwrap();
        assert!(
            fx.err().ends_with("skills/s is a symbolic link, refused"),
            "{}",
            fx.err()
        );
        std::fs::remove_file(&skill).unwrap();
        // a link on a listed path is refused
        put(fx.root.path(), ".armadai/agents/a.md", AGENT_A);
        std::os::unix::fs::symlink("a.md", fx.root.path().join(".armadai/agents/l.md")).unwrap();
        assert!(
            fx.err()
                .ends_with("agents/l.md is a symbolic link, refused"),
            "{}",
            fx.err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_plain_project_with_a_link_and_no_variable_is_read_in_place() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), ".armadai/project.yaml", MANIFEST);
        put(root.path(), ".armadai/agents/real.md", AGENT_A);
        std::os::unix::fs::symlink("real.md", root.path().join(".armadai/agents/x.md")).unwrap();
        let none = |_: &str| None;
        let e = open_in(root.path(), Vars::Resolve(&none), root.path()).unwrap();
        assert_eq!(e.sources, root.path());
        // with a variable to render the link is refused, as in a pack view
        put(
            root.path(),
            ".armadai/agents/real.md",
            "---\ndescription: d\n---\n{{var.v}}\n",
        );
        let e = open_in(root.path(), Vars::Resolve(&none), root.path())
            .err()
            .unwrap();
        assert!(
            e.to_string()
                .ends_with("agents/x.md is a symbolic link, refused"),
            "{e}"
        );
    }

    #[test]
    fn files_outside_the_list_stay_out_of_the_view() {
        let fx = Fx::new();
        fx.snapshot(
            C1,
            &[
                ("project.yaml", MANIFEST),
                ("flows/x.md", "x"),
                ("agents/n.txt", "x"),
            ],
        );
        put(fx.root.path(), ".armadai/agents/notes.txt", "x");
        put(fx.root.path(), ".armadai/flows/y.md", "x");
        put(fx.root.path(), ".armadai/agents/a.md", AGENT_A);
        let e = fx.open(Vars::Literal).unwrap();
        let mut seen = Vec::new();
        let mut todo = vec![e.sources.join(".armadai")];
        while let Some(d) = todo.pop() {
            for f in std::fs::read_dir(d).unwrap() {
                let p = f.unwrap().path();
                if p.is_dir() {
                    todo.push(p)
                } else {
                    seen.push(
                        p.strip_prefix(e.sources.join(".armadai"))
                            .unwrap()
                            .to_path_buf(),
                    );
                }
            }
        }
        seen.sort();
        assert_eq!(
            seen,
            [PathBuf::from("agents/a.md"), PathBuf::from("project.yaml")]
        );
    }

    #[test]
    fn an_invalid_manifest_names_its_file_and_layer() {
        let fx = Fx::new();
        let snap = fx.snapshot(C1, &[("project.yaml", "targets: [claude\nx: 1\n")]);
        let e = fx.err();
        let yaml = |t: &str| armadai_model::yaml::from_str_manifest(t).unwrap_err();
        let want = format!(
            "{}: {}",
            snap.join("project.yaml").display(),
            yaml("targets: [claude\nx: 1\n")
        );
        assert_eq!(e, want);
        let pack_msg = e.clone();
        // the project's own: no pack manifest to read first, the project's is merged
        fx.snapshot(C1, &[]);
        std::fs::write(snap.join("project.yaml"), MANIFEST).unwrap();
        put(
            fx.root.path(),
            ".armadai/project.yaml",
            &format!("{REF}targets: [claude\n"),
        );
        // an unreadable project.yaml hides its pack key: the lock is read alone
        let e = open_with_lock_in(
            fx.root.path(),
            &lock(C1).into(),
            Vars::Literal,
            fx.registry.path(),
        )
        .err()
        .unwrap()
        .to_string();
        let want = format!(
            "{}: ",
            fx.root.path().join(".armadai/project.yaml").display()
        );
        assert!(e.starts_with(&want), "{e}");
        assert_ne!(e, pack_msg);
    }

    // ---- standards packs ----

    const C3: &str = "3333333333333333333333333333333333333333";
    const STD_PACK: &str = "format: 1\nname: std\nkind: standards\ndescription: d\n";

    fn std_url(name: &str) -> String {
        format!("https://example.com/{name}.git")
    }

    fn std_entry(name: &str) -> String {
        format!(
            "  - {{ name: {name}, source: '{}', version: latest }}\n",
            std_url(name)
        )
    }

    fn std_lock(name: &str, commit: &str) -> PackLock {
        let mut l = lock(commit);
        l.pack = name.into();
        l.source = std_url(name);
        l
    }

    fn skill_of(id: &str) -> String {
        format!("---\nname: {id}\ndescription: d {id}\n---\nbody of {id}\n")
    }

    /// A project with `standards:` entries (and a `pack:` when `team`), a registry, a lock.
    struct Sx {
        registry: tempfile::TempDir,
        root: tempfile::TempDir,
    }

    impl Sx {
        fn new(team: bool, standards: &[&str]) -> Sx {
            let sx = Sx {
                registry: tempfile::tempdir().unwrap(),
                root: tempfile::tempdir().unwrap(),
            };
            let mut yaml = String::from(MANIFEST);
            if team {
                yaml.push_str(REF);
            }
            if !standards.is_empty() {
                yaml.push_str("standards:\n");
                standards.iter().for_each(|n| yaml.push_str(&std_entry(n)));
            }
            put(sx.root.path(), ".armadai/project.yaml", &yaml);
            sx
        }
        fn lock(&self, team: bool, standards: &[PackLock]) {
            let lf = LockFile {
                team: team.then(|| lock(C1)),
                standards: standards.to_vec(),
            };
            lock::write(self.root.path(), &lf).unwrap();
        }
        fn snap(&self, name: &str, commit: &str, files: &[(&str, &str)]) -> PathBuf {
            let dir = crate::pack::cache::snapshot_dir_in(
                self.registry.path(),
                &std_url(name),
                name,
                commit,
            );
            for (rel, text) in files {
                put(&dir, rel, text);
            }
            dir
        }
        fn team_snap(&self, files: &[(&str, &str)]) -> PathBuf {
            let dir = crate::pack::cache::snapshot_dir_in(self.registry.path(), URL, "demo", C1);
            put(&dir, "project.yaml", MANIFEST);
            for (rel, text) in files {
                put(&dir, rel, text);
            }
            dir
        }
        fn open(&self) -> anyhow::Result<Effective> {
            open_in(self.root.path(), Vars::Literal, self.registry.path())
        }
        fn err(&self) -> String {
            self.open().err().expect("an error").to_string()
        }
    }

    fn skill_ids(e: &Effective) -> Vec<String> {
        let (p, _) = e.load_checked().unwrap();
        p.skills.keys().cloned().collect()
    }

    fn codes(ds: &[Diagnostic]) -> Vec<&'static str> {
        ds.iter().map(|d| d.code).collect()
    }

    #[test]
    fn a_standards_only_project_reads_the_standards_skill() {
        let sx = Sx::new(false, &["std-a"]);
        sx.lock(false, &[std_lock("std-a", C1)]);
        let snap = sx.snap(
            "std-a",
            C1,
            &[
                ("pack.yaml", STD_PACK),
                ("skills/s/SKILL.md", &skill_of("s")),
            ],
        );
        let e = sx.open().unwrap();
        assert_ne!(e.sources, sx.root.path());
        assert!(e.pack.is_none());
        let (p, o) = e.load_checked().unwrap();
        assert_eq!(p.skills["s"].body.trim(), "body of s");
        let files: Vec<&PathBuf> = o.files.values().map(|(f, _)| f).collect();
        assert!(
            files.contains(&&snap.join("skills/s/SKILL.md")),
            "{files:?}"
        );
        assert_eq!(e.standards[0].snapshot, snap);
    }

    #[test]
    fn team_and_two_standards_are_all_read() {
        let sx = Sx::new(true, &["std-a", "std-b"]);
        sx.lock(true, &[std_lock("std-a", C2), std_lock("std-b", C3)]);
        sx.team_snap(&[("skills/t/SKILL.md", &skill_of("t"))]);
        for (n, c, id) in [("std-a", C2, "a-s"), ("std-b", C3, "b-s")] {
            sx.snap(
                n,
                c,
                &[
                    ("pack.yaml", STD_PACK),
                    (&format!("skills/{id}/SKILL.md"), &skill_of(id)),
                ],
            );
        }
        let e = sx.open().unwrap();
        assert_eq!(skill_ids(&e), ["a-s", "b-s", "t"]);
        assert_eq!(
            e.standards
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["std-a", "std-b"]
        );
    }

    #[test]
    fn standards_lock_messages_are_the_planned_texts() {
        let ym = ".armadai/project.yaml";
        // E1: no lock at all
        let sx = Sx::new(false, &["std-a"]);
        assert_eq!(
            sx.err(),
            ".armadai/project.yaml references standards std-a but .armadai/pack.lock is missing: run `armadai pack update`"
        );
        // E2: a lock without the entry
        sx.lock(false, &[std_lock("std-b", C1)]);
        assert_eq!(
            sx.err(),
            ".armadai/project.yaml references standards std-a but .armadai/pack.lock has no entry for it: run `armadai pack update`"
        );
        // E4: the lock is for another source
        let mut other = std_lock("std-a", C1);
        other.source = "https://example.com/else.git".into();
        sx.lock(false, &[other]);
        assert_eq!(
            sx.err(),
            ".armadai/pack.lock is for standards std-a from https://example.com/else.git, but .armadai/project.yaml references std-a from https://example.com/std-a.git: run `armadai pack update`"
        );
        // E6: the snapshot is not in the cache
        sx.lock(false, &[std_lock("std-a", C1)]);
        assert_eq!(
            sx.err(),
            "standards std-a 1.0.0 (111111111111) is not in the local cache: run `armadai pack sync`"
        );
        // E10: an id that is not a commit name
        sx.lock(false, &[std_lock("std-a", "../../etc")]);
        assert_eq!(
            sx.err(),
            "standards std-a has an invalid name or version id: ../../etc"
        );
        // E7: the same name twice
        let twice = Sx::new(false, &["std-a", "std-a"]);
        twice.lock(false, &[std_lock("std-a", C1)]);
        assert_eq!(
            twice.err(),
            ".armadai/project.yaml references standards std-a twice: keep one entry"
        );
        // E3 and E5: under a pin of this checkout
        let pinned = Sx::new(true, &[]);
        std::fs::create_dir_all(pinned.root.path().join(".git")).unwrap();
        pinned.lock(true, &[]);
        lock::pin(pinned.root.path()).unwrap();
        let committed = {
            let scratch = tempfile::tempdir().unwrap();
            let lf = LockFile {
                team: Some(lock(C1)),
                standards: vec![std_lock("std-a", C1)],
            };
            lock::write(scratch.path(), &lf).unwrap();
            std::fs::read_to_string(scratch.path().join(lock::LOCK)).unwrap()
        };
        std::fs::write(pinned.root.path().join(lock::LOCK), &committed).unwrap();
        put(
            pinned.root.path(),
            ym,
            &format!("{MANIFEST}{REF}standards:\n{}", std_entry("std-a")),
        );
        assert_eq!(
            pinned.err(),
            ".armadai/project.yaml references standards std-a but the pack pin of this checkout has no entry for it: run armadai pack unpin, then armadai pack update"
        );
        lock::unpin(pinned.root.path()).unwrap();
        // the pin holds std-a from another source than the project's
        let mut moved = std_lock("std-a", C1);
        moved.source = "https://example.com/else.git".into();
        let moved_text = {
            let scratch = tempfile::tempdir().unwrap();
            let lf = LockFile {
                team: Some(lock(C1)),
                standards: vec![moved],
            };
            lock::write(scratch.path(), &lf).unwrap();
            std::fs::read_to_string(scratch.path().join(lock::LOCK)).unwrap()
        };
        pinned.team_snap(&[]);
        let pin_file = lock::pin_path(pinned.root.path()).unwrap().unwrap();
        put(pin_file.parent().unwrap(), "pack.lock", &moved_text);
        assert_eq!(
            pinned.err(),
            "the pack pin of this checkout is for standards std-a from https://example.com/else.git, but .armadai/project.yaml references std-a from https://example.com/std-a.git: run armadai pack unpin"
        );
    }

    #[test]
    fn a_broken_dot_git_refuses_a_standards_project() {
        let sx = Sx::new(false, &["std-a"]);
        sx.lock(false, &[std_lock("std-a", C1)]);
        sx.snap(
            "std-a",
            C1,
            &[
                ("pack.yaml", STD_PACK),
                ("skills/s/SKILL.md", &skill_of("s")),
            ],
        );
        let dot_git = sx.root.path().join(".git");
        std::fs::write(
            &dot_git,
            format!("gitdir: {}\n", sx.root.path().join("absent").display()),
        )
        .unwrap();
        assert_eq!(
            sx.err(),
            format!(
                "{} does not lead to a git directory, where this checkout keeps its pack pin; repair it (git worktree repair) and run the command again",
                dot_git.display()
            )
        );
    }

    #[test]
    fn a_team_pack_with_standards_is_refused_by_name() {
        let fx = Fx::new();
        fx.snapshot(
            C1,
            &[(
                "project.yaml",
                &format!("{MANIFEST}standards:\n{}", std_entry("std-a")),
            )],
        );
        assert_eq!(
            fx.err(),
            "pack demo: its own project.yaml references standards packs (standards:), which is not supported"
        );
    }

    #[test]
    fn a_team_pack_named_in_standards_is_wrong_kind() {
        let sx = Sx::new(false, &["std-a"]);
        sx.lock(false, &[std_lock("std-a", C1)]);
        let snap = sx.snap(
            "std-a",
            C1,
            &[
                ("pack.yaml", "format: 1\nname: std-a\ndescription: d\n"),
                ("agents/a.md", AGENT_A),
                ("skills/s/SKILL.md", &skill_of("s")),
            ],
        );
        let e = sx.open().unwrap();
        let ds = e.validate();
        // the wrong kind and, as for any standards pack, each file outside skills/
        let got: Vec<(&str, PathBuf, &str)> = (ds.iter())
            .filter(|d| d.code.starts_with("standards-"))
            .map(|d| (d.code, d.file.clone(), d.message.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (
                    "standards-wrong-kind",
                    snap.join("pack.yaml"),
                    "standards std-a is a team pack (its pack.yaml has no kind: standards): reference it with pack:, not standards:"
                ),
                (
                    "standards-foreign-file",
                    snap.join("agents/a.md"),
                    "a standards pack holds only skills: `agents/a.md` is not under skills/"
                ),
            ]
        );
        assert!(e.load_checked().is_err());
    }

    #[test]
    fn a_foreign_file_of_a_standards_pack_is_an_error_but_pack_yaml_and_readme_are_not() {
        let sx = Sx::new(false, &["std-a"]);
        sx.lock(false, &[std_lock("std-a", C1)]);
        let snap = sx.snap(
            "std-a",
            C1,
            &[
                ("pack.yaml", STD_PACK),
                ("README.md", "readme"),
                ("agents/a.md", AGENT_A),
                ("skills/s/SKILL.md", &skill_of("s")),
            ],
        );
        let e = sx.open().unwrap();
        let ds = e.validate();
        let foreign: Vec<&Diagnostic> = (ds.iter())
            .filter(|d| d.code == "standards-foreign-file")
            .collect();
        assert_eq!(foreign.len(), 1, "{foreign:?}");
        assert_eq!(foreign[0].file, snap.join("agents/a.md"));
        assert_eq!(
            foreign[0].message,
            "a standards pack holds only skills: `agents/a.md` is not under skills/"
        );
        assert!(e.load_checked().is_err());
    }

    #[test]
    fn two_sources_of_one_skill_are_named() {
        // two standards packs
        let sx = Sx::new(false, &["std-a", "std-b"]);
        sx.lock(false, &[std_lock("std-a", C2), std_lock("std-b", C3)]);
        sx.snap(
            "std-a",
            C2,
            &[
                ("pack.yaml", STD_PACK),
                ("skills/s/SKILL.md", &skill_of("s")),
            ],
        );
        let b = sx.snap(
            "std-b",
            C3,
            &[
                ("pack.yaml", STD_PACK),
                ("skills/s/SKILL.md", &skill_of("s")),
            ],
        );
        let e = sx.open().unwrap();
        let ds = e.validate();
        let d = ds
            .iter()
            .find(|d| d.code == "duplicate-standards-skill")
            .unwrap();
        assert_eq!(
            d.message,
            "skill s comes from both standards std-a and standards std-b"
        );
        assert_eq!(d.file, b.join("skills/s/SKILL.md"));
        assert_eq!(
            codes(&ds)
                .iter()
                .filter(|c| **c == "duplicate-standards-skill")
                .count(),
            1
        );
        assert!(e.load_checked().is_err());
        // the project's own folder lifts it, and its skill is the one read
        put(
            sx.root.path(),
            ".armadai/skills/s/SKILL.md",
            "---\nname: s\ndescription: mine\n---\nmine\n",
        );
        let e = sx.open().unwrap();
        let (p, _) = e.load_checked().unwrap();
        assert_eq!(p.skills["s"].body.trim(), "mine");
        // the team pack and a standards pack
        let tm = Sx::new(true, &["std-a"]);
        tm.lock(true, &[std_lock("std-a", C2)]);
        tm.team_snap(&[("skills/s/SKILL.md", &skill_of("s"))]);
        tm.snap(
            "std-a",
            C2,
            &[
                ("pack.yaml", STD_PACK),
                ("skills/s/SKILL.md", &skill_of("s")),
            ],
        );
        let ds = tm.open().unwrap().validate();
        let d = ds
            .iter()
            .find(|d| d.code == "duplicate-standards-skill")
            .unwrap();
        assert_eq!(
            d.message,
            "skill s comes from both pack demo and standards std-a"
        );
    }

    #[test]
    fn an_empty_standards_source_is_a_diagnostic_not_a_lock_error() {
        let sx = Sx::new(false, &[]);
        put(
            sx.root.path(),
            ".armadai/project.yaml",
            &format!("{MANIFEST}standards:\n  - {{ name: std-a, source: '', version: latest }}\n"),
        );
        let e = sx.open().unwrap();
        assert_eq!(codes(&e.validate()), ["standards-empty-source"]);
    }

    #[test]
    fn skill_packs_names_only_standards_skills() {
        let sx = Sx::new(true, &["std-a"]);
        sx.lock(true, &[std_lock("std-a", C2)]);
        sx.team_snap(&[("skills/t/SKILL.md", &skill_of("t"))]);
        sx.snap(
            "std-a",
            C2,
            &[
                ("pack.yaml", STD_PACK),
                ("skills/a-s/SKILL.md", &skill_of("a-s")),
            ],
        );
        put(sx.root.path(), ".armadai/skills/p/SKILL.md", &skill_of("p"));
        let e = sx.open().unwrap();
        assert_eq!(
            e.skill_packs(),
            BTreeMap::from([("a-s".to_string(), "std-a".to_string())])
        );
    }

    #[test]
    fn a_lock_given_is_the_teams_before_the_pack_key_exists() {
        let fx = Fx::new();
        put(fx.root.path(), ".armadai/project.yaml", MANIFEST);
        fx.snapshot(
            C1,
            &[("project.yaml", MANIFEST), ("agents/one.md", AGENT_A)],
        );
        let e = open_with_lock_in(
            fx.root.path(),
            &lock(C1).into(),
            Vars::Literal,
            fx.registry.path(),
        )
        .unwrap();
        assert_eq!(agents(&e), ["one"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_executable_bit_of_a_standards_script_is_kept() {
        use std::os::unix::fs::PermissionsExt;
        let sx = Sx::new(false, &["std-a"]);
        sx.lock(false, &[std_lock("std-a", C1)]);
        let snap = sx.snap(
            "std-a",
            C1,
            &[
                ("pack.yaml", STD_PACK),
                ("skills/s/SKILL.md", &skill_of("s")),
                ("skills/s/run.sh", "#!/bin/sh\n"),
            ],
        );
        std::fs::set_permissions(
            snap.join("skills/s/run.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let e = sx.open().unwrap();
        let mode = |f: &str| {
            std::fs::metadata(e.sources.join(f))
                .unwrap()
                .permissions()
                .mode()
                & 0o111
        };
        assert_ne!(mode(".armadai/skills/s/run.sh"), 0);
        assert_eq!(mode(".armadai/skills/s/SKILL.md"), 0);
    }
}
