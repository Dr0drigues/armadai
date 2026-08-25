use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::sync::{source_dir, source_key, sources_dir};
use armadai_core::config::registry_cache_dir;

// ---------------------------------------------------------------------------
// Index data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// Relative path inside the source repo (e.g. "agents/official/security.agent.md")
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
    /// Source key this entry was scanned from (see `sync::source_key`), used
    /// to relocate the file across multiple registry sources. Defaults to
    /// the empty string for indexes built before multi-source support (B2
    /// Lot A Task 2) — those all came from the single default source.
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub entries: Vec<IndexEntry>,
}

// ---------------------------------------------------------------------------
// Index building
// ---------------------------------------------------------------------------

/// Build a search index by scanning every synced source repo in `sources`.
///
/// Sources that haven't been cloned yet (e.g. sync failed or wasn't run) are
/// silently skipped so a single broken/unreachable source doesn't prevent
/// indexing the others.
pub fn build_index(sources: &[String]) -> anyhow::Result<Index> {
    let mut entries = Vec::new();

    for url in sources {
        let key = source_key(url);
        let dir = source_dir(url);
        if dir.is_dir() {
            scan_dir(&dir, &dir, &key, &mut entries)?;
        }
    }

    let index = Index { entries };
    save_index(&index)?;
    Ok(index)
}

/// Load the cached index, or build it from whatever sources are already
/// synced.
///
/// A cached index built before multi-source support (B2 Lot A Task 2, entries
/// with `source == ""`) is treated as invalid rather than served: its
/// `path`s are relative to the old single `repo/` clone, which no longer
/// exists at the new per-source `sources_dir()/<key>/` layout, so serving it
/// would either point `list`/`search`/`info` at stale data silently or make
/// `registry add` fail with a cryptic "source ''" error (see
/// `sync::legacy_repo_dir`, `has_legacy_cache`). When a legacy index is
/// detected, we fall through and rebuild from whatever *new*-layout sources
/// are already synced (typically none yet, so this resolves to an empty
/// index — same as a fresh install — until the user runs `registry sync`).
pub fn load_or_build_index(sources: &[String]) -> anyhow::Result<Index> {
    let index_path = index_file_path();
    if index_path.is_file() {
        let content = std::fs::read_to_string(&index_path)?;
        let index: Index = serde_json::from_str(&content)?;
        if !is_legacy(&index) {
            return Ok(index);
        }
    }

    if sources_dir().is_dir() {
        return build_index(sources);
    }

    Ok(Index::default())
}

/// True when `index` was built before multi-source support (B2 Lot A Task
/// 2): any entry with an empty `source` came from the single pre-Task-2
/// default registry, whose on-disk paths no longer resolve under the
/// current per-source layout.
fn is_legacy(index: &Index) -> bool {
    index.entries.iter().any(|e| e.source.is_empty())
}

