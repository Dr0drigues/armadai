use std::path::Path;

use clap::Subcommand;

use crate::core::config as app_config;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show provider status, API keys, and environment variables
    Providers,
    /// Initialize or manage secrets (SOPS + age)
    #[command(long_about = "Initialize or manage secrets (SOPS + age).\n\n\
            Uses age for key generation and SOPS for encrypting provider API keys. \
            Secrets are stored in config/providers.sops.yaml.")]
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
    /// Manage custom starter pack directories
    #[command(name = "starters-dir")]
    StartersDir {
        #[command(subcommand)]
        action: StartersDirAction,
    },
}

#[derive(Subcommand)]
pub enum SecretsAction {
    /// Generate an age key and .sops.yaml, create encrypted secrets template
    Init,
    /// Decrypt secrets, generate a new age key, and re-encrypt
    Rotate,
}

#[derive(Subcommand)]
pub enum StartersDirAction {
    /// List all starter directories and their sources
    List,
    /// Add a custom starter directory to config.yaml
    Add {
        /// Path to the directory containing starter packs
        path: String,
    },
    /// Remove a custom starter directory from config.yaml
    Remove {
        /// Path to remove
        path: String,
    },
}

pub async fn execute(action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Providers => show_providers().await,
        ConfigAction::Secrets { action } => match action {
            SecretsAction::Init => secrets_init().await,
            SecretsAction::Rotate => secrets_rotate().await,
        },
        ConfigAction::StartersDir { action } => match action {
            StartersDirAction::List => starters_dir_list().await,
            StartersDirAction::Add { path } => starters_dir_add(&path).await,
            StartersDirAction::Remove { path } => starters_dir_remove(&path).await,
        },
    }
}

