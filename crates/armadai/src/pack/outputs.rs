//! What `link` may write outside `.armadai/`, copied before it runs and put back when it
//! fails: a failed `pack add` or `pack update` leaves the project as it found it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Folders and files the linkers of every target write (or patch), and the link manifest.
const ROOTS: [&str; 10] = [
    ".claude",
    ".opencode",
    ".codex",
    ".github",
    ".agents",
    "CLAUDE.md",
    "AGENTS.md",
    "opencode.json",
    "opencode.jsonc",
    ".armadai/link-manifest.yaml",
];
/// A file bigger than this, or a `node_modules`, is none of link's: left alone.
const BIG: u64 = 8 << 20;

pub struct Outputs {
    root: PathBuf,
    files: BTreeMap<PathBuf, (Vec<u8>, u32)>,
    dirs: BTreeSet<PathBuf>,
}

/// Every file and folder under the roots (relative paths), not through symbolic links.
fn walk(root: &Path) -> std::io::Result<(BTreeMap<PathBuf, u32>, BTreeSet<PathBuf>)> {
    let (mut files, mut dirs) = (BTreeMap::new(), BTreeSet::new());
    let mut todo: Vec<PathBuf> = ROOTS.iter().map(PathBuf::from).collect();
    while let Some(rel) = todo.pop() {
        let meta = match std::fs::symlink_metadata(root.join(&rel)) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if meta.is_dir() {
            if rel.file_name().is_some_and(|n| n == "node_modules") {
                continue;
            }
            for e in std::fs::read_dir(root.join(&rel))? {
                todo.push(rel.join(e?.file_name()));
            }
            dirs.insert(rel);
        } else if meta.is_file() && meta.len() <= BIG {
            #[cfg(unix)]
            let mode = std::os::unix::fs::PermissionsExt::mode(&meta.permissions());
            #[cfg(not(unix))]
            let mode = 0;
            files.insert(rel, mode);
        }
    }
    Ok((files, dirs))
}

impl Outputs {
    pub fn take(root: &Path) -> std::io::Result<Outputs> {
        let (found, dirs) = walk(root)?;
        let mut files = BTreeMap::new();
        for (rel, mode) in found {
            files.insert(rel.clone(), (std::fs::read(root.join(&rel))?, mode));
        }
        Ok(Outputs {
            root: root.to_path_buf(),
            files,
            dirs,
        })
    }

    /// Best effort: what cannot be put back is said, not hidden.
    pub fn restore(&self) {
        let warn = |p: &Path, e: std::io::Error| {
            eprintln!("  warn: could not put {} back: {e}", p.display());
        };
        let Ok((now_files, now_dirs)) = walk(&self.root) else {
            eprintln!("  warn: could not read the files link wrote, they are left as they are");
            return;
        };
        // what link added, or put where another kind of entry was
        for rel in now_files.keys() {
            if !self.files.contains_key(rel) {
                let _ = std::fs::remove_file(self.root.join(rel));
            }
        }
        for rel in now_dirs.iter().rev() {
            if !self.dirs.contains(rel) {
                let _ = std::fs::remove_dir_all(self.root.join(rel));
            }
        }
        for rel in &self.dirs {
            if let Err(e) = std::fs::create_dir_all(self.root.join(rel)) {
                warn(rel, e);
            }
        }
        for (rel, (bytes, mode)) in &self.files {
            let at = self.root.join(rel);
            if std::fs::read(&at).is_ok_and(|b| &b == bytes) {
                continue;
            }
            let done = std::fs::write(&at, bytes);
            #[cfg(unix)]
            let done = done.and_then(|()| {
                std::fs::set_permissions(&at, std::os::unix::fs::PermissionsExt::from_mode(*mode))
            });
            #[cfg(not(unix))]
            let _ = mode;
            if let Err(e) = done {
                warn(rel, e);
            }
        }
    }
}
