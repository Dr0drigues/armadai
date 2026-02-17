use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use serde::Deserialize;

use super::config::{user_agents_dir, user_prompts_dir, user_skills_dir};

static EMBEDDED_STARTERS: Dir = include_dir!("$CARGO_MANIFEST_DIR/starters");

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct StarterPack {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub prompts: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

impl StarterPack {
    /// Load a starter pack from a directory containing `pack.yaml`.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let pack_file = dir.join("pack.yaml");
        if !pack_file.is_file() {
            anyhow::bail!("No pack.yaml found in {}", dir.display());
        }
        let content = std::fs::read_to_string(&pack_file)?;
        let pack: StarterPack = serde_yaml_ng::from_str(&content)?;
        Ok(pack)
    }

    /// Install the starter pack's agents, prompts, and skills to the user library.
    ///
    /// Copies agent `.md` files from `<pack_dir>/agents/` to `~/.config/armadai/agents/`,
    /// prompt `.md` files from `<pack_dir>/prompts/` to `~/.config/armadai/prompts/`,
    /// and skill directories from `<pack_dir>/skills/` to `~/.config/armadai/skills/`.
    ///
    /// Skills listed in the pack but not bundled (e.g. built-in skills already
    /// installed by `armadai init`) are silently skipped.
    ///
    /// Returns `(installed_agents, installed_prompts, installed_skills)` counts.
    pub fn install(&self, pack_dir: &Path, force: bool) -> anyhow::Result<(usize, usize, usize)> {
        let agents_src = pack_dir.join("agents");
        let prompts_src = pack_dir.join("prompts");
        let skills_src = pack_dir.join("skills");
        let agents_dst = user_agents_dir();
        let prompts_dst = user_prompts_dir();
        let skills_dst = user_skills_dir();

        std::fs::create_dir_all(&agents_dst)?;
        std::fs::create_dir_all(&prompts_dst)?;
        std::fs::create_dir_all(&skills_dst)?;

        let mut agents_count = 0;
        let mut prompts_count = 0;
        let mut skills_count = 0;

        // Install agents
        for name in &self.agents {
            let filename = if name.ends_with(".md") {
                name.clone()
            } else {
                format!("{name}.md")
            };
            let src = agents_src.join(&filename);
            let dst = agents_dst.join(&filename);

            if !src.is_file() {
                eprintln!(
                    "  warn: agent '{name}' not found in pack at {}",
                    src.display()
                );
                continue;
            }

            if dst.exists() && !force {
                println!("  skip (exists): {}", dst.display());
                continue;
            }

            std::fs::copy(&src, &dst)?;
            println!("  installed: {}", dst.display());
            agents_count += 1;
        }

        // Install prompts
        for name in &self.prompts {
            let filename = if name.ends_with(".md") {
                name.clone()
            } else {
                format!("{name}.md")
            };
            let src = prompts_src.join(&filename);
            let dst = prompts_dst.join(&filename);

            if !src.is_file() {
                eprintln!(
                    "  warn: prompt '{name}' not found in pack at {}",
                    src.display()
                );
                continue;
            }

            if dst.exists() && !force {
                println!("  skip (exists): {}", dst.display());
                continue;
            }

            std::fs::copy(&src, &dst)?;
            println!("  installed: {}", dst.display());
            prompts_count += 1;
        }

        // Install skills (directories)
        for name in &self.skills {
            let src = skills_src.join(name);
            let dst = skills_dst.join(name);

            // Skills not bundled in the pack (e.g. built-in) are silently skipped
            if !src.is_dir() {
                continue;
            }

            if dst.exists() && !force {
                println!("  skip (exists): {}", dst.display());
                continue;
            }

            copy_dir_recursive(&src, &dst)?;
            println!("  installed: {}", dst.display());
            skills_count += 1;
        }

        Ok((agents_count, prompts_count, skills_count))
    }
}

/// Recursively copy a directory tree from `src` to `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if dst.exists() {
        std::fs::remove_dir_all(dst)?;
    }
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)?.flatten() {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Discover available starter packs directory.
///
/// Resolution order:
/// 1. `./starters` relative to CWD (dev)
/// 2. `CARGO_MANIFEST_DIR/starters` (dev, compile-time path)
/// 3. Next to the binary (packaged installs)
/// 4. `~/.config/armadai/starters/` — extracted from embedded data on first use
pub fn starters_dir() -> PathBuf {
    let candidates = [
        // Dev: relative to CWD
        PathBuf::from("starters"),
        // Dev: relative to project root at compile time
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/starters")),
        // Installed: next to binary
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("starters")))
            .unwrap_or_default(),
    ];

    for c in &candidates {
        if c.is_dir() {
            return c.clone();
        }
    }

    // Fallback: extract embedded starters to config dir
    let config_starters = super::config::config_dir().join("starters");
    if !config_starters.is_dir() {
        let _ = EMBEDDED_STARTERS.extract(&config_starters);
    }
    config_starters
}

/// List all available starter pack names.
pub fn list_available_packs() -> Vec<String> {
    let dir = starters_dir();
    let mut packs = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return packs,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && path.join("pack.yaml").is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            packs.push(name.to_string());
        }
    }
    packs.sort();
    packs
}

