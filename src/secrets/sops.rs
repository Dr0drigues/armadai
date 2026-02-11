use std::path::Path;
use std::process::Command;

use super::ProviderSecrets;

/// Decrypt a SOPS-encrypted YAML file using the `sops` CLI.
pub fn decrypt_file(path: &Path) -> anyhow::Result<ProviderSecrets> {
    let output = Command::new("sops")
        .args(["--decrypt", &path.display().to_string()])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("SOPS decryption failed: {stderr}");
    }

    let decrypted = String::from_utf8(output.stdout)?;
    let secrets: ProviderSecrets = serde_yml::from_str(&decrypted)?;
    Ok(secrets)
}

/// Initialize SOPS + age for this project.
pub fn init_sops(config_dir: &Path) -> anyhow::Result<()> {
    // Generate age key if not present
    let key_path = config_dir.join("age-key.txt");
    if !key_path.exists() {
        let output = Command::new("age-keygen")
            .args(["-o", &key_path.display().to_string()])
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "age-keygen failed. Is age installed? (brew install age)\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        println!("Age key generated at: {}", key_path.display());
    }

    // Read public key from key file
    let key_content = std::fs::read_to_string(&key_path)?;
    let public_key = key_content
        .lines()
        .find(|l| l.starts_with("# public key:"))
        .and_then(|l| l.strip_prefix("# public key: "))
        .ok_or_else(|| anyhow::anyhow!("Could not extract public key from age key file"))?;

    // Create .sops.yaml
    let sops_config = format!(
        r#"creation_rules:
  - path_regex: \.sops\.yaml$
    age: "{public_key}"
"#
    );

    let sops_path = config_dir.parent().unwrap_or(config_dir).join(".sops.yaml");
    std::fs::write(&sops_path, sops_config)?;
    println!("SOPS config written to: {}", sops_path.display());

    println!(
        "\nSetup complete. Add to your shell profile:\n  export SOPS_AGE_KEY_FILE={}",
        key_path.display()
    );

    Ok(())
}
