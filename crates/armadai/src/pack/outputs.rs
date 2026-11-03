//! What `link` is about to write, copied before it runs and put back when it fails: a
//! failed `pack add` or `pack update` leaves the project as it found it.
//!
//! Only the paths `link` itself names are copied (what it compiles, the files it patches,
//! what the last link recorded, the link manifest), never a folder: `.claude/` may hold
//! whole worktrees of someone else's. And only those paths are touched on the way back.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

enum Saved {
    Absent,
    File(Vec<u8>, u32),
    /// A folder or a link: link does not write there, nothing to put back.
    Other,
}

pub struct Outputs {
    root: PathBuf,
    saved: BTreeMap<PathBuf, Saved>,
    /// Folders on the way to a missing path that existed already: never pruned.
    folders: BTreeSet<PathBuf>,
}

fn ancestors(rel: &Path) -> impl Iterator<Item = &Path> {
    rel.ancestors()
        .skip(1)
        .filter(|a| !a.as_os_str().is_empty())
}

impl Outputs {
    /// `paths` are relative to `root`, plain components only.
    pub fn take(root: &Path, paths: &[PathBuf]) -> std::io::Result<Outputs> {
        let mut saved = BTreeMap::new();
        let mut folders = BTreeSet::new();
        for rel in paths {
            let at = root.join(rel);
            let s = match std::fs::symlink_metadata(&at) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    folders.extend(
                        ancestors(rel)
                            .filter(|a| root.join(a).is_dir())
                            .map(Path::to_path_buf),
                    );
                    Saved::Absent
                }
                Err(e) => return Err(e),
                Ok(m) if m.is_file() => {
                    #[cfg(unix)]
                    let mode = std::os::unix::fs::PermissionsExt::mode(&m.permissions());
                    #[cfg(not(unix))]
                    let mode = 0;
                    Saved::File(std::fs::read(&at)?, mode)
                }
                Ok(_) => Saved::Other,
            };
            saved.insert(rel.clone(), s);
        }
        Ok(Outputs {
            root: root.to_path_buf(),
            saved,
            folders,
        })
    }

    /// Best effort: what cannot be put back is said, not hidden.
    pub fn restore(&self) {
        let warn = |rel: &Path, why: &dyn std::fmt::Display| {
            eprintln!(
                "  warn: could not put {} back: {why}",
                self.root.join(rel).display()
            );
        };
        for (rel, saved) in &self.saved {
            let at = self.root.join(rel);
            let linked = std::iter::once(rel.as_path())
                .chain(ancestors(rel))
                .any(|p| {
                    std::fs::symlink_metadata(self.root.join(p))
                        .is_ok_and(|m| m.file_type().is_symlink())
                });
            if linked {
                warn(rel, &"it is, or is behind, a symbolic link");
                continue;
            }
            match saved {
                Saved::Other => {}
                Saved::Absent => {
                    if at.is_file() {
                        if let Err(e) = std::fs::remove_file(&at) {
                            warn(rel, &e);
                            continue;
                        }
                        // the folders link made for it, if nothing else is in them
                        for a in ancestors(rel).filter(|a| !self.folders.contains(*a)) {
                            if std::fs::remove_dir(self.root.join(a)).is_err() {
                                break;
                            }
                        }
                    }
                }
                Saved::File(bytes, mode) => {
                    if std::fs::read(&at).is_ok_and(|b| &b == bytes) {
                        continue;
                    }
                    if at.is_dir() {
                        warn(rel, &"link left a folder in its place");
                        continue;
                    }
                    let done = at
                        .parent()
                        .map_or(Ok(()), std::fs::create_dir_all)
                        .and_then(|()| std::fs::write(&at, bytes));
                    #[cfg(unix)]
                    let done = done.and_then(|()| {
                        std::fs::set_permissions(
                            &at,
                            std::os::unix::fs::PermissionsExt::from_mode(*mode),
                        )
                    });
                    #[cfg(not(unix))]
                    let _ = mode;
                    if let Err(e) = done {
                        warn(rel, &e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn restore_touches_only_the_paths_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, ".claude/agents/a.md", "A1");
        put(root, ".claude/agents/gone.md", "G");
        let paths: Vec<PathBuf> = [
            ".claude/agents/a.md",
            ".claude/agents/gone.md",
            ".claude/agents/new.md",
            ".opencode/new/deep.md",
            ".codex/x/y.md",
            ".claude/agents/theirs-dir",
        ]
        .map(PathBuf::from)
        .into();
        let outputs = Outputs::take(root, &paths).unwrap();
        // what link does ...
        put(root, ".claude/agents/a.md", "A2");
        std::fs::remove_file(root.join(".claude/agents/gone.md")).unwrap();
        put(root, ".claude/agents/new.md", "N");
        put(root, ".opencode/new/deep.md", "D");
        put(root, ".codex/x/y.md", "Y");
        // ... and what someone else did meanwhile
        put(root, ".github/workflows/new.yml", "w");
        put(root, ".claude/worktrees/w/target/debug/new.rlib", "r");
        put(root, ".claude/agents/theirs.md", "t");
        put(root, ".opencode/new/theirs.md", "t2");
        put(root, ".claude/agents/theirs-dir/x.md", "t3");
        outputs.restore();
        let read = |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
        assert_eq!(read(".claude/agents/a.md").as_deref(), Some("A1"));
        assert_eq!(read(".claude/agents/gone.md").as_deref(), Some("G"));
        assert_eq!(read(".claude/agents/new.md"), None);
        assert!(
            !root.join(".codex").exists(),
            "the folders link made go too"
        );
        assert_eq!(read(".opencode/new/deep.md"), None);
        assert_eq!(read(".opencode/new/theirs.md").as_deref(), Some("t2"));
        assert_eq!(read(".github/workflows/new.yml").as_deref(), Some("w"));
        assert_eq!(
            read(".claude/worktrees/w/target/debug/new.rlib").as_deref(),
            Some("r")
        );
        assert_eq!(read(".claude/agents/theirs.md").as_deref(), Some("t"));
        assert_eq!(
            read(".claude/agents/theirs-dir/x.md").as_deref(),
            Some("t3")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_behind_a_symbolic_link_is_said_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        put(&root, "lnk/a.md", "A1");
        let paths = [PathBuf::from("lnk/a.md")];
        let outputs = Outputs::take(&root, &paths).unwrap();
        // link left the folder as a link to somewhere else, with another file in it
        std::fs::remove_dir_all(root.join("lnk")).unwrap();
        std::os::unix::fs::symlink(&out, root.join("lnk")).unwrap();
        std::fs::write(out.join("a.md"), "OTHER").unwrap();
        outputs.restore();
        assert_eq!(std::fs::read_to_string(out.join("a.md")).unwrap(), "OTHER");
    }
}
