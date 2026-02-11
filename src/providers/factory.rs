use crate::core::agent::Agent;

use super::traits::Provider;

/// Create the appropriate provider for an agent based on its metadata.
pub fn create_provider(agent: &Agent) -> anyhow::Result<Box<dyn Provider>> {
    match agent.metadata.provider.as_str() {
        "cli" => create_cli_provider(agent),
        provider => create_api_provider(provider, agent),
    }
}

fn create_cli_provider(agent: &Agent) -> anyhow::Result<Box<dyn Provider>> {
    let command = agent
        .metadata
        .command
        .clone()
        .ok_or_else(|| anyhow::anyhow!("CLI provider requires 'command' in Metadata"))?;
    let args = agent.metadata.args.clone().unwrap_or_default();
    let timeout = agent.metadata.timeout.unwrap_or(300);
    Ok(Box::new(super::cli::CliProvider::new(
        command, args, timeout,
    )))
}

#[cfg(feature = "providers-api")]
fn create_api_provider(provider: &str, _agent: &Agent) -> anyhow::Result<Box<dyn Provider>> {
    match provider {
        "anthropic" => {
            let api_key = get_api_key("ANTHROPIC_API_KEY", "anthropic")?;
            let mut p = super::api::anthropic::AnthropicProvider::new(api_key);
            // Allow base_url override from config
            if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
                p.base_url = url;
            }
            Ok(Box::new(p))
        }
        "openai" | "google" | "proxy" => {
            anyhow::bail!("Provider '{provider}' is not yet implemented")
        }
        other => anyhow::bail!("Unknown provider: '{other}'"),
    }
}

#[cfg(not(feature = "providers-api"))]
fn create_api_provider(provider: &str, _agent: &Agent) -> anyhow::Result<Box<dyn Provider>> {
    anyhow::bail!(
        "Provider '{provider}' requires the 'providers-api' feature. \
         Build with: cargo build --features providers-api"
    )
}

/// Resolve an API key from environment variable or secrets file.
#[cfg(feature = "providers-api")]
fn get_api_key(env_var: &str, provider_name: &str) -> anyhow::Result<String> {
    // 1. Try environment variable
    if let Ok(key) = std::env::var(env_var)
        && !key.is_empty()
    {
        return Ok(key);
    }

    // 2. Try secrets file
    let config_dir = std::path::Path::new("config");
    if let Ok(secrets) = crate::secrets::load_secrets(config_dir)
        && let Some(creds) = secrets.providers.get(provider_name)
    {
        return Ok(creds.api_key.clone());
    }

    anyhow::bail!(
        "No API key found for '{provider_name}'. \
         Set {env_var} or add to config/providers.secret.yaml"
    )
}
