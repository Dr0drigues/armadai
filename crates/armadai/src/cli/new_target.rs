//! The one file an `armadai new` command writes, and the guard around it.
//!
//! An id becomes a path: it is checked strictly (no normalisation), the file stays
//! lexically under `.armadai/<kind>/`, no symbolic link is followed between
//! `.armadai/` and the file (the destination included), and the file is created with
//! `create_new` so a link posed at the destination is never written through.
//! `compile::check_destination` is not reused: it refuses `.armadai/` by design.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Agent,
    Skill,
    Flow,
    Automation,
}

impl Kind {
    /// Path of the element `id`, relative to `.armadai/`.
    fn rel(self, id: &str) -> PathBuf {
        match self {
            Kind::Agent => Path::new("agents").join(format!("{id}.md")),
            Kind::Skill => Path::new("skills").join(id).join("SKILL.md"),
            Kind::Flow => Path::new("flows").join(format!("{id}.yaml")),
            Kind::Automation => Path::new("automations").join(format!("{id}.yaml")),
        }
    }
}

/// Names Windows refuses as a file name, whatever the extension.
fn reserved(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul")
        || ((id.starts_with("com") || id.starts_with("lpt"))
            && id.len() == 4
            && matches!(id.as_bytes()[3], b'1'..=b'9'))
}

/// A strict id: `[a-z0-9][a-z0-9-]{0,63}`, never a Windows reserved name.
pub fn check_id(id: &str) -> anyhow::Result<()> {
    if !armadai_model::is_valid_id(id) {
        bail!(
            "`{id}` is not a valid id: use 1 to 64 lowercase letters, digits and hyphens, \
             starting with a letter or a digit (e.g. `code-reviewer`)"
        );
    }
    if reserved(id) {
        bail!("`{id}` is a name Windows reserves for a device; pick another id");
    }
    Ok(())
}

#[derive(Debug)]
pub struct NewTarget {
    /// The project root (holding `.armadai/`).
    pub root: PathBuf,
    /// The file, absolute.
    pub path: PathBuf,
    /// The file, relative to the project root (for messages).
    pub display: PathBuf,
    /// Content of the file it replaces (`--force`), restored by `undo`.
    backup: Option<Vec<u8>>,
    /// Folders `write` created, outermost first.
    created: Vec<PathBuf>,
}

impl NewTarget {
    /// The file for element `id` of `kind` in the project at or above the current
    /// directory; refused when it exists, unless `force`.
    pub fn resolve(kind: Kind, id: &str, force: bool) -> anyhow::Result<Self> {
        Self::resolve_in(&std::env::current_dir()?, kind, id, force)
    }

    fn resolve_in(cwd: &Path, kind: Kind, id: &str, force: bool) -> anyhow::Result<Self> {
        check_id(id)?;
        let root = crate::compile::find_project(cwd)?;
        let rel = kind.rel(id);
        let base = root.join(".armadai");
        // every component between `.armadai/` and the file, the file included
        let mut at = base.clone();
        for c in rel.components() {
            at.push(c);
            if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
                bail!(
                    "refusing to write through the symbolic link {}",
                    at.display()
                );
            }
        }
        let display = Path::new(".armadai").join(&rel);
        let backup = match std::fs::symlink_metadata(&at) {
            Err(_) => None,
            Ok(m) if !m.is_file() => bail!("{} exists and is not a file", display.display()),
            Ok(_) if !force => bail!(
                "{} already exists (use --force to replace it)",
                display.display()
            ),
            Ok(_) => Some(std::fs::read(&at).with_context(|| format!("reading {}", at.display()))?),
        };
        Ok(NewTarget {
            root,
            path: at,
            display,
            backup,
            created: Vec::new(),
        })
    }

    /// Create the missing folders, then the file with `content`.
    pub fn write(&mut self, content: &str) -> anyhow::Result<()> {
        let base = self.root.join(".armadai");
        let parent = self
            .path
            .parent()
            .context("a file has a parent")?
            .to_path_buf();
        let missing: Vec<PathBuf> = (parent.ancestors())
            .take_while(|p| p.starts_with(&base) && !p.exists())
            .map(Path::to_path_buf)
            .collect();
        for dir in missing.into_iter().rev() {
            std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
            self.created.push(dir);
        }
        // defence in depth: the components were checked in `resolve`, this holds
        // against a link swapped in since
        let real_base = base.canonicalize()?;
        if !parent.canonicalize()?.starts_with(&real_base) {
            bail!("{} leaves {}", self.display.display(), base.display());
        }
        if self.backup.is_some() {
            std::fs::remove_file(&self.path)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .with_context(|| format!("creating {}", self.path.display()))?;
        f.write_all(content.as_bytes())?;
        Ok(())
    }

    /// Put the tree back: the previous file (under `--force`), or nothing at all.
    pub fn undo(self) -> anyhow::Result<()> {
        match &self.backup {
            Some(bytes) => std::fs::write(&self.path, bytes)?,
            None => {
                if self.path.is_file() {
                    std::fs::remove_file(&self.path)?;
                }
                for dir in self.created.iter().rev() {
                    std::fs::remove_dir(dir)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reserved_names() {
        for id in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
            assert!(check_id(id).is_err(), "{id}");
        }
        for id in [
            "com",
            "com0",
            "com10",
            "console",
            "lpt",
            "auxiliary",
            "null",
        ] {
            assert!(check_id(id).is_ok(), "{id}");
        }
    }
}
