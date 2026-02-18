use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::agent::AgentMode;
use super::config::{registry_cache_dir, user_agents_dir, user_prompts_dir, user_skills_dir};
use super::fleet::FleetDefinition;

// ---------------------------------------------------------------------------
// Project config file names
// ---------------------------------------------------------------------------

const PROJECT_FILENAMES: &[&str] = &["armadai.yaml", "armadai.yml"];

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Default settings applied to all agents in the project.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProjectDefaults {
    pub mode: Option<AgentMode>,
}

/// Project-level configuration declared in `armadai.yaml`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProjectConfig {
    pub agents: Vec<AgentRef>,
    pub prompts: Vec<PromptRef>,
    pub skills: Vec<SkillRef>,
    pub sources: Vec<String>,
    pub link: Option<LinkConfig>,
    #[serde(default)]
    pub defaults: ProjectDefaults,
}

/// Reference to an agent — resolved at runtime.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AgentRef {
    Named { name: String },
    Registry { registry: String },
    Path { path: PathBuf },
}

/// Reference to a prompt fragment.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PromptRef {
    Named { name: String },
    Path { path: PathBuf },
}

/// Reference to a skill directory/file.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SkillRef {
    Named { name: String },
    Path { path: PathBuf },
}

/// Linker configuration section.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct LinkConfig {
    pub target: Option<String>,
    pub coordinator: Option<String>,
    pub overrides: HashMap<String, LinkOverride>,
}

/// Per-target linker overrides.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct LinkOverride {
    pub output: Option<String>,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Intermediate struct used to detect the legacy fleet format.
/// If the YAML contains a `fleet` key, it's the old format.
#[derive(Deserialize)]
struct FormatProbe {
    fleet: Option<String>,
}

impl ProjectConfig {
    /// Load a project config from the given path.
    /// Supports both the new format and the legacy fleet format.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;

        // Detect format: if there's a `fleet:` key, it's the old format
        let probe: FormatProbe =
            serde_yaml_ng::from_str(&content).unwrap_or(FormatProbe { fleet: None });

