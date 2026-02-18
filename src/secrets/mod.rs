pub mod sops;

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSecrets {
    pub providers: std::collections::HashMap<String, ProviderCredentials>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredentials {
    pub api_key: String,
    #[serde(default)]
    pub org_id: Option<String>,
}

/// Load provider secrets from a SOPS-encrypted file, or fall back to a plain file.
pub fn load_secrets(config_dir: &Path) -> anyhow::Result<ProviderSecrets> {
    let sops_path = config_dir.join("providers.sops.yaml");
    if sops_path.exists() {
        return sops::decrypt_file(&sops_path);
    }

    let plain_path = config_dir.join("providers.secret.yaml");
    if plain_path.exists() {
        let content = std::fs::read_to_string(&plain_path)?;
        let secrets: ProviderSecrets = serde_yaml_ng::from_str(&content)?;
        tracing::warn!("Loaded secrets from unencrypted file — consider using SOPS + age");
        return Ok(secrets);
    }

    anyhow::bail!("No secrets file found in {}", config_dir.display())
}
