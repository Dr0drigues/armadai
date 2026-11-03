//! The registry of projects referencing a pack (`~/.config/armadai/pack-referents.yaml`).
//!
//! Independent of `projects.json`: `project_registry::prune_stale` never sees it. Nothing
//! here removes an entry; a project that moved or dropped the pack is reported.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE: &str = "pack-referents.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Referent {
    pub pack: String,
    pub source: String,
    pub project: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Present,
    /// No `.armadai/project.yaml` at that path any more.
    Missing,
    /// Its `pack:` names another pack or source, or none.
    NoLongerReferences,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Doc {
    format: u32,
    #[serde(default)]
    referents: Vec<Referent>,
}

pub fn path_in(config: &Path) -> PathBuf {
    config.join(FILE)
}

/// Only `format` is read first, so a newer format with new fields still says so.
#[derive(Deserialize)]
struct Head {
    format: u32,
}

/// Empty when the file is absent; an unreadable or foreign file is an error.
pub fn load_in(config: &Path) -> anyhow::Result<Vec<Referent>> {
    let p = path_in(config);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => anyhow::bail!("{}: {e}", p.display()),
    };
    let bad = |e: &dyn std::fmt::Display| anyhow::anyhow!("{}: {e}", p.display());
    let head: Head = armadai_model::yaml::from_str(&text).map_err(|e| bad(&e))?;
    if head.format != 1 {
        anyhow::bail!(
            "{}: unsupported format {} (expected 1)",
            p.display(),
            head.format
        );
    }
    let doc: Doc = armadai_model::yaml::from_str(&text).map_err(|e| bad(&e))?;
    Ok(doc.referents)
}

