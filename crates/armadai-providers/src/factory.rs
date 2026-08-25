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
    /// Environment variable for API key.
    // Populated for every entry but never read anywhere today (API key
    // resolution goes through `get_api_key` with the env var name inlined
    // per-provider in `api/*.rs`). Previously silent under the bin's blanket
    // `#[allow(dead_code)] mod providers;`; scoped here rather than adding an
    // allow at the crate root (OH7 #252 Lot 4, pure refactor, no behavior
    // change).
    #[allow(dead_code)]
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

/// Default `CliProvider` timeout (seconds) when neither the agent's own
/// frontmatter `timeout` nor an orchestration-level override sets one.
///
/// This value only applies to a `direct` (non-orchestrated) single-agent
/// run: `armadai/src/cli/run.rs`'s `apply_orchestrated_timeout` always sets
/// `agent.metadata.timeout` before `create_provider` runs for blackboard/
/// ring/hierarchical agents, using its own `ORCHESTRATED_DEFAULT_TIMEOUT_SECS`
/// (600s) — so this constant is never reached on the orchestrated path.
/// The two are intentionally different, not merely duplicated: a `direct`
/// run is one CLI call, whereas an orchestrated run's coordinator turn is
/// itself agentic (delegating, waiting on sub-agents) and legitimately
/// takes longer, see #270. They are named and documented on both sides so a
/// future change to one doesn't silently drift from the other.
///
/// Since #270, `CliProvider::timeout_secs` bounds *inactivity* (the gap
/// between consecutive lines of subprocess output), not the call's total
/// duration — see `cli::CliProvider::complete`.
///
/// Honest caveat: that change buys real headroom for an orchestrated,
/// multi-delegation run (each delegation's output resets the clock), but
/// it buys a `direct` single-agent run comparatively little. A `direct`
/// run is exactly one subprocess call with nothing inside it to reset the
/// clock on its own output, so the largest inactivity gap it can have is
/// approximately its *total* duration anyway — meaning 300s here still
/// behaves close to the old wall-clock ceiling for that path. Measured
/// machine-side turn durations on this project put a single agentic turn
/// at roughly 500s+ (the same order of magnitude `ORCHESTRATED_DEFAULT_
/// TIMEOUT_SECS`'s doc cites), i.e. *above* this 300s default — a long
/// single-turn `direct` run can still hit this ceiling. Not changed here
/// (raising it is a separate, deliberate call, not a side effect of this
/// fix); recorded so the number isn't read as more protective than it is.
///
/// Previously duplicated as a bare `300` literal at both call sites below
/// (`create_unified_provider` and `create_cli_provider`); collapsed to one
/// named constant so the two can't independently drift.
///
/// `pub` (not `pub(crate)`): `armadai/src/cli/run.rs` needs to reference
/// this exact value too (its own `ORCHESTRATED_DEFAULT_TIMEOUT_SECS` doc
/// comment, and its `direct`-vs-orchestrated test assertions) — the point
/// of naming this constant was to have ONE place a future change lands, so
/// a private constant that forces callers to restate "300s" in prose or
/// literals would just move the duplication one layer up instead of
/// closing it.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

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
        } else if crate::json_runner::supports_json(command) {
            // Default (no custom args) on a JSON-capable CLI: use the canonical
            // stream-json args so the provider captures real cost/tokens
            // instead of $0.00. The provider parses stdout opportunistically.
            crate::json_runner::json_mode_args(command)
        } else {
            tool.cli_args.iter().map(|s| (*s).to_string()).collect()
        };
        let timeout = agent.metadata.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);
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
    let timeout = agent.metadata.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);
    Ok(Box::new(super::cli::CliProvider::new(
        command, args, timeout,
    )))
}

#[cfg(feature = "api")]
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
        "openai" => {
            let api_key = get_api_key("OPENAI_API_KEY", "openai")?;
            let mut p = super::api::openai::OpenAiProvider::new(api_key);
            if let Some(url) = resolve_base_url("openai", "OPENAI_BASE_URL") {
                p.base_url = url;
            }
            Ok(Box::new(p))
        }
        "proxy" => {
            // A proxy has no universal home: the base URL is the whole
            // point of the provider. Env var wins, then `providers.yaml`,
            // then the port `armadai up`'s LiteLLM listens on.
            let base_url = resolve_base_url("proxy", "PROXY_BASE_URL")
                .unwrap_or_else(|| DEFAULT_PROXY_BASE_URL.to_string());
            // Optional on purpose: a gateway on localhost usually has no
            // key, and `Authorization: Bearer ` (empty) is rejected by some
            // servers — `ProxyProvider` sends no header at all for `None`.
            let api_key = optional_api_key("PROXY_API_KEY", "proxy");
            Ok(Box::new(super::proxy::ProxyProvider::new(
                base_url, api_key,
            )))
        }
        other => anyhow::bail!("Unknown API provider: '{other}'"),
    }
}