        if probe.fleet.is_some() {
            tracing::warn!(
                "Legacy fleet format detected in {}. \
                 Migrate to the modern armadai.yaml format (see `armadai init --project`). \
                 Fleet support will be removed in a future release.",
                path.display()
            );
            let fleet: FleetDefinition = serde_yaml_ng::from_str(&content)?;
            Ok(Self::from_legacy_fleet(&fleet))
        } else {
            let config: ProjectConfig = serde_yaml_ng::from_str(&content)?;
            Ok(config)
        }
    }

    /// Convert a legacy `FleetDefinition` into a `ProjectConfig`.
    pub fn from_legacy_fleet(fleet: &FleetDefinition) -> Self {
        let agents = fleet
            .agents
            .iter()
            .map(|name| AgentRef::Named { name: name.clone() })
            .collect();

        Self {
            agents,
            prompts: Vec::new(),
            skills: Vec::new(),
            sources: Vec::new(),
            link: None,
            defaults: ProjectDefaults::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Walk-up search
// ---------------------------------------------------------------------------

/// Search for `armadai.yaml` (or `.yml`) by walking up from the given
/// directory. Stops at a `.git/` boundary or filesystem root.
///
/// Returns `(project_root, config)` where `project_root` is the directory
/// containing the config file.
pub fn find_project_config() -> Option<(PathBuf, ProjectConfig)> {
    let start = std::env::current_dir().ok()?;
    find_project_config_from(&start)
}

/// Testable version that takes an explicit start directory.
pub fn find_project_config_from(start: &Path) -> Option<(PathBuf, ProjectConfig)> {
    let mut dir = start.to_path_buf();
    loop {
        for filename in PROJECT_FILENAMES {
            let candidate = dir.join(filename);
            if candidate.is_file()
                && let Ok(config) = ProjectConfig::load(&candidate)
            {
                return Some((dir.clone(), config));
            }
        }

        // Stop at .git boundary
        if dir.join(".git").exists() {
            return None;
        }

        // Move up
        if !dir.pop() {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// Agent resolution
// ---------------------------------------------------------------------------

/// Resolve a single `AgentRef` to an absolute path.
pub fn resolve_agent(agent_ref: &AgentRef, project_root: &Path) -> anyhow::Result<PathBuf> {
    match agent_ref {
        AgentRef::Path { path } => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                project_root.join(path)
            };
            if resolved.exists() {
                Ok(resolved)
            } else {
                anyhow::bail!("Agent file not found: {}", resolved.display());
            }
        }
        AgentRef::Named { name } => {
            let filename = if name.ends_with(".md") {
                name.clone()
            } else {
                format!("{name}.md")
            };

            // 1. Project-local agents/
            let local = project_root.join("agents").join(&filename);
            if local.exists() {
                return Ok(local);
            }

            // 2. User library ~/.config/armadai/agents/
            let global = user_agents_dir().join(&filename);
            if global.exists() {
                return Ok(global);
            }

            anyhow::bail!(
                "Agent '{name}' not found in {} or {}",
                local.display(),
                global.display()
            );
        }
        AgentRef::Registry { registry } => {
            let filename = if registry.ends_with(".md") {
                registry.clone()
            } else {
                format!("{registry}.md")
            };
            let path = registry_cache_dir().join(&filename);
            if path.exists() {
                Ok(path)
            } else {
                anyhow::bail!(
                    "Registry agent '{registry}' not found at {}. Run `armadai sync` first.",
                    path.display()
                );
            }
        }
    }
}

/// Resolve all agent refs in the config, collecting paths.
/// Returns resolved paths and skipped refs (with error messages).
pub fn resolve_all_agents(
    config: &ProjectConfig,
    project_root: &Path,
) -> (Vec<PathBuf>, Vec<String>) {
    let mut resolved = Vec::new();
    let mut errors = Vec::new();

    for agent_ref in &config.agents {
        match resolve_agent(agent_ref, project_root) {
            Ok(path) => resolved.push(path),
            Err(e) => errors.push(format!("{e}")),
        }
    }

    (resolved, errors)
}

// ---------------------------------------------------------------------------
// Prompt resolution
// ---------------------------------------------------------------------------

/// Resolve a single `PromptRef` to an absolute path.
pub fn resolve_prompt(prompt_ref: &PromptRef, project_root: &Path) -> anyhow::Result<PathBuf> {
    match prompt_ref {
        PromptRef::Path { path } => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                project_root.join(path)
            };
            if resolved.exists() {
                Ok(resolved)
            } else {
                anyhow::bail!("Prompt file not found: {}", resolved.display());
            }
        }
        PromptRef::Named { name } => {
            let filename = if name.ends_with(".md") {
                name.clone()
            } else {
                format!("{name}.md")
            };

            // 1. Project-local prompts/
            let local = project_root.join("prompts").join(&filename);
            if local.exists() {
                return Ok(local);
            }

            // 2. User library ~/.config/armadai/prompts/
            let global = user_prompts_dir().join(&filename);
            if global.exists() {
                return Ok(global);
            }

            anyhow::bail!(
                "Prompt '{name}' not found in {} or {}",
                local.display(),
                global.display()
            );
        }
    }
}

/// Resolve all prompt refs in the config, collecting paths.
/// Returns resolved paths and skipped refs (with error messages).
pub fn resolve_all_prompts(
    config: &ProjectConfig,
    project_root: &Path,
) -> (Vec<PathBuf>, Vec<String>) {
    let mut resolved = Vec::new();
    let mut errors = Vec::new();

    for prompt_ref in &config.prompts {
        match resolve_prompt(prompt_ref, project_root) {
            Ok(path) => resolved.push(path),
            Err(e) => errors.push(format!("{e}")),
        }
    }

    (resolved, errors)
}

// ---------------------------------------------------------------------------
// Skill resolution
// ---------------------------------------------------------------------------

/// Resolve a single `SkillRef` to an absolute path (directory containing SKILL.md).
pub fn resolve_skill(skill_ref: &SkillRef, project_root: &Path) -> anyhow::Result<PathBuf> {
    match skill_ref {
        SkillRef::Path { path } => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                project_root.join(path)
            };
            if resolved.exists() {
                Ok(resolved)
            } else {
                anyhow::bail!("Skill path not found: {}", resolved.display());
            }
        }
        SkillRef::Named { name } => {
            // 1. Project-local skills/<name>/
            let local = project_root.join("skills").join(name);
            if local.join("SKILL.md").exists() {
                return Ok(local);
            }

            // 2. User library ~/.config/armadai/skills/<name>/
            let global = user_skills_dir().join(name);
            if global.join("SKILL.md").exists() {
                return Ok(global);
            }

            anyhow::bail!(
                "Skill '{name}' not found in {} or {}",
                local.display(),
                global.display()
            );
        }
    }
}

