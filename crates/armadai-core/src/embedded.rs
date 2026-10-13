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

/// Whether `dest` (present) belongs to the user: a symbolic link, no marker, or a
/// marker that does not name it (a renamed copy). ArmadAI never writes, marks,
/// replaces nor removes such a folder; it masks the embedded one of the same name.
pub fn owned_by_user(dest: &Path) -> bool {
    match std::fs::symlink_metadata(dest) {
        Err(_) => false,
        Ok(m) if m.file_type().is_symlink() => true,
        Ok(_) => {
            let lines = marker_lines(dest);
            lines.len() != 1 && !managed(dest)
        }
    }
}

/// Bring `dest` to the content of `dir`, by the one ownership rule:
/// - absent: extracted fresh;
/// - ours (a two-line marker naming it): replaced whole when its stamp differs
///   (or `force`);
/// - a one-line marker (an older ArmadAI): moved aside once to
///   `<name>.armadai-legacy-<secs>`, then extracted fresh;
/// - anything else is the user's ([`owned_by_user`]): left as it is.
///
/// Returns whether anything was written.
pub(crate) fn ensure(dir: &Dir<'_>, dest: &Path, force: bool) -> anyhow::Result<bool> {
    if std::fs::symlink_metadata(dest).is_err() {
        replace(dir, dest)?;
        return Ok(true);
    }
    if owned_by_user(dest) {
        return Ok(false);
    }
    let lines = marker_lines(dest);
    if lines.len() == 1 {
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
        return Ok(true);
    }
    if force || lines.first().map(String::as_str) != Some(&stamp(dir)) {
        replace(dir, dest)?;
        return Ok(true);
    }
    Ok(false)
}

/// Whether process `pid` still runs (never true for an absurd pid).
fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only checks that the process exists; nothing is sent
        let r = unsafe { libc::kill(pid, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true // no cheap check: rely on the age alone
    }
}

/// Remove the temporaries of `name` another extraction left behind, when their
/// process is gone or they are older than an hour.
fn purge_orphans(parent: &Path, name: &str) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let me = std::process::id();
    for e in entries.flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let pid = [format!(".{name}.extracting."), format!(".{name}.old.")]
            .iter()
            .find_map(|p| {
                file.strip_prefix(p.as_str())
                    .and_then(|n| n.parse::<u32>().ok())
            });
        let Some(pid) = pid.filter(|p| *p != me) else {
            continue;
        };
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|d| d > std::time::Duration::from_secs(3600));
        if !alive(pid) || old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Extract next to `dest`, then swap by renames (the old folder aside first), so an