/// Clap value parser that provides completion for available starter pack names.
pub fn pack_value_parser() -> clap::builder::PossibleValuesParser {
    let names = list_available_packs();
    // Leak strings to get 'static references needed by clap's PossibleValuesParser.
    // This is called once at startup, so the leak is negligible.
    let leaked: Vec<&'static str> = names
        .into_iter()
        .map(|s| &*Box::leak(s.into_boxed_str()))
        .collect();
    clap::builder::PossibleValuesParser::new(leaked)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_pack() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
name: test-pack
description: A test starter pack
agents:
  - code-reviewer
  - test-writer
prompts:
  - style-guide
skills:
  - my-skill
";
        std::fs::write(dir.path().join("pack.yaml"), yaml).unwrap();

        let pack = StarterPack::load(dir.path()).unwrap();
        assert_eq!(pack.name, "test-pack");
        assert_eq!(pack.description, "A test starter pack");
        assert_eq!(pack.agents, vec!["code-reviewer", "test-writer"]);
        assert_eq!(pack.prompts, vec!["style-guide"]);
        assert_eq!(pack.skills, vec!["my-skill"]);
    }

    #[test]
    fn test_load_pack_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(StarterPack::load(dir.path()).is_err());
    }

    #[test]
    fn test_load_pack_empty_lists() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
name: minimal
description: Minimal pack
";
        std::fs::write(dir.path().join("pack.yaml"), yaml).unwrap();

        let pack = StarterPack::load(dir.path()).unwrap();
        assert!(pack.agents.is_empty());
        assert!(pack.prompts.is_empty());
        assert!(pack.skills.is_empty());
    }

    #[test]
    fn test_install_pack() {
        let config_dir = tempfile::tempdir().unwrap();

        // Override config dir for test (single env-var test to avoid parallel races)
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", config_dir.path());
        }

        // --- Sub-test 1: full install with agents, prompts, skills ---
        {
            let pack_dir = tempfile::tempdir().unwrap();
            let yaml = "\
name: test
description: Test pack
agents:
  - my-agent
prompts:
  - my-prompt
skills:
  - my-skill
  - builtin-only
";
            std::fs::write(pack_dir.path().join("pack.yaml"), yaml).unwrap();

            let agents_dir = pack_dir.path().join("agents");
            std::fs::create_dir_all(&agents_dir).unwrap();
            std::fs::write(agents_dir.join("my-agent.md"), "# My Agent\n").unwrap();

            let prompts_dir = pack_dir.path().join("prompts");
            std::fs::create_dir_all(&prompts_dir).unwrap();
            std::fs::write(prompts_dir.join("my-prompt.md"), "# My Prompt\n").unwrap();

            // Create a bundled skill directory with a subdirectory
            let skill_dir = pack_dir.path().join("skills").join("my-skill");
            std::fs::create_dir_all(skill_dir.join("references")).unwrap();
            std::fs::write(skill_dir.join("SKILL.md"), "# My Skill\n").unwrap();
            std::fs::write(skill_dir.join("references").join("api.md"), "# API\n").unwrap();
            // "builtin-only" is NOT bundled in the pack — should be silently skipped

            let pack = StarterPack::load(pack_dir.path()).unwrap();
            let (agents, prompts, skills) = pack.install(pack_dir.path(), false).unwrap();
            assert_eq!(agents, 1);
            assert_eq!(prompts, 1);
            assert_eq!(skills, 1); // only my-skill installed, builtin-only skipped

            // Verify files exist
            assert!(config_dir.path().join("agents/my-agent.md").exists());
            assert!(config_dir.path().join("prompts/my-prompt.md").exists());
            assert!(config_dir.path().join("skills/my-skill/SKILL.md").exists());
            // Verify recursive copy of subdirectory
            assert!(
                config_dir
                    .path()
                    .join("skills/my-skill/references/api.md")
                    .exists()
            );

            // Second install without force should skip
            let (agents2, prompts2, skills2) = pack.install(pack_dir.path(), false).unwrap();
            assert_eq!(agents2, 0);
            assert_eq!(prompts2, 0);
            assert_eq!(skills2, 0);

            // With force should overwrite
            let (agents3, prompts3, skills3) = pack.install(pack_dir.path(), true).unwrap();
            assert_eq!(agents3, 1);
            assert_eq!(prompts3, 1);
            assert_eq!(skills3, 1);
        }

        // --- Sub-test 2: skills not bundled in pack are silently skipped ---
        {
            let pack_dir = tempfile::tempdir().unwrap();
            let yaml = "\
name: ref-only
description: Pack with skill references only
skills:
  - armadai-agent-authoring
  - armadai-prompt-authoring
";
            std::fs::write(pack_dir.path().join("pack.yaml"), yaml).unwrap();

            let pack = StarterPack::load(pack_dir.path()).unwrap();
            let (_, _, skills) = pack.install(pack_dir.path(), false).unwrap();
            assert_eq!(skills, 0);
        }

        unsafe {
            std::env::remove_var("ARMADAI_CONFIG_DIR");
        }
    }

    #[test]
    fn test_list_available_packs_includes_analysis() {
        let names = list_available_packs();
        assert!(
            names.contains(&"code-analysis-rust".to_string()),
            "Expected code-analysis-rust in {names:?}"
        );
        assert!(
            names.contains(&"code-analysis-web".to_string()),
            "Expected code-analysis-web in {names:?}"
        );
    }
}