/// Resolve all skill refs in the config, collecting paths.
/// Returns resolved paths and skipped refs (with error messages).
pub fn resolve_all_skills(
    config: &ProjectConfig,
    project_root: &Path,
) -> (Vec<PathBuf>, Vec<String>) {
    let mut resolved = Vec::new();
    let mut errors = Vec::new();

    for skill_ref in &config.skills {
        match resolve_skill(skill_ref, project_root) {
            Ok(path) => resolved.push(path),
            Err(e) => errors.push(format!("{e}")),
        }
    }

    (resolved, errors)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_ref_named() {
        let yaml = "- name: code-reviewer\n";
        let refs: Vec<AgentRef> = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            refs[0],
            AgentRef::Named {
                name: "code-reviewer".to_string()
            }
        );
    }

    #[test]
    fn test_agent_ref_registry() {
        let yaml = "- registry: official/security\n";
        let refs: Vec<AgentRef> = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            refs[0],
            AgentRef::Registry {
                registry: "official/security".to_string()
            }
        );
    }

    #[test]
    fn test_agent_ref_path() {
        let yaml = "- path: .armadai/agents/team.md\n";
        let refs: Vec<AgentRef> = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            refs[0],
            AgentRef::Path {
                path: PathBuf::from(".armadai/agents/team.md")
            }
        );
    }

    #[test]
    fn test_full_config_deserialize() {
        let yaml = r#"
agents:
  - name: code-reviewer
  - registry: official/security
  - path: .armadai/agents/team.md

prompts:
  - name: rust-conventions
  - path: .armadai/prompts/style.md

skills:
  - name: docker-compose
  - path: .armadai/skills/deploy/

sources:
  - docs/architecture.md
  - CONTRIBUTING.md

defaults:
  mode: guided

link:
  target: claude
  overrides:
    claude:
      output: .claude/
    copilot:
      output: .github/agents/
"#;
        let config: ProjectConfig = serde_yaml_ng::from_str(yaml).unwrap();

        assert_eq!(config.agents.len(), 3);
        assert_eq!(
            config.agents[0],
            AgentRef::Named {
                name: "code-reviewer".to_string()
            }
        );
        assert_eq!(
            config.agents[1],
            AgentRef::Registry {
                registry: "official/security".to_string()
            }
        );
        assert_eq!(
            config.agents[2],
            AgentRef::Path {
                path: PathBuf::from(".armadai/agents/team.md")
            }
        );

        assert_eq!(config.prompts.len(), 2);
        assert_eq!(
            config.prompts[0],
            PromptRef::Named {
                name: "rust-conventions".to_string()
            }
        );
        assert_eq!(
            config.prompts[1],
            PromptRef::Path {
                path: PathBuf::from(".armadai/prompts/style.md")
            }
        );

        assert_eq!(config.skills.len(), 2);
        assert_eq!(config.sources.len(), 2);
        assert_eq!(config.defaults.mode, Some(AgentMode::Guided));

        let link = config.link.unwrap();
        assert_eq!(link.target.as_deref(), Some("claude"));
        assert_eq!(link.overrides.len(), 2);
        assert_eq!(link.overrides["claude"].output.as_deref(), Some(".claude/"));
    }

    #[test]
    fn test_defaults_mode_parsing() {
        let yaml = "defaults:\n  mode: guided\n";
        let config: ProjectConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.defaults.mode, Some(AgentMode::Guided));
    }

    #[test]
    fn test_defaults_absent_gives_default() {
        let yaml = "agents:\n  - name: my-agent\n";
        let config: ProjectConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.defaults.mode.is_none());
    }

    #[test]
    fn test_partial_config_deserialize() {
        let yaml = "agents:\n  - name: my-agent\n";
        let config: ProjectConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.agents.len(), 1);
        assert!(config.prompts.is_empty());
        assert!(config.skills.is_empty());
        assert!(config.sources.is_empty());
        assert!(config.link.is_none());
    }

    #[test]
    fn test_empty_config_deserialize() {
        let yaml = "";
        let config: ProjectConfig = serde_yaml_ng::from_str(yaml).unwrap_or_default();
        assert!(config.agents.is_empty());
        assert!(config.prompts.is_empty());
    }

    #[test]
    fn test_from_legacy_fleet() {
        let fleet = FleetDefinition {
            fleet: "my-fleet".to_string(),
            agents: vec!["code-reviewer".to_string(), "test-writer".to_string()],
            source: PathBuf::from("/home/user/armadai"),
        };

        let config = ProjectConfig::from_legacy_fleet(&fleet);
        assert_eq!(config.agents.len(), 2);
        assert_eq!(
            config.agents[0],
            AgentRef::Named {
                name: "code-reviewer".to_string()
            }
        );
        assert_eq!(
            config.agents[1],
            AgentRef::Named {
                name: "test-writer".to_string()
            }
        );
        assert!(config.prompts.is_empty());
        assert!(config.link.is_none());
    }

    #[test]
    fn test_legacy_format_detection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("armadai.yaml");

        let legacy_yaml = "\
fleet: my-fleet
agents:
  - code-reviewer
  - test-writer
