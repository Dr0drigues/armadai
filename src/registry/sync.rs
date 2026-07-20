use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::config::registry_cache_dir;
use crate::core::project::find_project_config;
use crate::core::registries::{RegistryKind, load_user_registries, resolved_sources};

/// Built-in default community registry (awesome-copilot).
pub const DEFAULT_REGISTRY_URL: &str = "https://github.com/github/awesome-copilot.git";

/// Resolve the effective list of agent registry sources for this run: the
/// built-in default, plus any user-level (`~/.config/armadai/registries.yaml`)
/// and project-level (`armadai.yaml` / `.armadai/config.yaml`) custom sources.
///
/// Project config is looked up via `core::project::find_project_config`,
/// which walks up from the current working directory — this matches how the
/// `armadai registry` CLI commands are invoked. When no project config is
/// found (or it has no `registries:` section), only defaults + user sources
/// apply.
pub fn effective_sources() -> Vec<String> {
    let user = load_user_registries();
    let project = find_project_config().map(|(_, cfg)| cfg);
    let project_registries = project.as_ref().and_then(|cfg| cfg.registries.as_ref());
    resolved_sources(
        RegistryKind::Agents,
        &[DEFAULT_REGISTRY_URL],
        &user,
        project_registries,
    )
}

/// Root directory holding all cloned registry sources, one subdirectory per
/// source (see [`source_dir`]).
pub fn sources_dir() -> PathBuf {
    registry_cache_dir().join("sources")
}

/// Derive a filesystem-safe, collision-resistant key for a registry source
/// URL.
///
/// Delegates to `core::registries::cache_key`, the single sanitization
/// scheme shared with `model_registry::fetch::source_cache_path` (previously
/// each had its own ad hoc, collision-prone sanitization — see B2 Task 2
/// review). The key always carries a hash suffix derived from the full URL,
/// so two sources that happen to sanitize to the same readable prefix (or a
/// prefix that would otherwise be a bare `.`/`..`, which is unsafe as a path
/// segment) still get distinct, safe directory names.
pub fn source_key(url: &str) -> String {
    crate::core::registries::cache_key(url)
}

/// Directory for a source already identified by its [`source_key`]. Used to
/// relocate a source's clone without needing its original URL again (e.g.
/// from a cached [`super::cache::IndexEntry`]).
pub fn dir_for_key(key: &str) -> PathBuf {
    sources_dir().join(key)
}

/// Directory where a given source URL is cloned.
pub fn source_dir(url: &str) -> PathBuf {
    dir_for_key(&source_key(url))
}

/// Clone or pull a single registry source by URL.
pub fn sync_source(url: &str) -> anyhow::Result<PathBuf> {
    let dest = source_dir(url);

    if dest.join(".git").is_dir() {
        pull(&dest)?;
    } else {
        clone(url, &dest)?;
    }

    Ok(dest)
}

/// Sync (clone or pull) every source in `sources`. Failures on individual
/// sources are logged and skipped rather than aborting the whole sync, so a
/// single unreachable custom registry doesn't block the default one (same
/// approach as `skills_registry::sync::sync_all`).
///
/// Records a sync-completed marker (see [`mark_synced`]) when at least one
/// source synced successfully, so [`is_stale`] can report accurate freshness
/// afterwards.
pub fn registry_sync(sources: &[String]) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for url in sources {
        match sync_source(url) {
            Ok(dir) => dirs.push(dir),
            Err(e) => eprintln!("  warn: failed to sync {url}: {e}"),
        }
    }
    if !dirs.is_empty()
        && let Err(e) = mark_synced()
    {
        eprintln!("  warn: failed to record sync timestamp: {e}");
    }
    Ok(dirs)
}

fn clone(url: &str, dest: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let output = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(dest)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git clone failed for {url}: {stderr}");
    }

    println!("Cloned {url}");
    Ok(())
}

fn pull(repo: &Path) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["pull", "--ff-only"])
        .current_dir(repo)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git pull failed for {}: {stderr}", repo.display());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("Already up to date") {
        println!("  {} already up to date.", repo.display());
    } else {
        println!("  {} updated.", repo.display());
    }

    Ok(())
}

/// Marker file touched at the end of a successful [`registry_sync`] run, and
/// read by [`is_stale`] to determine freshness.
///
/// A plain directory mtime (the previous approach — see git history) is
/// *not* a reliable freshness signal here: `sources_dir()` is the parent of
/// every source's own clone, and pulling into a subdirectory that already
/// exists does not bump the parent directory's mtime on most filesystems.
/// That made `is_stale` report "outdated" forever after the very first sync,
/// even immediately after a successful `armadai registry sync`. A dedicated
/// marker file, rewritten on every successful sync, sidesteps that.
fn last_sync_marker() -> PathBuf {
    sources_dir().join(".last_sync")
}

