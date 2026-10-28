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

/// `~/.config/armadai/config.yaml`. Only `starters_dirs` (starter-pack lookup,
/// `armadai config starters-dir`) and `model_tiers` (per-target model choice for
/// a tier, read by the compilers) are read; keys written by
/// earlier versions (`defaults`, `storage`, `rate_limits`, `costs`,
/// `logging`) are ignored on load, not rejected.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UserConfig {
    pub starters_dirs: Vec<String>,
    /// Target -> tier -> model id (`opencode: {fast: opencode/free-a}`), as written:
    /// it never fails the read of the file, the commands that use it check it
    /// ([`try_load_user_config_for`]).
    #[serde(skip_serializing_if = "is_null")]
    pub model_tiers: armadai_model::NativeValue,
}

fn is_null(v: &armadai_model::NativeValue) -> bool {
    v.is_null()
}

fn tier_name(tier: armadai_model::Tier) -> String {
    serde_json::to_value(tier)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The model the user chose for `tier` on `target`; a faulty entry is `None`.
pub fn model_for_tier(
    config: &UserConfig,
    target: &str,
    tier: armadai_model::Tier,
) -> Option<String> {
    config
        .model_tiers
        .get(target)?
        .get(tier_name(tier))?
        .as_str()
        .map(str::to_owned)
}

/// Every problem under `model_tiers` as `(path, reason)`, e.g.
/// `("model_tiers.opencode.medium", "is not a tier (fast, pro, max)")`. The name of a
/// target is not checked.
pub fn model_tier_problems(config: &UserConfig) -> Vec<(String, String)> {
    use armadai_model::Tier;
    use serde_json::Value;
    let tiers = [Tier::Fast, Tier::Pro, Tier::Max].map(tier_name);
    let mut out = Vec::new();
    let top = match &*config.model_tiers {
        Value::Null => return out,
        Value::Object(m) => m,
        _ => {
            out.push(("model_tiers".into(), "must map targets to tiers".into()));
            return out;
        }
    };
    for (target, v) in top {
        let path = format!("model_tiers.{target}");
        let Value::Object(m) = v else {
            out.push((path, "must map tiers to model ids".into()));
            continue;
        };
        for (tier, id) in m {
            let path = format!("{path}.{tier}");
            if !tiers.contains(tier) {
                out.push((path, format!("is not a tier ({})", tiers.join(", "))));
            } else if !id.is_string() {
                out.push((path, "must be a model id (a string)".into()));
            }
        }
    }
    out
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

fn read_config() -> anyhow::Result<UserConfig> {
    Ok(read_user_yaml(&config_file_path())?.unwrap_or_default())
}

/// A problem under `model_tiers` that this read does not use, as a warning (once).
fn warn_model_tiers(path: &str, why: &str) {
    let refuses = if path == "model_tiers" || path.starts_with("model_tiers.opencode") {
        "; ignored here, but `armadai link --target opencode` refuses it"
    } else {
        "; ignored"
    };
    warn_once(format!(
        "{}: {path} {why}{refuses}",
        config_file_path().display()
    ));
}

/// Load user config from `~/.config/armadai/config.yaml`, for a write: an existing file
/// that does not read is an error, never replaced by defaults. A problem under
/// `model_tiers` is a warning, not an error: it does not hide the rest of the file.
pub fn try_load_user_config() -> anyhow::Result<UserConfig> {
    let config = read_config()?;
    for (path, why) in model_tier_problems(&config) {
        warn_model_tiers(&path, &why);
    }
    Ok(config)
}

/// Strict read for a command that uses `model_tiers.<target>`: a problem under it is an
/// error naming the key; one under another target stays a warning.
pub fn try_load_user_config_for(target: &str) -> anyhow::Result<UserConfig> {
    let config = read_config()?;
    let own = format!("model_tiers.{target}");
    for (path, why) in model_tier_problems(&config) {
        if path == "model_tiers" || path == own || path.starts_with(&format!("{own}.")) {
            anyhow::bail!(
                "{}: {path} {why}; fix it or remove it",
                config_file_path().display()
            );
        }
        warn_model_tiers(&path, &why);
    }
    Ok(config)
}

/// Load user config for reading only: defaults if the file is missing, defaults and a
/// warning if it does not read.
pub fn load_user_config() -> UserConfig {
    try_load_user_config().unwrap_or_else(|e| {
        warn_once(format!("{e:#}; using no custom starter directory"));
        UserConfig::default()
    })
}

// ---------------------------------------------------------------------------
// AppPaths — resolved paths for the current context
// ---------------------------------------------------------------------------

/// Resolved paths with project-local → global fallback.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub templates_dir: PathBuf,
}

impl AppPaths {
    /// Resolve paths: project-local `templates/`, else the user's.
    pub fn resolve() -> Self {
        let local_templates = Path::new("templates");

        Self {
            templates_dir: if local_templates.exists() {
                local_templates.to_path_buf()
            } else {
                config_dir().join("templates")
            },
        }
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
    fn model_tiers_are_read_per_target() {
        use armadai_model::Tier;
        let cfg: UserConfig = armadai_model::yaml::from_str(
            "model_tiers:\n  opencode:\n    fast: opencode/free-a\n    max: anthropic/claude-x\n",
        )
        .unwrap();
        assert_eq!(
            model_for_tier(&cfg, "opencode", Tier::Fast),
            Some("opencode/free-a".to_string())
        );
        assert_eq!(
            model_for_tier(&cfg, "opencode", Tier::Max),
            Some("anthropic/claude-x".to_string())
        );
        assert_eq!(model_for_tier(&cfg, "opencode", Tier::Pro), None);
        assert_eq!(model_for_tier(&cfg, "codex", Tier::Fast), None);
    }

    #[test]
    fn a_config_without_model_tiers_still_reads() {
        let cfg: UserConfig =
            armadai_model::yaml::from_str("starters_dirs:\n  - /opt/a\n").unwrap();
        assert_eq!(cfg.starters_dirs, vec!["/opt/a".to_string()]);
        assert!(cfg.model_tiers.is_null());
    }

    fn parse(text: &str) -> UserConfig {
        armadai_model::yaml::from_str(text).unwrap()
    }

    #[test]
    fn an_unknown_tier_does_not_hide_the_starter_dirs() {
        let cfg = parse(
            "starters_dirs: [/opt/a]\nmodel_tiers:\n  opencode:\n    medium: x\n    fast: m1\n",
        );
        assert_eq!(cfg.starters_dirs, vec!["/opt/a".to_string()]);
        assert_eq!(
            model_tier_problems(&cfg),
            vec![(
                "model_tiers.opencode.medium".to_string(),
                "is not a tier (fast, pro, max)".to_string()
            )]
        );
        assert_eq!(
            model_for_tier(&cfg, "opencode", armadai_model::Tier::Fast),
            Some("m1".to_string())
        );
    }

    #[test]
    fn each_shape_problem_is_named() {
        let p = |t: &str| model_tier_problems(&parse(t));
        let one = |path: &str, why: &str| vec![(path.to_string(), why.to_string())];
        assert_eq!(
            p("model_tiers: [a]\n"),
            one("model_tiers", "must map targets to tiers")
        );
        assert_eq!(
            p("model_tiers:\n  opencode: x\n"),
            one("model_tiers.opencode", "must map tiers to model ids")
        );
        assert_eq!(
            p("model_tiers:\n  opencode:\n    fast: 3\n"),
            one("model_tiers.opencode.fast", "must be a model id (a string)")
        );
        assert_eq!(p("model_tiers:\n  anything:\n    pro: m\n"), vec![]);
        assert_eq!(p("starters_dirs: []\n"), vec![]);
    }

    #[test]
    fn the_strict_read_fails_on_its_own_target_only() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        let file = iso.config_dir().join("config.yaml");
        std::fs::write(&file, "model_tiers:\n  opencode:\n    medium: x\n").unwrap();
        let e = try_load_user_config_for("opencode")
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            format!(
                "{}: model_tiers.opencode.medium is not a tier (fast, pro, max); fix it or remove it",
                file.display()
            )
        );
        assert!(try_load_user_config_for("codex").is_ok());
        assert!(try_load_user_config().is_ok());
        std::fs::write(&file, "model_tiers: 3\n").unwrap();
        assert!(try_load_user_config_for("codex").is_err());
    }

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
