use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use serde::Deserialize;

use std::collections::HashMap;

use super::config::{load_user_config, user_starters_dir};

static EMBEDDED_STARTERS: Dir = include_dir!("$CARGO_MANIFEST_DIR/starters");

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct StarterPack {
    pub name: String,
    pub description: String,
    /// `1`: a native pack (a canonical tree, copied). Absent: the legacy format
    /// (`agents:`/`prompts:` lists, converted by `migrate`), whatever its origin.
    #[serde(default)]
    pub format: Option<u32>,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub prompts: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

impl StarterPack {
    /// Load a starter pack from a directory containing `pack.yaml`.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let pack_file = dir.join("pack.yaml");
        if !pack_file.is_file() {
            anyhow::bail!("No pack.yaml found in {}", dir.display());
        }
        let content = std::fs::read_to_string(&pack_file)?;
        let pack: StarterPack = armadai_model::yaml::from_str(&content)?;
        Ok(pack)
    }
}

/// Discover the built-in starter packs directory.
///
/// Resolution order:
/// 1. `CARGO_MANIFEST_DIR/starters` (dev, compile-time path)
/// 2. Next to the binary (packaged installs)
/// 3. `~/.config/armadai/starters/` — extracted from embedded data on first use
///
/// Never a folder of the current directory: a cloned repository's `starters/` would
/// pass for the embedded packs (#490).
pub fn builtin_starters_dir() -> PathBuf {
    let candidates = [
        // Dev: relative to project root at compile time
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/starters")),
        // Installed: next to binary
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("starters")))
            .unwrap_or_default(),
    ];

    for c in &candidates {
        if c.is_dir() {
            return c.clone();
        }
    }

    // Fallback: extract embedded starters to config dir (per-pack versioning)
    let config_starters = super::config::config_dir().join("starters");
    std::fs::create_dir_all(&config_starters).ok();
    sync_extracted(&config_starters, true);
    config_starters
}

/// Return all starter directories in priority order.
///
/// Resolution order:
/// 1. Built-in (embedded/bundled)
/// 2. Project-local `.armadai/starters/`
/// 3. User directory (`~/.config/armadai/starters/`)
/// 4. `ARMADAI_STARTERS_DIRS` env var (colon-separated)
/// 5. Custom directories from `config.yaml` `starters_dirs`
pub fn all_starters_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    let builtin = builtin_starters_dir();
    let user = user_starters_dir();

    // 1. Built-in
    dirs.push(builtin.clone());

    // 2. Project-local .armadai/starters/
    let project_starters = Path::new(".armadai/starters");
    if project_starters.is_dir() && !dirs.contains(&project_starters.to_path_buf()) {
        dirs.push(project_starters.to_path_buf());
    }

    // 3. User dir (only if different from builtin); its extracted embedded packs are
    // brought up to date first
    if user != builtin {
        sync_extracted(&user, false);
        dirs.push(user);
    }

    // A relative entry would be read from the current directory, a cloned repository's
    // folder (#490): ignored, with a warning.
    let mut add = |entry: &str, from: &str| {
        let p = PathBuf::from(entry);
        if !p.is_absolute() {
            crate::config::warn_once(format!(
                "starter folder '{}' ({from}) is a relative path: ignored, give an absolute path",
                printable(entry)
            ));
        } else if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    };

    // 4. ARMADAI_STARTERS_DIRS env var (colon-separated)
    if let Ok(env_dirs) = std::env::var("ARMADAI_STARTERS_DIRS") {
        for entry in env_dirs.split(':') {
            let trimmed = entry.trim();
            if !trimmed.is_empty() {
                add(trimmed, "ARMADAI_STARTERS_DIRS");
            }
        }
    }

    // 5. Custom dirs from config.yaml
    for custom in &load_user_config().starters_dirs {
        add(custom, "starters_dirs");
    }

    dirs
}

/// `text` with its control characters escaped (`\u{1b}`): a URL or a path read from a
/// project never drives the terminal.
pub fn printable(text: &str) -> String {
    (text.chars())
        .map(|c| match c.is_control() {
            true => c.escape_unicode().to_string(),
            false => c.to_string(),
        })
        .collect()
}

/// The error of [`find_pack`] when several project sources hold the pack: one of the
/// user's sources not synced yet may still hold it, so `init` syncs, then retries.
#[derive(Debug)]
pub struct ProjectAmbiguity {
    pub name: String,
    /// `(source label, pack folder)` of each project source holding the pack.
    pub candidates: Vec<(String, PathBuf)>,
}

impl std::fmt::Display for ProjectAmbiguity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&ambiguity(&self.name, "project sources", &self.candidates))
    }
}

