//! Slash commands for the ArmadAI shell TUI
//!
//! Provides slash command support for shell-internal operations like help, clear, costs, etc.

#![cfg(feature = "tui")]

use super::runner::{MessageRole, ShellRunner};
use std::path::PathBuf;

/// A slash command definition
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
}

/// All available commands
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        aliases: &["h", "?"],
        description: "Show available commands",
    },
    SlashCommand {
        name: "clear",
        aliases: &["c"],
        description: "Clear the conversation",
    },
    SlashCommand {
        name: "cost",
        aliases: &["costs"],
        description: "Show session cost summary",
    },
    SlashCommand {
        name: "agents",
        aliases: &["a"],
        description: "List available agents",
    },
    SlashCommand {
        name: "model",
        aliases: &["m"],
        description: "Show current model info",
    },
    SlashCommand {
        name: "history",
        aliases: &["hist"],
        description: "Show prompt history",
    },
    SlashCommand {
        name: "quit",
        aliases: &["q", "exit"],
        description: "Exit the shell",
    },
];

/// Result of executing a slash command
#[derive(Debug)]
pub enum CommandResult {
    /// Display text in the messages area
    Display(String),
    /// Clear conversation
    Clear,
    /// Exit shell
    Quit,
}

/// Find a command by name or alias
pub fn find_command(input: &str) -> Option<&'static SlashCommand> {
    COMMANDS
        .iter()
        .find(|cmd| cmd.name == input || cmd.aliases.contains(&input))
}

/// Try to parse and execute a slash command.
/// Returns Some(CommandResult) if the input was a command, None if it's a normal message.
pub fn try_execute(
    input: &str,
    runner: &ShellRunner,
    provider_name: &str,
    model_name: &str,
) -> Option<CommandResult> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let cmd_part = trimmed[1..].split_whitespace().next().unwrap_or("");

    match find_command(cmd_part) {
        Some(c) if c.name == "help" => Some(CommandResult::Display(format_help())),
        Some(c) if c.name == "clear" => Some(CommandResult::Clear),
        Some(c) if c.name == "cost" => Some(CommandResult::Display(format_cost(runner))),
        Some(c) if c.name == "agents" => Some(CommandResult::Display(format_agents())),
        Some(c) if c.name == "model" => Some(CommandResult::Display(format_model(
            provider_name,
            model_name,
        ))),
        Some(c) if c.name == "history" => Some(CommandResult::Display(format_history(runner))),
        Some(c) if c.name == "quit" => Some(CommandResult::Quit),
        _ => Some(CommandResult::Display(format!(
            "Unknown command: /{cmd_part}\nType /help for available commands."
        ))),
    }
}

/// Format help text showing all available commands
fn format_help() -> String {
    let mut text = "Available commands:\n".to_string();
    text.push_str("─".repeat(40).as_str());
    text.push('\n');

    for cmd in COMMANDS {
        text.push_str(&format!("  /{:<10}", cmd.name));
        if !cmd.aliases.is_empty() {
            text.push_str(&format!("({})", cmd.aliases.join(", ")));
        }
        text.push_str(&format!("  — {}\n", cmd.description));
    }

    text
}

/// Format cost summary
fn format_cost(runner: &ShellRunner) -> String {
    let metrics = runner.session_metrics();
    let provider_names = ["Gemini", "Claude", "OpenAI"];
    let provider_name = provider_names.first().copied().unwrap_or("Unknown");

    format!(
        "Session Cost Summary\n{}\nTurns:      {}\nTokens in:  {} (~estimated)\nTokens out: {} (~estimated)\nEst. cost:  ${:.6}\nProvider:   {}",
        "─".repeat(40),
        metrics.turn_count,
        metrics.total_tokens_in,
        metrics.total_tokens_out,
        metrics.total_cost_estimate,
        provider_name,
    )
}

/// Format agents list from config
fn format_agents() -> String {
    let mut text = "Available Agents\n".to_string();
    text.push_str("─".repeat(40).as_str());
    text.push('\n');

    // Try to find agents from project config
    let agents = list_agents_from_config();

    if agents.is_empty() {
        text.push_str("(no agents found in project config)\n");
    } else {
        for agent in agents {
            text.push_str(&format!("  • {}\n", agent));
        }
    }

    text
}

