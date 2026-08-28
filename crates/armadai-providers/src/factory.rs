use armadai_core::agent::Agent;

use armadai_core::provider::Provider;

/// Known tool definitions for unified provider names.
/// Each entry maps a user-friendly name to the CLI command it spawns, when
/// it has one, and to the API backend it calls, when it has one. At least
/// one of the two is always present — enforced by
/// `every_known_tool_declares_at_least_one_backing`.
struct ToolDef {
    /// CLI command name (e.g. "claude", "gemini"), or `None` for an
    /// API-only tool.
    ///
    /// `Option`, symmetrically with [`ToolDef::api_backend`], because a
    /// unified name does not always own a binary. `gpt` is the case that
    /// forced it (issue #402): `/usr/sbin/gpt` is the GUID-partition-table
    /// tool macOS ships on **every** machine, so the `which gpt` probe
    /// always succeeded, the OpenAI fallback was never reached, and
    /// `armadai run` on a `provider: gpt` agent answered `gpt: unknown
    /// command: <system>…` — from a disk-partitioning utility.
    ///
    /// The alternative would have been to validate what the probe finds
    /// (a `--version` on an unknown binary is itself a risk) or to
    /// special-case the string `"gpt"` in `create_unified_provider`. Saying
    /// it in the type is what makes it a property of the entry rather than
    /// of one branch, and mirrors what #369 established in the other
    /// direction for `codex`/`copilot`/`opencode`.
    ///
    /// `None` does **not** disable `command:`: an agent that names a binary
    /// explicitly still gets the CLI path (see `create_unified_provider`),
    /// which is the escape hatch for anyone who does have a real `gpt` CLI.
    cli_command: Option<&'static str>,
    /// Default CLI args for this tool, used only when the CLI has no JSON
    /// mode (`json_runner::supports_json`) and the agent declares no `args:`.
    ///
    /// Empty for an API-only tool: it has no argv of its own, and an agent
    /// pointing one at a binary with `command:` supplies its own `args:`.
    cli_args: &'static [&'static str],
    /// API provider to fall back to when the CLI is not installed, or `None`
    /// for a CLI-only tool.
    ///
    /// `Option` rather than a required string because three of these tools
    /// (`codex`, `copilot`, `opencode`) have no API ArmadAI can reach with an
    /// agent's plain-text exchange: their vendors expose the agent behind the
    /// CLI itself. Naming an arbitrary backend for them would turn "the
    /// binary is missing" into "no API key found for openai", which sends the
    /// user looking for the wrong thing (issue #369).
    ///
    /// The struct previously also carried an `api_key_env` field, never read
    /// anywhere and silenced with `#[allow(dead_code)]`: API keys are
    /// resolved by `get_api_key` with the variable name inlined per provider
    /// in `create_api_provider`. Widening it to `Option` alongside this field
    /// would have added three more never-read `None`s, so it was dropped.
    api_backend: Option<&'static str>,
}