/// Add or keep `(pack, source, project)` (project made absolute, canonical when it
/// exists); never removes an entry. Read and write run under an exclusive lock on
/// `pack-referents.yaml.lock`, so concurrent calls lose nothing.
pub fn record_in(config: &Path, mut r: Referent) -> anyhow::Result<()> {
    let abs = std::path::absolute(&r.project).unwrap_or(r.project);
    r.project = std::fs::canonicalize(&abs).unwrap_or(abs);
    std::fs::create_dir_all(config)?;
    let lock = path_in(config).with_extension("yaml.lock");
    if std::fs::symlink_metadata(&lock).is_ok_and(|m| m.file_type().is_symlink()) {
        anyhow::bail!(
            "{} is a symbolic link; it is not written through",
            lock.display()
        );
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .map_err(|e| anyhow::anyhow!("opening the lock {}: {e}", lock.display()))?;
    file.lock()
        .map_err(|e| anyhow::anyhow!("locking {}: {e}", lock.display()))?;
    let mut all = load_in(config)?;
    if all.contains(&r) {
        return Ok(());
    }
    all.push(r);
    write(config, all) // dropping `file` releases the lock
}

/// Written whole (`.tmp` then rename), never through a symbolic link.
fn write(config: &Path, referents: Vec<Referent>) -> anyhow::Result<()> {
    let p = path_in(config);
    let tmp = p.with_extension("yaml.tmp");
    for f in [&p, &tmp] {
        if std::fs::symlink_metadata(f).is_ok_and(|m| m.file_type().is_symlink()) {
            anyhow::bail!(
                "{} is a symbolic link; it is not written through",
                f.display()
            );
        }
    }
    let text = armadai_model::yaml::to_string(&Doc {
        format: 1,
        referents,
    })
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

fn standing(r: &Referent) -> Standing {
    if !r.project.join(".armadai/project.yaml").is_file() {
        return Standing::Missing;
    }
    match super::effective::pack_ref(&r.project) {
        Some((name, source)) if name == r.pack && source == r.source => Standing::Present,
        _ => Standing::NoLongerReferences,
    }
}

/// The other projects referencing `pack` from `source`, each with its standing.
pub fn others(
    config: &Path,
    pack: &str,
    source: &str,
    current: &Path,
) -> anyhow::Result<Vec<(PathBuf, Standing)>> {
    let current = std::fs::canonicalize(current).unwrap_or_else(|_| current.to_path_buf());
    Ok(load_in(config)?
        .into_iter()
        .filter(|r| r.pack == pack && r.source == source && r.project != current)
        .map(|r| {
            let s = standing(&r);
            (r.project, s)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(dir: &Path, name: &str, pack: Option<&str>) -> PathBuf {
        let root = dir.join(name);
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        let mut y = String::from("schema_version: 1\n");
        if let Some(p) = pack {
            y.push_str(&format!(
                "pack:\n  name: {p}\n  source: https://example.com/packs.git\n  version: latest\n"
            ));
        }
        std::fs::write(root.join(".armadai/project.yaml"), y).unwrap();
        std::fs::canonicalize(root).unwrap()
    }

    fn r(pack: &str, project: &Path) -> Referent {
        Referent {
            pack: pack.into(),
            source: "https://example.com/packs.git".into(),
            project: project.to_path_buf(),
        }
    }

    #[test]
    fn record_is_idempotent_and_keeps_order() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("cfg");
        let (a, b, c) = (
            project(t.path(), "a", Some("demo")),
            project(t.path(), "b", Some("demo")),
            project(t.path(), "c", Some("demo")),
        );
        for p in [&b, &b, &a, &c, &a] {
            record_in(&cfg, r("demo", p)).unwrap();
        }
        let got: Vec<_> = load_in(&cfg)
            .unwrap()
            .into_iter()
            .map(|x| x.project)
            .collect();
        assert_eq!(got, vec![b, a, c]);
    }

    #[test]
    fn a_moved_project_is_reported_never_removed() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("cfg");
        let (cur, gone, other) = (
            project(t.path(), "cur", Some("demo")),
            project(t.path(), "gone", Some("demo")),
            project(t.path(), "other", Some("demo")),
        );
        for p in [&cur, &gone, &other] {
            record_in(&cfg, r("demo", p)).unwrap();
        }
        record_in(&cfg, r("elsewhere", &other)).unwrap(); // another pack: not listed
        record_in(
            &cfg,
            Referent {
                source: "https://example.com/else.git".into(),
                ..r("demo", &other)
            },
        )
        .unwrap(); // another source: not listed
        std::fs::remove_dir_all(&gone).unwrap();
        std::fs::write(
            other.join(".armadai/project.yaml"),
            "schema_version: 1\npack:\n  name: elsewhere\n  source: https://example.com/packs.git\n  version: latest\n",
        )
        .unwrap();
        let before = std::fs::read(path_in(&cfg)).unwrap();
        let got = others(&cfg, "demo", "https://example.com/packs.git", &cur).unwrap();
        assert_eq!(
            got,
            vec![
                (gone, Standing::Missing),
                (other, Standing::NoLongerReferences)
            ]
        );
        assert_eq!(std::fs::read(path_in(&cfg)).unwrap(), before);
    }

    #[test]
    fn a_referencing_project_is_present() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("cfg");
        let (cur, ok) = (
            project(t.path(), "cur", Some("demo")),
            project(t.path(), "ok", Some("demo")),
        );
        record_in(&cfg, r("demo", &ok)).unwrap();
        let got = others(&cfg, "demo", "https://example.com/packs.git", &cur).unwrap();
        assert_eq!(got, vec![(ok, Standing::Present)]);
    }

    #[test]
    fn prune_stale_leaves_the_referents_alone() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("cfg");
        let gone = t.path().join("gone"); // never a project
        record_in(&cfg, r("demo", &gone)).unwrap();
        let before = std::fs::read(path_in(&cfg)).unwrap();
        // projects.json lives next to it and holds the same absent project.
        let registry = cfg.join("projects.json");
        assert_ne!(path_in(&cfg), registry);
        let mut reg = armadai_core::project_registry::ProjectRegistry {
            projects: vec![armadai_core::project_registry::ProjectEntry {
                path: gone.to_string_lossy().into(),
                last_seen: "2026-01-01T00:00:00Z".into(),
            }],
        };
        let pruned = armadai_core::project_registry::prune_stale(&mut reg);
        std::fs::write(&registry, serde_json::to_string(&reg).unwrap()).unwrap();
        assert_eq!(pruned.len(), 1);
        assert_eq!(std::fs::read(path_in(&cfg)).unwrap(), before);
        assert_eq!(load_in(&cfg).unwrap().len(), 1);
    }

    #[test]
    fn a_symbolic_link_is_not_written_through() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let target = t.path().join("target");
        std::fs::write(&target, "format: 1\n").unwrap();
        std::os::unix::fs::symlink(&target, path_in(&cfg)).unwrap();
        let err = record_in(&cfg, r("demo", t.path())).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "{} is a symbolic link; it is not written through",
                path_in(&cfg).display()
            )
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "format: 1\n");
    }

    #[test]
    fn an_unreadable_or_foreign_file_fails_record_and_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let p = path_in(&cfg);
        let proj = project(t.path(), "a", Some("demo"));
        let shown = p.display();
        for (text, msg) in [
            ("{{{ user data", None),
            (
                "format: 2\nreferents: []\n",
                Some(format!("{shown}: unsupported format 2 (expected 1)")),
            ),
            (
                "format: 2\nsomething_new: 1\n",
                Some(format!("{shown}: unsupported format 2 (expected 1)")),
            ),
            ("format: 1\nextra: 1\n", None),
        ] {
            std::fs::write(&p, text).unwrap();
            let err = record_in(&cfg, r("demo", &proj)).unwrap_err().to_string();
            match msg {
                Some(m) => assert_eq!(err, m),
                None => assert!(err.starts_with(&format!("{shown}: ")), "{err}"),
            }
            assert_eq!(std::fs::read_to_string(&p).unwrap(), text);
            assert!(load_in(&cfg).is_err());
        }
    }

    #[test]
    fn concurrent_records_keep_every_entry() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("cfg");
        let projects: Vec<PathBuf> = (0..8)
            .map(|i| project(t.path(), &format!("p{i}"), Some("demo")))
            .collect();
        for _ in 0..10 {
            let _ = std::fs::remove_dir_all(&cfg);
            std::thread::scope(|s| {
                for p in &projects {
                    let (cfg, p) = (&cfg, p);
                    s.spawn(move || record_in(cfg, r("demo", p)).unwrap());
                }
            });
            let mut got: Vec<_> = load_in(&cfg)
                .unwrap()
                .into_iter()
                .map(|x| x.project)
                .collect();
            let mut want = projects.clone();
            got.sort();
            want.sort();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn a_relative_project_is_stored_absolute() {
        let t = tempfile::tempdir().unwrap();
        record_in(t.path(), r("demo", Path::new("no/such/dir"))).unwrap();
        assert!(load_in(t.path()).unwrap()[0].project.is_absolute());
    }
}
