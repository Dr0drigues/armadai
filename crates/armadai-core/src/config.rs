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
/// Written by earlier versions of ArmadAI (nothing writes it any more); read
/// (for IDs) by `core::model_resolution`.
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

/// Parse a user file: `Ok(None)` when it is missing, empty or only comments; an error
/// naming the file when it exists but does not read.
pub fn read_user_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => anyhow::bail!("cannot read {}: {e}", path.display()),
    };
    armadai_model::yaml::from_str::<Option<T>>(&content).map_err(|e| {
        anyhow::anyhow!(
            "{} does not read: {e}; fix it or move it away",
            path.display()
        )
    })
}

/// Print `msg` as a warning, once per process.
pub fn warn_once(msg: String) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.contains(&msg) {
        eprintln!("warning: {msg}");
        seen.push(msg);
    }
}

/// Load user config from `~/.config/armadai/config.yaml`, for a write: an existing file
/// that does not read is an error, never replaced by defaults.
pub fn try_load_user_config() -> anyhow::Result<UserConfig> {
    Ok(read_user_yaml(&config_file_path())?.unwrap_or_default())
}

/// Load user config for reading only: defaults if the file is missing, defaults and a
/// warning if it does not read.
pub fn load_user_config() -> UserConfig {
    try_load_user_config().unwrap_or_else(|e| {
        warn_once(format!("{e:#}; using no custom starter directory"));
        UserConfig::default()
    })
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
// AppPaths — resolved paths for the current context
// ---------------------------------------------------------------------------

/// Resolved paths with project-local → global fallback.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub agents_dir: PathBuf,
    pub templates_dir: PathBuf,
}

impl AppPaths {
    /// Resolve paths: prefer `.armadai/` → project-local → global.
    pub fn resolve() -> Self {
        let dotarmadai_agents = Path::new(".armadai/agents");
        let local_agents = Path::new("agents");
        let local_templates = Path::new("templates");

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
    fn a_user_file_reads_as_missing_when_absent_empty_or_comments_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        assert!(read_user_yaml::<UserConfig>(&path).unwrap().is_none());
        for text in ["", "# none yet\n"] {
            std::fs::write(&path, text).unwrap();
            assert!(
                read_user_yaml::<UserConfig>(&path).unwrap().is_none(),
                "{text:?}"
            );
        }
        std::fs::write(&path, "starters_dirs: []\nstarters_dirs: []\n").unwrap();
        let e = read_user_yaml::<UserConfig>(&path).unwrap_err().to_string();
        assert!(e.contains("config.yaml") && e.contains("line 2"), "{e}");
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