/// interruption never leaves `dest` missing nor half written. Temporaries are unique
/// to this process and removed on failure; every error names its path.
fn replace(dir: &Dir<'_>, dest: &Path) -> anyhow::Result<()> {
    use anyhow::Context;
    let name = dest
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{} has no name", dest.display()))?
        .to_string_lossy()
        .into_owned();
    if let Some(parent) = dest.parent() {
        purge_orphans(parent, &name);
    }
    let pid = std::process::id();
    let fresh = dest.with_file_name(format!(".{name}.extracting.{pid}"));
    let old = dest.with_file_name(format!(".{name}.old.{pid}"));
    for scratch in [&fresh, &old] {
        if std::fs::symlink_metadata(scratch).is_ok() {
            std::fs::remove_dir_all(scratch)
                .with_context(|| format!("removing {}", scratch.display()))?;
        }
    }
    let swap = || -> anyhow::Result<()> {
        crate::skill::extract_embedded_dir(dir, &fresh)
            .with_context(|| format!("extracting into {}", fresh.display()))?;
        std::fs::write(fresh.join(MARKER), format!("{}\n{name}\n", stamp(dir)))
            .with_context(|| format!("writing {}", fresh.join(MARKER).display()))?;
        let had = std::fs::symlink_metadata(dest).is_ok();
        if had {
            std::fs::rename(dest, &old)
                .with_context(|| format!("moving {} aside to {}", dest.display(), old.display()))?;
        }
        if let Err(e) = std::fs::rename(&fresh, dest) {
            // another process put the same content in place first: fine
            if marker_lines(dest).first().map(String::as_str) == Some(&stamp(dir)) {
                let _ = std::fs::remove_dir_all(&fresh);
            } else {
                if had {
                    let _ = std::fs::rename(&old, dest);
                }
                return Err(e).with_context(|| format!("moving {} into place", dest.display()));
            }
        }
        if had {
            std::fs::remove_dir_all(&old).with_context(|| format!("removing {}", old.display()))?;
        }
        Ok(())
    };
    let r = swap();
    if r.is_err() {
        for scratch in [&fresh, &old] {
            if scratch.exists() && dest.exists() {
                let _ = std::fs::remove_dir_all(scratch);
            }
        }
    }
    r
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

    /// Temporaries left by a process that is gone (or older than an hour) are
    /// purged; those of a live process are left to it.
    #[test]
    fn orphan_temporaries_are_purged() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("rust-dev");
        let dead = 3_999_999u32; // above any pid_max: no such process
        for t in [
            format!(".rust-dev.extracting.{dead}"),
            format!(".rust-dev.old.{dead}"),
            ".rust-dev.extracting.1".to_string(), // pid 1 is alive
        ] {
            std::fs::create_dir_all(tmp.path().join(&t)).unwrap();
        }
        ensure(pack("rust-dev"), &dest, false).unwrap();
        assert_eq!(names(tmp.path()), [".rust-dev.extracting.1", "rust-dev"]);
    }

    /// A failure names the path it failed on, and leaves no temporary behind.
    #[cfg(unix)]
    #[test]
    fn a_failed_replacement_names_its_path_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("starters");
        let dest = parent.join("rust-dev");
        ensure(pack("rust-dev"), &dest, false).unwrap();
        std::fs::write(dest.join(MARKER), "0.0.1 x\nrust-dev\n").unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555)).unwrap();
        let e = ensure(pack("rust-dev"), &dest, false).unwrap_err();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            format!("{e:#}").contains(&parent.display().to_string()),
            "{e:#}"
        );
        assert_eq!(names(&parent), ["rust-dev"]);
        assert!(dest.join("pack.yaml").is_file());
    }

    /// Every file under `dir` with its bytes, links as their target.
    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut todo = vec![dir.to_path_buf()];
        while let Some(d) = todo.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                let m = std::fs::symlink_metadata(&p).unwrap();
                if m.file_type().is_symlink() {
                    let t = std::fs::read_link(&p).unwrap();
                    out.push((rel, t.to_string_lossy().as_bytes().to_vec()));
                } else if m.is_dir() {
                    todo.push(p);
                } else {
                    out.push((rel, std::fs::read(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    /// A folder that is not ours (no marker, a marker naming another folder) is the
    /// user's: never written, never marked, never removed; it masks the embedded one.
    #[test]
    fn a_folder_that_is_not_ours_is_never_touched() {
        for marker in [None, Some("0.0.1 x\nfullstack\n")] {
            let tmp = tempfile::tempdir().unwrap();
            let dest = tmp.path().join("rust-dev");
            std::fs::create_dir_all(&dest).unwrap();
            std::fs::write(dest.join("my-agent.md"), "mine").unwrap();
            if let Some(m) = marker {
                std::fs::write(dest.join(MARKER), m).unwrap();
            }
            let before = snapshot(tmp.path());
            assert!(
                !ensure(pack("rust-dev"), &dest, false).unwrap(),
                "{marker:?}"
            );
            assert!(
                !ensure(pack("rust-dev"), &dest, true).unwrap(),
                "{marker:?} forced"
            );
            assert_eq!(snapshot(tmp.path()), before, "{marker:?}");
        }
    }

    /// A symbolic link at the name, dangling or not, is the user's too: kept as is,
    /// never followed.
    #[cfg(unix)]
    #[test]
    fn a_link_at_the_name_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        for target in [tmp.path().join("nowhere"), outside.clone()] {
            let dest = tmp.path().join("rust-dev");
            let _ = std::fs::remove_file(&dest);
            std::os::unix::fs::symlink(&target, &dest).unwrap();
            let before = snapshot(tmp.path());
            assert!(!ensure(pack("rust-dev"), &dest, false).unwrap());
            assert_eq!(snapshot(tmp.path()), before, "{target:?}");
        }
    }
}
