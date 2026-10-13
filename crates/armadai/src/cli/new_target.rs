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

#[cfg(test)]
thread_local! {
    /// Test hook: fail `write` right after the original was moved aside.
    static FAIL_AFTER_ASIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Debug)]
pub struct NewTarget {
    /// The project root (holding `.armadai/`).
    pub root: PathBuf,
    /// The file, absolute.
    pub path: PathBuf,
    /// The file, relative to the project root (for messages).
    pub display: PathBuf,
    /// Under `--force`: where the replaced file is moved aside while the new one is
    /// validated (`<file>.armadai-backup`, a name the loader ignores), and its
    /// permissions. Moving keeps its bytes and mode; nothing is ever re-written.
    backup: Option<(PathBuf, std::fs::Permissions)>,
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
        // `.armadai/` itself, then every component down to the file, the file included
        let mut at = base.clone();
        for c in std::iter::once(None).chain(rel.components().map(Some)) {
            if let Some(c) = c {
                at.push(c);
            }
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
            Ok(m) => {
                let mut name = at.file_name().context("a file has a name")?.to_owned();
                name.push(".armadai-backup");
                let aside = at.with_file_name(name);
                if std::fs::symlink_metadata(&aside).is_ok() {
                    bail!(
                        "{} exists (a backup left by an interrupted `new --force`?): \
                         restore or remove it first",
                        aside.display()
                    );
                }
                Some((aside, m.permissions()))
            }
        };
        Ok(NewTarget {
            root,
            path: at,
            display,
            backup,
            created: Vec::new(),
        })
    }

    /// Create the missing folders, then the file with `content`. A failure puts the
    /// tree back first (the original, under `--force`; the folders it created).
    pub fn write(&mut self, content: &str) -> anyhow::Result<()> {
        let r = self.write_inner(content);
        if r.is_err() {
            self.rollback()?;
        }
        r
    }

    fn write_inner(&mut self, content: &str) -> anyhow::Result<()> {
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
        if let Some((aside, _)) = &self.backup {
            std::fs::rename(&self.path, aside)
                .with_context(|| format!("moving {} aside", self.path.display()))?;
        }
        #[cfg(test)]
        if FAIL_AFTER_ASIDE.with(|f| f.get()) {
            bail!("simulated failure after the move aside");
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .with_context(|| format!("creating {}", self.path.display()))?;
        f.write_all(content.as_bytes())?;
        // the replaced file's permissions, not the umask's
        if let Some((_, perms)) = &self.backup {
            std::fs::set_permissions(&self.path, perms.clone())?;
        }
        Ok(())
    }

    /// The creation stands: drop the replaced file moved aside.
    pub fn commit(self) -> anyhow::Result<()> {
        if let Some((aside, _)) = &self.backup {
            std::fs::remove_file(aside)
                .with_context(|| format!("removing the backup {}", aside.display()))?;
        }
        Ok(())
    }

    /// Put the tree back: the previous file (under `--force`), or nothing at all.
    pub fn undo(self) -> anyhow::Result<()> {
        self.rollback()
    }

    fn rollback(&self) -> anyhow::Result<()> {
        match &self.backup {
            // not moved aside yet: the original is where it was, untouched
            Some((aside, _)) if std::fs::symlink_metadata(aside).is_err() => {}
            // the original was only moved: moving it back restores bytes and mode
            Some((aside, _)) => {
                let back = (|| {
                    if std::fs::symlink_metadata(&self.path).is_ok() {
                        std::fs::remove_file(&self.path)?;
                    }
                    std::fs::rename(aside, &self.path)
                })();
                back.with_context(|| {
                    format!(
                        "could not restore {}: the original is kept at {}",
                        self.path.display(),
                        aside.display()
                    )
                })?;
            }
            None => {
                if self.path.is_file() {
                    std::fs::remove_file(&self.path)?;
                }
                for dir in self.created.iter().rev() {
                    if dir.exists() {
                        std::fs::remove_dir(dir)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_dev(text: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai/agents")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/project.yaml"),
            "schema_version: 1\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(".armadai/agents/dev.md"), text).unwrap();
        dir
    }

    fn files_of(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir.join(".armadai/agents"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// A write that fails after the original was moved aside, then the undo the
    /// caller runs: the original is back.
    #[test]
    fn a_write_failing_after_the_move_is_undone() {
        let dir = project_with_dev("orig\n");
        let mut t = NewTarget::resolve_in(dir.path(), Kind::Agent, "dev", true).unwrap();
        FAIL_AFTER_ASIDE.with(|f| f.set(true));
        let r = t.write("new\n");
        FAIL_AFTER_ASIDE.with(|f| f.set(false));
        assert!(r.is_err());
        // `write` put the original back itself, before reporting the failure
        let path = dir.path().join(".armadai/agents/dev.md");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "orig\n");
        assert_eq!(files_of(dir.path()), ["dev.md"]);
    }

    /// An undo before anything was moved never deletes the original.
    #[test]
    fn an_undo_before_the_write_keeps_the_original() {
        let dir = project_with_dev("orig\n");
        let t = NewTarget::resolve_in(dir.path(), Kind::Agent, "dev", true).unwrap();
        t.undo().unwrap();
        let path = dir.path().join(".armadai/agents/dev.md");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "orig\n");
    }

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
