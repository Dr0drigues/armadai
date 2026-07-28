use armadai_core::agent::Agent;

use armadai_core::provider::Provider;

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

    let inner: Box<dyn Provider> = match provider {
        // Explicit CLI mode
        "cli" => create_cli_provider(agent)?,

        // Explicit API providers
        "anthropic" | "openai" | "google" | "proxy" => create_api_provider(provider, agent)?,

        // Unified tool names — auto-detect CLI vs API
        _ => {
            if let Some(tool) = find_tool(provider) {
                create_unified_provider(provider, tool, agent)?
            } else {
                anyhow::bail!(
                    "Unknown provider: '{provider}'. \
                     Known providers: cli, anthropic, openai, google, claude, gemini, gpt, aider"
                )
            }
        }
    };
    Ok(wrap_rate_limited(agent, inner))
}

/// Map an agent's `provider` string to the `config.rate_limits` key, or `None`
/// for providers with no per-account API quota (pure CLI).
fn rate_limit_key(provider: &str) -> Option<String> {
    match provider {
        "anthropic" | "openai" | "google" | "proxy" => Some(provider.to_string()),
        "claude" => Some("anthropic".to_string()),
        "gemini" => Some("google".to_string()),
        "gpt" => Some("openai".to_string()),
        _ => None, // "cli", unknown, or unified-resolving-to-cli
    }
}

/// Wrap `inner` with the shared per-provider limiter (from `config.rate_limits`)
/// and the optional per-agent limiter (from frontmatter `rate_limit`). Always
/// wraps: with both limiters `None` the decorator's `throttle()` awaits
/// nothing, so this is a zero-cost pass-through.
fn wrap_rate_limited(agent: &Agent, inner: Box<dyn Provider>) -> Box<dyn Provider> {
    use super::{Rate, RateLimitedProvider, RateLimiter, shared_provider_limiter};

    let provider_limiter = rate_limit_key(agent.metadata.provider.as_str()).and_then(|key| {
        let rate = armadai_core::config::load_user_config()
            .rate_limits
            .get(&key)
            .map(|&per_min| Rate::from_per_minute(per_min as f64));
        shared_provider_limiter(&key, rate)
    });

    let agent_limiter = agent
        .metadata
        .rate_limit
        .as_deref()
        .and_then(Rate::parse)
        .map(|r| std::sync::Arc::new(RateLimiter::new(r)));

    Box::new(RateLimitedProvider::new(
        std::sync::Arc::from(inner),
        provider_limiter,
        agent_limiter,
    ))
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
            // Respect the agent's explicit args verbatim (never override them).
            agent.metadata.args.clone().unwrap_or_default()
        } else if crate::providers::json_runner::supports_json(command) {
            // Default (no custom args) on a JSON-capable CLI: use the canonical
            // stream-json args so the provider captures real cost/tokens
            // instead of $0.00. The provider parses stdout opportunistically.
            crate::providers::json_runner::json_mode_args(command)
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
        "google" => {
            let api_key = get_api_key("GOOGLE_API_KEY", "google")?;
            let mut p = super::api::google::GoogleProvider::new(api_key);
            if let Ok(url) = std::env::var("GOOGLE_BASE_URL") {
                p.base_url = url;
            }
            Ok(Box::new(p))
        }
        "openai" | "proxy" => {
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

    let config_dir = armadai_core::config::AppPaths::resolve().config_dir;
    if let Ok(secrets) = armadai_secrets::load_secrets(&config_dir)
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

    #[test]
    fn rate_limit_key_maps_providers() {
        assert_eq!(rate_limit_key("anthropic"), Some("anthropic".to_string()));
        assert_eq!(rate_limit_key("openai"), Some("openai".to_string()));
        assert_eq!(rate_limit_key("google"), Some("google".to_string()));
        assert_eq!(rate_limit_key("proxy"), Some("proxy".to_string()));
        // unified names map to their API backend key
        assert_eq!(rate_limit_key("claude"), Some("anthropic".to_string()));
        assert_eq!(rate_limit_key("gemini"), Some("google".to_string()));
        assert_eq!(rate_limit_key("gpt"), Some("openai".to_string()));
        // pure CLI: no per-provider quota key
        assert_eq!(rate_limit_key("cli"), None);
        assert_eq!(rate_limit_key("unknown-tool"), None);
    }

    /// `wrap_rate_limited` with an agent-level `rate_limit` throttles even
    /// when no shared provider-key limiter applies. Uses `provider: "cli"`
    /// (maps to no `rate_limit_key`) so this test never touches the
    /// process-global `shared_provider_limiter` registry — no unique-key
    /// concern here (contrast with tests that DO hit that registry, which
    /// must use a key suffixed for the test to avoid cross-test collision
    /// under parallel `cargo test`).
    #[tokio::test]
    async fn wrap_rate_limited_agent_limiter_throttles_without_provider_key() {
        use armadai_core::agent::AgentMetadata;
        use armadai_core::provider::{
            ChatMessage, CompletionRequest, CompletionResponse, ProviderMetadata, TokenStream,
        };
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        struct CountingProvider {
            calls: Arc<AtomicUsize>,
        }
        #[async_trait::async_trait]
        impl Provider for CountingProvider {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> anyhow::Result<CompletionResponse> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(CompletionResponse {
                    content: "ok".into(),
                    model: "m".into(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                })
            }
            async fn stream(&self, _req: CompletionRequest) -> anyhow::Result<TokenStream> {
                anyhow::bail!("unused")
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "counting".into(),
                    models: vec![],
                    supports_streaming: false,
                }
            }
        }

        fn req() -> CompletionRequest {
            CompletionRequest {
                model: "m".into(),
                system_prompt: String::new(),
                messages: vec![ChatMessage {
                    role: "user".into(),
                    content: "hi".into(),
                }],
                temperature: 0.0,
                max_tokens: None,
            }
        }

        let agent = Agent {
            name: "test-agent".into(),
            source: std::path::PathBuf::from("test.md"),
            metadata: AgentMetadata {
                provider: "cli".into(),
                model: None,
                command: Some("echo".into()),
                args: None,
                temperature: 0.7,
                max_tokens: None,
                timeout: None,
                tags: vec![],
                stacks: vec![],
                scope: vec![],
                model_fallback: vec![],
                cost_limit: None,
                rate_limit: Some("1/sec".to_string()),
                context_window: None,
                mode: None,
                orchestration: None,
                triggers: None,
                ring_config: None,
            },
            system_prompt: String::new(),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        };

        let calls = Arc::new(AtomicUsize::new(0));
        let inner: Box<dyn Provider> = Box::new(CountingProvider {
            calls: calls.clone(),
        });
        let wrapped = wrap_rate_limited(&agent, inner);

        wrapped.complete(req()).await.unwrap();
        let start = Instant::now();
        wrapped.complete(req()).await.unwrap();
        // Agent-level "1/sec" burst 1: the 2nd call must wait ~1s.
        assert!(start.elapsed() >= Duration::from_millis(800));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
