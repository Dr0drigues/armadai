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
        let pack: StarterPack = serde_yaml_ng::from_str(&content)?;
        Ok(pack)
    }
}

/// Discover the built-in starter packs directory.
///
/// Resolution order:
/// 1. `./starters` relative to CWD (dev)
/// 2. `CARGO_MANIFEST_DIR/starters` (dev, compile-time path)
/// 3. Next to the binary (packaged installs)
/// 4. `~/.config/armadai/starters/` — extracted from embedded data on first use
pub fn builtin_starters_dir() -> PathBuf {
    let candidates = [
        // Dev: relative to CWD
        PathBuf::from("starters"),
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

    // 4. ARMADAI_STARTERS_DIRS env var (colon-separated)
    if let Ok(env_dirs) = std::env::var("ARMADAI_STARTERS_DIRS") {
        for entry in env_dirs.split(':') {
            let trimmed = entry.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                if p.is_dir() && !dirs.contains(&p) {
                    dirs.push(p);
                }
            }
        }
    }

    // 5. Custom dirs from config.yaml
    let config = load_user_config();
    for custom in &config.starters_dirs {
        let p = PathBuf::from(custom);
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    }

    dirs
}

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
        && let Ok(manifest) = serde_yaml_ng::from_str::<StartersManifest>(&content)
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
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && !p.file_name().is_some_and(|n| n == ".git") {
            scan_for_packs(&p, depth + 1, out);
        }
    }
}

/// Pack directories discovered under the remote starters cache (each
/// synced source is a subdir; packs are found by `discover_packs`).
pub fn remote_starter_pack_dirs() -> Vec<PathBuf> {
    remote_starter_pack_dirs_in(&starters_cache_dir())
}

/// Testable core: scan a cache root's per-source subdirs for packs.
fn remote_starter_pack_dirs_in(cache_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(sources) = std::fs::read_dir(cache_root) else {
        return out;
    };
    for src in sources.flatten() {
        if src.path().is_dir() {
            out.extend(discover_packs(&src.path()));
        }
    }
    out
}

/// Find a starter pack directory by name across all source directories.
///
/// User and custom directories take priority over built-in (last match wins).
/// If no local match is found, falls back to the remote starters cache: first
/// by directory basename, then — for single-pack repos whose dir differs from
/// the pack's declared `name:` — by reading each remote pack's `pack.yaml`
/// (local always wins over remote either way).
pub fn find_pack_dir(name: &str) -> Option<PathBuf> {
    let mut found: Option<PathBuf> = None;
    for dir in all_starters_dirs() {
        let candidate = dir.join(name);
        if !skipped(&candidate) && candidate.is_dir() && candidate.join("pack.yaml").is_file() {
            found = Some(candidate);
        }
    }
    if found.is_none() {
        found = remote_starter_pack_dirs()
            .into_iter()
            .find(|d| d.file_name().is_some_and(|n| n == name));
    }
    if found.is_none() {
        found = find_remote_pack_by_name(name);
    }
    found
}

/// Find a remote pack by its `pack.yaml` `name:` (not just its dir basename),
/// so single-pack repos whose dir != name still resolve. Testable core.
fn find_remote_pack_by_name_in(cache_root: &Path, name: &str) -> Option<PathBuf> {
    for dir in remote_starter_pack_dirs_in(cache_root) {
        if let Ok(pack) = StarterPack::load(&dir)
            && pack.name == name
        {
            return Some(dir);
        }
    }
    None
}

/// Find a remote pack by its declared `name:` across the synced starters
/// cache. See [`find_remote_pack_by_name_in`] for the testable core.
pub fn find_remote_pack_by_name(name: &str) -> Option<PathBuf> {
    find_remote_pack_by_name_in(&starters_cache_dir(), name)
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
    fn remote_cache_packs_are_discovered() {
        // A pack under the starters cache dir should be found by find_pack_dir.
        // Isolate the cache via a temp registry_cache_dir if overridable; else
        // assert discover_packs surfaces the cache packs through
        // remote_starter_pack_dirs().
        use std::fs;
        let tmp = std::env::temp_dir().join(format!("armadai-remote-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pack = tmp.join("src1/remote-demo/pack.yaml");
        fs::create_dir_all(pack.parent().unwrap()).unwrap();
        fs::write(&pack, "name: remote-demo\n").unwrap();
        // remote_starter_pack_dirs scans a given cache root for packs:
        let dirs = remote_starter_pack_dirs_in(&tmp);
        assert!(
            dirs.iter()
                .any(|d| d.file_name().unwrap().to_string_lossy() == "remote-demo")
        );
        let _ = fs::remove_dir_all(&tmp);
    }

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
        assert_eq!(seen, 7);
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
    fn test_find_pack_dir_builtin() {
        // Built-in packs should be findable
        let found = find_pack_dir("code-analysis-rust");
        assert!(found.is_some(), "Expected to find code-analysis-rust pack");
        assert!(found.unwrap().join("pack.yaml").is_file());
    }

    #[test]
    fn test_find_pack_dir_not_found() {
        assert!(find_pack_dir("nonexistent-pack-xyz").is_none());
    }

    #[test]
    fn find_remote_pack_by_name_by_manifest_name_not_dir() {
        // A remote pack whose dir differs from its `name:` should still
        // resolve by name, not just by directory basename.
        use std::fs;
        let tmp = std::env::temp_dir().join(format!("armadai-nm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let d = tmp.join("some-hash-dir/inner");
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("pack.yaml"),
            "name: cool-pack\ndescription: A cool pack\n",
        )
        .unwrap();
        let found = find_remote_pack_by_name_in(&tmp, "cool-pack");
        assert!(found.is_some());
        let _ = fs::remove_dir_all(&tmp);
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
        let found = find_pack_dir("env-pack");
        assert!(found.is_some(), "Expected to find env-pack");

        // Restore
        // SAFETY: Restoring original env state at end of test scope.
        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_STARTERS_DIRS", v) },
            None => unsafe { std::env::remove_var("ARMADAI_STARTERS_DIRS") },
        }
    }
}