/// Record that the registry was just synced successfully.
///
/// Called automatically by [`registry_sync`]; exposed so callers (and
/// tests) can record a sync without needing to actually shell out to `git`.
pub fn mark_synced() -> anyhow::Result<()> {
    let marker = last_sync_marker();
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&marker, b"")?;
    Ok(())
}

/// The pre-multi-source registry clone directory (a single `repo/` folder,
/// used before B2 Lot A Task 2 introduced per-source `sources/<key>/`
/// directories). Kept around only so a cache from an older ArmadAI version
/// can be *detected* (see `cache::has_legacy_cache`) rather than silently
/// served or mistaken for an empty registry.
pub fn legacy_repo_dir() -> PathBuf {
    registry_cache_dir().join("repo")
}

/// Check if the registry cache was last synced more than `days` ago.
/// Returns `true` if a re-sync is recommended. Based on [`mark_synced`]'s
/// marker file rather than any directory's mtime (see [`last_sync_marker`]).
pub fn is_stale(days: u64) -> bool {
    match std::fs::metadata(last_sync_marker()).and_then(|m| m.modified()) {
        Ok(modified) => {
            let age = std::time::SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default();
            age.as_secs() > days * 86400
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::registries::{RegistriesConfig, RegistrySource};

    #[test]
    fn source_key_github_url_is_readable_and_stable() {
        let key = source_key("https://github.com/github/awesome-copilot.git");
        assert!(key.contains("github"));
        assert!(key.contains("awesome-copilot"));
        // Deterministic: same URL always maps to the same key.
        assert_eq!(
            key,
            source_key("https://github.com/github/awesome-copilot.git")
        );
    }

    #[test]
    fn source_key_non_github_url_is_sanitized() {
        let key = source_key("https://example.com/my-registry.git");
        assert!(!key.is_empty());
        assert!(
            key.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
        );
        // No path separators or other characters that would escape the
        // "sources" cache directory.
        assert!(!key.contains('/'));
    }

    #[test]
    fn source_key_distinguishes_urls_with_same_readable_prefix() {
        // Two distinct URLs (different owners) whose naive sanitization
        // could plausibly collide must still yield distinct keys — this is
        // what the hash suffix in `core::registries::cache_key` guarantees.
        let a = source_key("https://github.com/acme/registry");
        let b = source_key("https://github.com/acme/registry-fork");
        assert_ne!(a, b);
    }

    #[test]
    fn source_key_traversal_like_input_is_not_dot_dot() {
        // A URL that sanitizes to a bare ".." would be a path-traversal-
        // shaped directory name; the hash suffix rules that out.
        let key = source_key("..");
        assert_ne!(key, "..");
        assert_ne!(key, ".");
    }

    #[test]
    fn is_stale_true_when_never_synced() {
        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
        }

        assert!(is_stale(7));

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }

    #[test]
    fn is_stale_false_right_after_mark_synced() {
        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
        }

        mark_synced().expect("mark_synced should succeed");
        assert!(!is_stale(7), "should be fresh immediately after a sync");

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }

    #[test]
    fn is_stale_true_when_marker_older_than_ttl() {
        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
        }

        mark_synced().expect("mark_synced should succeed");
        let marker = last_sync_marker();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86400);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&marker)
            .expect("marker should be openable");
        file.set_modified(old).expect("should backdate mtime");

        assert!(
            is_stale(7),
            "8-day-old marker should be stale with a 7-day TTL"
        );

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }

    #[test]
    fn effective_sources_glue_includes_user_level_custom_source() {
        // Exercises the real `effective_sources()` glue (not just the pure
        // `resolved_sources` helper) with a `RegistriesConfig` loaded from
        // disk, per the B2 Task 2 review follow-up.
        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
        }

        std::fs::write(
            dir.path().join("registries.yaml"),
            "agents:\n  - url: https://example.com/custom-glue-agents.git\n",
        )
        .unwrap();

        let sources = effective_sources();

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }

        assert!(sources.contains(&DEFAULT_REGISTRY_URL.to_string()));
        assert!(sources.contains(&"https://example.com/custom-glue-agents.git".to_string()));
    }

    #[test]
    fn effective_sources_defaults_only_without_config() {
        // No project config in scope here (None passed directly rather than
        // via `find_project_config`, to keep this test independent of cwd).
        let user = RegistriesConfig::default();
        let result = resolved_sources(RegistryKind::Agents, &[DEFAULT_REGISTRY_URL], &user, None);
        assert_eq!(result, vec![DEFAULT_REGISTRY_URL.to_string()]);
    }

    #[test]
    fn effective_sources_includes_default_and_custom() {
        let user = RegistriesConfig {
            agents: vec![RegistrySource {
                url: "https://example.com/custom-agents.git".to_string(),
            }],
            ..Default::default()
        };
        let result = resolved_sources(RegistryKind::Agents, &[DEFAULT_REGISTRY_URL], &user, None);
        assert_eq!(
            result,
            vec![
                DEFAULT_REGISTRY_URL.to_string(),
                "https://example.com/custom-agents.git".to_string(),
            ]
        );
    }
}