/// True when a pre-multi-source registry cache is present on disk: either
/// the old single-repo clone (`registry/repo/`) or a cached index with
/// legacy (empty `source`) entries. Used to show the user a clear
/// "run `armadai registry sync`" hint instead of silently serving stale
/// data, or erroring cryptically on `registry add`/convert.
pub fn has_legacy_cache() -> bool {
    if super::sync::legacy_repo_dir().is_dir() {
        return true;
    }

    let index_path = index_file_path();
    match std::fs::read_to_string(&index_path) {
        Ok(content) => serde_json::from_str::<Index>(&content)
            .map(|index| is_legacy(&index))
            .unwrap_or(false),
        Err(_) => false,
    }
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
fn scan_dir(
    dir: &Path,
    repo_root: &Path,
    source: &str,
    entries: &mut Vec<IndexEntry>,
) -> anyhow::Result<()> {
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
            scan_dir(&path, repo_root, source, entries)?;
        } else if is_agent_file(&path)
            && let Ok(entry) = extract_entry(&path, repo_root, source)
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
fn extract_entry(path: &Path, repo_root: &Path, source: &str) -> anyhow::Result<IndexEntry> {
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
        source: source.to_string(),
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

        let entry = extract_entry(&file, dir.path(), "github/awesome-copilot").unwrap();
        assert_eq!(entry.name, "security");
        assert_eq!(
            entry.description.as_deref(),
            Some("Analyze code for OWASP vulnerabilities.")
        );
        assert_eq!(entry.category.as_deref(), Some("agents"));
        assert_eq!(entry.tags, vec!["security", "review"]);
        assert_eq!(entry.source, "github/awesome-copilot");
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
                source: "github/awesome-copilot".to_string(),
            }],
        };

        let json = serde_json::to_string(&index).unwrap();
        let deserialized: Index = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.entries.len(), 1);
        assert_eq!(deserialized.entries[0].name, "test");
    }

    #[test]
    fn test_index_deserializes_without_source_field() {
        // Back-compat: indexes cached before multi-source support (B2 Lot A
        // Task 2) have no `source` field.
        let json = r#"{"entries":[{"path":"a.md","name":"a","description":null,"tags":[],"category":null}]}"#;
        let index: Index = serde_json::from_str(json).unwrap();
        assert_eq!(index.entries[0].source, "");
    }

    #[test]
    fn is_legacy_true_for_index_with_empty_source_entry() {
        let index = Index {
            entries: vec![IndexEntry {
                path: "a.md".to_string(),
                name: "a".to_string(),
                description: None,
                tags: vec![],
                category: None,
                source: String::new(),
            }],
        };
        assert!(is_legacy(&index));
    }

    #[test]
    fn is_legacy_false_for_current_format_index() {
        let index = Index {
            entries: vec![IndexEntry {
                path: "a.md".to_string(),
                name: "a".to_string(),
                description: None,
                tags: vec![],
                category: None,
                source: "github/awesome-copilot-abc12345".to_string(),
            }],
        };
        assert!(!is_legacy(&index));
    }

    #[test]
    fn is_legacy_false_for_empty_index() {
        assert!(!is_legacy(&Index::default()));
    }

    fn with_config_dir<F: FnOnce(&std::path::Path)>(f: F) {
        let _guard = armadai_core::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: serialised via `env_lock()`; restored at end of scope.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
        }

        f(dir.path());

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }

    #[test]
    fn has_legacy_cache_false_when_nothing_on_disk() {
        with_config_dir(|_| {
            assert!(!has_legacy_cache());
        });
    }

    #[test]
    fn has_legacy_cache_true_for_old_repo_dir() {
        with_config_dir(|config_dir| {
            let legacy_repo = config_dir.join("registry").join("repo");
            std::fs::create_dir_all(&legacy_repo).unwrap();
            assert!(has_legacy_cache());
        });
    }

    #[test]
    fn has_legacy_cache_true_for_legacy_index_json() {
        with_config_dir(|config_dir| {
            let registry_dir = config_dir.join("registry");
            std::fs::create_dir_all(&registry_dir).unwrap();
            std::fs::write(
                registry_dir.join("index.json"),
                r#"{"entries":[{"path":"a.md","name":"a","description":null,"tags":[],"category":null,"source":""}]}"#,
            )
            .unwrap();
            assert!(has_legacy_cache());
        });
    }

    #[test]
    fn load_or_build_index_ignores_legacy_index_instead_of_serving_it() {
        with_config_dir(|config_dir| {
            let registry_dir = config_dir.join("registry");
            std::fs::create_dir_all(&registry_dir).unwrap();
            std::fs::write(
                registry_dir.join("index.json"),
                r#"{"entries":[{"path":"agents/x.md","name":"x","description":null,"tags":[],"category":null,"source":""}]}"#,
            )
            .unwrap();

            // No new-layout `sources/` dir exists, so with the legacy index
            // ignored this must resolve to an empty index rather than the
            // stale legacy entry (which would otherwise point `registry add`
            // at a nonexistent `sources_dir()/agents/x.md`).
            let index = load_or_build_index(&[]).expect("should not error");
            assert!(index.entries.is_empty());
        });
    }
}
