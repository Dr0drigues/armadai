use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use serde::Deserialize;

use crate::parser::frontmatter::extract_frontmatter;

use super::config::user_skills_dir;

static EMBEDDED_SKILLS: Dir = include_dir!("$CARGO_MANIFEST_DIR/skills");

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

/// Install embedded built-in skills to `~/.config/armadai/skills/`.
///
/// Copies entire skill directories (SKILL.md + references/) from the
/// compile-time embedded `skills/` directory.
///
/// Returns the count of installed skills.
pub fn install_embedded_skills(force: bool) -> anyhow::Result<usize> {
    let dst_root = user_skills_dir();
    std::fs::create_dir_all(&dst_root)?;

    let mut count = 0;

    for skill_dir in EMBEDDED_SKILLS.dirs() {
        // Each top-level dir in skills/ is one skill
        // include_dir stores paths relative to the include root,
        // so we need to use the full path for get_file().
        if skill_dir
            .get_file(skill_dir.path().join("SKILL.md"))
            .is_none()
        {
            continue;
        }

        let skill_name = skill_dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let dest = dst_root.join(skill_name);

        if dest.exists() && !force && !super::embedded::needs_update(&dest) {
            continue;
        }

        // Copy all files recursively
        extract_embedded_dir(skill_dir, &dest)?;
        super::embedded::write_version_marker(&dest);
        count += 1;
    }

    Ok(count)
}

/// Recursively extract an embedded directory to a filesystem path.
pub(crate) fn extract_embedded_dir(dir: &Dir<'_>, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;

    for file in dir.files() {
        let relative = file.path().strip_prefix(dir.path()).unwrap_or(file.path());
        let file_dest = dest.join(relative);
        if let Some(parent) = file_dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file_dest, file.contents())?;
    }

    for sub_dir in dir.dirs() {
        let relative = sub_dir
            .path()
            .strip_prefix(dir.path())
            .unwrap_or(sub_dir.path());
        let sub_dest = dest.join(relative);
        extract_embedded_dir(sub_dir, &sub_dest)?;
    }

    Ok(())
}

/// Read a file as UTF-8 text, returning `None` if the file can't be read or isn't valid UTF-8.
pub fn read_text_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    String::from_utf8(bytes).ok()
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

    #[test]
    fn test_install_embedded_skills() {
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
        }

        let count = install_embedded_skills(false).unwrap();
        assert!(
            count >= 3,
            "Expected at least 3 built-in skills, got {count}"
        );

        // Verify skills are installed
        let skills_dir = dir.path().join("skills");
        assert!(
            skills_dir
                .join("armadai-agent-authoring")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            skills_dir
                .join("armadai-skill-authoring")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            skills_dir
                .join("armadai-prompt-authoring")
                .join("SKILL.md")
                .exists()
        );

        // Verify references are copied
        assert!(
            skills_dir
                .join("armadai-agent-authoring")
                .join("references")
                .join("format.md")
                .exists()
        );

        unsafe {
            std::env::remove_var("ARMADAI_CONFIG_DIR");
        }
    }
}
