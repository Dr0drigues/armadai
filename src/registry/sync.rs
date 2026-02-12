use std::path::Path;
use std::process::Command;

use crate::core::config::registry_cache_dir;

const DEFAULT_REGISTRY_URL: &str = "https://github.com/anthropics/awesome-copilot.git";

/// Return the path to the cloned registry repository.
pub fn repo_dir() -> std::path::PathBuf {
    registry_cache_dir().join("repo")
}

/// Clone or pull the registry repository.
///
/// Uses system `git` — no libgit2 dependency.
pub fn registry_sync(url: Option<&str>) -> anyhow::Result<()> {
    let url = url.unwrap_or(DEFAULT_REGISTRY_URL);
    let repo = repo_dir();

    if repo.join(".git").is_dir() {
        pull(&repo)?;
    } else {
        clone(url, &repo)?;
    }

    Ok(())
}

fn clone(url: &str, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;

    let output = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(dest)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git clone failed: {stderr}");
    }

    println!("Cloned registry from {url}");
    Ok(())
}

fn pull(repo: &Path) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["pull", "--ff-only"])
        .current_dir(repo)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git pull failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("Already up to date") {
        println!("Registry already up to date.");
    } else {
        println!("Registry updated.");
    }

    Ok(())
}

/// Check if the registry was last synced more than `days` ago.
/// Returns `true` if a re-sync is recommended.
pub fn is_stale(days: u64) -> bool {
    let repo = repo_dir();
    let git_dir = repo.join(".git");
    if !git_dir.is_dir() {
        return true;
    }

    let fetch_head = git_dir.join("FETCH_HEAD");
    let check_path = if fetch_head.exists() {
        fetch_head
    } else {
        git_dir.join("HEAD")
    };

    match std::fs::metadata(&check_path).and_then(|m| m.modified()) {
        Ok(modified) => {
            let age = std::time::SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default();
            age.as_secs() > days * 86400
        }
        Err(_) => true,
    }
}
