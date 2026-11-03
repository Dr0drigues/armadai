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

/// Empty when the file is absent.
pub fn load_in(config: &Path) -> anyhow::Result<Vec<Referent>> {
    let p = path_in(config);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => anyhow::bail!("{}: {e}", p.display()),
    };
    let doc: Doc = armadai_model::yaml::from_str(&text)
        .map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
    if doc.format != 1 {
        anyhow::bail!(
            "{}: unsupported format {} (expected 1)",
            p.display(),
            doc.format
        );
    }
    Ok(doc.referents)
}

/// Add or keep `(pack, source, project)` (project canonicalised); never removes an entry.
pub fn record_in(config: &Path, mut r: Referent) -> anyhow::Result<()> {
    r.project = std::fs::canonicalize(&r.project).unwrap_or(r.project);
    let mut all = load_in(config)?;
    if all.contains(&r) {
        return Ok(());
    }
    all.push(r);
    write(config, all)
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
    std::fs::create_dir_all(config)?;
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
        assert!(
            err.to_string()
                .ends_with("is a symbolic link; it is not written through")
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "format: 1\n");
    }
}