/// `text` quoted for a POSIX shell: a printed command runs as pasted.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// One `armadai init --pack '<folder>'` command per candidate, under its source's label.
pub fn pack_commands(candidates: &[(String, PathBuf)]) -> String {
    (candidates.iter())
        .map(|(label, dir)| {
            let dir = shell_quote(&printable(&dir.display().to_string()));
            format!("  {label}:\n    armadai init --pack {dir}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn ambiguity(name: &str, level: &str, candidates: &[(String, PathBuf)]) -> String {
    format!(
        "starter pack '{name}' is provided by several {level}:\n{}\n\
         install one with its command above, or remove the other sources",
        pack_commands(candidates)
    )
}

impl std::error::Error for ProjectAmbiguity {}

// ---------------------------------------------------------------------------
// Remote starter registry: cache dir + pack discovery
// ---------------------------------------------------------------------------
//
// Moved here from the bin-side `starters_registry` module (which owns the
// online fetch path — git/archive sources —
// and re-exports these two functions for backward compatibility): scanning
// an already-synced cache dir for packs is pure filesystem/YAML logic with
// no coupling to fetching, so it belongs with the rest of `core::starter`'s
// pack-resolution logic.

/// Root cache dir for synced starter registries.
pub fn starters_cache_dir() -> PathBuf {
    crate::config::registry_cache_dir().join("starters")
}

/// Optional registry manifest that enriches/restricts discovery.
#[derive(Debug, Deserialize, Default)]
struct StartersManifest {
    #[serde(default)]
    packs: Vec<ManifestPack>,
}
#[derive(Debug, Deserialize)]
struct ManifestPack {
    path: String,
}

/// A folder the extraction migration moved aside (`<name>.armadai-legacy-<secs>`):
/// kept for the user, never a pack nor a skill.
pub fn is_legacy_copy(name: &str) -> bool {
    name.contains(".armadai-legacy-")
}

/// A folder name a pack lookup skips: a legacy copy, or a hidden one (the
/// extraction's own temporary folders start with a dot).
fn skipped(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.') || is_legacy_copy(n))
}

/// Bring every embedded pack already extracted under `dir` up to date (a legacy
/// extraction is moved aside first): a stale copy in the user starters folder must
/// never shadow the embedded pack, even when the embedded set is read from elsewhere.
fn sync_extracted(dir: &Path, all: bool) {
    // once per folder and process: one lookup can read the starter folders many times
    static DONE: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    {
        let mut done = DONE.lock().unwrap_or_else(|e| e.into_inner());
        if done.iter().any(|d| d == dir) {
            return;
        }
        done.push(dir.to_path_buf());
    }
    let r = super::embedded::with_lock(dir, || sync_extracted_locked(dir, all));
    if let Err(e) = r {
        tracing::debug!("{e:#}");
        let text = format!("{e:#}");
        if text.contains("Permission denied") || text.contains("Read-only") {
            eprintln!(
                "cannot refresh the embedded packs (read-only config); using the previous \
                 extraction"
            );
        } else {
            eprintln!("cannot refresh the embedded packs ({text}); using the previous extraction");
        }
    }
}

fn sync_extracted_locked(dir: &Path, all: bool) -> anyhow::Result<()> {
    let mut first = None;
    for pack in EMBEDDED_STARTERS.dirs() {
        if pack.get_file(pack.path().join("pack.yaml")).is_some()
            && let Some(name) = pack.path().file_name().and_then(|n| n.to_str())
        {
            let dest = dir.join(name);
            if !all && std::fs::symlink_metadata(&dest).is_err() {
                continue;
            }
            if let Err(e) = super::embedded::ensure(pack, &dest, false) {
                first.get_or_insert(e);
            }
        }
    }
    first.map_or(Ok(()), Err)
}

/// The user's folders, in the user starters folder, that carry an embedded pack's
/// name: they mask that pack (`(name, folder)`).
pub fn user_packs_masking_embedded() -> Vec<(String, PathBuf)> {
    let user = user_starters_dir();
    (EMBEDDED_STARTERS.dirs())
        .filter(|d| d.get_file(d.path().join("pack.yaml")).is_some())
        .filter_map(|d| d.path().file_name().and_then(|n| n.to_str()))
        .map(|n| (n.to_string(), user.join(n)))
        .filter(|(_, p)| super::embedded::owned_by_user(p))
        .collect()
}

/// Discover pack directories under a fetched registry dir.
///
/// Hybrid: an `armadai-starters.yaml` at the root RESTRICTS to its listed
/// `path:`s; otherwise every directory containing a `pack.yaml` is a pack.
pub fn discover_packs(registry_dir: &Path) -> Vec<PathBuf> {
    let manifest_path = registry_dir.join("armadai-starters.yaml");
    if manifest_path.is_file()
        && let Ok(content) = std::fs::read_to_string(&manifest_path)
        && let Ok(manifest) = armadai_model::yaml::from_str::<StartersManifest>(&content)
        && !manifest.packs.is_empty()
    {
        return manifest
            .packs
            .iter()
            .filter(|p| {
                let path = std::path::Path::new(&p.path);
                !path.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir | std::path::Component::RootDir
                    )
                })
            })
            .map(|p| registry_dir.join(&p.path))
            .filter(|d| d.join("pack.yaml").is_file())
            .collect();
    }
    let mut out = Vec::new();
    scan_for_packs(registry_dir, 0, &mut out);
    out
}