/// Where `armadai up`'s LiteLLM listens — the fallback base URL for
/// `provider: proxy` when neither `PROXY_BASE_URL` nor `providers.yaml`
/// says otherwise. Matches `DEFAULT_PROVIDERS_YAML`'s `proxy` entry.
#[cfg(feature = "api")]
const DEFAULT_PROXY_BASE_URL: &str = "http://localhost:4000/v1";

/// Resolve an OpenAI-compatible provider's base URL: the environment
/// variable first, then `providers.yaml`'s `providers.<key>.base_url`.
///
/// `anthropic` and `google` read only their own env var (they predate
/// this and their vendor URL is fixed); `openai` and `proxy` also honour
/// `providers.yaml` because pointing them somewhere else — a gateway, a
/// local runtime — is the normal case rather than the exception, and that
/// file is where a user would reasonably write it down. Documented in
/// `docs/wiki/providers.md`.
#[cfg(feature = "api")]
fn resolve_base_url(config_key: &str, env_var: &str) -> Option<String> {
    if let Ok(url) = std::env::var(env_var)
        && !url.trim().is_empty()
    {
        return Some(url);
    }
    armadai_core::config::load_providers_config()
        .providers
        .get(config_key)
        .and_then(|p| p.base_url.clone())
        .filter(|u| !u.trim().is_empty())
}

/// Like `get_api_key`, but a missing key is a normal outcome rather than an
/// error: an OpenAI-compatible gateway or local runtime frequently needs no
/// authentication at all.
#[cfg(feature = "api")]
fn optional_api_key(env_var: &str, provider_name: &str) -> Option<String> {
    get_api_key(env_var, provider_name).ok()
}

#[cfg(not(feature = "api"))]
fn create_api_provider(provider: &str, _agent: &Agent) -> anyhow::Result<Box<dyn Provider>> {
    anyhow::bail!(
        "Provider '{provider}' requires the 'providers-api' feature. \
         Build with: cargo build --features providers-api"
    )
}

