use std::path::Path;

use clap::Subcommand;

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
}

#[derive(Subcommand)]
pub enum SecretsAction {
    /// Generate an age key and .sops.yaml, create encrypted secrets template
    Init,
    /// Decrypt secrets, generate a new age key, and re-encrypt
    Rotate,
}

pub async fn execute(action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Providers => show_providers().await,
        ConfigAction::Secrets { action } => match action {
            SecretsAction::Init => secrets_init().await,
            SecretsAction::Rotate => secrets_rotate().await,
        },
    }
}

/// Show configured providers and their status.
async fn show_providers() -> anyhow::Result<()> {
    let config_dir = Path::new("config");

    // Show provider config
    let config_path = config_dir.join("providers.yaml");
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        println!("Provider configuration ({}):\n", config_path.display());
        println!("{content}");
    } else {
        println!(
            "No provider configuration found at {}",
            config_path.display()
        );
    }

    println!("---");

    // Show secrets status
    let sops_path = config_dir.join("providers.sops.yaml");
    let plain_path = config_dir.join("providers.secret.yaml");

    if sops_path.exists() {
        println!("Secrets: encrypted (SOPS) at {}", sops_path.display());
        // Try to list provider names from decrypted content
        match crate::secrets::load_secrets(config_dir) {
            Ok(secrets) => {
                let names: Vec<&String> = secrets.providers.keys().collect();
                println!(
                    "  Configured API keys: {}",
                    names
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Err(e) => {
                println!("  (could not decrypt: {e})");
            }
        }
    } else if plain_path.exists() {
        println!(
            "Secrets: unencrypted at {} (consider running: armadai config secrets init)",
            plain_path.display()
        );
        match crate::secrets::load_secrets(config_dir) {
            Ok(secrets) => {
                let names: Vec<&String> = secrets.providers.keys().collect();
                println!(
                    "  Configured API keys: {}",
                    names
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Err(e) => {
                println!("  (could not read: {e})");
            }
        }
    } else {
        println!("No secrets file found. Create one:");
        println!("  Option A: armadai config secrets init  (encrypted with SOPS + age)");
        println!("  Option B: Create config/providers.secret.yaml manually (unencrypted)");
    }

    // Check environment variables
    println!("\n--- Environment variables ---");
    for (name, var) in [
        ("Anthropic", "ANTHROPIC_API_KEY"),
        ("OpenAI", "OPENAI_API_KEY"),
        ("Google", "GOOGLE_API_KEY"),
    ] {
        let status = if std::env::var(var).is_ok_and(|v| !v.is_empty()) {
            "set"
        } else {
            "not set"
        };
        println!("  {name}: ${var} = {status}");
    }

    Ok(())
}

/// Initialize SOPS + age encryption.
async fn secrets_init() -> anyhow::Result<()> {
    let config_dir = Path::new("config");
    std::fs::create_dir_all(config_dir)?;

    // Run init_sops which generates age key + .sops.yaml
    crate::secrets::sops::init_sops(config_dir)?;

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
        println!("Template created at: {}", sops_path.display());

        // Try to encrypt it with sops
        let encrypt = std::process::Command::new("sops")
            .args(["--encrypt", "--in-place", &sops_path.display().to_string()])
            .output();

        match encrypt {
            Ok(output) if output.status.success() => {
                println!("File encrypted successfully.");
                println!("\nEdit your secrets with:");
                println!("  sops config/providers.sops.yaml");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                println!(
                    "\nWarning: could not encrypt file: {stderr}\n\
                     Make sure SOPS_AGE_KEY_FILE is set, then encrypt manually:\n  \
                     sops --encrypt --in-place config/providers.sops.yaml"
                );
            }
            Err(_) => {
                println!(
                    "\nWarning: sops not found. Install it and encrypt manually:\n  \
                     sops --encrypt --in-place config/providers.sops.yaml"
                );
            }
        }
    } else {
        println!("Secrets file already exists at: {}", sops_path.display());
        println!("Edit with: sops config/providers.sops.yaml");
    }

    Ok(())
}

/// Rotate the age encryption key.
async fn secrets_rotate() -> anyhow::Result<()> {
    let config_dir = Path::new("config");
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
    println!("Decrypting current secrets...");
    let secrets = crate::secrets::sops::decrypt_file(&sops_path)?;

    // 2. Backup old key
    let backup_path = config_dir.join("age-key.txt.bak");
    std::fs::copy(&key_path, &backup_path)?;
    println!("Old key backed up to: {}", backup_path.display());

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
    println!("New age key generated.");

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
    println!("Updated .sops.yaml with new public key.");

    // 5. Re-encrypt secrets with new key
    let yaml = serde_yml::to_string(&secrets)?;
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

    println!("Secrets re-encrypted with new key.");
    println!("\nDon't forget to update SOPS_AGE_KEY_FILE if needed.");
    println!(
        "You can delete the backup when confirmed: rm {}",
        backup_path.display()
    );

    Ok(())
}