fn scan_for_packs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    if depth > 0 && skipped(dir) {
        return;
    }
    if dir.join("pack.yaml").is_file() {
        out.push(dir.to_path_buf());
        return; // a pack dir isn't itself scanned deeper
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // sorted: the pack a source yields never depends on `read_dir` order
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for p in dirs {
        if p.is_dir() && !p.file_name().is_some_and(|n| n == ".git") {
            scan_for_packs(&p, depth + 1, out);
        }
    }
}

/// A starter pack found by name: its folder, and the project source it comes from
/// (`None`: the embedded packs, the user's folders or the user's sources).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundPack {
    pub dir: PathBuf,
    pub project_source: Option<String>,
}

/// Find a starter pack by name (#490). Trusted places first: the starter folders
/// (built-in, user, `ARMADAI_STARTERS_DIRS`, `starters_dirs`; the last one holding it
/// wins), then the user's sources of `registries.yaml`. Only then the project's: its
/// `.armadai/starters/` and the sources of its legacy config. Two sources of the same
/// level holding the pack is an error naming them. A cache no declared source owns
/// is never read: nothing tells whose it is.
pub fn find_pack(name: &str) -> anyhow::Result<Option<FoundPack>> {
    let project_local = Path::new(".armadai/starters");
    let trusted: Vec<PathBuf> = (all_starters_dirs().into_iter())
        .filter(|d| d != project_local)
        .collect();
    let cache = |url: &str| {
        let dir = starters_cache_dir().join(crate::registries::cache_key(url));
        (printable(&crate::registries::redact_url(url)), dir)
    };
    let mut user_urls: Vec<String> = Vec::new();
    for s in crate::registries::load_user_registries().starters {
        if !user_urls.contains(&s.url) {
            user_urls.push(s.url);
        }
    }
    let user: Vec<_> = user_urls.iter().map(|u| cache(u)).collect();
    let mut project = Vec::new();
    if let Some(local) = child_pack(project_local, name) {
        let label = ".armadai/starters/ of the current directory".to_string();
        project.push((label, local));
    }
    if let Some((root, cfg)) = crate::project::find_project_config() {
        let mut seen = user_urls;
        for s in cfg.registries.map(|r| r.starters).unwrap_or_default() {
            if !seen.contains(&s.url) {
                let (shown, dir) = cache(&s.url);
                if let Some(pack) = pack_in_source(&dir, name) {
                    let root = printable(&root.display().to_string());
                    project.push((format!("{shown} (from {root})"), pack));
                }
                seen.push(s.url);
            }
        }
    }
    resolve_pack(name, &trusted, &user, project)
}

/// Testable core of [`find_pack`]: `user` lists `(shown URL, cache folder)` in
/// declaration order, `project` the project packs already found, as `(label, pack)`.
fn resolve_pack(
    name: &str,
    trusted: &[PathBuf],
    user: &[(String, PathBuf)],
    project: Vec<(String, PathBuf)>,
) -> anyhow::Result<Option<FoundPack>> {
    if let Some(dir) = trusted.iter().rev().find_map(|d| child_pack(d, name)) {
        let project_source = None;
        return Ok(Some(FoundPack {
            dir,
            project_source,
        }));
    }
    let user: Vec<_> = (user.iter())
        .filter_map(|(shown, cache)| Some((shown.clone(), pack_in_source(cache, name)?)))
        .collect();
    let user = only_one(user).map_err(|c| anyhow::anyhow!(ambiguity(name, "of your sources", &c)));
    if let Some((_, dir)) = user? {
        let project_source = None;
        return Ok(Some(FoundPack {
            dir,
            project_source,
        }));
    }
    Ok((only_one(project).map_err(|candidates| ProjectAmbiguity {
        name: name.to_string(),
        candidates,
    })?)
    .map(|(label, dir)| FoundPack {
        dir,
        project_source: Some(label),
    }))
}