const KNOWN_TOOLS: &[(&str, ToolDef)] = &[
    (
        "claude",
        ToolDef {
            cli_command: Some("claude"),
            cli_args: &["-p", "--output-format", "text"],
            api_backend: Some("anthropic"),
        },
    ),
    (
        "gemini",
        ToolDef {
            cli_command: Some("gemini"),
            cli_args: &["-p"],
            api_backend: Some("google"),
        },
    ),
    // API-only: `gpt` names no LLM CLI anyone ships, but it *does* name the
    // GUID-partition-table tool macOS installs at `/usr/sbin/gpt` (issue
    // #402). Probing `PATH` for it can only find the wrong binary, so it
    // goes straight to OpenAI.
    (
        "gpt",
        ToolDef {
            cli_command: None,
            cli_args: &[],
            api_backend: Some("openai"),
        },
    ),
    // Not API-only: `aider` is a real CLI, and — measured against
    // `/bin`, `/sbin`, `/usr/bin`, `/usr/sbin` on macOS 25.5 and against
    // `debian:stable-slim` — nothing else on either system claims the name.
    (
        "aider",
        ToolDef {
            cli_command: Some("aider"),
            cli_args: &["--message"],
            api_backend: Some("openai"),
        },
    ),
    // CLI-only from here on: `armadai link` already writes a native config
    // for each of these, and `armadai shell` already relays them in JSON
    // mode, but `armadai run` used to refuse them as unknown (issue #369).
    // Their `cli_args` are the non-interactive form each tool needs; in
    // practice `create_unified_provider` uses `json_runner::json_mode_args`
    // for all three, since they do speak JSON.
    (
        "codex",
        ToolDef {
            cli_command: Some("codex"),
            cli_args: &["exec"],
            api_backend: None,
        },
    ),
    (
        "copilot",
        ToolDef {
            cli_command: Some("copilot"),
            cli_args: &["-p"],
            api_backend: None,
        },
    ),
    (
        "opencode",
        ToolDef {
            cli_command: Some("opencode"),
            cli_args: &["run"],
            api_backend: None,
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

/// Providers served over HTTP rather than by spawning a command.
pub const API_PROVIDER_NAMES: &[&str] = &["anthropic", "openai", "google", "proxy"];

/// The unified tool names `KNOWN_TOOLS` declares, in declaration order.
pub fn known_tool_names() -> Vec<&'static str> {
    KNOWN_TOOLS.iter().map(|(name, _)| *name).collect()
}

/// Every value `provider:` accepts.
pub fn accepted_provider_names() -> Vec<&'static str> {
    let mut names = vec!["cli"];
    names.extend_from_slice(API_PROVIDER_NAMES);
    names.extend(known_tool_names());
    names
}

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
    API_PROVIDER_NAMES
        .iter()
        .find(|n| **n == name)
        .copied()
        .or_else(|| find_tool(name).and_then(|t| t.api_backend))
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
/// 2. A name from [`API_PROVIDER_NAMES`] — explicit API mode
/// 3. A unified tool name from [`known_tool_names`] — auto-detects:
///    a. If the CLI tool is installed → use CLI provider
///    b. Otherwise → its API backend, or a report of the missing binary
pub fn create_provider(agent: &Agent) -> anyhow::Result<Box<dyn Provider>> {
    let provider = agent.metadata.provider.as_str();

    let inner: Box<dyn Provider> = match provider {
        // Explicit CLI mode
        "cli" => create_cli_provider(agent)?,

        // Explicit API providers
        p if API_PROVIDER_NAMES.contains(&p) => create_api_provider(p, agent)?,

        // Unified tool names — auto-detect CLI vs API
        _ => match find_tool(provider) {
            Some(tool) => create_unified_provider(provider, tool, agent)?,
            None => anyhow::bail!("{}", unknown_provider_message(provider)),
        },
    };
    Ok(wrap_rate_limited(agent, inner))
}

/// What to answer for a `provider:` value nothing recognises.
///
/// The list is derived from the inventories `create_provider` actually
/// branches on, not typed out: the hand-written version drifted, advertising
/// neither `proxy` (accepted since the gateway provider landed) nor `codex`,
/// `copilot`, `opencode`.
///
/// It also names the generic escape hatch. `provider: cli` runs *any*
/// binary, which is what a user with an unlisted tool needs to hear —
/// listing only the known names left them believing the tool was
/// unsupported.
fn unknown_provider_message(provider: &str) -> String {
    format!(
        "Unknown provider: '{provider}'. Known providers: {}. \
         Any other command-line tool can still be run as an agent with \
         `provider: cli` plus `command: <binary>` (and `args:` if it needs \
         flags to answer non-interactively).",
        accepted_provider_names().join(", ")
    )
}

/// Map an agent's `provider` string to the `config.rate_limits` key, or `None`
/// for providers with no per-account API quota (pure CLI).
fn rate_limit_key(provider: &str) -> Option<String> {
    // The unified-name -> backend mapping already lives in `ToolDef`; a
    // second copy here is how `codex` would silently acquire an OpenAI quota
    // it never spends. `None` for "cli", for an unknown name, and for a
    // CLI-only tool — none of them consume a per-account API quota.
    api_backend_for_tool(provider).map(str::to_string)
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
///
/// The binary to probe for is the agent's own `command:` when it names one,
/// otherwise the tool's declared `cli_command` — and an API-only tool
/// declares none, so nothing is probed at all and the API backend is reached
/// directly (issue #402). An explicit `command:` still wins in every case:
/// it is the user pointing at a binary they know is the right one.
fn create_unified_provider(
    name: &str,
    tool: &ToolDef,
    agent: &Agent,
) -> anyhow::Result<Box<dyn Provider>> {
    // Use explicit command/args from agent metadata if provided
    let command = agent.metadata.command.as_deref().or(tool.cli_command);
    let has_custom_args = agent.metadata.args.is_some();

    if let Some(command) = command.filter(|c| cli_available(c)) {
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
    } else if let Some(backend) = tool.api_backend {
        match command {
            Some(c) => tracing::info!(
                "Provider '{name}': CLI '{c}' not found, falling back to API ({backend})"
            ),
            None => tracing::info!("Provider '{name}': API-only, calling {backend}"),
        }
        create_api_provider(backend, agent)
    } else {
        // No API to fall back to. Say which binary was looked for and how to
        // point at it — an arbitrary backend here would answer a missing
        // binary with "no API key found", which is the wrong hunt.
        //
        // `expect`: a `ToolDef` with neither a CLI command nor an API
        // backend could only ever fail, and the table declares none — see
        // `every_known_tool_declares_at_least_one_backing`.
        let declared = tool
            .cli_command
            .expect("a tool with no API backend must declare a CLI command");
        let command = command.unwrap_or(declared);
        anyhow::bail!(
            "Provider '{name}' runs the `{command}` CLI, which was not found on PATH, \
             and it has no API backend to fall back to. Install it, or point this \
             agent at the executable with `command: /full/path/to/{declared}`."
        )
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
            if let Some(url) = resolve_base_url("anthropic", "ANTHROPIC_BASE_URL") {
                p.base_url = url;
            }
            Ok(Box::new(p))
        }
        "google" => {
            let api_key = get_api_key("GOOGLE_API_KEY", "google")?;
            let mut p = super::api::google::GoogleProvider::new(api_key);
            if let Some(url) = resolve_base_url("google", "GOOGLE_BASE_URL") {
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

/// Resolve an API provider's base URL: the environment variable first, then
/// `providers.yaml`'s `providers.<key>.base_url`.
///
/// All four API providers go through this. They did not at first: `openai`
/// and `proxy` read the file while `anthropic` and `google` kept reading
/// only their env var — which turned a file `armadai init` writes with a
/// `base_url` for **all four** (`DEFAULT_PROVIDERS_YAML`) from uniformly
/// decorative into honoured-for-two-silently-ignored-for-two. A file whose
/// keys work for half its entries is a worse trap than one whose keys work
/// for none, so the reading was widened rather than the file trimmed.
///
/// A blank value is not a configuration, in either source. The env-var-only
/// version accepted one (`if let Ok(url) = var(..)`), so
/// `ANTHROPIC_BASE_URL=""` used to blank the vendor URL and every call then
/// failed against a relative path; it is now ignored.
///
/// Documented in `docs/wiki/providers.md`.
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
    use armadai_core::agent::AgentMetadata;

    /// An agent carrying nothing but the two fields provider resolution
    /// reads: `provider:` and (optionally) `command:`.
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
    fn find_known_tools() {
        assert!(find_tool("claude").is_some());
        assert!(find_tool("gemini").is_some());
        assert!(find_tool("gpt").is_some());
        assert!(find_tool("aider").is_some());
        assert!(find_tool("unknown").is_none());
    }

    /// The three CLIs `armadai link` already writes a config for and the
    /// shell already relays in JSON mode were absent from `KNOWN_TOOLS`, so
    /// `armadai run` answered `Unknown provider: 'codex'` (issue #369).
    ///
    /// `command: echo` pins the CLI branch without depending on whether the
    /// real tool happens to be installed on the machine running the suite.
    #[test]
    fn the_cli_only_tools_are_providers_run_accepts() {
        for tool in ["codex", "copilot", "opencode"] {
            let provider = create_provider(&agent_with(tool, Some("echo")))
                .unwrap_or_else(|e| panic!("provider '{tool}' must build: {e}"));
            assert_eq!(
                provider.metadata().name,
                "cli:echo",
                "provider '{tool}' must resolve to the CLI branch"
            );
        }
    }

    /// Their canonical argv is the one `json_runner` already carries — the
    /// whole reason `provider: cli` + `command: codex` is not an equivalent
    /// workaround, since that path passes no flags at all and `codex` with a
    /// bare positional opens its interactive UI.
    #[test]
    fn a_cli_only_tool_gets_its_canonical_json_argv() {
        for (tool, expected) in [
            ("codex", vec!["exec", "--json"]),
            ("copilot", vec!["--output-format", "json", "-p"]),
            ("opencode", vec!["run", "--format", "json"]),
        ] {
            let def = find_tool(tool).unwrap_or_else(|| panic!("{tool} must be a known tool"));
            let cli = def
                .cli_command
                .unwrap_or_else(|| panic!("{tool} must declare a CLI command"));
            assert_eq!(
                crate::json_runner::json_mode_args(cli),
                expected,
                "{tool} must be spawned with the argv the shell relay already uses"
            );
        }
    }

    /// A CLI-only tool has no API equivalent ArmadAI can reach. With the
    /// binary missing the answer must name the binary and say so — not
    /// dead-end in an API fallback that can only fail on a missing key.
    #[test]
    fn a_missing_cli_only_binary_is_reported_as_such() {
        let missing = "this_command_does_not_exist_xyz";
        assert!(!cli_available(missing));

        for tool in ["codex", "copilot", "opencode"] {
            let err = match create_provider(&agent_with(tool, Some(missing))) {
                Ok(_) => panic!("'{tool}' built a provider with no binary present"),
                Err(e) => e.to_string(),
            };
            assert!(
                err.contains(missing),
                "must name the binary looked for: {err}"
            );
            assert!(
                err.contains("command:"),
                "must say how to point at the binary: {err}"
            );
            assert!(
                !err.contains("Unknown provider"),
                "'{tool}' is a known provider: {err}"
            );
            assert!(
                !err.contains("API_KEY"),
                "'{tool}' has no API fallback, so no key is the answer: {err}"
            );
        }
    }

    /// The list `create_provider` advertises must be the list it accepts.
    ///
    /// Held as a literal on purpose: deriving the expectation from
    /// `accepted_provider_names()` would make the test agree with any drift
    /// of that function, which is the defect (`proxy` was accepted by
    /// `create_provider` and missing from the message for as long as the
    /// message was a hand-typed string).
    #[test]
    fn the_advertised_provider_names_are_exactly_the_accepted_ones() {
        let expected = [
            "cli",
            "anthropic",
            "openai",
            "google",
            "proxy",
            "claude",
            "gemini",
            "gpt",
            "aider",
            "codex",
            "copilot",
            "opencode",
        ];
        let mut advertised = accepted_provider_names();
        advertised.sort_unstable();
        let mut want = expected.to_vec();
        want.sort_unstable();
        assert_eq!(advertised, want);

        let message = match create_provider(&agent_with("nope-not-a-provider", None)) {
            Ok(_) => panic!("a bogus provider name must be refused"),
            Err(e) => e.to_string(),
        };
        for name in expected {
            assert!(
                message.contains(name),
                "the refusal must advertise '{name}': {message}"
            );
        }
    }

    /// Every advertised name must reach its own branch: none of them may
    /// come back as `Unknown provider`. The API ones legitimately fail on a
    /// missing key here; that is a different, actionable answer.
    #[test]
    fn no_advertised_provider_name_falls_through_to_unknown() {
        for name in accepted_provider_names() {
            if let Err(e) = create_provider(&agent_with(name, Some("echo"))) {
                let msg = e.to_string();
                assert!(
                    !msg.contains("Unknown provider"),
                    "'{name}' is advertised but not accepted: {msg}"
                );
            }
        }
    }

    /// The refusal must teach the escape hatch: any binary at all is
    /// runnable through `provider: cli`, which the old message never said.
    #[test]
    fn the_refusal_teaches_the_generic_cli_escape_hatch() {
        let message = match create_provider(&agent_with("some-other-tool", None)) {
            Ok(_) => panic!("a bogus provider name must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(
            message.contains("provider: cli"),
            "must name the escape hatch: {message}"
        );
        assert!(
            message.contains("command:"),
            "must name the field it needs: {message}"
        );
    }

    #[test]
    fn cli_available_echo() {
        // echo should be available on all systems
        assert!(cli_available("echo"));
        assert!(!cli_available("this_command_does_not_exist_xyz"));
    }

    // ── API-only tools never probe `PATH` (issue #402) ───────────────

    /// Plant an executable named `name` in `dir` that answers nothing —
    /// enough for `which` to find it, which is all `cli_available` asks.
    #[cfg(feature = "api")]
    fn plant_stub_binary(dir: &std::path::Path, name: &str) {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// A `PATH` holding only `dir` plus the system directories `which`
    /// itself lives in.
    #[cfg(feature = "api")]
    fn path_with(dir: &std::path::Path) -> String {
        format!("{}:/usr/bin:/bin", dir.display())
    }

    /// `provider: gpt` must reach OpenAI even when some unrelated binary
    /// named `gpt` sits on `PATH`.
    ///
    /// Measured on macOS 25.5 (issue #402): `/usr/sbin/gpt` is the system
    /// GUID-partition-table tool, present on **every** macOS machine, so
    /// `cli_available("gpt")` was unconditionally true, the OpenAI fallback
    /// was never reached, and `armadai run` on a `provider: gpt` agent
    /// answered `Error: CLI command failed (exit status: 1): gpt: unknown
    /// command: <system>…` — a message from a disk-partitioning utility.
    ///
    /// The stub binary is what makes this a test rather than a platform
    /// observation: it reproduces the collision on Linux CI too, where no
    /// `gpt` exists and the defect is invisible.
    #[cfg(feature = "api")]
    #[test]
    fn gpt_reaches_its_api_even_with_a_same_named_binary_on_path() {
        let bin = tempfile::tempdir().unwrap();
        plant_stub_binary(bin.path(), "gpt");
        let _iso = armadai_core::test_support::IsolatedConfigDir::enter()
            .with_var("PATH", Some(&path_with(bin.path())))
            .with_var("OPENAI_API_KEY", Some("sk-not-a-real-key"))
            .with_var("OPENAI_BASE_URL", None);

        assert!(
            cli_available("gpt"),
            "fixture is degenerate: the stub must be visible to the PATH probe"
        );
        let provider = create_provider(&agent_with("gpt", None))
            .unwrap_or_else(|e| panic!("provider 'gpt' must build: {e}"));
        assert_eq!(
            provider.metadata().name,
            "openai",
            "'gpt' must resolve to the OpenAI API, not to whatever `gpt` PATH happens to hold"
        );
    }

    /// The escape hatch survives: an agent that really does have a CLI it
    /// wants to call under `provider: gpt` says so with `command:`, and that
    /// still wins over the API. Guards the fix against over-reaching into
    /// "API-only means the agent's own `command:` is ignored".
    #[test]
    fn an_explicit_command_still_runs_a_cli_for_an_api_only_tool() {
        let provider = create_provider(&agent_with("gpt", Some("echo")))
            .unwrap_or_else(|e| panic!("provider 'gpt' with an explicit command must build: {e}"));
        assert_eq!(provider.metadata().name, "cli:echo");
    }

    /// A tool with neither a CLI command nor an API backend can only ever
    /// fail, so the table must not be able to declare one.
    #[test]
    fn every_known_tool_declares_at_least_one_backing() {
        for (name, def) in KNOWN_TOOLS {
            assert!(
                def.cli_command.is_some() || def.api_backend.is_some(),
                "'{name}' declares neither a CLI command nor an API backend"
            );
        }
    }

    /// The inventory itself, pinned. `gpt` is API-only *because* its name
    /// collides with a system binary; the other six own their names (checked
    /// against `/bin`, `/sbin`, `/usr/bin`, `/usr/sbin` on macOS 25.5 and
    /// against `debian:stable-slim`: only `gpt` collides).
    #[test]
    fn only_gpt_is_api_only() {
        let api_only: Vec<&str> = KNOWN_TOOLS
            .iter()
            .filter(|(_, def)| def.cli_command.is_none())
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(api_only, vec!["gpt"]);
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
        // CLI-only tools bill through their own subscription, not through an
        // ArmadAI-held API key: giving them a backend would also give them a
        // shared quota bucket they never draw on.
        for tool in ["codex", "copilot", "opencode"] {
            assert_eq!(rate_limit_key(tool), None, "{tool}");
            assert_eq!(api_backend_for_tool(tool), None, "{tool}");
        }
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
            vars.iter()
                .fold(IsolatedConfigDir::enter(), |scope, (name, value)| {
                    scope.with_var(name, *value)
                })
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
        ///
        /// Before this went through `resolve_base_url`, `anthropic`'s
        /// `if let Ok(url) = var(..)` accepted the empty string and blanked
        /// the vendor URL with it.
        #[test]
        fn an_empty_base_url_env_var_is_ignored() {
            // One guard at a time: `env_scope` takes the shared env lock, and
            // a second one on the same thread would deadlock on it.
            {
                let _env = env_scope(&[("PROXY_BASE_URL", Some("   "))]);
                assert_eq!(resolve_base_url("proxy", "PROXY_BASE_URL"), None);
            }
            {
                let _env = env_scope(&[("ANTHROPIC_BASE_URL", Some(""))]);
                assert_eq!(resolve_base_url("anthropic", "ANTHROPIC_BASE_URL"), None);
            }
        }

        /// `armadai init` writes a `base_url` for all four providers
        /// (`DEFAULT_PROVIDERS_YAML`). Two of them honouring it and two
        /// ignoring it is the trap; this pins that all four read the file.
        #[test]
        fn providers_yaml_base_url_is_honoured_for_every_api_provider() {
            let env = env_scope(&[
                ("ANTHROPIC_BASE_URL", None),
                ("GOOGLE_BASE_URL", None),
                ("OPENAI_BASE_URL", None),
                ("PROXY_BASE_URL", None),
            ]);
            std::fs::write(
                env.config_dir().join("providers.yaml"),
                "providers:\n  \
                 anthropic:\n    base_url: http://anthropic.test/v1\n  \
                 google:\n    base_url: http://google.test/v1\n  \
                 openai:\n    base_url: http://openai.test/v1\n  \
                 proxy:\n    base_url: http://proxy.test/v1\n",
            )
            .expect("write providers.yaml");

            for (key, env_var, expected) in [
                (
                    "anthropic",
                    "ANTHROPIC_BASE_URL",
                    "http://anthropic.test/v1",
                ),
                ("google", "GOOGLE_BASE_URL", "http://google.test/v1"),
                ("openai", "OPENAI_BASE_URL", "http://openai.test/v1"),
                ("proxy", "PROXY_BASE_URL", "http://proxy.test/v1"),
            ] {
                assert_eq!(
                    resolve_base_url(key, env_var).as_deref(),
                    Some(expected),
                    "{key} must read its providers.yaml base_url"
                );
            }
        }

        fn probe_request() -> armadai_core::provider::CompletionRequest {
            armadai_core::provider::CompletionRequest {
                model: "probe".to_string(),
                system_prompt: String::new(),
                messages: vec![armadai_core::provider::ChatMessage {
                    role: "user".to_string(),
                    content: "hi".to_string(),
                }],
                temperature: 0.0,
                max_tokens: None,
            }
        }

        /// The whole reason the file is read: a provider built through the
        /// factory must actually **talk to** the URL the file supplies.
        ///
        /// Asserting that `resolve_base_url` returns it proves only that the
        /// helper works — the arm can still ignore it. Measured: the first
        /// version of this test did exactly that and survived the mutation
        /// restoring `anthropic`'s env-var-only read. So the assertion is on
        /// a real socket: the scripted server must receive the call.
        #[tokio::test]
        #[allow(clippy::await_holding_lock)]
        async fn an_anthropic_provider_really_calls_the_file_supplied_base_url() {
            use crate::api::test_server::{ScriptedResponse, ScriptedServer};

            let server = ScriptedServer::start(vec![ScriptedResponse::body(
                200,
                r#"{"content":[{"type":"text","text":"ok"}],"model":"m",
                    "usage":{"input_tokens":1,"output_tokens":1}}"#,
            )]);
            let env = env_scope(&[
                ("ANTHROPIC_BASE_URL", None),
                ("ANTHROPIC_API_KEY", Some("sk-ant-test")),
            ]);
            std::fs::write(
                env.config_dir().join("providers.yaml"),
                format!("providers:\n  anthropic:\n    base_url: {}\n", server.url()),
            )
            .expect("write providers.yaml");

            let provider =
                create_provider(&agent_with("anthropic", None)).expect("anthropic must build");
            // The answer itself is irrelevant; where the call went is not.
            let _ = provider.complete(probe_request()).await;

            assert_eq!(
                server.request_count(),
                1,
                "the factory-built provider never called the base URL providers.yaml supplies"
            );
            let raw = server.request(0).expect("one request received");
            assert!(raw.contains("POST /messages "), "wrong path in:\n{raw}");
        }

        /// Same proof for `google`: a second arm, a second chance to ignore
        /// the file.
        #[tokio::test]
        #[allow(clippy::await_holding_lock)]
        async fn a_google_provider_really_calls_the_file_supplied_base_url() {
            use crate::api::test_server::{ScriptedResponse, ScriptedServer};

            let server = ScriptedServer::start(vec![ScriptedResponse::body(
                200,
                r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}"#,
            )]);
            let env = env_scope(&[
                ("GOOGLE_BASE_URL", None),
                ("GOOGLE_API_KEY", Some("g-test")),
            ]);
            std::fs::write(
                env.config_dir().join("providers.yaml"),
                format!("providers:\n  google:\n    base_url: {}\n", server.url()),
            )
            .expect("write providers.yaml");

            let provider = create_provider(&agent_with("google", None)).expect("google must build");
            let _ = provider.complete(probe_request()).await;

            assert_eq!(
                server.request_count(),
                1,
                "the factory-built provider never called the base URL providers.yaml supplies"
            );
            let raw = server.request(0).expect("one request received");
            assert!(raw.contains("POST /models/probe:"), "wrong path in:\n{raw}");
        }
    }
}
