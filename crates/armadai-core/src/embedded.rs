//! Content embedded in the binary (starter packs, built-in skills) and extracted to
//! `~/.config/armadai/`: a `.armadai-version` marker records what was extracted, so a
//! different binary (another version, or a dev build with other content) re-extracts.

use std::path::Path;

use include_dir::Dir;
use sha2::{Digest, Sha256};

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

/// Whether `dest` holds something else than the extraction of `dir` by this binary.
pub(crate) fn needs_update(dest: &Path, dir: &Dir<'_>) -> bool {
    std::fs::read_to_string(dest.join(".armadai-version")).map_or(true, |v| v.trim() != stamp(dir))
}

/// Extract `dir` to `dest`, replacing what was there: extracted next to it, then
/// swapped in, so no file of a previous extraction survives. Writes the marker.
pub(crate) fn refresh(dir: &Dir<'_>, dest: &Path) -> anyhow::Result<()> {
    let name = dest
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{} has no name", dest.display()))?;
    let fresh = dest.with_file_name(format!(".{}.extracting", name.to_string_lossy()));
    if fresh.exists() {
        std::fs::remove_dir_all(&fresh)?;
    }
    crate::skill::extract_embedded_dir(dir, &fresh)?;
    std::fs::write(fresh.join(".armadai-version"), stamp(dir))?;
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::rename(&fresh, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static STARTERS: Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/starters");

    fn pack(name: &str) -> &'static Dir<'static> {
        STARTERS.get_dir(name).unwrap()
    }

    #[test]
    fn a_missing_marker_needs_an_update() {
        assert!(needs_update(
            Path::new("/nonexistent/path"),
            pack("rust-dev")
        ));
        let dir = tempfile::tempdir().unwrap();
        assert!(needs_update(dir.path(), pack("rust-dev")));
    }

    #[test]
    fn the_marker_holds_the_version_and_the_content() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("rust-dev");
        refresh(pack("rust-dev"), &dest).unwrap();
        assert!(!needs_update(&dest, pack("rust-dev")));
        // same version, other content (a dev build): re-extracted
        assert!(needs_update(&dest, pack("fullstack")));
        let marker = std::fs::read_to_string(dest.join(".armadai-version")).unwrap();
        assert!(marker.starts_with(env!("CARGO_PKG_VERSION")), "{marker}");
        std::fs::write(dest.join(".armadai-version"), env!("CARGO_PKG_VERSION")).unwrap();
        assert!(
            needs_update(&dest, pack("rust-dev")),
            "a version alone is stale"
        );
    }
}
