//! Discovers Claude Code's transcript files for a given project root.
//! Called from `scan` (`super::scan`), which is wired into `armadai audit`.
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
/// their `cwd` — that field is in the data and is authoritative. Both tiers
/// are tried against every acceptable string form of `root` (see
/// `root_forms`), so a non-canonical `root` (relative, trailing separator, or
/// symlinked) still matches.
pub fn transcript_files(root: &Path) -> Vec<PathBuf> {
    let Some(projects) = projects_root() else {
        return Vec::new();
    };
    let forms = root_forms(root);
    for form in &forms {
        let by_slug = projects.join(slug_for(Path::new(form)));
        if by_slug.is_dir() {
            return jsonl_in(&by_slug);
        }
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
        if files.iter().any(|f| declares_cwd(f, &forms)) {
            found.extend(files);
        }
    }
    found.sort();
    found
}

/// The acceptable string forms of `root` for matching against a slug
/// directory name or a transcript's recorded `cwd` — **only absolute forms**.
/// Both resolution tiers compare against absolute paths: Claude Code's slugs
/// encode an absolute path, and the `cwd` it records is always absolute too,
/// so a relative form is guaranteed not to match either tier. Worse, keeping
/// it used to actively misfire: for a relative root like `.`, `slug_for(".")`
/// is `"."` unchanged, and `<projects_root>.join(".")` resolves to the
/// projects root directory itself (a real directory) — the slug-lookup tier
/// then matched *there*, returning an empty result before the canonical
/// form — the only form that could ever legitimately match — was tried.
///
/// Two forms are still kept when both are absolute (the as-given one and its
/// canonicalized form, each with a trailing separator stripped), because
/// Claude Code may have recorded either the resolved or the unresolved path.
/// When `root` is relative and does not canonicalize (e.g. it no longer
/// exists on disk), this yields no forms at all: nothing here could ever
/// match, so an empty result is the honest answer rather than a guess.
fn root_forms(root: &Path) -> Vec<String> {
    let mut forms = Vec::new();
    if root.is_absolute() {
        forms.push(strip_trailing_sep(&root.to_string_lossy()));
    }
    if let Ok(canonical) = root.canonicalize() {
        let canonical = strip_trailing_sep(&canonical.to_string_lossy());
        if !forms.contains(&canonical) {
            forms.push(canonical);
        }
    }
    forms
}

