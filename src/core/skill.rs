use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::parser::frontmatter::extract_frontmatter;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub tools: Vec<String>,
    pub body: String,
    pub source: PathBuf,
    pub scripts: Vec<PathBuf>,
    pub references: Vec<PathBuf>,
    pub assets: Vec<PathBuf>,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

impl Skill {
    /// Load a skill from a directory containing a `SKILL.md` file.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let skill_file = dir.join("SKILL.md");
        if !skill_file.is_file() {
            anyhow::bail!("No SKILL.md found in {}", dir.display());
        }

        let content = std::fs::read_to_string(&skill_file)?;
        let (fm_str, body) = extract_frontmatter(&content);

        let fm: SkillFrontmatter = match fm_str {
            Some(yaml) => serde_yaml_ng::from_str(yaml)?,
            None => SkillFrontmatter::default(),
        };

        let name = fm.name.unwrap_or_else(|| name_from_dir(dir));

        let scripts = list_dir_entries(&dir.join("scripts"));
        let references = list_dir_entries(&dir.join("references"));
        let assets = list_dir_entries(&dir.join("assets"));

        Ok(Self {
            name,
            description: fm.description,
            version: fm.version,
            tools: fm.tools,
            body: body.to_string(),
            source: dir.to_path_buf(),
            scripts,
            references,
            assets,
        })
    }
}

/// Derive a skill name from its directory name.
fn name_from_dir(dir: &Path) -> String {
    dir.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// List file entries in a subdirectory (non-recursive). Returns empty vec if
/// the directory doesn't exist.
fn list_dir_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return entries,
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_file() {
            entries.push(path);
        }
    }
    entries.sort();
    entries
}

/// Scan a directory for skill subdirectories (each containing a `SKILL.md`).
pub fn load_all_skills(dir: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return skills,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("SKILL.md").is_file() {
            match Skill::load(&path) {
                Ok(s) => skills.push(s),
                Err(e) => eprintln!("  warn: failed to load skill {}: {e}", path.display()),
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("docker-compose");
        std::fs::create_dir_all(&skill_dir).unwrap();

        let content = "---\nname: docker-compose\ndescription: Docker Compose management\nversion: \"1.0\"\ntools:\n  - docker\n  - compose\n---\n# Docker Compose\n\nManage multi-container apps.";
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();

        // Create subdirectories
        let scripts_dir = skill_dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        std::fs::write(scripts_dir.join("deploy.sh"), "#!/bin/bash").unwrap();

        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        std::fs::write(refs_dir.join("docs.md"), "# Docs").unwrap();

        let skill = Skill::load(&skill_dir).unwrap();
        assert_eq!(skill.name, "docker-compose");
        assert_eq!(
            skill.description.as_deref(),
            Some("Docker Compose management")
        );
        assert_eq!(skill.version.as_deref(), Some("1.0"));
        assert_eq!(skill.tools, vec!["docker", "compose"]);
        assert!(skill.body.contains("# Docker Compose"));
        assert_eq!(skill.scripts.len(), 1);
        assert_eq!(skill.references.len(), 1);
        assert!(skill.assets.is_empty());
    }

    #[test]
    fn test_load_skill_no_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("simple-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Simple skill\n\nJust a body.").unwrap();

        let skill = Skill::load(&skill_dir).unwrap();
        assert_eq!(skill.name, "simple-skill");
        assert!(skill.description.is_none());
        assert!(skill.tools.is_empty());
    }

    #[test]
    fn test_load_skill_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = Skill::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_all_skills() {
        let dir = tempfile::tempdir().unwrap();

        // Skill A
        let a = dir.path().join("alpha");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("SKILL.md"), "---\nname: alpha\n---\nAlpha.").unwrap();

        // Skill B
        let b = dir.path().join("beta");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("SKILL.md"), "---\nname: beta\n---\nBeta.").unwrap();

        // Not a skill (no SKILL.md)
        let c = dir.path().join("not-a-skill");
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(c.join("README.md"), "nothing").unwrap();

        let skills = load_all_skills(dir.path());
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[1].name, "beta");
    }

    #[test]
    fn test_load_all_skills_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let skills = load_all_skills(dir.path());
        assert!(skills.is_empty());
    }

    #[test]
    fn test_load_all_skills_nonexistent_dir() {
        let skills = load_all_skills(Path::new("/nonexistent/dir"));
        assert!(skills.is_empty());
    }
}
