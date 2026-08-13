// This module is the entry point of a multi-task plan: nothing calls these
// functions yet — the scan (a later task) is what wires them in. Until then
// they are unreachable from `main`, which `-D warnings` flags as dead code.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Root holding Claude Code's per-project transcript directories.
/// `ARMADAI_CLAUDE_PROJECTS_DIR` overrides it (used by tests and the e2e suite).
pub fn projects_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ARMADAI_CLAUDE_PROJECTS_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

/// Claude Code's directory-name encoding for a project path: separators become
/// dashes (`/Users/x/proj` -> `-Users-x-proj`).
pub fn slug_for(root: &Path) -> String {
    root.to_string_lossy().replace(['/', '\\'], "-")
}

/// Every `.jsonl` transcript belonging to `root`.
///
/// Two-tier resolution: the slug is only an access shortcut, so when it misses
/// (its exact encoding of `.`, `_` and spaces is not publicly specified) we
/// scan every project directory and keep those whose entries declare `root` as
/// their `cwd` — that field is in the data and is authoritative.
pub fn transcript_files(root: &Path) -> Vec<PathBuf> {
    let Some(projects) = projects_root() else {
        return Vec::new();
    };
    let by_slug = projects.join(slug_for(root));
    if by_slug.is_dir() {
        return jsonl_in(&by_slug);
    }
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let files = jsonl_in(&entry.path());
        if files.iter().any(|f| declares_cwd(f, root)) {
            found.extend(files);
        }
    }
    found.sort();
    found
}

fn jsonl_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();
    files
}

/// True if any of the file's first lines declares `root` as its `cwd`.
/// Only the head is read: `cwd` is repeated on every entry, so a few lines
/// settle it without reading a multi-megabyte transcript.
fn declares_cwd(file: &Path, root: &Path) -> bool {
    use std::io::BufRead;
    let Ok(handle) = std::fs::File::open(file) else {
        return false;
    };
    let wanted = root.to_string_lossy();
    std::io::BufReader::new(handle)
        .lines()
        .map_while(Result::ok)
        .take(20)
        .any(|line| {
            serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .and_then(|v| {
                    v.get("cwd")
                        .and_then(serde_json::Value::as_str)
                        .map(|c| c == wanted)
                })
                .unwrap_or(false)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises `ARMADAI_CLAUDE_PROJECTS_DIR` mutation across the crate,
    /// mirroring the SessionIndexEnvGuard pattern in `cli/watch.rs`.
    struct ProjectsDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ProjectsDirGuard {
        fn set(path: &Path) -> Self {
            let lock = armadai_core::config::ENV_MUTEX.lock().unwrap();
            // SAFETY: modifies the global environment; serialised via ENV_MUTEX.
            unsafe { std::env::set_var("ARMADAI_CLAUDE_PROJECTS_DIR", path) }
            Self { _lock: lock }
        }
    }

    impl Drop for ProjectsDirGuard {
        fn drop(&mut self) {
            // SAFETY: restoring env state at end of test scope.
            unsafe { std::env::remove_var("ARMADAI_CLAUDE_PROJECTS_DIR") }
        }
    }

    #[test]
    fn slug_replaces_path_separators_with_dashes() {
        assert_eq!(
            slug_for(Path::new("/Users/x/work/misc/armadai")),
            "-Users-x-work-misc-armadai"
        );
    }

    #[test]
    fn finds_transcripts_by_slug() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let project = Path::new("/Users/x/proj");
        let slug_dir = dir.path().join(slug_for(project));
        std::fs::create_dir_all(&slug_dir).unwrap();
        std::fs::write(slug_dir.join("a.jsonl"), "").unwrap();
        std::fs::write(slug_dir.join("ignored.txt"), "").unwrap();

        let found = transcript_files(project);
        assert_eq!(found.len(), 1, "only .jsonl files count: {found:?}");
        assert!(found[0].ends_with("a.jsonl"));
    }

    #[test]
    fn falls_back_to_cwd_matching_when_slug_misses() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        // A directory whose name does NOT match the slug rule, but whose
        // entries declare the audited root as their cwd.
        let odd = dir.path().join("some-unexpected-name");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(
            odd.join("s.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/Users/x/proj\"}\n",
        )
        .unwrap();

        let found = transcript_files(Path::new("/Users/x/proj"));
        assert_eq!(found.len(), 1, "cwd fallback must find it: {found:?}");
    }

    #[test]
    fn missing_projects_dir_yields_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(&dir.path().join("does-not-exist"));
        assert!(transcript_files(Path::new("/Users/x/proj")).is_empty());
    }
}
