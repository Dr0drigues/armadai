use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::sync::{default_sources, parse_source, repo_dir, repos_dir};
use crate::core::config::skills_registry_dir;
use crate::core::skill::SkillFrontmatter;
use crate::parser::frontmatter::extract_frontmatter;

// ---------------------------------------------------------------------------
// Index data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillIndexEntry {
    /// Skill name from SKILL.md frontmatter (or directory name)
    pub name: String,
    /// Optional description from frontmatter
    pub description: Option<String>,
    /// Source repo slug (e.g. "anthropics/skills")
    pub source_repo: String,
    /// Relative path inside the repo (e.g. "skills/webapp-testing")
    pub path: String,
    /// Tags extracted from frontmatter tools or metadata
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillIndex {
    pub entries: Vec<SkillIndexEntry>,
}

// ---------------------------------------------------------------------------
// Index building
// ---------------------------------------------------------------------------

/// Build a search index from all synced skill repos.
pub fn build_index(sources: &[String]) -> anyhow::Result<SkillIndex> {
    let mut entries = Vec::new();

    for source in sources {
        if let Some((owner, repo)) = parse_source(source) {
            let dir = repo_dir(&owner, &repo);
            if dir.is_dir() {
                let slug = format!("{owner}/{repo}");
                scan_repo(&dir, &dir, &slug, &mut entries)?;
            }
        }
    }

    let index = SkillIndex { entries };
    save_index(&index)?;
    Ok(index)
}

/// Load the cached index, or build it from synced repos.
pub fn load_or_build_index(sources: &[String]) -> anyhow::Result<SkillIndex> {
    let index_path = index_file_path();
    if index_path.is_file() {
        let content = std::fs::read_to_string(&index_path)?;
        let index: SkillIndex = serde_json::from_str(&content)?;
        return Ok(index);
    }

    // Try to build from whatever is already cloned
    if repos_dir().is_dir() {
        return build_index(sources);
    }

    Ok(SkillIndex::default())
}

fn index_file_path() -> PathBuf {
    skills_registry_dir().join("index.json")
}

fn save_index(index: &SkillIndex) -> anyhow::Result<()> {
    let path = index_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(index)?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Recursively scan a repo for directories containing SKILL.md.
fn scan_repo(
    dir: &Path,
    repo_root: &Path,
    source_slug: &str,
    entries: &mut Vec<SkillIndexEntry>,
) -> anyhow::Result<()> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    for entry in read.flatten() {
        let path = entry.path();

        // Skip hidden directories
        if path
            .file_name()
            .is_some_and(|n| n.to_str().is_some_and(|s| s.starts_with('.')))
        {
            continue;
        }

        if path.is_dir() {
            let skill_file = path.join("SKILL.md");
            if skill_file.is_file()
                && let Ok(entry) = extract_skill_entry(&path, &skill_file, repo_root, source_slug)
            {
                entries.push(entry);
            }
            // Continue scanning subdirectories
            scan_repo(&path, repo_root, source_slug, entries)?;
        }
    }

    Ok(())
}

/// Extract a skill index entry from a directory with a SKILL.md.
fn extract_skill_entry(
    skill_dir: &Path,
    skill_file: &Path,
    repo_root: &Path,
    source_slug: &str,
) -> anyhow::Result<SkillIndexEntry> {
    let content = std::fs::read_to_string(skill_file)?;
    let (fm_str, _body) = extract_frontmatter(&content);

    let fm: SkillFrontmatter = match fm_str {
        Some(yaml) => serde_yaml_ng::from_str(yaml).unwrap_or_default(),
        None => SkillFrontmatter::default(),
    };

    let name = fm.name.unwrap_or_else(|| {
        skill_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    let rel_path = skill_dir
        .strip_prefix(repo_root)
        .unwrap_or(skill_dir)
        .to_string_lossy()
        .to_string();

    Ok(SkillIndexEntry {
        name,
        description: fm.description,
        source_repo: source_slug.to_string(),
        path: rel_path,
        tags: fm.tools, // use tools as tags for searchability
    })
}

/// Get the effective sources list (from defaults).
pub fn effective_sources() -> Vec<String> {
    default_sources()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_skill_entry() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("webapp-testing");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: webapp-testing\ndescription: Web application testing skill\ntools:\n  - playwright\n  - jest\n---\n# WebApp Testing\n\nTest web apps.",
        ).unwrap();

        let entry = extract_skill_entry(
            &skill_dir,
            &skill_dir.join("SKILL.md"),
            dir.path(),
            "anthropics/skills",
        )
        .unwrap();

        assert_eq!(entry.name, "webapp-testing");
        assert_eq!(
            entry.description.as_deref(),
            Some("Web application testing skill")
        );
        assert_eq!(entry.source_repo, "anthropics/skills");
        assert_eq!(entry.path, "webapp-testing");
        assert_eq!(entry.tags, vec!["playwright", "jest"]);
    }

    #[test]
    fn test_extract_skill_entry_no_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("simple");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Simple Skill\n\nJust a body.").unwrap();

        let entry = extract_skill_entry(
            &skill_dir,
            &skill_dir.join("SKILL.md"),
            dir.path(),
            "owner/repo",
        )
        .unwrap();

        assert_eq!(entry.name, "simple");
        assert!(entry.description.is_none());
        assert!(entry.tags.is_empty());
    }

    #[test]
    fn test_index_roundtrip() {
        let index = SkillIndex {
            entries: vec![SkillIndexEntry {
                name: "docker-compose".to_string(),
                description: Some("Docker Compose management".to_string()),
                source_repo: "anthropics/skills".to_string(),
                path: "skills/docker-compose".to_string(),
                tags: vec!["docker".to_string(), "compose".to_string()],
            }],
        };

        let json = serde_json::to_string(&index).unwrap();
        let deserialized: SkillIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.entries.len(), 1);
        assert_eq!(deserialized.entries[0].name, "docker-compose");
    }
}