/// List agent names from armadai.yaml or .armadai/config.yaml
fn list_agents_from_config() -> Vec<String> {
    // Try .armadai/config.yaml first
    let config_paths = [
        PathBuf::from(".armadai/config.yaml"),
        PathBuf::from("armadai.yaml"),
        PathBuf::from(".gemini/agents"),
    ];

    for path in &config_paths {
        if path.ends_with("agents") && path.is_dir() {
            // List .md files in agents directory
            if let Ok(entries) = std::fs::read_dir(path) {
                let mut agents: Vec<String> = entries
                    .filter_map(|e| {
                        let entry = e.ok()?;
                        let path = entry.path();
                        if path.extension().is_some_and(|ext| ext == "md") {
                            path.file_stem()
                                .and_then(|stem| stem.to_str())
                                .map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                agents.sort();
                return agents;
            }
        } else if path.exists() && path.is_file() {
            // Try to parse YAML config for agents list
            if let Ok(content) = std::fs::read_to_string(path) {
                let agents = parse_agents_from_yaml(&content);
                if !agents.is_empty() {
                    return agents;
                }
            }
        }
    }

    Vec::new()
}

/// Parse agent names from YAML config content
fn parse_agents_from_yaml(content: &str) -> Vec<String> {
    let mut agents = Vec::new();

    let mut in_agents_section = false;
    for line in content.lines() {
        let trimmed = line.trim();

        // Check if we're entering the agents section
        if trimmed.starts_with("agents:") {
            in_agents_section = true;
            continue;
        }

        // If we hit another top-level key, exit agents section
        if in_agents_section
            && !trimmed.is_empty()
            && !trimmed.starts_with('-')
            && !trimmed.starts_with(' ')
            && !trimmed.starts_with('\t')
        {
            break;
        }

        // Parse agent entries
        if in_agents_section
            && trimmed.starts_with("- name:")
            && let Some(name) = trimmed.strip_prefix("- name:").map(|s| s.trim())
        {
            let clean_name = name.trim_matches('"').trim_matches('\'').to_string();
            if !clean_name.is_empty() {
                agents.push(clean_name);
            }
        }
    }

    agents
}

/// Format model info
fn format_model(provider: &str, model: &str) -> String {
    let mut text = "Current Model\n".to_string();
    text.push_str("─".repeat(40).as_str());
    text.push('\n');

    text.push_str(&format!("Provider:  {}\n", provider));
    text.push_str(&format!(
        "Model:     {}\n",
        if model.is_empty() { "(not set)" } else { model }
    ));

    // Add pricing info for common models
    match (
        provider.to_lowercase().as_str(),
        model.to_lowercase().as_str(),
    ) {
        ("gemini", "gemini-2.5-flash") | ("gemini", _) => {
            text.push_str("Pricing:   $0.075/1M tokens in, $0.30/1M tokens out\n");
        }
        ("claude", _) if model.contains("3.5") => {
            text.push_str("Pricing:   $3.00/1M tokens in, $15.00/1M tokens out\n");
        }
        ("claude", _) if model.contains("opus") => {
            text.push_str("Pricing:   $15.00/1M tokens in, $75.00/1M tokens out\n");
        }
        ("openai", _) if model.contains("gpt-4") => {
            text.push_str("Pricing:   $0.03/1K tokens in, $0.06/1K tokens out\n");
        }
        _ => {
            text.push_str("Pricing:   (unknown)\n");
        }
    }

    text
}

/// Format conversation history
fn format_history(runner: &ShellRunner) -> String {
    let mut text = "Conversation History\n".to_string();
    text.push_str("─".repeat(40).as_str());
    text.push('\n');

    let history = runner.history();
    if history.is_empty() {
        text.push_str("(no messages in history)\n");
        return text;
    }

    let mut turn = 1;
    for msg in history {
        match msg.role {
            MessageRole::User => {
                let preview = if msg.content.len() > 60 {
                    format!("{}…", &msg.content[..60])
                } else {
                    msg.content.clone()
                };
                text.push_str(&format!("Turn {}: You — {}\n", turn, preview));
                turn += 1;
            }
            MessageRole::Assistant => {
                let preview = if msg.content.len() > 60 {
                    format!("{}…", &msg.content[..60])
                } else {
                    msg.content.clone()
                };
                text.push_str(&format!("        → {}\n", preview));
            }
            MessageRole::System => {}
        }
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_command_by_name() {
        let cmd = find_command("help");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().name, "help");
    }

    #[test]
    fn test_find_command_by_alias() {
        let cmd = find_command("h");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().name, "help");

        let cmd = find_command("?");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().name, "help");
    }

    #[test]
    fn test_find_command_not_found() {
        let cmd = find_command("nonexistent");
        assert!(cmd.is_none());
    }

    #[test]
    fn test_parse_agents_from_yaml() {
        let yaml = r#"
agents:
  - name: dev-lead
  - name: core-specialist
  - name: ui-specialist

prompts: []
"#;
        let agents = parse_agents_from_yaml(yaml);
        assert_eq!(agents.len(), 3);
        assert!(agents.contains(&"dev-lead".to_string()));
        assert!(agents.contains(&"core-specialist".to_string()));
        assert!(agents.contains(&"ui-specialist".to_string()));
    }

    #[test]
    fn test_parse_agents_from_yaml_empty() {
        let yaml = "prompts: []";
        let agents = parse_agents_from_yaml(yaml);
        assert!(agents.is_empty());
    }

    #[test]
    fn test_try_execute_unknown_command() {
        let runner = ShellRunner::new(Default::default());
        let result = try_execute("/unknown", &runner, "Gemini", "gemini-2.5-flash");
        assert!(result.is_some());
        match result.unwrap() {
            CommandResult::Display(text) => {
                assert!(text.contains("Unknown command"));
            }
            _ => panic!("Expected Display"),
        }
    }
}
