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
        // `cwd` is a per-directory invariant: every transcript Claude Code
        // writes into one project-session directory carries the same `cwd`,
        // so a single readable file settles the whole directory — trying
        // every file (as this used to) reads the head of every transcript in
        // every non-matching directory just to reach the same verdict
        // slower. Measured on this machine on the "no match" path alone:
        // 227,505,065 bytes read across 2154 transcripts, ~0.5s. Only an
        // *unreadable* file is skipped in favour of the next one (via
        // `find_map`), so one stray locked/missing file can't mask real
        // transcripts sitting right behind it in the same directory.
        let matches = files
            .iter()
            .find_map(|f| declares_cwd(f, &forms))
            .unwrap_or(false);
        if matches {
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
        let s = strip_trailing_sep(&root.to_string_lossy());
        // `/` is the one absolute path whose `strip_trailing_sep` collapses
        // to `""` (its only character is the trailing separator being
        // stripped). An empty form must never surface: `projects.join("")`
        // is the projects root directory itself, which `is_dir()` happily
        // confirms — the exact shape of the `.` bug fixed in `54bebc9`, this
        // time for `/` instead of `.`.
        if !s.is_empty() {
            forms.push(s);
        }
    }
    if let Ok(canonical) = root.canonicalize() {
        let canonical = strip_trailing_sep(&canonical.to_string_lossy());
        if !canonical.is_empty() && !forms.contains(&canonical) {
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

/// A sub-agent's transcript, paired with the metadata Claude Code writes
/// beside it.
///
/// Claude Code stores sub-agent work under
/// `<projects>/<slug>/<session-id>/subagents/agent-<id>.{jsonl,meta.json}` —
/// NOT in the session file, which only records the delegation call. The scan
/// missed all of it until 2026-08-20. The meta is the interesting half: it
/// carries `agentType`, `parentAgentId` and `spawnDepth`, so the delegation
/// tree is stated outright rather than inferred.
///
/// Only agents that actually ran get a meta — a delegation refused by a
/// policy hook leaves none. Counting metas therefore counts executions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentFiles {
    /// The sub-agent's own transcript.
    pub transcript: PathBuf,
    /// Its sidecar metadata.
    pub meta: PathBuf,
}

/// Every sub-agent transcript belonging to `root`, across all its sessions.
///
/// Deliberately narrow: only `subagents/` is walked. Sibling directories
/// (`tool-results/`, `memory/`) hold an order of magnitude more files and
/// nothing this audit measures.
pub fn subagent_files(root: &Path) -> Vec<SubagentFiles> {
    let mut found = Vec::new();
    for session in transcript_files(root) {
        // `<session>.jsonl` -> `<session>/subagents/`
        let Some(dir) = session.parent().map(|p| {
            p.join(session.file_stem().unwrap_or_default())
                .join("subagents")
        }) else {
            continue;
        };
        for transcript in jsonl_in(&dir) {
            let meta = transcript.with_extension("meta.json");
            if meta.is_file() {
                found.push(SubagentFiles { transcript, meta });
            }
        }
    }
    found.sort_by(|a, b| a.transcript.cmp(&b.transcript));
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

/// Whether `file`'s entries declare `cwd` as one of `forms` (the acceptable
/// string forms of the audited root — see `root_forms`); comparing against a
/// single raw string would miss whichever of the resolved/unresolved form
/// Claude Code happened to record.
///
/// `cwd` is not guaranteed to appear within a fixed number of lines: some
/// real transcripts open with dozens of metadata-only entries
/// (`file-history-snapshot`, `queue-operation`, `ai-title`, `mode`,
/// `permission-mode`, `attachment`…) before the first entry that actually
/// carries the field — a fixed line-count bound risks giving up before ever
/// seeing it. So this scans until it finds an entry that carries `cwd` and
/// decides on that one: semantically correct, and in the common case (`cwd`
/// on the very first line) cheaper than reading a fixed number of lines
/// regardless of content. `MAX_LINES` is only an anti-pathology ceiling — a
/// corrupt or genuinely `cwd`-less file must never be read forever.
///
/// Three-state result, because `cwd` is a per-*directory* invariant (see
/// `transcript_files`'s fallback tier) and only a file that actually carried
/// the field is entitled to settle that directory one way or the other:
/// - `Some(true)` — this file carries a `cwd` and it matches `forms`.
/// - `Some(false)` — this file carries a `cwd` and it does **not** match.
///   Still final: every file in the directory shares the same `cwd`.
/// - `None` — inconclusive. Either `file` could not be opened, or no entry
///   within `MAX_LINES` carried `cwd` at all (empty file, or metadata-only
///   head). A caller trying several files in a directory (see
///   `transcript_files`) must move on to the next candidate in this case —
///   concluding "not a match" from a file that never actually carried the
///   field would be wrong, not just untried.
fn declares_cwd(file: &Path, forms: &[String]) -> Option<bool> {
    use std::io::BufRead;
    const MAX_LINES: usize = 500;
    let handle = std::fs::File::open(file).ok()?;
    // No `map_while(Result::ok)`: that stops at the first line that fails to
    // read (e.g. invalid UTF-8) and never tries any line after it, which
    // used to silence the rest of this bounded head for one bad line. Errors
    // are skipped in place instead; `.take(MAX_LINES)` still bounds the loop
    // to at most `MAX_LINES` polls of the underlying iterator regardless of
    // how many of them error, so this can never run away.
    for line in std::io::BufReader::new(handle).lines().take(MAX_LINES) {
        let Ok(line) = line else {
            continue;
        };
        let Some(cwd) = serde_json::from_str::<serde_json::Value>(&line)
            .ok()
            .and_then(|v| {
                v.get("cwd")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
        else {
            continue;
        };
        let cwd = strip_trailing_sep(&cwd);
        return Some(forms.iter().any(|f| f == &cwd));
    }
    None
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

    /// Regression: some real transcripts open with dozens of metadata-only
    /// entries before the first entry that carries `cwd` at all — the fixed
    /// 20-line bound this replaces would give up before ever seeing it,
    /// silently losing the session. Measured on this machine: 2 of 207
    /// transcripts have their first `cwd` past line 20.
    #[test]
    fn falls_back_tier_matches_when_the_first_cwd_line_is_past_the_old_twenty_line_bound() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let odd = dir.path().join("some-unexpected-name");
        std::fs::create_dir_all(&odd).unwrap();
        let mut lines: Vec<String> = (0..30)
            .map(|i| format!("{{\"type\":\"queue-operation\",\"n\":{i}}}"))
            .collect();
        lines.push("{\"type\":\"user\",\"cwd\":\"/Users/x/proj\"}".to_string());
        std::fs::write(odd.join("s.jsonl"), lines.join("\n") + "\n").unwrap();

        let found = transcript_files(Path::new("/Users/x/proj"));
        assert_eq!(
            found.len(),
            1,
            "a cwd appearing after 30 metadata lines (past the old 20-line bound) must still match: {found:?}"
        );
    }

    /// The per-directory `cwd` invariant means only the first readable file
    /// needs to be opened to settle a directory — but every `.jsonl` file in
    /// a matching directory must still be returned.
    #[test]
    fn fallback_tier_returns_every_jsonl_file_once_the_first_confirms_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let odd = dir.path().join("some-unexpected-name");
        std::fs::create_dir_all(&odd).unwrap();
        // Sorted order matters: `a.jsonl` is read first and settles the
        // directory; `b.jsonl` never needs to be opened to be included.
        std::fs::write(
            odd.join("a.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/Users/x/proj\"}\n",
        )
        .unwrap();
        std::fs::write(odd.join("b.jsonl"), "").unwrap();

        let found = transcript_files(Path::new("/Users/x/proj"));
        assert_eq!(
            found.len(),
            2,
            "both files in the matching directory: {found:?}"
        );
    }

    /// Regression: a readable file that carries **no** `cwd` at all (here,
    /// empty) must be inconclusive, not a final "not a match" for the whole
    /// directory — only a file that actually carries a `cwd` (whether it
    /// matches or not) may settle it, since that is the per-directory
    /// invariant Fix 2 relies on. `a.jsonl` (empty, no `cwd` anywhere)
    /// deliberately sorts before `b.jsonl` (carries the matching `cwd`): this
    /// is exactly the ordering that used to make `find_map` stop at `a.jsonl`
    /// and conclude "no match" before `b.jsonl` was ever tried.
    #[test]
    fn fallback_tier_skips_a_file_with_no_cwd_at_all_to_reach_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let odd = dir.path().join("some-unexpected-name");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("a.jsonl"), "").unwrap();
        std::fs::write(
            odd.join("b.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/Users/x/proj\"}\n",
        )
        .unwrap();

        let found = transcript_files(Path::new("/Users/x/proj"));
        assert_eq!(
            found.len(),
            2,
            "a cwd-less file must not settle the directory as a non-match; the sibling that \
             actually carries the cwd must still be found: {found:?}"
        );
    }

    /// Builds a project dir holding one session plus its `subagents/`
    /// sidecar layout, the shape Claude Code actually writes.
    fn project_with_subagents(base: &Path, project: &Path) -> PathBuf {
        let slug = base.join(slug_for(project));
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("sess1.jsonl"), "{}\n").unwrap();
        let sub = slug.join("sess1").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        for id in ["a1", "a2"] {
            std::fs::write(sub.join(format!("agent-{id}.jsonl")), "{}\n").unwrap();
            std::fs::write(sub.join(format!("agent-{id}.meta.json")), "{}").unwrap();
        }
        // A transcript with no meta: the agent never actually ran.
        std::fs::write(sub.join("agent-orphan.jsonl"), "{}\n").unwrap();
        // A sibling directory holding far more files, none of them ours.
        let noise = slug.join("sess1").join("tool-results");
        std::fs::create_dir_all(&noise).unwrap();
        std::fs::write(noise.join("r1.jsonl"), "{}\n").unwrap();
        slug
    }

    #[test]
    fn subagent_files_pairs_transcripts_with_their_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let project = Path::new("/Users/x/proj");
        project_with_subagents(dir.path(), project);

        let found = subagent_files(project);
        assert_eq!(found.len(), 2, "only meta-backed pairs count: {found:?}");
        for sa in &found {
            assert!(sa.meta.is_file(), "meta must exist: {sa:?}");
            assert!(sa.transcript.to_string_lossy().ends_with(".jsonl"));
        }
    }

    #[test]
    fn a_transcript_without_metadata_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let project = Path::new("/Users/x/proj");
        project_with_subagents(dir.path(), project);
        // `agent-orphan.jsonl` has no meta, so it never ran and must not count.
        assert!(
            !subagent_files(project)
                .iter()
                .any(|sa| sa.transcript.to_string_lossy().contains("orphan")),
            "a meta-less transcript means the agent did not run"
        );
    }

    #[test]
    fn sibling_directories_are_not_walked() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let project = Path::new("/Users/x/proj");
        project_with_subagents(dir.path(), project);
        // `tool-results/` holds an order of magnitude more files in practice
        // and nothing this audit measures.
        assert!(
            !subagent_files(project)
                .iter()
                .any(|sa| sa.transcript.to_string_lossy().contains("tool-results")),
            "only subagents/ is ours to read"
        );
    }

    #[test]
    fn a_project_without_subagents_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let project = Path::new("/Users/x/plain");
        let slug = dir.path().join(slug_for(project));
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("sess.jsonl"), "{}\n").unwrap();
        assert!(subagent_files(project).is_empty());
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

    /// Regression for the `/` shape of the `.` bug fixed in `54bebc9`:
    /// `strip_trailing_sep("/")` is `""`, and `<projects_root>.join("")` is
    /// the projects root directory itself — a real directory whose
    /// `is_dir()` would wrongly succeed at the slug-lookup tier.
    #[test]
    fn root_forms_never_yields_an_empty_form_for_the_filesystem_root() {
        let forms = root_forms(Path::new("/"));
        assert!(
            forms.iter().all(|f| !f.is_empty()),
            "an empty form must never surface — projects.join(\"\") is the projects dir itself: {forms:?}"
        );
    }
}