fn is_pack(dir: &Path) -> bool {
    !skipped(dir) && dir.is_dir() && dir.join("pack.yaml").is_file()
}

/// The pack folder `dir/<name>`, its name compared to the real entries of `dir`, case
/// included: a case-insensitive file system never matches `Shared` to `shared`.
fn child_pack(dir: &Path, name: &str) -> Option<PathBuf> {
    (std::fs::read_dir(dir).ok()?.flatten())
        .find(|e| e.file_name() == name)
        .map(|e| e.path())
        .filter(|p| is_pack(p))
}

/// The pack `name` in one source's cache: by folder name, else by `pack.yaml` `name:`
/// (a single-pack repository's folder is not named after its pack).
fn pack_in_source(cache: &Path, name: &str) -> Option<PathBuf> {
    let packs = discover_packs(cache);
    (packs
        .iter()
        .find(|d| d.file_name().is_some_and(|n| n == name)))
    .or_else(|| (packs.iter()).find(|d| StarterPack::load(d).is_ok_and(|p| p.name == name)))
    .cloned()
}

/// The single match of one level; several is an error carrying them all.
type Candidates = Vec<(String, PathBuf)>;
fn only_one(found: Candidates) -> Result<Option<(String, PathBuf)>, Candidates> {
    if found.len() < 2 {
        return Ok(found.into_iter().next());
    }
    Err(found)
}

/// List all available starter pack names.
pub fn list_available_packs() -> Vec<String> {
    let mut seen = HashMap::new();

    for dir in all_starters_dirs() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && !skipped(&path)
                && path.join("pack.yaml").is_file()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                seen.insert(name.to_string(), ());
            }
        }
    }

    let mut packs: Vec<String> = seen.into_keys().collect();
    packs.sort();
    packs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_pack() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
name: test-pack
description: A test starter pack
agents:
  - code-reviewer
  - test-writer
prompts:
  - style-guide
skills:
  - my-skill
";
        std::fs::write(dir.path().join("pack.yaml"), yaml).unwrap();

        let pack = StarterPack::load(dir.path()).unwrap();
        assert_eq!(pack.name, "test-pack");
        assert_eq!(pack.description, "A test starter pack");
        assert_eq!(pack.agents, vec!["code-reviewer", "test-writer"]);
        assert_eq!(pack.prompts, vec!["style-guide"]);
        assert_eq!(pack.skills, vec!["my-skill"]);
    }

    #[test]
    fn test_load_pack_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(StarterPack::load(dir.path()).is_err());
    }

    #[test]
    fn test_load_pack_empty_lists() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
