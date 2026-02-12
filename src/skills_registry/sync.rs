use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::config::skills_registry_dir;

/// Default skill sources (well-known GitHub repos containing skills).
pub fn default_sources() -> Vec<String> {
    vec![
        "https://github.com/anthropics/skills".to_string(),
        "https://github.com/openai/skills".to_string(),
    ]
}

/// Return the root directory for cloned skill repos.
pub fn repos_dir() -> PathBuf {
    skills_registry_dir().join("repos")
}

/// Return the directory for a specific owner/repo clone.
pub fn repo_dir(owner: &str, repo: &str) -> PathBuf {
    repos_dir().join(owner).join(repo)
}

/// Parse a GitHub URL into (owner, repo).
///
/// Accepts:
///  - `https://github.com/owner/repo`
///  - `https://github.com/owner/repo.git`
///  - `owner/repo`
pub fn parse_source(source: &str) -> Option<(String, String)> {
    // Try URL format first
    let stripped = source
        .strip_prefix("https://github.com/")
        .or_else(|| source.strip_prefix("http://github.com/"))
        .unwrap_or(source);

    let stripped = stripped.trim_end_matches(".git").trim_end_matches('/');
    let parts: Vec<&str> = stripped.splitn(3, '/').collect();
    if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Clone or pull a single skill repo by GitHub URL or owner/repo slug.
pub fn sync_repo(source: &str) -> anyhow::Result<PathBuf> {
    let (owner, repo) = parse_source(source).ok_or_else(|| {
        anyhow::anyhow!("Invalid source '{source}'. Use owner/repo or a GitHub URL.")
    })?;

    let url = format!("https://github.com/{owner}/{repo}.git");
    let dest = repo_dir(&owner, &repo);

    if dest.join(".git").is_dir() {
        pull(&dest)?;
    } else {
        clone(&url, &dest)?;
    }

    Ok(dest)
}

/// Sync all configured sources.
pub fn sync_all(sources: &[String]) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for source in sources {
        match sync_repo(source) {
            Ok(dir) => dirs.push(dir),
            Err(e) => eprintln!("  warn: failed to sync {source}: {e}"),
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

    println!("  Cloned {url}");
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

/// Check if the skills registry was last synced more than `days` ago.
pub fn is_stale(days: u64) -> bool {
    let dir = repos_dir();
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

    #[test]
    fn test_parse_source_url() {
        let (owner, repo) = parse_source("https://github.com/anthropics/skills").unwrap();
        assert_eq!(owner, "anthropics");
        assert_eq!(repo, "skills");
    }

    #[test]
    fn test_parse_source_url_with_git() {
        let (owner, repo) = parse_source("https://github.com/anthropics/skills.git").unwrap();
        assert_eq!(owner, "anthropics");
        assert_eq!(repo, "skills");
    }

    #[test]
    fn test_parse_source_slug() {
        let (owner, repo) = parse_source("openai/skills").unwrap();
        assert_eq!(owner, "openai");
        assert_eq!(repo, "skills");
    }

    #[test]
    fn test_parse_source_invalid() {
        assert!(parse_source("invalid").is_none());
        assert!(parse_source("").is_none());
        assert!(parse_source("/").is_none());
    }

    #[test]
    fn test_repo_dir_structure() {
        let dir = repo_dir("anthropics", "skills");
        assert!(dir.ends_with("anthropics/skills"));
    }
}
