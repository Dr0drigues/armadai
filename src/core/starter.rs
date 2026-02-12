use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use serde::Deserialize;

use super::config::{user_agents_dir, user_prompts_dir};

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

    /// Install the starter pack's agents and prompts to the user library.
    ///
    /// Copies agent `.md` files from `<pack_dir>/agents/` to `~/.config/armadai/agents/`
    /// and prompt `.md` files from `<pack_dir>/prompts/` to `~/.config/armadai/prompts/`.
    ///
    /// Returns `(installed_agents, installed_prompts)` counts.
    pub fn install(&self, pack_dir: &Path, force: bool) -> anyhow::Result<(usize, usize)> {
        let agents_src = pack_dir.join("agents");
        let prompts_src = pack_dir.join("prompts");
        let agents_dst = user_agents_dir();
        let prompts_dst = user_prompts_dir();

        std::fs::create_dir_all(&agents_dst)?;
        std::fs::create_dir_all(&prompts_dst)?;

        let mut agents_count = 0;
        let mut prompts_count = 0;

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

        Ok((agents_count, prompts_count))
    }
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
";
        std::fs::write(dir.path().join("pack.yaml"), yaml).unwrap();

        let pack = StarterPack::load(dir.path()).unwrap();
        assert_eq!(pack.name, "test-pack");
        assert_eq!(pack.description, "A test starter pack");
        assert_eq!(pack.agents, vec!["code-reviewer", "test-writer"]);
        assert_eq!(pack.prompts, vec!["style-guide"]);
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
    }

    #[test]
    fn test_install_pack() {
        let pack_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();

        // Create pack structure
        let yaml = "\
name: test
description: Test pack
agents:
  - my-agent
prompts:
  - my-prompt
";
        std::fs::write(pack_dir.path().join("pack.yaml"), yaml).unwrap();

        let agents_dir = pack_dir.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("my-agent.md"), "# My Agent\n").unwrap();

        let prompts_dir = pack_dir.path().join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();
        std::fs::write(prompts_dir.join("my-prompt.md"), "# My Prompt\n").unwrap();

        // Override config dir for test
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", config_dir.path());
        }

        let pack = StarterPack::load(pack_dir.path()).unwrap();
        let (agents, prompts) = pack.install(pack_dir.path(), false).unwrap();
        assert_eq!(agents, 1);
        assert_eq!(prompts, 1);

        // Verify files exist
        assert!(
            config_dir
                .path()
                .join("agents")
                .join("my-agent.md")
                .exists()
        );
        assert!(
            config_dir
                .path()
                .join("prompts")
                .join("my-prompt.md")
                .exists()
        );

        // Second install without force should skip
        let (agents2, prompts2) = pack.install(pack_dir.path(), false).unwrap();
        assert_eq!(agents2, 0);
        assert_eq!(prompts2, 0);

        // With force should overwrite
        let (agents3, prompts3) = pack.install(pack_dir.path(), true).unwrap();
        assert_eq!(agents3, 1);
        assert_eq!(prompts3, 1);

        unsafe {
            std::env::remove_var("ARMADAI_CONFIG_DIR");
        }
    }
}