name: minimal
description: Minimal pack
";
        std::fs::write(dir.path().join("pack.yaml"), yaml).unwrap();

        let pack = StarterPack::load(dir.path()).unwrap();
        assert!(pack.agents.is_empty());
        assert!(pack.prompts.is_empty());
        assert!(pack.skills.is_empty());
    }

    #[test]
    fn test_list_available_packs_includes_analysis() {
        let names = list_available_packs();
        assert!(
            names.contains(&"code-analysis-rust".to_string()),
            "Expected code-analysis-rust in {names:?}"
        );
        assert!(
            names.contains(&"code-analysis-web".to_string()),
            "Expected code-analysis-web in {names:?}"
        );
        assert!(
            names.contains(&"armadai-authoring".to_string()),
            "Expected armadai-authoring in {names:?}"
        );
    }

    /// Every pack embedded in the binary is native (`format: 1`), named after its
    /// folder, with no legacy `prompts/`; its content is validated end to end by the
    /// binary's `init_pack` tests.
    #[test]
    fn every_embedded_pack_is_native() {
        let starters = Path::new(env!("CARGO_MANIFEST_DIR")).join("starters");
        let mut seen = 0;
        for entry in std::fs::read_dir(&starters).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.join("pack.yaml").is_file() {
                continue;
            }
            seen += 1;
            let pack = StarterPack::load(&dir).unwrap();
            assert_eq!(pack.format, Some(1), "{}", dir.display());
            assert_eq!(dir.file_name().unwrap().to_str(), Some(pack.name.as_str()));
            assert!(pack.agents.is_empty() && pack.prompts.is_empty() && pack.skills.is_empty());
            assert!(!dir.join("prompts").exists(), "{}", dir.display());
            assert!(dir.join("agents").is_dir(), "{}", dir.display());
        }
        assert_eq!(seen, EMBEDDED_STARTERS.dirs().count());
        assert_eq!(seen, 8);
    }

    /// A pack extracted by an older binary keeps no stale file (R-B2): a managed
    /// folder (two-line marker) is replaced, a legacy one (one-line marker) moved
    /// aside then extracted fresh.
    #[test]
    fn a_re_extracted_pack_keeps_no_stale_file() {
        for marker in ["0.0.1 x\nrust-dev\n", "0.0.1"] {
            let tmp = tempfile::tempdir().unwrap();
            let dest = tmp.path().join("rust-dev");
            std::fs::create_dir_all(dest.join("prompts")).unwrap();
            std::fs::write(dest.join("prompts/old.md"), "stale").unwrap();
            std::fs::write(dest.join(".armadai-version"), marker).unwrap();
            let dir = EMBEDDED_STARTERS.get_dir("rust-dev").unwrap();
            super::super::embedded::ensure(dir, &dest, false).unwrap();
            assert!(!dest.join("prompts/old.md").exists(), "{marker}");
            assert!(dest.join("pack.yaml").is_file(), "{marker}");
        }
    }

    /// A folder moved aside by the migration is never offered as a pack.
    #[test]
    fn a_legacy_copy_is_not_a_pack() {
        assert!(is_legacy_copy("fullstack.armadai-legacy-1760000000"));
        assert!(!is_legacy_copy("fullstack"));
        let tmp = tempfile::tempdir().unwrap();
        for d in ["mine", "mine.armadai-legacy-1"] {
            std::fs::create_dir_all(tmp.path().join(d)).unwrap();
            std::fs::write(
                tmp.path().join(d).join("pack.yaml"),
                "name: x\ndescription: d\n",
            )
            .unwrap();
        }
        let found: Vec<_> = discover_packs(tmp.path())
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["mine"]);
    }

    #[test]
    fn test_find_pack_builtin() {
        let found = find_pack("code-analysis-rust").unwrap().unwrap();
        assert!(found.dir.join("pack.yaml").is_file());
        assert_eq!(found.project_source, None);
    }

    #[test]
    fn test_find_pack_not_found() {
        assert_eq!(find_pack("nonexistent-pack-xyz").unwrap(), None);
    }

    /// A source's pack whose folder differs from its `name:` is found by that name.
    #[test]
    fn a_source_pack_is_found_by_its_declared_name() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("inner");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("pack.yaml"), "name: cool-pack\ndescription: d\n").unwrap();
        assert_eq!(pack_in_source(tmp.path(), "cool-pack"), Some(d));
        assert_eq!(pack_in_source(tmp.path(), "inner2"), None);
    }

    /// In one source, a folder named after the pack comes before a pack declaring that
    /// `name:` in another folder.
    #[test]
    fn a_source_pack_is_found_by_folder_before_name() {
        let tmp = tempfile::tempdir().unwrap();
        for (dir, name) in [("a", "b"), ("b", "x")] {
            let d = tmp.path().join(dir);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("pack.yaml"),
                format!("name: {name}\ndescription: d\n"),
            )
            .unwrap();
        }
        assert_eq!(pack_in_source(tmp.path(), "b"), Some(tmp.path().join("b")));
    }

    #[test]
    fn test_env_var_starters_dirs() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        // Create a pack in dir1
        let pack_dir = dir1.path().join("env-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(
            pack_dir.join("pack.yaml"),
            "name: env-pack\ndescription: From env\n",
        )
        .unwrap();

        let orig = std::env::var("ARMADAI_STARTERS_DIRS").ok();
        let env_val = format!("{}:{}", dir1.path().display(), dir2.path().display());
        // SAFETY: This test modifies the global environment which is unsafe in Rust 2024.
        // This is the only test touching ARMADAI_STARTERS_DIRS, but must still be run
        // with `--test-threads=1` for safety.
        unsafe {
            std::env::set_var("ARMADAI_STARTERS_DIRS", &env_val);
        }

        let dirs = all_starters_dirs();
        // Both dirs should be present (dir2 exists even though empty)
        assert!(
            dirs.contains(&dir1.path().to_path_buf()),
            "Expected dir1 in starters dirs"
        );
        assert!(
            dirs.contains(&dir2.path().to_path_buf()),
            "Expected dir2 in starters dirs"
        );

        // The env pack should be findable
        let found = find_pack("env-pack").unwrap().map(|f| f.dir);
        assert_eq!(found, Some(pack_dir.clone()));

        // Restore
        // SAFETY: Restoring original env state at end of test scope.
        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_STARTERS_DIRS", v) },
            None => unsafe { std::env::remove_var("ARMADAI_STARTERS_DIRS") },
        }
    }
}
