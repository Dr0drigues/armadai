//! Content embedded in the binary (starter packs, built-in skills) and extracted to
//! `~/.config/armadai/`.
//!
//! Each extracted folder holds a `.armadai-version` marker: the binary version and a
//! fingerprint of the embedded content on its first line, and, on the second, the name
//! of the folder ArmadAI created. Only a folder whose marker names it is **managed**:
//! re-extracting it replaces it whole. Any other folder (no marker, an older marker, a
//! user's renamed copy) only gets the embedded files written over, nothing removed.

use std::path::Path;

use include_dir::Dir;
use sha2::{Digest, Sha256};

const MARKER: &str = ".armadai-version";

/// `<binary version> <sha256 of the embedded files, paths and bytes>`.
pub(crate) fn stamp(dir: &Dir<'_>) -> String {
    fn walk<'a>(d: &'a Dir<'a>, out: &mut Vec<(&'a Path, &'a [u8])>) {
        out.extend(d.files().map(|f| (f.path(), f.contents())));
        d.dirs().for_each(|s| walk(s, out));
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    let mut h = Sha256::new();
    for (p, bytes) in files {
        h.update(p.to_string_lossy().as_bytes());
        h.update([0]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("{} {hex}", env!("CARGO_PKG_VERSION"))
}

fn marker_lines(dest: &Path) -> Vec<String> {
    std::fs::read_to_string(dest.join(MARKER))
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Whether ArmadAI created `dest`: its marker names this very folder.
pub(crate) fn managed(dest: &Path) -> bool {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned());
    !std::fs::symlink_metadata(dest).is_ok_and(|m| m.file_type().is_symlink())
        && marker_lines(dest).get(1).cloned() == name
}

/// Bring `dest` to the content of `dir`: nothing when its marker already says so
/// (unless `force`); a managed or absent folder is replaced whole; any other folder
/// only gets the embedded files written over. Returns whether anything was written.
pub(crate) fn ensure(dir: &Dir<'_>, dest: &Path, force: bool) -> anyhow::Result<bool> {
    let exists = std::fs::symlink_metadata(dest).is_ok();
    if exists && !force && marker_lines(dest).first().map(String::as_str) == Some(&stamp(dir)) {
        return Ok(false);
    }
    let lines = marker_lines(dest);
    if exists && lines.len() == 1 {
        // extracted by a binary older than the named marker: ours, but holding
        // whatever it held (stale files, perhaps the user's): moved aside whole
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let name = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut aside = dest.with_file_name(format!("{name}.armadai-legacy-{secs}"));
        let mut n = 1;
        while std::fs::symlink_metadata(&aside).is_ok() {
            n += 1;
            aside = dest.with_file_name(format!("{name}.armadai-legacy-{secs}-{n}"));
        }
        std::fs::rename(dest, &aside)
            .map_err(|e| anyhow::anyhow!("moving {} aside: {e}", dest.display()))?;
        eprintln!(
            "moved the previous extraction of {name} to {}; your own files, if any, are in it",
            aside.display()
        );
        replace(dir, dest)?;
    } else if !exists || managed(dest) {
        replace(dir, dest)?;
    } else {
        crate::skill::extract_embedded_dir(dir, dest)?;
        // a marker without a name: up to date, still not ours to remove
        std::fs::write(dest.join(MARKER), format!("{}\n", stamp(dir)))?;
    }
    Ok(true)
}

/// Extract next to `dest`, then swap by renames (the old folder aside first), so an
/// interruption never leaves `dest` missing nor half written.
fn replace(dir: &Dir<'_>, dest: &Path) -> anyhow::Result<()> {
    let name = dest
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{} has no name", dest.display()))?
        .to_string_lossy()
        .into_owned();
    let pid = std::process::id();
    let fresh = dest.with_file_name(format!(".{name}.extracting.{pid}"));
    let old = dest.with_file_name(format!(".{name}.old.{pid}"));
    for scratch in [&fresh, &old] {
        if std::fs::symlink_metadata(scratch).is_ok() {
            std::fs::remove_dir_all(scratch)?;
        }
    }
    crate::skill::extract_embedded_dir(dir, &fresh)?;
    std::fs::write(fresh.join(MARKER), format!("{}\n{name}\n", stamp(dir)))?;
    let had = std::fs::symlink_metadata(dest).is_ok();
    if had {
        std::fs::rename(dest, &old)?;
    }
    std::fs::rename(&fresh, dest)?;
    if had {
        std::fs::remove_dir_all(&old)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static STARTERS: Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/starters");

    fn pack(name: &str) -> &'static Dir<'static> {
        STARTERS.get_dir(name).unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_fresh_extraction_is_managed_and_up_to_date() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("rust-dev");
        assert!(ensure(pack("rust-dev"), &dest, false).unwrap());
        assert!(managed(&dest));
        assert!(
            !ensure(pack("rust-dev"), &dest, false).unwrap(),
            "nothing to do"
        );
        assert_eq!(names(tmp.path()), ["rust-dev"], "no scratch folder left");
        // same version, other content (a dev build): written again
        assert!(ensure(pack("fullstack"), &dest, false).unwrap());
    }

    #[test]
    fn a_managed_folder_is_replaced_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("rust-dev");
        ensure(pack("rust-dev"), &dest, false).unwrap();
        std::fs::write(dest.join("stale.md"), "old").unwrap();
        std::fs::write(dest.join(MARKER), "0.0.1 x\nrust-dev\n").unwrap();
        ensure(pack("rust-dev"), &dest, false).unwrap();
        assert!(!dest.join("stale.md").exists());
        assert!(dest.join("pack.yaml").is_file());
        assert_eq!(names(tmp.path()), ["rust-dev"]);
    }

    /// An extraction by a binary older than the named marker (one line: a version,
    /// maybe a fingerprint) is moved aside whole, then extracted fresh: nothing lost,
    /// nothing stale left. Once.
    #[test]
    fn a_legacy_extraction_is_moved_aside_then_extracted_fresh() {
        for marker in ["1.0.0-rc.2", "1.0.0-rc.5 abc123"] {
            let tmp = tempfile::tempdir().unwrap();
            let dest = tmp.path().join("rust-dev");
            std::fs::create_dir_all(dest.join("agents")).unwrap();
            std::fs::write(dest.join(MARKER), marker).unwrap();
            std::fs::write(dest.join("agents/removed.md"), "# Old\n").unwrap();
            std::fs::write(dest.join("my-notes.md"), "mine").unwrap();
            assert!(ensure(pack("rust-dev"), &dest, false).unwrap(), "{marker}");
            assert!(
                !dest.join("agents/removed.md").exists(),
                "{marker}: stale left"
            );
            assert!(managed(&dest), "{marker}");
            let legacy: Vec<_> = names(tmp.path())
                .into_iter()
                .filter(|n| n.starts_with("rust-dev.armadai-legacy-"))
                .collect();
            assert_eq!(legacy.len(), 1, "{marker}: {:?}", names(tmp.path()));
            let kept = tmp.path().join(&legacy[0]);
            assert_eq!(
                std::fs::read_to_string(kept.join("my-notes.md")).unwrap(),
                "mine"
            );
            assert!(kept.join("agents/removed.md").is_file());
            // a second run moves nothing more
            ensure(pack("rust-dev"), &dest, false).unwrap();
            ensure(pack("fullstack"), &dest, false).unwrap();
            assert_eq!(
                names(tmp.path()).len(),
                2,
                "{marker}: {:?}",
                names(tmp.path())
            );
        }
    }

    /// No marker, or a marker naming another folder (a renamed copy): the user's
    /// files stay, nothing is moved.
    #[test]
    fn an_unmanaged_folder_keeps_the_users_files() {
        for marker in [None, Some("0.0.1 x\nfullstack\n")] {
            let tmp = tempfile::tempdir().unwrap();
            let dest = tmp.path().join("rust-dev");
            std::fs::create_dir_all(&dest).unwrap();
            std::fs::write(dest.join("my-agent.md"), "mine").unwrap();
            if let Some(m) = marker {
                std::fs::write(dest.join(MARKER), m).unwrap();
            }
            assert!(
                ensure(pack("rust-dev"), &dest, false).unwrap(),
                "{marker:?}"
            );
            assert_eq!(
                std::fs::read_to_string(dest.join("my-agent.md")).unwrap(),
                "mine"
            );
            assert!(dest.join("pack.yaml").is_file(), "{marker:?}");
            assert!(!managed(&dest), "{marker:?}: still not ours");
            assert!(
                !ensure(pack("rust-dev"), &dest, false).unwrap(),
                "{marker:?}: up to date"
            );
        }
    }
}
