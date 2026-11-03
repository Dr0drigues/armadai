//! The effective project of a directory: its `.armadai/` as the commands must read it.
//!
//! With a `pack:` key, the locked snapshot of the pack overlaid by the project's own
//! files; with variables to resolve, `{{var.name}}` rendered. The result is a scratch
//! copy that is dropped with the [`Effective`]; `.armadai/` and the cache are never
//! written. A project with neither is read in place, nothing copied.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_model::pack::{Layer, OverlayError};
use armadai_model::{Diagnostic, Origins, Project};

use super::cache::{Available, valid_id, valid_name};
use super::lock::{self, PackLock};
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

pub struct Effective {
    /// The directory holding the `.armadai/` to read: `root`, or a scratch copy.
    pub sources: PathBuf,
    pub pack: Option<LockedPack>,
    /// Variables no layer sets (`Vars::Resolve` only): rendered empty.
    pub missing_vars: BTreeSet<String>,
    root: PathBuf,
    /// path relative to `.armadai/` -> (layer, the real file)
    origins: BTreeMap<PathBuf, (Layer, PathBuf)>,
    _scratch: Option<tempfile::TempDir>,
}

fn id12(id: &str) -> &str {
    let id = id.strip_prefix("sha256-").unwrap_or(id);
    &id[..id.len().min(12)]
}

/// The `pack:` key of the project's `project.yaml` as (name, source); `None` when
/// there is none, or the file is not readable (the loader reports that, with a position).
pub(super) fn pack_ref(root: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(root.join(".armadai/project.yaml")).ok()?;
    let v = armadai_model::yaml::from_str_manifest(&text).ok()?;
    let p = v.get("pack")?;
    Some((
        p.get("name")?.as_str()?.to_string(),
        p.get("source")?.as_str()?.to_string(),
    ))
}

pub fn open(root: &Path, vars: Vars) -> anyhow::Result<Effective> {
    open_in(root, vars, &armadai_core::config::registry_cache_dir())
}

/// [`open`] with the registry directory given.
pub fn open_in(root: &Path, vars: Vars, registry: &Path) -> anyhow::Result<Effective> {
    let Some((name, _)) = pack_ref(root) else {
        return build(root, None, vars, registry);
    };
    let Some(lock) = lock::read(root)? else {
        bail!(
            ".armadai/project.yaml references pack {name} but .armadai/pack.lock is missing: \
             run `armadai pack update`"
        );
    };
    build(root, Some(&lock), vars, registry)
}

/// Same, with the lock given (`pack add` and `pack update` check a lock before writing it).
pub fn open_with_lock(root: &Path, lock: &PackLock, vars: Vars) -> anyhow::Result<Effective> {
    open_with_lock_in(
        root,
        lock,
        vars,
        &armadai_core::config::registry_cache_dir(),
    )
}

pub fn open_with_lock_in(
    root: &Path,
    lock: &PackLock,
    vars: Vars,
    registry: &Path,
) -> anyhow::Result<Effective> {
    build(root, Some(lock), vars, registry)
}

