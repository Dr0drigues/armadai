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

/// Derive a filesystem-safe, stable key for a registry source URL.
///
/// GitHub URLs (`https://github.com/owner/repo[.git]`) are keyed as
/// `owner/repo`, mirroring `skills_registry::sync::repo_dir`'s layout so the
/// two registries stay consistent. Any other URL (e.g. a self-hosted git
/// remote) falls back to a sanitized version of the whole URL so every
/// source still gets its own directory.
pub fn source_key(url: &str) -> String {
    match crate::skills_registry::sync::parse_source(url) {
        Some((owner, repo)) => format!("{owner}/{repo}"),
        None => url
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect(),
    }
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
pub fn registry_sync(sources: &[String]) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for url in sources {
        match sync_source(url) {
            Ok(dir) => dirs.push(dir),
            Err(e) => eprintln!("  warn: failed to sync {url}: {e}"),
        }
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

/// Check if the registry cache was last synced more than `days` ago.
/// Returns `true` if a re-sync is recommended (mirrors
/// `skills_registry::sync::is_stale`, checking the shared sources root
/// rather than a single repo since there may be several sources now).
pub fn is_stale(days: u64) -> bool {
    let dir = sources_dir();
    if !dir.is_dir() {
        return true;
    }

    match std::fs::metadata(&dir).and_then(|m| m.modified()) {
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
    fn source_key_github_url_uses_owner_repo() {
        assert_eq!(
            source_key("https://github.com/github/awesome-copilot.git"),
            "github/awesome-copilot"
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
