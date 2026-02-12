use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::sync::repo_dir;
use crate::core::config::registry_cache_dir;

// ---------------------------------------------------------------------------
// Index data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// Relative path inside the registry repo (e.g. "agents/official/security.agent.md")
    pub path: String,
    /// Agent name derived from the file
    pub name: String,
    /// Optional description extracted from the file
    pub description: Option<String>,
    /// Tags extracted from the file
    #[serde(default)]
    pub tags: Vec<String>,
    /// Category from the directory structure (e.g. "official", "community")
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub entries: Vec<IndexEntry>,
}

// ---------------------------------------------------------------------------
// Index building
// ---------------------------------------------------------------------------

/// Build a search index from the registry repo.
///
/// Scans for `.agent.md` and `.md` files in the repo and extracts metadata.
pub fn build_index() -> anyhow::Result<Index> {
    let repo = repo_dir();
    if !repo.is_dir() {
        anyhow::bail!("Registry not synced. Run `armadai registry sync` first.");
    }

    let mut entries = Vec::new();
    scan_dir(&repo, &repo, &mut entries)?;

    let index = Index { entries };
    save_index(&index)?;
    Ok(index)
}

/// Load the cached index, or build it if missing.
pub fn load_or_build_index() -> anyhow::Result<Index> {
    let index_path = index_file_path();
    if index_path.is_file() {
        let content = std::fs::read_to_string(&index_path)?;
        let index: Index = serde_json::from_str(&content)?;
        return Ok(index);
    }
    build_index()
}

fn index_file_path() -> PathBuf {
    registry_cache_dir().join("index.json")
}

fn save_index(index: &Index) -> anyhow::Result<()> {
    let path = index_file_path();
    let content = serde_json::to_string_pretty(index)?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Recursively scan a directory for agent markdown files.
fn scan_dir(dir: &Path, repo_root: &Path, entries: &mut Vec<IndexEntry>) -> anyhow::Result<()> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    for entry in read.flatten() {
        let path = entry.path();

        // Skip .git directory
        if path.file_name().is_some_and(|n| n == ".git") {
            continue;
        }

        if path.is_dir() {
            scan_dir(&path, repo_root, entries)?;
        } else if is_agent_file(&path)
            && let Ok(entry) = extract_entry(&path, repo_root)
        {
            entries.push(entry);
        }
    }

    Ok(())
}

fn is_agent_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".agent.md")
        || (name.ends_with(".md") && !name.eq_ignore_ascii_case("README.md"))
}

/// Extract an index entry from a file by reading its first lines.
fn extract_entry(path: &Path, repo_root: &Path) -> anyhow::Result<IndexEntry> {
    let content = std::fs::read_to_string(path)?;
    let rel_path = path
        .strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    // Derive name from filename
    let file_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let name = file_name.trim_end_matches(".agent").to_string();

    // Extract description from first non-heading, non-empty line
    let description = content
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .next();

    // Derive category from parent directory name
    let category = path
        .parent()
        .and_then(|p| p.strip_prefix(repo_root).ok())
        .and_then(|p| p.components().next())
        .and_then(|c| c.as_os_str().to_str().map(String::from));

    // Try to extract tags from the content
    let tags = extract_tags(&content);

    Ok(IndexEntry {
        path: rel_path,
        name,
        description,
        tags,
        category,
    })
}

/// Try to extract tags from agent content (looks for `tags:` in metadata).
fn extract_tags(content: &str) -> Vec<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("- tags:")
            .or_else(|| trimmed.strip_prefix("tags:"))
        {
            let cleaned = rest.trim().trim_start_matches('[').trim_end_matches(']');
            return cleaned
                .split(',')
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Converted agent cache
// ---------------------------------------------------------------------------

/// Return the directory for cached converted agents.
pub fn converted_dir() -> PathBuf {
    registry_cache_dir().join("converted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tags() {
        let content = "# Agent\n\n## Metadata\n- tags: [dev, review, security]\n";
        let tags = extract_tags(content);
        assert_eq!(tags, vec!["dev", "review", "security"]);
    }

    #[test]
    fn test_extract_tags_empty() {
        let content = "# Agent\n\nNo tags here.";
        let tags = extract_tags(content);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_extract_entry() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents").join("security.agent.md");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            "# Security Reviewer\n\nAnalyze code for OWASP vulnerabilities.\n\n## Metadata\n- tags: [security, review]\n",
        )
        .unwrap();

        let entry = extract_entry(&file, dir.path()).unwrap();
        assert_eq!(entry.name, "security");
        assert_eq!(
            entry.description.as_deref(),
            Some("Analyze code for OWASP vulnerabilities.")
        );
        assert_eq!(entry.category.as_deref(), Some("agents"));
        assert_eq!(entry.tags, vec!["security", "review"]);
    }

    #[test]
    fn test_is_agent_file() {
        assert!(is_agent_file(Path::new("security.agent.md")));
        assert!(is_agent_file(Path::new("code-reviewer.md")));
        assert!(!is_agent_file(Path::new("README.md")));
        assert!(!is_agent_file(Path::new("readme.md")));
        assert!(!is_agent_file(Path::new("data.json")));
    }

    #[test]
    fn test_index_roundtrip() {
        let index = Index {
            entries: vec![IndexEntry {
                path: "agents/test.agent.md".to_string(),
                name: "test".to_string(),
                description: Some("A test agent".to_string()),
                tags: vec!["test".to_string()],
                category: Some("agents".to_string()),
            }],
        };

        let json = serde_json::to_string(&index).unwrap();
        let deserialized: Index = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.entries.len(), 1);
        assert_eq!(deserialized.entries[0].name, "test");
    }
}