fn snapshot(root: &Path, lock: &PackLock, registry: &Path) -> anyhow::Result<PathBuf> {
    if let Some((name, source)) = pack_ref(root)
        && (name != lock.pack || source != lock.source)
    {
        bail!(
            ".armadai/pack.lock is for pack {} from {}, but .armadai/project.yaml references {name} \
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
        bail!("pack {} has a version without commit or digest", lock.pack);
    }
    for (bad, ok) in [(&lock.pack, valid_name(&lock.pack)), (&id, valid_id(&id))] {
        if !ok {
            bail!(
                "pack {} has an invalid name or version id: {bad}",
                lock.pack
            );
        }
    }
    let dir = super::cache::snapshot_dir_in(registry, &lock.source, &lock.pack, &id);
    if !dir.is_dir() {
        bail!(
            "pack {} {} ({}) is not in the local cache: run `armadai pack sync`",
            lock.pack,
            lock.resolved.version,
            id12(&id)
        );
    }
    Ok(dir)
}

fn build(
    root: &Path,
    lock: Option<&PackLock>,
    vars: Vars,
    registry: &Path,
) -> anyhow::Result<Effective> {
    let in_place = |pack| Effective {
        sources: root.to_path_buf(),
        pack,
        missing_vars: BTreeSet::new(),
        root: root.to_path_buf(),
        origins: BTreeMap::new(),
        _scratch: None,
    };
    if lock.is_none() && matches!(vars, Vars::Literal) {
        return Ok(in_place(None));
    }
    let base = root.join(".armadai");
    // A plain project is only copied when a variable must be rendered: until then a link
    // is no reason to refuse it (the loader reads it as before).
    if lock.is_none() {
        let looked: BTreeMap<PathBuf, Vec<u8>> = native_pack::read_listed(&base, false)?
            .into_iter()
            .map(|(p, b, _)| (p, b))
            .collect();
        if !matches!(armadai_model::pack_vars::files_names(&looked), Ok(n) if !n.is_empty()) {
            return Ok(in_place(None));
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
    let mut files: BTreeMap<PathBuf, (Layer, Vec<u8>)> = BTreeMap::new();
    if let Some(lock) = lock {
        let snap = snapshot(root, lock, registry)?;
        let pack = native_pack::files(&snap)?;
        exec_pack.extend(pack.files.iter().filter(|f| f.2).map(|f| f.0.clone()));
        let pack: BTreeMap<PathBuf, Vec<u8>> =
            pack.files.into_iter().map(|(p, b, _)| (p, b)).collect();
        files = armadai_model::pack::overlay(&pack, &project).map_err(|e| match e {
            OverlayError::NestedPack => anyhow::anyhow!(
                "pack {}: its own project.yaml references a pack (pack:), which is not supported",
                lock.pack
            ),
            OverlayError::InvalidYaml { layer, message } => {
                let file = match layer {
                    Layer::Pack => snap.join("project.yaml"),
                    _ => base.join("project.yaml"),
                };
                anyhow::anyhow!("{}: {message}", file.display())
            }
        })?;
        locked = Some(LockedPack {
            name: lock.pack.clone(),
            lock: lock.clone(),
            snapshot: snap,
        });
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
            Err(_) if locked.is_none() => return Ok(in_place(None)),
            Err(e) => bail!(e),
        }
    }
    if locked.is_none() && !rendered {
        return Ok(in_place(None));
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
            Layer::Project => exec_project.contains(rel),
            Layer::Merged => false,
        } {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o755))?;
        }
        let real = match (layer, &locked) {
            (Layer::Pack, Some(l)) => l.snapshot.join(rel),
            (Layer::Merged, _) => base.join("project.yaml"),
            _ => base.join(rel),
        };
        origins.insert(rel.clone(), (*layer, real));
    }
    Ok(Effective {
        sources: scratch.path().to_path_buf(),
        pack: locked,
        missing_vars,
        root: root.to_path_buf(),
        origins,
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

    /// `armadai_model::load_checked` of the view; diagnostics and origins on the real files.
    pub fn load_checked(&self) -> Result<(Project, Origins), Vec<Diagnostic>> {
        match armadai_model::load_checked(&self.sources) {
            Ok((p, mut o)) => {
                self.remap_origins(&mut o);
                Ok((p, o))
            }
            Err(ds) => Err(self.remap(ds)),
        }
    }

    /// `armadai_model::load_project` of the view, as far as it loads; diagnostics and
    /// origins on the real files.
    pub fn load_project(&self) -> armadai_model::Loaded {
        let mut l = armadai_model::load_project(&self.sources);
        l.diagnostics = self.remap(std::mem::take(&mut l.diagnostics));
        self.remap_origins(&mut l.origins);
        l
    }

    pub fn validate(&self) -> Vec<Diagnostic> {
        self.remap(armadai_model::validate(&self.sources))
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
            lock::write(fx.root.path(), &lock(C1)).unwrap();
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
        lock::write(fx.root.path(), &lock(C2)).unwrap();
        assert_eq!(agents(&fx.open(Vars::Literal).unwrap()), ["two"]);
        // a lock given by the caller wins over the file
        let e = open_with_lock_in(fx.root.path(), &lock(C1), Vars::Literal, fx.registry.path())
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
        lock::write(fx.root.path(), &other).unwrap();
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
        lock::write(fx.root.path(), &bad).unwrap();
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
        let e = open_with_lock_in(fx.root.path(), &lock(C1), Vars::Literal, fx.registry.path())
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
}
