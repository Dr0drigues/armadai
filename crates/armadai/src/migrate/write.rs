//! Writing a migration: backup, removal of the legacy files, new files.

use std::path::{Path, PathBuf};

use anyhow::Context;
use armadai_model::Project;

use super::convert::SkillCopy;
use super::read::{CONFIG_FILES, OldProject};

/// Legacy paths under the project root that the new format replaces: backed up, then removed.
/// `project.yaml`, `flows/`, `automations/` only exist on a `--force` rerun.
const REPLACED: [&str; 7] = [
    ".armadai/agents.yaml",
    ".armadai/agents",
    ".armadai/prompts",
    ".armadai/skills",
    ".armadai/project.yaml",
    ".armadai/flows",
    ".armadai/automations",
];

/// What a migration does to the legacy files, decided before anything is touched.
#[derive(Debug, Default)]
pub struct Plan {
    /// Relative paths copied to the backup.
    pub backup: Vec<PathBuf>,
    /// Relative paths removed after the backup.
    pub remove: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

fn rel(root: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(root).unwrap_or(p).to_path_buf()
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

pub fn plan(old: &OldProject, copies: &[SkillCopy]) -> Plan {
    let root = &old.root;
    let mut plan = Plan::default();
    // only the config file actually read goes; another one is left, reported below
    let config = rel(root, &old.config_file);
    let replaced = std::iter::once(config.clone())
        .chain(REPLACED.iter().map(PathBuf::from))
        .filter(|p| root.join(p).exists());
    for p in replaced {
        plan.backup.push(p.clone());
        plan.remove.push(p);
    }
    for other in CONFIG_FILES.iter().map(PathBuf::from) {
        if other != config && root.join(&other).is_file() {
            plan.warnings.push(format!(
                "legacy file left in place: {} (not read: {} takes precedence)",
                other.display(),
                config.display()
            ));
            plan.backup.push(other);
        }
    }

    // every source the conversion read from inside the project
    let mut sources: Vec<PathBuf> = old.agents.iter().map(|a| a.source.clone()).collect();
    sources.extend(old.prompts.iter().map(|p| p.source.clone()));
    sources.extend(copies.iter().map(|c| c.from.clone()));
    sources.extend(old.fragment_sources.iter().cloned());
    let removed = |p: &Path| plan.remove.iter().any(|r| p.starts_with(root.join(r)));
    let mut left = Vec::new();
    for s in sources.iter().filter(|s| s.starts_with(root)) {
        if !removed(s) && !left.contains(s) {
            left.push(s.clone());
        }
    }
    for s in left {
        plan.warnings.push(format!(
            "legacy file left in place: {}",
            rel(root, &s).display()
        ));
        plan.backup.push(rel(root, &s));
    }

    // files about to be removed that no conversion read
    let mut removed_files = Vec::new();
    for r in &plan.remove {
        let p = root.join(r);
        if p.is_dir() {
            files_under(&p, &mut removed_files);
        }
    }
    let config = |p: &Path| p == old.config_file || p.ends_with(".armadai/agents.yaml");
    for f in removed_files {
        if !config(&f) && !sources.iter().any(|s| f.starts_with(s)) {
            plan.warnings.push(format!(
                "not converted, kept only in the backup: {}",
                rel(root, &f).display()
            ));
        }
    }
    plan
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
    } else {
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// Copy `plan.backup` to a fresh `.armadai/.migrated/<unix seconds>[-n]/`; returns it, relative.
pub fn backup(root: &Path, plan: &Plan, secs: u64) -> anyhow::Result<PathBuf> {
    let base = Path::new(".armadai/.migrated");
    let mut dir = base.join(secs.to_string());
    let mut n = 1;
    while root.join(&dir).exists() {
        n += 1;
        dir = base.join(format!("{secs}-{n}"));
    }
    for p in &plan.backup {
        copy_tree(&root.join(p), &root.join(&dir).join(p))
            .with_context(|| format!("backing up {}", p.display()))?;
    }
    Ok(dir)
}

pub fn remove_legacy(root: &Path, plan: &Plan) -> anyhow::Result<()> {
    for p in &plan.remove {
        let path = root.join(p);
        let res = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        res.with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// Write `project` under `dest/.armadai/`, then the skill files other than `SKILL.md`.
/// A copy source inside `src_root/.armadai/` is read from `src_base` instead (the backup,
/// since the legacy directory is gone by then). Returns written paths relative to `dest`.
pub fn write_all(
    project: &Project,
    copies: &[SkillCopy],
    dest: &Path,
    src_root: &Path,
    src_base: &Path,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut written = armadai_model::write_project(project, dest)?;
    for c in copies {
        let from = match c.from.strip_prefix(src_root.join(".armadai")) {
            Ok(r) => src_base.join(".armadai").join(r),
            Err(_) => c.from.clone(),
        };
        let to = dest.join(".armadai/skills").join(&c.id);
        let mut files = Vec::new();
        files_under(&from, &mut files);
        for f in files {
            let r = f.strip_prefix(&from).unwrap_or(&f);
            if r == Path::new("SKILL.md") {
                continue;
            }
            copy_tree(&f, &to.join(r))?;
            written.push(to.join(r));
        }
    }
    let mut written: Vec<PathBuf> = written.iter().map(|p| rel(dest, p)).collect();
    written.sort();
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_backups_in_the_same_second_do_not_merge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(dir.path().join(".armadai/config.yaml"), "agents: []\n").unwrap();
        let plan = Plan {
            backup: vec![".armadai/config.yaml".into()],
            ..Default::default()
        };
        let a = backup(dir.path(), &plan, 7).unwrap();
        let b = backup(dir.path(), &plan, 7).unwrap();
        assert_eq!(a, Path::new(".armadai/.migrated/7"));
        assert_eq!(b, Path::new(".armadai/.migrated/7-2"));
        assert!(dir.path().join(&b).join(".armadai/config.yaml").is_file());
    }
}