/// Resolve an API key from environment variable or secrets file.
#[cfg(feature = "api")]
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

    // --- the four configurations that used to be refused (#368) ---
    //
    // Before this change `create_api_provider` answered `openai` and
    // `proxy` with `bail!("... is not yet implemented")`, which also made
    // `create_provider`'s documented "CLI not installed -> fall back to the
    // API" promise false for `gpt` and `aider`. These tests drive the real
    // `create_provider` entry point for all four.

    #[cfg(feature = "api")]
    mod api_wiring {
        use super::*;
        use armadai_core::agent::AgentMetadata;
        use armadai_core::test_support::IsolatedConfigDir;

        /// Redirect the config dir (so no real `providers.yaml`/secrets are
        /// read) and pin the env vars named, restoring everything on drop.
        ///
        /// `IsolatedConfigDir` is the workspace's shared guard (#372): it
        /// already holds the env lock, plants a temp `ARMADAI_CONFIG_DIR`
        /// and restores it. The first version of these tests re-implemented
        /// all of that privately — the very duplication this module's own
        /// doc-comments call out — and stopped compiling the moment #372
        /// moved the lock behind `test_support`.
        fn env_scope(vars: &[(&str, Option<&str>)]) -> IsolatedConfigDir {
            vars.iter().fold(IsolatedConfigDir::enter(), |scope, (name, value)| {
                scope.with_var(name, *value)
            })
        }

        fn agent_with(provider: &str, command: Option<&str>) -> Agent {
            Agent {
                name: "t".into(),
                source: std::path::PathBuf::from("t.md"),
                metadata: AgentMetadata {
                    provider: provider.into(),
                    model: None,
                    command: command.map(str::to_string),
                    args: None,
                    temperature: 0.7,
                    max_tokens: None,
                    timeout: None,
                    tags: vec![],
                    stacks: vec![],
                    scope: vec![],
                    model_fallback: vec![],
                    cost_limit: None,
                    rate_limit: None,
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
            }
        }

        #[test]
        fn provider_openai_builds_a_real_provider_instead_of_being_refused() {
            let _env = env_scope(&[
                ("OPENAI_API_KEY", Some("sk-test")),
                ("OPENAI_BASE_URL", None),
            ]);

            let provider = create_provider(&agent_with("openai", None))
                .expect("provider: openai must build now");
            assert_eq!(provider.metadata().name, "openai");
            assert!(provider.metadata().supports_streaming);
        }

        /// A missing key must be reported as a missing key — the actionable
        /// error — not as "not yet implemented".
        #[test]
        fn provider_openai_without_a_key_names_the_key_not_the_feature() {
            let _env = env_scope(&[("OPENAI_API_KEY", None)]);

            let err = match create_provider(&agent_with("openai", None)) {
                Ok(_) => panic!("no key configured, yet a provider was built"),
                Err(e) => e.to_string(),
            };
            assert!(err.contains("OPENAI_API_KEY"), "got: {err}");
            assert!(!err.contains("not yet implemented"), "got: {err}");
        }

        /// The whole point of `proxy`: a gateway with no credentials at all
        /// must still produce a usable provider.
        #[test]
        fn provider_proxy_builds_with_no_api_key_at_all() {
            let _env = env_scope(&[
                ("PROXY_API_KEY", None),
                ("PROXY_BASE_URL", None),
                ("OPENAI_API_KEY", None),
            ]);

            let provider =
                create_provider(&agent_with("proxy", None)).expect("keyless proxy must build");
            assert_eq!(provider.metadata().name, "proxy");
        }

        /// `create_provider` documents "CLI installed -> CLI, otherwise ->
        /// API". With the CLI absent, the API fallback used to dead-end in
        /// the `bail!`; `gpt` and `aider` now really reach the OpenAI path.
        #[test]
        fn gpt_and_aider_fall_back_to_the_api_when_their_cli_is_missing() {
            let _env = env_scope(&[("OPENAI_API_KEY", Some("sk-test"))]);
            let missing = "this_command_does_not_exist_xyz";
            assert!(!cli_available(missing));

            for tool in ["gpt", "aider"] {
                let provider = create_provider(&agent_with(tool, Some(missing)))
                    .unwrap_or_else(|e| panic!("{tool} API fallback failed: {e}"));
                assert_eq!(
                    provider.metadata().name,
                    "openai",
                    "{tool} must fall back to the OpenAI API backend"
                );
            }
        }

        #[test]
        fn the_base_url_env_var_wins_over_providers_yaml() {
            let env = env_scope(&[("PROXY_BASE_URL", Some("http://from-env:9/v1"))]);
            std::fs::write(
                env.config_dir().join("providers.yaml"),
                "providers:\n  proxy:\n    base_url: http://from-file:8/v1\n",
            )
            .expect("write providers.yaml");

            assert_eq!(
                resolve_base_url("proxy", "PROXY_BASE_URL").as_deref(),
                Some("http://from-env:9/v1")
            );
        }

        #[test]
        fn providers_yaml_supplies_the_base_url_when_no_env_var_is_set() {
            let env = env_scope(&[("PROXY_BASE_URL", None)]);
            std::fs::write(
                env.config_dir().join("providers.yaml"),
                "providers:\n  proxy:\n    base_url: http://from-file:8/v1\n",
            )
            .expect("write providers.yaml");

            assert_eq!(
                resolve_base_url("proxy", "PROXY_BASE_URL").as_deref(),
                Some("http://from-file:8/v1")
            );
        }

        /// Neither source configured: the caller falls back to the LiteLLM
        /// port `armadai up` starts.
        #[test]
        fn a_proxy_with_nothing_configured_lands_on_the_documented_default() {
            let _env = env_scope(&[("PROXY_BASE_URL", None)]);
            assert_eq!(resolve_base_url("proxy", "PROXY_BASE_URL"), None);
            assert_eq!(DEFAULT_PROXY_BASE_URL, "http://localhost:4000/v1");
        }

        /// An env var set to the empty string is not a configuration.
        #[test]
        fn an_empty_base_url_env_var_is_ignored() {
            let _env = env_scope(&[("PROXY_BASE_URL", Some("   "))]);
            assert_eq!(resolve_base_url("proxy", "PROXY_BASE_URL"), None);
        }
    }
}
