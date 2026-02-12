use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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

pub fn user_agents_dir() -> PathBuf {
    config_dir().join("agents")
}

pub fn user_prompts_dir() -> PathBuf {
    config_dir().join("prompts")
}

pub fn user_skills_dir() -> PathBuf {
    config_dir().join("skills")
}

pub fn user_fleets_dir() -> PathBuf {
    config_dir().join("fleets")
}

pub fn registry_cache_dir() -> PathBuf {
    config_dir().join("registry")
}

pub fn config_file_path() -> PathBuf {
    config_dir().join("config.yaml")
}

pub fn providers_file_path() -> PathBuf {
    config_dir().join("providers.yaml")
}

/// Create the full directory tree under the config root.
pub fn ensure_config_dirs() -> anyhow::Result<()> {
    let dirs = [
        config_dir(),
        user_agents_dir(),
        user_prompts_dir(),
        user_skills_dir(),
        user_fleets_dir(),
        registry_cache_dir(),
    ];
    for dir in &dirs {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// UserConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UserConfig {
    pub defaults: DefaultsConfig,
    pub storage: StorageConfig,
    pub rate_limits: HashMap<String, u32>,
    pub costs: CostsConfig,
    pub logging: LoggingConfig,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            defaults: DefaultsConfig::default(),
            storage: StorageConfig::default(),
            rate_limits: [
                ("anthropic".to_string(), 50),
                ("openai".to_string(), 60),
                ("google".to_string(), 60),
                ("proxy".to_string(), 100),
            ]
            .into_iter()
            .collect(),
            costs: CostsConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DefaultsConfig {
    pub provider: String,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub timeout: u64,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            temperature: 0.7,
            max_tokens: 4096,
            timeout: 120,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub mode: String,
    pub path: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            mode: "embedded".to_string(),
            path: "data/armadai.db".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CostsConfig {
    pub enabled: bool,
    pub daily_alert: f64,
}

impl Default for CostsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            daily_alert: 10.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub output: String,
    pub file_path: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            output: "stdout".to_string(),
            file_path: "logs/armadai.log".to_string(),
        }
    }
}

/// Load user config from `~/.config/armadai/config.yaml`.
/// Returns defaults if the file doesn't exist.
pub fn load_user_config() -> UserConfig {
    let path = config_file_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_yml::from_str(&content).unwrap_or_default(),
        Err(_) => UserConfig::default(),
    }
}

/// Apply environment variable overrides on top of a loaded config.
pub fn with_env_overrides(mut config: UserConfig) -> UserConfig {
    if let Ok(val) = std::env::var("ARMADAI_PROVIDER") {
        config.defaults.provider = val;
    }
    if let Ok(val) = std::env::var("ARMADAI_MODEL") {
        config.defaults.model = val;
    }
    if let Ok(val) = std::env::var("ARMADAI_TEMPERATURE")
        && let Ok(t) = val.parse::<f32>()
    {
        config.defaults.temperature = t;
    }
    config
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
        && let Ok(cfg) = serde_yml::from_str(&content)
    {
        return cfg;
    }
    // Fallback to project-local
    let local_path = Path::new("config/providers.yaml");
    if let Ok(content) = std::fs::read_to_string(local_path)
        && let Ok(cfg) = serde_yml::from_str(&content)
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
    /// Resolve paths: prefer project-local directories, fall back to global.
    pub fn resolve() -> Self {
        let local_agents = Path::new("agents");
        let local_templates = Path::new("templates");
        let local_config = Path::new("config");

        Self {
            agents_dir: if local_agents.exists() {
                local_agents.to_path_buf()
            } else {
                user_agents_dir()
            },
            templates_dir: if local_templates.exists() {
                local_templates.to_path_buf()
            } else {
                config_dir().join("templates")
            },
            config_dir: if local_config.exists() {
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

defaults:
  provider: anthropic
  model: claude-sonnet-4-5-20250929
  temperature: 0.7
  max_tokens: 4096
  timeout: 120

storage:
  mode: embedded
  path: data/armadai.db

rate_limits:
  anthropic: 50
  openai: 60
  google: 60
  proxy: 100

costs:
  enabled: true
  daily_alert: 10.0

logging:
  level: info
  output: stdout
  file_path: logs/armadai.log
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
        assert_eq!(cfg.defaults.provider, "anthropic");
        assert_eq!(cfg.defaults.model, "claude-sonnet-4-5-20250929");
        assert!((cfg.defaults.temperature - 0.7).abs() < f32::EPSILON);
        assert_eq!(cfg.defaults.max_tokens, 4096);
        assert_eq!(cfg.defaults.timeout, 120);
        assert_eq!(cfg.storage.mode, "embedded");
        assert!(cfg.costs.enabled);
        assert!((cfg.costs.daily_alert - 10.0).abs() < f64::EPSILON);
        assert_eq!(cfg.logging.level, "info");
    }

    #[test]
    fn test_partial_deserialize() {
        let yaml = "defaults:\n  provider: openai\n";
        let cfg: UserConfig = serde_yml::from_str(yaml).unwrap();
        assert_eq!(cfg.defaults.provider, "openai");
        // Other fields keep defaults
        assert_eq!(cfg.defaults.max_tokens, 4096);
        assert_eq!(cfg.storage.mode, "embedded");
    }

    #[test]
    fn test_empty_deserialize() {
        let yaml = "";
        let cfg: UserConfig = serde_yml::from_str(yaml).unwrap_or_default();
        assert_eq!(cfg.defaults.provider, "anthropic");
    }

    #[test]
    fn test_config_dir_respects_env() {
        // Save and restore env
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", "/tmp/test-armadai-config");
        }
        assert_eq!(config_dir(), PathBuf::from("/tmp/test-armadai-config"));
        // Restore
        match orig {
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
        let cfg: ProvidersConfig = serde_yml::from_str(yaml).unwrap();
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
    fn test_env_overrides() {
        let cfg = UserConfig::default();
        // Save originals
        let orig_provider = std::env::var("ARMADAI_PROVIDER").ok();
        let orig_model = std::env::var("ARMADAI_MODEL").ok();
        let orig_temp = std::env::var("ARMADAI_TEMPERATURE").ok();

        unsafe {
            std::env::set_var("ARMADAI_PROVIDER", "openai");
            std::env::set_var("ARMADAI_MODEL", "gpt-4o");
            std::env::set_var("ARMADAI_TEMPERATURE", "0.3");
        }

        let cfg = with_env_overrides(cfg);
        assert_eq!(cfg.defaults.provider, "openai");
        assert_eq!(cfg.defaults.model, "gpt-4o");
        assert!((cfg.defaults.temperature - 0.3).abs() < f32::EPSILON);

        // Restore
        unsafe {
            for (var, orig) in [
                ("ARMADAI_PROVIDER", orig_provider),
                ("ARMADAI_MODEL", orig_model),
                ("ARMADAI_TEMPERATURE", orig_temp),
            ] {
                match orig {
                    Some(v) => std::env::set_var(var, v),
                    None => std::env::remove_var(var),
                }
            }
        }
    }

    #[test]
    fn test_user_dirs() {
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", "/tmp/armadai-test");
        }
        assert_eq!(user_agents_dir(), PathBuf::from("/tmp/armadai-test/agents"));
        assert_eq!(user_fleets_dir(), PathBuf::from("/tmp/armadai-test/fleets"));
        assert_eq!(
            registry_cache_dir(),
            PathBuf::from("/tmp/armadai-test/registry")
        );
        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }
}
