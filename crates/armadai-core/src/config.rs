use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Global mode flag
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// XDG path helpers
// ---------------------------------------------------------------------------

/// Return the ArmadAI config root directory.
///
/// Resolution order:
/// 1. `$ARMADAI_CONFIG_DIR`
/// 2. `$XDG_CONFIG_HOME/armadai`
/// 3. `$HOME/.config/armadai`
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ARMADAI_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    let config_base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        });
    config_base.join("armadai")
}

/// Filename of the shared models.dev catalogue cache under the config dir.
/// Written by `armadai-providers`' model_registry; read (for IDs) by
/// `core::model_resolution`. Single source of truth for the two readers.
pub const MODELS_CACHE_FILE: &str = "models-cache.json";

pub fn user_agents_dir() -> PathBuf {
    config_dir().join("agents")
}

pub fn user_prompts_dir() -> PathBuf {
    config_dir().join("prompts")
}

pub fn user_skills_dir() -> PathBuf {
    config_dir().join("skills")
}

pub fn user_starters_dir() -> PathBuf {
    config_dir().join("starters")
}

pub fn registry_cache_dir() -> PathBuf {
    config_dir().join("registry")
}

pub fn skills_registry_dir() -> PathBuf {
    config_dir().join("skills-registry")
}

pub fn config_file_path() -> PathBuf {
    config_dir().join("config.yaml")
}

pub fn providers_file_path() -> PathBuf {
    config_dir().join("providers.yaml")
}

/// Path to the user-level custom registries config
/// (`~/.config/armadai/registries.yaml`). See `crate::registries`.
pub fn registries_config_path() -> PathBuf {
    config_dir().join("registries.yaml")
}

/// Create the full directory tree under the config root.
pub fn ensure_config_dirs() -> anyhow::Result<()> {
    // no agents/ nor prompts/ library: only `migrate` reads one, for a legacy
    // project that already has it
    let dirs = [config_dir(), user_skills_dir(), registry_cache_dir()];
    for dir in &dirs {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// UserConfig
// ---------------------------------------------------------------------------

/// `~/.config/armadai/config.yaml`. Only `starters_dirs` is read (by the
/// starter-pack lookup and `armadai config starters-dir`); keys written by
/// earlier versions (`defaults`, `storage`, `rate_limits`, `costs`,
/// `logging`) are ignored on load, not rejected.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UserConfig {
    pub starters_dirs: Vec<String>,
}

/// Load user config from `~/.config/armadai/config.yaml`.
/// Returns defaults if the file doesn't exist.
pub fn load_user_config() -> UserConfig {
    let path = config_file_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => armadai_model::yaml::from_str(&content).unwrap_or_default(),
        Err(_) => UserConfig::default(),
    }
}

/// Save user config back to `~/.config/armadai/config.yaml`.
pub fn save_user_config(config: &UserConfig) -> anyhow::Result<()> {
    let path = config_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let yaml = armadai_model::yaml::to_string(config)?;
    std::fs::write(&path, yaml)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// ProvidersConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProvidersConfig {
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProviderConfig {
    pub base_url: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
}

/// Load providers config from `~/.config/armadai/providers.yaml`,
/// falling back to project-local `config/providers.yaml`, then defaults.
pub fn load_providers_config() -> ProvidersConfig {
    // Try global config first
    let global_path = providers_file_path();
    if let Ok(content) = std::fs::read_to_string(&global_path)
        && let Ok(cfg) = armadai_model::yaml::from_str(&content)
    {
        return cfg;
    }
    // Fallback to project-local
    let local_path = Path::new("config/providers.yaml");
    if let Ok(content) = std::fs::read_to_string(local_path)
        && let Ok(cfg) = armadai_model::yaml::from_str(&content)
    {
        return cfg;
    }
    ProvidersConfig::default()
}

// ---------------------------------------------------------------------------
// AppPaths — resolved paths for the current context
// ---------------------------------------------------------------------------

/// Resolved paths with project-local → global fallback.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub agents_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub config_dir: PathBuf,
}

impl AppPaths {
    /// Resolve paths: prefer `.armadai/` → project-local → global.
    pub fn resolve() -> Self {
        let dotarmadai_agents = Path::new(".armadai/agents");
        let local_agents = Path::new("agents");
        let local_templates = Path::new("templates");
        let dotarmadai_config = Path::new(".armadai");
        let local_config = Path::new("config");

        Self {
            agents_dir: if dotarmadai_agents.exists() {
                dotarmadai_agents.to_path_buf()
            } else if local_agents.exists() {
                local_agents.to_path_buf()
            } else {
                user_agents_dir()
            },
            templates_dir: if local_templates.exists() {
                local_templates.to_path_buf()
            } else {
                config_dir().join("templates")
            },
            config_dir: if dotarmadai_config.exists() {
                dotarmadai_config.to_path_buf()
            } else if local_config.exists() {
                local_config.to_path_buf()
            } else {
                config_dir()
            },
        }
    }
}

