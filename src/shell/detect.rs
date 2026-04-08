//! Provider detection for the shell runner
//!
//! Auto-detects available CLI tools and returns a ready-to-use RunnerConfig.

use super::runner::RunnerConfig;
use std::time::Duration;

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
    // Try each provider in order of preference
    let providers = vec![
        ("gemini", vec!["-p"]),
        ("claude", vec![]),
        ("aider", vec!["--yes"]),
        ("codex", vec![]),
    ];

    for (command, args) in providers {
        if is_command_available(command) {
            return Some(RunnerConfig {
                command: command.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                max_history_turns: 5,
                timeout: Duration::from_secs(120),
            });
        }
    }

    None
}

/// Check if a command is available in PATH.
fn is_command_available(command: &str) -> bool {
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

/// Get the provider display name for the statusbar.
pub fn provider_display_name(command: &str) -> &str {
    match command {
        "gemini" => "Gemini",
        "claude" => "Claude",
        "aider" => "Aider",
        "codex" => "Codex",
        _ => command,
    }
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
