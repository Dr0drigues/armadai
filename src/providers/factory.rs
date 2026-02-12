use crate::core::agent::Agent;

use super::traits::Provider;

/// Known tool definitions for unified provider names.
/// Each entry maps a user-friendly name to its CLI command and API backend.
struct ToolDef {
    /// CLI command name (e.g. "claude", "gemini")
    cli_command: &'static str,
    /// Default CLI args for this tool
    cli_args: &'static [&'static str],
    /// Corresponding API provider name (e.g. "anthropic")
    api_backend: &'static str,
    /// Environment variable for API key
    api_key_env: &'static str,
}

const KNOWN_TOOLS: &[(&str, ToolDef)] = &[
    (
        "claude",
        ToolDef {
            cli_command: "claude",
            cli_args: &["-p", "--output-format", "text"],
            api_backend: "anthropic",
            api_key_env: "ANTHROPIC_API_KEY",
        },
    ),
    (
        "gemini",
        ToolDef {
            cli_command: "gemini",
            cli_args: &["-p"],
            api_backend: "google",
            api_key_env: "GOOGLE_API_KEY",
        },
    ),
    (
        "gpt",
        ToolDef {
            cli_command: "gpt",
            cli_args: &[],
            api_backend: "openai",
            api_key_env: "OPENAI_API_KEY",
        },
    ),
    (
        "aider",
        ToolDef {
            cli_command: "aider",
            cli_args: &["--message"],
            api_backend: "openai",
            api_key_env: "OPENAI_API_KEY",
        },
    ),
];

fn find_tool(name: &str) -> Option<&'static ToolDef> {
    KNOWN_TOOLS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, def)| def)
}

/// Map a unified tool name to its API backend name.
/// Returns the backend directly for explicit API providers.
/// e.g. "claude" → "anthropic", "gemini" → "google", "anthropic" → "anthropic"
pub fn api_backend_for_tool(name: &str) -> Option<&'static str> {
    match name {
        "anthropic" => Some("anthropic"),
        "openai" => Some("openai"),
        "google" => Some("google"),
        "proxy" => Some("proxy"),
        _ => find_tool(name).map(|t| t.api_backend),
    }
}

/// Check if a CLI command is available on the system.
fn cli_available(command: &str) -> bool {
    std::process::Command::new("which")
        .arg(command)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Create the appropriate provider for an agent based on its metadata.
///
/// Provider resolution order:
/// 1. `provider: cli` — explicit CLI mode, requires `command` field
/// 2. `provider: anthropic|openai|google` — explicit API mode
/// 3. `provider: claude|gemini|gpt|aider` — unified name, auto-detects:
///    a. If the CLI tool is installed → use CLI provider
///    b. Otherwise → fall back to API provider
pub fn create_provider(agent: &Agent) -> anyhow::Result<Box<dyn Provider>> {
    let provider = agent.metadata.provider.as_str();

    match provider {
        // Explicit CLI mode
        "cli" => create_cli_provider(agent),

        // Explicit API providers
        "anthropic" | "openai" | "google" | "proxy" => create_api_provider(provider, agent),

        // Unified tool names — auto-detect CLI vs API
        _ => {
            if let Some(tool) = find_tool(provider) {
                create_unified_provider(provider, tool, agent)
            } else {
                anyhow::bail!(
                    "Unknown provider: '{provider}'. \
                     Known providers: cli, anthropic, openai, google, claude, gemini, gpt, aider"
                )
            }
        }
    }
}

/// Create a provider from a unified tool name, preferring CLI if available.
fn create_unified_provider(
    name: &str,
    tool: &ToolDef,
    agent: &Agent,
) -> anyhow::Result<Box<dyn Provider>> {
    // Use explicit command/args from agent metadata if provided
    let command = agent
        .metadata
        .command
        .as_deref()
        .unwrap_or(tool.cli_command);
    let has_custom_args = agent.metadata.args.is_some();

    if cli_available(command) {
        let args = if has_custom_args {
            agent.metadata.args.clone().unwrap_or_default()
        } else {
            tool.cli_args.iter().map(|s| (*s).to_string()).collect()
        };
        let timeout = agent.metadata.timeout.unwrap_or(300);
        tracing::info!("Provider '{name}': using CLI ({command}) — tool detected on system");
        Ok(Box::new(super::cli::CliProvider::new(
            command.to_string(),
            args,
            timeout,
        )))
    } else {
        tracing::info!(
            "Provider '{name}': CLI '{command}' not found, falling back to API ({})",
            tool.api_backend
        );
        create_api_provider(tool.api_backend, agent)
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
            if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
                p.base_url = url;
            }
            Ok(Box::new(p))
        }
        "openai" | "google" | "proxy" => {
            anyhow::bail!("Provider '{provider}' is not yet implemented")
        }
        other => anyhow::bail!("Unknown API provider: '{other}'"),
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
    if let Ok(key) = std::env::var(env_var)
        && !key.is_empty()
    {
        return Ok(key);
    }

    let config_dir = crate::core::config::AppPaths::resolve().config_dir;
    if let Ok(secrets) = crate::secrets::load_secrets(&config_dir)
        && let Some(creds) = secrets.providers.get(provider_name)
    {
        return Ok(creds.api_key.clone());
    }

    anyhow::bail!(
        "No API key found for '{provider_name}'. \
         Set {env_var} or add to config/providers.secret.yaml"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_known_tools() {
        assert!(find_tool("claude").is_some());
        assert!(find_tool("gemini").is_some());
        assert!(find_tool("gpt").is_some());
        assert!(find_tool("aider").is_some());
        assert!(find_tool("unknown").is_none());
    }

    #[test]
    fn cli_available_echo() {
        // echo should be available on all systems
        assert!(cli_available("echo"));
        assert!(!cli_available("this_command_does_not_exist_xyz"));
    }
}
