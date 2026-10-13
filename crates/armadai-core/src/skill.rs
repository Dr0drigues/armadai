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

/// The built-in skills ArmadAI extracted, one name per line, at the root of the user
/// skills folder: what it may remove once a newer binary no longer ships them.
pub const EXTRACTED_REGISTRY: &str = ".armadai-extracted";

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

        if super::embedded::ensure(skill_dir, &dest, force)? {
            count += 1;
        }
    }

    // a built-in skill this binary no longer ships is removed only when ArmadAI
    // recorded it as extracted (the registry) AND its folder still says it is ours
    // (a marker naming it): never on a marker alone, never a user's folder
    let shipped: Vec<String> = (EMBEDDED_SKILLS.dirs())
        .filter(|d| d.get_file(d.path().join("SKILL.md")).is_some())
        .filter_map(|d| {
            d.path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .collect();
    let registry = dst_root.join(EXTRACTED_REGISTRY);
    let recorded = std::fs::read_to_string(&registry).unwrap_or_default();
    for name in recorded
        .lines()
        .filter(|n| !n.is_empty() && !shipped.iter().any(|s| s == n))
    {
        let path = dst_root.join(name);
        // a registry line is a folder name, never a path
        let plain = !name.starts_with('.') && !name.contains(['/', '\\']);
        if plain && super::embedded::managed(&path) {
            std::fs::remove_dir_all(&path)?;
        }
    }
    let ours: Vec<&str> = (shipped.iter())
        .filter(|n| super::embedded::managed(&dst_root.join(n)))
        .map(String::as_str)
        .collect();
    std::fs::write(&registry, ours.join("\n") + "\n")?;

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
        let _guard = crate::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: This test modifies the global environment which is unsafe in Rust 2024.
        // Serialised via `env_lock()` to avoid data races with other tests.
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
                .join("armadai-starter-authoring")
                .join("SKILL.md")
                .exists()
        );

        let reg = skills_dir.join(EXTRACTED_REGISTRY);
        assert!(
            std::fs::read_to_string(&reg)
                .unwrap()
                .contains("armadai-agent-authoring\n")
        );
        let put = |p: &std::path::Path, text: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        // a retired built-in skill: recorded AND marked with its own name -> removed
        let retired = skills_dir.join("armadai-prompt-authoring");
        put(
            &retired.join(".armadai-version"),
            "0.0.1 x\narmadai-prompt-authoring\n",
        );
        // marked but never recorded -> kept; recorded but no marker naming it -> kept
        let unrecorded = skills_dir.join("old-builtin");
        put(
            &unrecorded.join(".armadai-version"),
            "0.0.1 x\nold-builtin\n",
        );
        let unmarked = skills_dir.join("recorded-only");
        put(&unmarked.join("SKILL.md"), "mine");
        // a user's copy of a built-in skill, its marker copied along -> kept
        let copy = skills_dir.join("my-copy");
        put(
            &copy.join(".armadai-version"),
            "0.0.1 x\narmadai-agent-authoring\n",
        );
        // a user's note in an unmanaged built-in folder -> kept
        let note = skills_dir.join("armadai-skill-authoring/my-notes.md");
        put(&note, "mine");
        std::fs::write(
            skills_dir.join("armadai-skill-authoring/.armadai-version"),
            "0.0.1",
        )
        .unwrap();
        // a stale file of a managed built-in folder -> removed
        let old = skills_dir.join("armadai-agent-authoring/references/old.md");
        put(&old, "old");
        std::fs::write(
            skills_dir.join("armadai-agent-authoring/.armadai-version"),
            "0.0.1 x\narmadai-agent-authoring\n",
        )
        .unwrap();
        let mut recorded = std::fs::read_to_string(&reg).unwrap();
        recorded.push_str("armadai-prompt-authoring\nrecorded-only\n../escape\n");
        std::fs::write(&reg, recorded).unwrap();

        install_embedded_skills(false).unwrap();
        assert!(!retired.exists());
        assert!(unrecorded.exists());
        assert!(unmarked.join("SKILL.md").is_file());
        assert!(copy.exists());
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "mine");
        assert!(!old.exists());
        assert!(
            !std::fs::read_to_string(&reg)
                .unwrap()
                .contains("armadai-prompt-authoring")
        );

        // Verify references are copied
        assert!(
            skills_dir
                .join("armadai-agent-authoring")
                .join("references")
                .join("format.md")
                .exists()
        );

        // SAFETY: Cleaning up env state at end of test scope.
        unsafe {
            std::env::remove_var("ARMADAI_CONFIG_DIR");
        }
    }
}