/// Check if a project-local `config/settings.yaml` exists and print a
/// migration hint to stderr.
pub fn check_migration_hint() {
    let legacy = Path::new("config/settings.yaml");
    if legacy.exists() {
        eprintln!(
            "hint: local config/settings.yaml detected. \
             Consider running `armadai init` to migrate to ~/.config/armadai/"
        );
    }
}

// ---------------------------------------------------------------------------
// Default config file contents (for `armadai init`)
// ---------------------------------------------------------------------------

pub const DEFAULT_CONFIG_YAML: &str = "\
# ArmadAI user configuration

# Extra directories searched for starter packs (`armadai init --pack`),
# after the project and user ones. Manage with `armadai config starters-dir`.
# starters_dirs:
#   - /path/to/my/starters
";

pub const DEFAULT_PROVIDERS_YAML: &str = "\
# Provider configurations (non-sensitive)
# API keys go in environment variables or SOPS-encrypted files

providers:
  anthropic:
    base_url: https://api.anthropic.com/v1
    models:
      - claude-opus-4-6
      - claude-sonnet-4-5-20250929
      - claude-haiku-4-5-20251001

  openai:
    base_url: https://api.openai.com/v1
    models:
      - gpt-4o
      - gpt-4o-mini
      - o1
      - o3-mini

  google:
    base_url: https://generativelanguage.googleapis.com/v1beta
    models:
      - gemini-2.0-flash
      - gemini-2.0-pro

  proxy:
    base_url: http://localhost:4000/v1
    models: []
";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_user_config() {
        let cfg = UserConfig::default();
        assert!(cfg.starters_dirs.is_empty());
    }

    #[test]
    fn test_empty_deserialize() {
        let yaml = "";
        let cfg: UserConfig = armadai_model::yaml::from_str(yaml).unwrap_or_default();
        assert!(cfg.starters_dirs.is_empty());
    }

    /// Config files written by earlier versions carry keys nothing reads any
    /// more (`config.yaml`: `defaults`, `storage`, `rate_limits`, `costs`,
    /// `logging`; `armadai.yaml`: `shell`). They must still parse — and keep
    /// the keys that are read — rather than fail: `load_user_config` falls
    /// back to defaults on a parse error, which would silently drop
    /// `starters_dirs`, and an unparsable project config is skipped whole.
    #[test]
    fn old_style_config_files_still_parse() {
        let user_yaml = "\
defaults:
  provider: anthropic
  model: claude-sonnet-4-5-20250929
  temperature: 0.7
storage:
  mode: embedded
  path: /tmp/armadai.sqlite
rate_limits:
  anthropic: 50
costs:
  enabled: true
  daily_alert: 10.0
logging:
  level: info
starters_dirs:
  - /opt/starters
";
        let user: UserConfig = armadai_model::yaml::from_str(user_yaml)
            .expect("an old-style config.yaml must still parse");
        assert_eq!(user.starters_dirs, vec!["/opt/starters".to_string()]);

        let project_yaml = "\
agents:
  - name: reviewer
shell:
  default_provider: claude
  timeout: 60
  tandem:
    - provider: claude
  pipeline:
    steps:
      - name: review
        providers:
          - provider: claude
";
        let project: crate::project::ProjectConfig = armadai_model::yaml::from_str(project_yaml)
            .expect("an old-style armadai.yaml with `shell:` must still parse");
        assert_eq!(project.agents.len(), 1);
    }

    #[test]
    fn test_config_dir_respects_env() {
        let _guard = crate::test_support::env_lock();
        // Save and restore env
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: This test modifies the global environment which is unsafe in Rust 2024.
        // Serialised via `env_lock()` to avoid data races with other tests.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", "/tmp/test-armadai-config");
        }
        assert_eq!(config_dir(), PathBuf::from("/tmp/test-armadai-config"));
        // Restore
        match orig {
            // SAFETY: Restoring original env state at end of test scope.
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }

    #[test]
    fn test_providers_config_deserialize() {
        let yaml = r#"
providers:
  anthropic:
    base_url: https://api.anthropic.com/v1
    models:
      - claude-sonnet-4-5-20250929
  openai:
    base_url: https://api.openai.com/v1
    models:
      - gpt-4o
"#;
        let cfg: ProvidersConfig = armadai_model::yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.providers.len(), 2);
        assert!(cfg.providers.contains_key("anthropic"));
        let anthropic = &cfg.providers["anthropic"];
        assert_eq!(
            anthropic.base_url.as_deref(),
            Some("https://api.anthropic.com/v1")
        );
        assert_eq!(anthropic.models.len(), 1);
    }

    #[test]
    fn test_user_dirs() {
        let _guard = crate::test_support::env_lock();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        // SAFETY: This test modifies the global environment which is unsafe in Rust 2024.
        // Serialised via `env_lock()` to avoid data races with other tests.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", "/tmp/armadai-test");
        }
        assert_eq!(user_agents_dir(), PathBuf::from("/tmp/armadai-test/agents"));
        assert_eq!(
            registry_cache_dir(),
            PathBuf::from("/tmp/armadai-test/registry")
        );
        match orig {
            // SAFETY: Restoring original env state at end of test scope.
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }
}
