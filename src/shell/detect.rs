//! Provider detection for the shell runner
//!
//! Auto-detects available CLI tools and returns a ready-to-use RunnerConfig.

use super::runner::RunnerConfig;
use std::time::Duration;

/// Information about a provider's availability and configuration.
#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub command: String,
    pub args: Vec<String>,
    pub display_name: String,
    pub model_name: String,
    pub available: bool,
}

/// Detect the best available CLI tool and return a RunnerConfig.
///
/// Checks in order of preference:
/// 1. gemini (Google Gemini CLI)
/// 2. claude (Anthropic Claude CLI)
/// 3. aider (Aider CLI)
/// 4. codex (OpenAI Codex CLI)
///
/// Returns None if no supported CLI is found.
pub fn detect_provider() -> Option<RunnerConfig> {
    let providers = ["gemini", "claude", "codex", "copilot", "opencode", "aider"];

    for command in providers {
        if is_command_available(command) {
            // Use JSON mode args if supported, otherwise text fallback
            let args = super::json_runner::json_mode_args(command);
            return Some(RunnerConfig {
                command: command.to_string(),
                args,
                max_history_turns: 5,
                timeout: Duration::from_secs(120),
            });
        }
    }

    None
}

/// List all known providers with their availability.
///
/// Returns a vector of all providers we know how to work with,
/// including whether each one is currently available (installed in PATH).
pub fn list_providers() -> Vec<ProviderInfo> {
    let providers = [
        ("gemini", "Gemini"),
        ("claude", "Claude"),
        ("codex", "Codex"),
        ("copilot", "Copilot"),
        ("opencode", "OpenCode"),
        ("aider", "Aider"),
    ];

    providers
        .iter()
        .map(|(cmd, name)| {
            let available = is_command_available(cmd);
            ProviderInfo {
                command: cmd.to_string(),
                args: super::json_runner::json_mode_args(cmd),
                display_name: name.to_string(),
                model_name: if available {
                    detect_model_name(cmd)
                } else {
                    String::new()
                },
                available,
            }
        })
        .collect()
}

/// Check if a command is available in PATH.
pub fn is_command_available(command: &str) -> bool {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("which")
            .arg(command)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        use std::process::Command;
        Command::new("where")
            .arg(command)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Fallback for other platforms - always return false
        let _ = command;
        false
    }
}

/// Get the base CLI args for a provider (uses JSON mode if available).
pub fn args_for_provider(command: &str) -> Vec<String> {
    super::json_runner::json_mode_args(command)
}

/// Get the provider display name for the statusbar.
pub fn provider_display_name(command: &str) -> &str {
    match command {
        "gemini" => "Gemini",
        "claude" => "Claude",
        "aider" => "Aider",
        "codex" => "Codex",
        "copilot" => "Copilot",
        "opencode" => "OpenCode",
        _ => command,
    }
}

/// Try to detect the model name for a given provider.
///
/// For Gemini: reads `.gemini/settings.json` or runs `gemini --version`.
/// For others: returns a default based on the provider.
pub fn detect_model_name(command: &str) -> String {
    match command {
        "gemini" => detect_gemini_model(),
        "claude" => "claude-sonnet-4-5".to_string(),
        "aider" => "gpt-4o".to_string(),
        "codex" => "codex".to_string(),
        _ => "unknown".to_string(),
    }
}

fn detect_gemini_model() -> String {
    // Try to read model from .gemini/settings.json
    if let Ok(content) = std::fs::read_to_string(".gemini/settings.json")
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
        && let Some(model) = json.get("model").and_then(|m| m.as_str())
    {
        return model.to_string();
    }
    "gemini-2.5-flash".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_display_name() {
        assert_eq!(provider_display_name("gemini"), "Gemini");
        assert_eq!(provider_display_name("claude"), "Claude");
        assert_eq!(provider_display_name("aider"), "Aider");
        assert_eq!(provider_display_name("codex"), "Codex");
        assert_eq!(provider_display_name("unknown"), "unknown");
    }

    #[test]
    fn test_detect_provider_returns_some() {
        // We can't guarantee which CLI is available in the test environment,
        // but we can test that the function doesn't panic
        let result = detect_provider();
        // On most systems, at least one basic command should be available
        // but we won't make assertions about which one
        let _ = result;
    }

    #[test]
    fn test_is_command_available_basic() {
        // Test with a command that should always exist
        #[cfg(unix)]
        assert!(is_command_available("ls"));

        #[cfg(windows)]
        assert!(is_command_available("cmd"));

        // Test with a command that should never exist
        assert!(!is_command_available(
            "this-command-definitely-does-not-exist-12345"
        ));
    }

    #[test]
    fn test_runner_config_has_correct_fields() {
        // If we can detect a provider, verify it has the expected structure
        if let Some(config) = detect_provider() {
            assert!(!config.command.is_empty());
            assert_eq!(config.max_history_turns, 5);
            assert_eq!(config.timeout, Duration::from_secs(120));
        }
    }
}