/// Show configured providers and their status.
async fn show_providers() -> anyhow::Result<()> {
    let config_dir = app_config::AppPaths::resolve().config_dir;

    // Show provider config
    let config_path = config_dir.join("providers.yaml");
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let h = crate::cli::style::header();
        anstream::println!(
            "{h}Provider configuration ({}):{h:#}\n",
            config_path.display()
        );
        anstream::println!("{content}");
    } else {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}No provider configuration found at {}{m:#}",
            config_path.display()
        );
    }

    let m = crate::cli::style::muted();
    anstream::println!("{m}---{m:#}");

    // Show secrets status
    let sops_path = config_dir.join("providers.sops.yaml");
    let plain_path = config_dir.join("providers.secret.yaml");

    if sops_path.exists() {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}Secrets: encrypted (SOPS) at {}{m:#}",
            sops_path.display()
        );
        // Try to list provider names from decrypted content
        match crate::secrets::load_secrets(&config_dir) {
            Ok(secrets) => {
                let names: Vec<&String> = secrets.providers.keys().collect();
                let m = crate::cli::style::muted();
                anstream::println!(
                    "{m}  Configured API keys:{m:#} {}",
                    names
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Err(e) => {
                let e_style = crate::cli::style::err();
                anstream::println!("{e_style}  (could not decrypt: {e}){e_style:#}");
            }
        }
    } else if plain_path.exists() {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}Secrets: unencrypted at {} (consider running: armadai config secrets init){m:#}",
            plain_path.display()
        );
        match crate::secrets::load_secrets(&config_dir) {
            Ok(secrets) => {
                let names: Vec<&String> = secrets.providers.keys().collect();
                let m = crate::cli::style::muted();
                anstream::println!(
                    "{m}  Configured API keys:{m:#} {}",
                    names
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Err(e) => {
                let e_style = crate::cli::style::err();
                anstream::println!("{e_style}  (could not read: {e}){e_style:#}");
            }
        }
    } else {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No secrets file found. Create one:{m:#}");
        anstream::println!(
            "{m}  Option A: armadai config secrets init  (encrypted with SOPS + age){m:#}"
        );
        anstream::println!(
            "{m}  Option B: Create config/providers.secret.yaml manually (unencrypted){m:#}"
        );
    }

    // Check environment variables
    let h = crate::cli::style::header();
    anstream::println!("\n{h}--- Environment variables ---{h:#}");
    for (name, var) in [
        ("Anthropic", "ANTHROPIC_API_KEY"),
        ("OpenAI", "OPENAI_API_KEY"),
        ("Google", "GOOGLE_API_KEY"),
    ] {
        let is_set = std::env::var(var).is_ok_and(|v| !v.is_empty());
        let status = if is_set { "set" } else { "not set" };
        if is_set {
            let o = crate::cli::style::ok();
            anstream::println!("  {name}: ${var} = {o}{status}{o:#}");
        } else {
            let m = crate::cli::style::muted();
            anstream::println!("  {name}: ${var} = {m}{status}{m:#}");
        }
    }

    Ok(())
}

/// Initialize SOPS + age encryption.
async fn secrets_init() -> anyhow::Result<()> {
    let config_dir = app_config::AppPaths::resolve().config_dir;
    std::fs::create_dir_all(&config_dir)?;

    // Run init_sops which generates age key + .sops.yaml
    crate::secrets::sops::init_sops(&config_dir)?;

    // Create template providers.sops.yaml if it doesn't exist
    let sops_path = config_dir.join("providers.sops.yaml");
    if !sops_path.exists() {
        let template = r#"# Provider API keys (encrypted with SOPS + age)
# Edit with: sops config/providers.sops.yaml
providers:
  anthropic:
    api_key: "sk-ant-your-key-here"
  openai:
    api_key: "sk-your-key-here"
  google:
    api_key: "AIza-your-key-here"
"#;
        std::fs::write(&sops_path, template)?;
        let m = crate::cli::style::muted();
        anstream::println!("{m}Template created at: {}{m:#}", sops_path.display());

        // Try to encrypt it with sops
        let encrypt = std::process::Command::new("sops")
            .args(["--encrypt", "--in-place", &sops_path.display().to_string()])
            .output();

        match encrypt {
            Ok(output) if output.status.success() => {
                let o = crate::cli::style::ok();
                anstream::println!("{o}File encrypted successfully.{o:#}");
                let m = crate::cli::style::muted();
                anstream::println!("{m}\nEdit your secrets with:{m:#}");
                anstream::println!("{m}  sops config/providers.sops.yaml{m:#}");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let w = crate::cli::style::warn();
                anstream::println!(
                    "{w}\nWarning: could not encrypt file: {stderr}\n\
                     Make sure SOPS_AGE_KEY_FILE is set, then encrypt manually:\n  \
                     sops --encrypt --in-place config/providers.sops.yaml{w:#}"
                );
            }
            Err(_) => {
                let w = crate::cli::style::warn();
                anstream::println!(
                    "{w}\nWarning: sops not found. Install it and encrypt manually:\n  \
                     sops --encrypt --in-place config/providers.sops.yaml{w:#}"
                );
            }
        }
    } else {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}Secrets file already exists at: {}{m:#}",
            sops_path.display()
        );
        anstream::println!("{m}Edit with: sops config/providers.sops.yaml{m:#}");
    }

    Ok(())
}

/// Rotate the age encryption key.
async fn secrets_rotate() -> anyhow::Result<()> {
    let config_dir = app_config::AppPaths::resolve().config_dir;
    let key_path = config_dir.join("age-key.txt");
    let sops_path = config_dir.join("providers.sops.yaml");

    if !key_path.exists() {
        anyhow::bail!(
            "No age key found at {}. Run 'armadai config secrets init' first.",
            key_path.display()
        );
    }

    if !sops_path.exists() {
        anyhow::bail!(
            "No encrypted secrets file found at {}. Run 'armadai config secrets init' first.",
            sops_path.display()
        );
    }

    // 1. Decrypt current secrets
    let m = crate::cli::style::muted();
    anstream::println!("{m}Decrypting current secrets...{m:#}");
    let secrets = crate::secrets::sops::decrypt_file(&sops_path)?;

    // 2. Backup old key
    let backup_path = config_dir.join("age-key.txt.bak");
    std::fs::copy(&key_path, &backup_path)?;
    let m = crate::cli::style::muted();
    anstream::println!("{m}Old key backed up to: {}{m:#}", backup_path.display());

    // 3. Generate new key
    std::fs::remove_file(&key_path)?;
    let output = std::process::Command::new("age-keygen")
        .args(["-o", &key_path.display().to_string()])
        .output()?;

    if !output.status.success() {
        // Restore backup
        std::fs::copy(&backup_path, &key_path)?;
        anyhow::bail!(
            "age-keygen failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let o = crate::cli::style::ok();
    anstream::println!("{o}New age key generated.{o:#}");

    // 4. Update .sops.yaml with new public key
    let key_content = std::fs::read_to_string(&key_path)?;
    let public_key = key_content
        .lines()
        .find(|l| l.starts_with("# public key:"))
        .and_then(|l| l.strip_prefix("# public key: "))
        .ok_or_else(|| anyhow::anyhow!("Could not extract public key"))?;

    let sops_config = format!(
        r#"creation_rules:
  - path_regex: \.sops\.yaml$
    age: "{public_key}"
"#
    );
    let sops_config_path = Path::new(".sops.yaml");
    std::fs::write(sops_config_path, sops_config)?;
    let m = crate::cli::style::muted();
    anstream::println!("{m}Updated .sops.yaml with new public key.{m:#}");

    // 5. Re-encrypt secrets with new key
    let yaml = serde_yaml_ng::to_string(&secrets)?;
    std::fs::write(&sops_path, yaml)?;

    let encrypt = std::process::Command::new("sops")
        .args(["--encrypt", "--in-place", &sops_path.display().to_string()])
        .output()?;

    if !encrypt.status.success() {
        anyhow::bail!(
            "Failed to re-encrypt with new key: {}",
            String::from_utf8_lossy(&encrypt.stderr)
        );
    }

    let o = crate::cli::style::ok();
    anstream::println!("{o}Secrets re-encrypted with new key.{o:#}");
    let m = crate::cli::style::muted();
    anstream::println!("{m}\nDon't forget to update SOPS_AGE_KEY_FILE if needed.{m:#}");
    anstream::println!(
        "{m}You can delete the backup when confirmed: rm {}{m:#}",
        backup_path.display()
    );

    Ok(())
}

/// List all starter directories with their source type.
async fn starters_dir_list() -> anyhow::Result<()> {
    use crate::core::starter::{all_starters_dirs, builtin_starters_dir};

    let builtin = builtin_starters_dir();
    let user = app_config::user_starters_dir();

    let config = app_config::load_user_config();
    let config_dirs: Vec<std::path::PathBuf> = config
        .starters_dirs
        .iter()
        .map(std::path::PathBuf::from)
        .collect();

    let env_dirs: Vec<std::path::PathBuf> = std::env::var("ARMADAI_STARTERS_DIRS")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.trim().is_empty())
        .map(|s| std::path::PathBuf::from(s.trim()))
        .collect();

    let project_starters = std::path::Path::new(".armadai/starters");

    for dir in all_starters_dirs() {
        let source = if dir == builtin {
            "built-in"
        } else if dir == project_starters.to_path_buf() {
            "project"
        } else if dir == user {
            "user"
        } else if env_dirs.contains(&dir) {
            "env"
        } else if config_dirs.contains(&dir) {
            "config"
        } else {
            "unknown"
        };
        let m = crate::cli::style::muted();
        anstream::println!("{m}  [{source}]{m:#} {}", dir.display());
    }

    Ok(())
}

/// Add a custom starter directory to config.yaml.
async fn starters_dir_add(path: &str) -> anyhow::Result<()> {
    let mut config = app_config::load_user_config();

    if config.starters_dirs.contains(&path.to_string()) {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Directory already in config: {path}{m:#}");
        return Ok(());
    }

    config.starters_dirs.push(path.to_string());
    app_config::save_user_config(&config)?;
    let o = crate::cli::style::ok();
    anstream::println!("{o}Added starter directory: {path}{o:#}");
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  Saved to {}{m:#}",
        app_config::config_file_path().display()
    );

    Ok(())
}

/// Remove a custom starter directory from config.yaml.
async fn starters_dir_remove(path: &str) -> anyhow::Result<()> {
    let mut config = app_config::load_user_config();

    let before = config.starters_dirs.len();
    config.starters_dirs.retain(|d| d != path);

    if config.starters_dirs.len() == before {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Directory not found in config: {path}{m:#}");
        return Ok(());
    }

    app_config::save_user_config(&config)?;
    let o = crate::cli::style::ok();
    anstream::println!("{o}Removed starter directory: {path}{o:#}");
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  Saved to {}{m:#}",
        app_config::config_file_path().display()
    );

    Ok(())
}