source: /home/user/armadai
";
        std::fs::write(&path, legacy_yaml).unwrap();

        let config = ProjectConfig::load(&path).unwrap();
        assert_eq!(config.agents.len(), 2);
        assert_eq!(
            config.agents[0],
            AgentRef::Named {
                name: "code-reviewer".to_string()
            }
        );
    }

    #[test]
    fn test_new_format_loading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("armadai.yaml");

        let yaml = "\
agents:
  - name: my-agent
  - path: custom/agent.md
sources:
  - README.md
";
        std::fs::write(&path, yaml).unwrap();

        let config = ProjectConfig::load(&path).unwrap();
        assert_eq!(config.agents.len(), 2);
        assert_eq!(
            config.agents[0],
            AgentRef::Named {
                name: "my-agent".to_string()
            }
        );
        assert_eq!(
            config.agents[1],
            AgentRef::Path {
                path: PathBuf::from("custom/agent.md")
            }
        );
        assert_eq!(config.sources, vec!["README.md"]);
    }

    #[test]
    fn test_find_project_config_walk_up() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("sub").join("deep");
        std::fs::create_dir_all(&sub).unwrap();

        // Create armadai.yaml at root
        let yaml = "agents:\n  - name: test-agent\n";
        std::fs::write(root.path().join("armadai.yaml"), yaml).unwrap();

        // Create .git to act as boundary (at root)
        std::fs::create_dir(root.path().join(".git")).unwrap();

        // Search from sub/deep/ should find root config
        let result = find_project_config_from(&sub);
        assert!(result.is_some());
        let (found_root, config) = result.unwrap();
        assert_eq!(found_root, root.path().to_path_buf());
        assert_eq!(config.agents.len(), 1);
    }

    #[test]
    fn test_find_project_config_stops_at_git() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        // Create .git in sub — this is the boundary
        std::fs::create_dir(sub.join(".git")).unwrap();

        // Config is above the .git boundary (at root)
        let yaml = "agents:\n  - name: test-agent\n";
        std::fs::write(root.path().join("armadai.yaml"), yaml).unwrap();

        // Search from sub/ should NOT find root config (stopped by .git)
        let result = find_project_config_from(&sub);
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_agent_path() {
        let dir = tempfile::tempdir().unwrap();
        let agent_path = dir.path().join("custom").join("agent.md");
        std::fs::create_dir_all(agent_path.parent().unwrap()).unwrap();
        std::fs::write(&agent_path, "# Agent\n").unwrap();

        let agent_ref = AgentRef::Path {
            path: PathBuf::from("custom/agent.md"),
        };
        let resolved = resolve_agent(&agent_ref, dir.path()).unwrap();
        assert_eq!(resolved, agent_path);
    }

    #[test]
    fn test_resolve_agent_named_local() {
        let dir = tempfile::tempdir().unwrap();
        let agents_dir = dir.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("my-agent.md"), "# Agent\n").unwrap();

        let agent_ref = AgentRef::Named {
            name: "my-agent".to_string(),
        };
        let resolved = resolve_agent(&agent_ref, dir.path()).unwrap();
        assert_eq!(resolved, agents_dir.join("my-agent.md"));
    }

    #[test]
    fn test_resolve_agent_named_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let agent_ref = AgentRef::Named {
            name: "nonexistent".to_string(),
        };
        assert!(resolve_agent(&agent_ref, dir.path()).is_err());
    }

    #[test]
    fn test_resolve_all_agents() {
        let dir = tempfile::tempdir().unwrap();
        let agents_dir = dir.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("found.md"), "# Agent\n").unwrap();

        let config = ProjectConfig {
            agents: vec![
                AgentRef::Named {
                    name: "found".to_string(),
                },
                AgentRef::Named {
                    name: "missing".to_string(),
                },
            ],
            ..Default::default()
        };

        let (resolved, errors) = resolve_all_agents(&config, dir.path());
        assert_eq!(resolved.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing"));
    }

    #[test]
    fn test_resolve_prompt_path() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("custom").join("style.md");
        std::fs::create_dir_all(prompt_path.parent().unwrap()).unwrap();
        std::fs::write(&prompt_path, "# Style\n").unwrap();

        let prompt_ref = PromptRef::Path {
            path: PathBuf::from("custom/style.md"),
        };
        let resolved = resolve_prompt(&prompt_ref, dir.path()).unwrap();
        assert_eq!(resolved, prompt_path);
    }

    #[test]
    fn test_resolve_prompt_named_local() {
        let dir = tempfile::tempdir().unwrap();
        let prompts_dir = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();
        std::fs::write(prompts_dir.join("rust-style.md"), "# Rust\n").unwrap();

        let prompt_ref = PromptRef::Named {
            name: "rust-style".to_string(),
        };
        let resolved = resolve_prompt(&prompt_ref, dir.path()).unwrap();
        assert_eq!(resolved, prompts_dir.join("rust-style.md"));
    }

    #[test]
    fn test_resolve_prompt_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_ref = PromptRef::Named {
            name: "nonexistent".to_string(),
        };
        assert!(resolve_prompt(&prompt_ref, dir.path()).is_err());
    }

    #[test]
    fn test_resolve_all_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let prompts_dir = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();
        std::fs::write(prompts_dir.join("found.md"), "# Found\n").unwrap();

        let config = ProjectConfig {
            prompts: vec![
                PromptRef::Named {
                    name: "found".to_string(),
                },
                PromptRef::Named {
                    name: "missing".to_string(),
                },
            ],
            ..Default::default()
        };

        let (resolved, errors) = resolve_all_prompts(&config, dir.path());
        assert_eq!(resolved.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing"));
    }

    #[test]
    fn test_resolve_skill_named_local() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("docker");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Docker\n").unwrap();

        let skill_ref = SkillRef::Named {
            name: "docker".to_string(),
        };
        let resolved = resolve_skill(&skill_ref, dir.path()).unwrap();
        assert_eq!(resolved, skill_dir);
    }

    #[test]
    fn test_resolve_skill_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let skill_ref = SkillRef::Named {
            name: "nonexistent".to_string(),
        };
        assert!(resolve_skill(&skill_ref, dir.path()).is_err());
    }

    #[test]
    fn test_resolve_all_skills() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("found");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Found\n").unwrap();

        let config = ProjectConfig {
            skills: vec![
                SkillRef::Named {
                    name: "found".to_string(),
                },
                SkillRef::Named {
                    name: "missing".to_string(),
                },
            ],
            ..Default::default()
        };

        let (resolved, errors) = resolve_all_skills(&config, dir.path());
        assert_eq!(resolved.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing"));
    }

    #[test]
    fn test_yml_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("armadai.yml");

        let yaml = "agents:\n  - name: yml-agent\n";
        std::fs::write(&path, yaml).unwrap();

        // Create .git to stop walk-up
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let result = find_project_config_from(dir.path());
        assert!(result.is_some());
        let (_, config) = result.unwrap();
        assert_eq!(config.agents.len(), 1);
    }
}