/// Strips one trailing path separator, if present, so `"/a/b/"` and `"/a/b"`
/// compare equal.
fn strip_trailing_sep(s: &str) -> String {
    s.trim_end_matches(['/', '\\']).to_string()
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

/// True if any of the file's first lines declares its `cwd` as one of
/// `forms` (the acceptable string forms of the audited root — see
/// `root_forms`); comparing against a single raw string would miss whichever
/// of the resolved/unresolved form Claude Code happened to record.
/// Only the head is read: `cwd` is repeated on every entry, so a few lines
/// settle it without reading a multi-megabyte transcript.
fn declares_cwd(file: &Path, forms: &[String]) -> bool {
    use std::io::BufRead;
    let Ok(handle) = std::fs::File::open(file) else {
        return false;
    };
    std::io::BufReader::new(handle)
        .lines()
        .map_while(Result::ok)
        .take(20)
        .any(|line| {
            serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .and_then(|v| {
                    v.get("cwd").and_then(serde_json::Value::as_str).map(|c| {
                        let c = strip_trailing_sep(c);
                        forms.iter().any(|f| f == &c)
                    })
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

    /// Removes `ARMADAI_CLAUDE_PROJECTS_DIR` and `HOME` for the test's
    /// duration, restoring both original values (present or absent) on drop.
    /// Serialises via `ENV_MUTEX`, like `ProjectsDirGuard` — the rest of the
    /// suite needs `HOME` back, since other tests rely on it implicitly.
    struct NoHomeGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        original_home: Option<String>,
        original_projects_dir: Option<String>,
    }

    impl NoHomeGuard {
        fn set() -> Self {
            let lock = armadai_core::config::ENV_MUTEX.lock().unwrap();
            let original_home = std::env::var("HOME").ok();
            let original_projects_dir = std::env::var("ARMADAI_CLAUDE_PROJECTS_DIR").ok();
            // SAFETY: modifies the global environment; serialised via ENV_MUTEX.
            unsafe {
                std::env::remove_var("HOME");
                std::env::remove_var("ARMADAI_CLAUDE_PROJECTS_DIR");
            }
            Self {
                _lock: lock,
                original_home,
                original_projects_dir,
            }
        }
    }

    impl Drop for NoHomeGuard {
        fn drop(&mut self) {
            // SAFETY: restoring env state at end of test scope.
            unsafe {
                match &self.original_home {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
                match &self.original_projects_dir {
                    Some(v) => std::env::set_var("ARMADAI_CLAUDE_PROJECTS_DIR", v),
                    None => std::env::remove_var("ARMADAI_CLAUDE_PROJECTS_DIR"),
                }
            }
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

    #[test]
    fn slug_lookup_tolerates_a_trailing_separator() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let project = Path::new("/Users/x/proj");
        let slug_dir = dir.path().join(slug_for(project));
        std::fs::create_dir_all(&slug_dir).unwrap();
        std::fs::write(slug_dir.join("a.jsonl"), "").unwrap();

        let found = transcript_files(Path::new("/Users/x/proj/"));
        assert_eq!(
            found.len(),
            1,
            "a trailing separator on the audited root must not break the slug lookup: {found:?}"
        );
    }

    #[test]
    fn absolute_non_canonical_root_matches_via_canonicalization() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let project = tempfile::tempdir().unwrap();
        let sub = project.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        // Deliberately not `.canonicalize()`-driven on the query side: the
        // slug directory is created for the resolved form, but the audited
        // root passed to `transcript_files` below is a *different*, absolute
        // string (containing a literal `..`) that only equals it once the
        // filesystem resolves it. `canonicalize()` on an absolute path never
        // consults the process's current directory, so this covers the same
        // `root_forms` branch as a relative root would, with no global state.
        let canonical_sub = sub.canonicalize().unwrap();
        let slug_dir = dir.path().join(slug_for(&canonical_sub));
        std::fs::create_dir_all(&slug_dir).unwrap();
        std::fs::write(slug_dir.join("a.jsonl"), "").unwrap();

        let non_canonical = sub.join("..").join("sub");
        let found = transcript_files(&non_canonical);

        assert_eq!(
            found.len(),
            1,
            "an absolute root containing `..` must still resolve via canonicalize: {found:?}"
        );
    }

    #[test]
    fn projects_root_is_none_without_home_or_override() {
        let _g = NoHomeGuard::set();
        assert!(projects_root().is_none());
    }

    /// Regression for the `armadai audit .` bug: `slug_for(".")` is `"."`
    /// unchanged, and `<projects_root>.join(".")` resolves to the projects
    /// root directory itself (a real directory) — which used to make the
    /// slug-lookup tier match there and return an empty result *before* the
    /// canonical form, the only form that could ever legitimately match, was
    /// tried. No global state is touched: this asserts the invariant on
    /// `root_forms` directly rather than exercising `.` end to end (which
    /// would require mutating the process cwd — rejected in Task 3's review).
    #[test]
    fn root_forms_drops_the_relative_as_given_form() {
        let forms = root_forms(Path::new("."));
        assert!(
            forms.iter().all(|f| Path::new(f).is_absolute()),
            "a relative root must never surface a relative form: {forms:?}"
        );
        assert!(
            !forms.contains(&".".to_string()),
            "the literal '.' must never appear: {forms:?}"
        );
    }

    /// Companion to the above: dropping relative forms must not also drop
    /// canonicalization for an absolute-but-non-canonical root. Mirrors
    /// `absolute_non_canonical_root_matches_via_canonicalization`'s `..`
    /// construction, but asserts on `root_forms` directly.
    #[test]
    fn root_forms_still_yields_the_canonical_form_for_an_absolute_non_canonical_root() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let canonical = strip_trailing_sep(&sub.canonicalize().unwrap().to_string_lossy());
        let non_canonical = sub.join("..").join("sub"); // absolute, not canonical
        assert!(non_canonical.is_absolute());

        let forms = root_forms(&non_canonical);
        assert!(
            forms.contains(&canonical),
            "dropping relative forms must not drop canonicalization itself: {forms:?}"
        );
    }
}
