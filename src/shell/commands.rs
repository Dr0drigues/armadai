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
        name: "providers",
        aliases: &["p"],
        description: "List available providers",
    },
    SlashCommand {
        name: "switch",
        aliases: &["sw"],
        description: "Switch provider (e.g. /switch claude)",
    },
    SlashCommand {
        name: "sessions",
        aliases: &["ss"],
        description: "List saved sessions",
    },
    SlashCommand {
        name: "resume",
        aliases: &["r"],
        description: "Resume a saved session (e.g. /resume 20260401_1430)",
    },
    SlashCommand {
        name: "save",
        aliases: &[],
        description: "Force save current session",
    },
    SlashCommand {
        name: "tandem",
        aliases: &["t"],
        description: "Send next message to N providers in parallel (e.g. /tandem gemini,claude)",
    },
    SlashCommand {
        name: "pipeline",
        aliases: &["pipe"],
        description: "Chain providers: A generates, B reviews (e.g. /pipeline gemini,claude)",
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
    /// Switch to a different provider
    SwitchProvider(String),
    /// Resume a saved session
    ResumeSession(String),
    /// Save current session
    SaveSession,
    /// Tandem mode: send next message to multiple providers in parallel
    Tandem(Vec<String>),
    /// Pipeline mode: chain providers sequentially (A generates → B reviews)
    Pipeline(Vec<String>),
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
    shell_config: &super::config::ShellConfig,
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
        Some(c) if c.name == "providers" => Some(CommandResult::Display(format_providers())),
        Some(c) if c.name == "switch" => {
            let arg = trimmed[1..].split_whitespace().nth(1).unwrap_or("");
            Some(CommandResult::SwitchProvider(arg.to_string()))
        }
        Some(c) if c.name == "sessions" => Some(CommandResult::Display(format_sessions())),
        Some(c) if c.name == "resume" => {
            let arg = trimmed[1..].split_whitespace().nth(1).unwrap_or("");
            if arg.is_empty() {
                Some(CommandResult::Display(
                    "Usage: /resume <session-id>\nExample: /resume 20260401_1430\n\nUse /sessions to see available sessions.".to_string()
                ))
            } else {
                Some(CommandResult::ResumeSession(arg.to_string()))
            }
        }
        Some(c) if c.name == "save" => Some(CommandResult::SaveSession),
        Some(c) if c.name == "tandem" => {
            let arg = trimmed[1..].split_whitespace().nth(1).unwrap_or("");
            if arg.is_empty() {
                // Use config defaults if available
                if !shell_config.tandem.is_empty() {
                    let providers = shell_config.tandem.iter().map(|e| e.provider.clone()).collect();
                    Some(CommandResult::Tandem(providers))
                } else {
                    Some(CommandResult::Display(
                        "# Tandem Mode\n\nUsage: `/tandem provider1,provider2`\n\nOr configure defaults in `armadai.yaml`:\n\n```yaml\nshell:\n  tandem:\n    - provider: gemini\n      model: latest:fast\n    - provider: claude\n      model: latest:pro\n```\n\nUse `/providers` to see available providers.".to_string()
                    ))
                }
            } else {
                let providers: Vec<String> = arg.split(',').map(|s| s.trim().to_string()).collect();
                Some(CommandResult::Tandem(providers))
            }
        }
        Some(c) if c.name == "pipeline" => {
            let arg = trimmed[1..].split_whitespace().nth(1).unwrap_or("");
            if arg.is_empty() {
                // Use config defaults if available
                if let Some(ref pipeline) = shell_config.pipeline {
                    if !pipeline.steps.is_empty() {
                        // Flatten step providers into a list for the pipeline executor
                        let providers = pipeline.steps.iter()
                            .flat_map(|step| step.providers.iter().map(|e| e.provider.clone()))
                            .collect();
                        Some(CommandResult::Pipeline(providers))
                    } else {
                        Some(CommandResult::Display("Pipeline configured but has no steps.".to_string()))
                    }
                } else {
                    Some(CommandResult::Display(
                        "# Pipeline Mode\n\nUsage: `/pipeline provider1,provider2`\n\nOr configure defaults in `armadai.yaml`:\n\n```yaml\nshell:\n  pipeline:\n    steps:\n      - name: analyze\n        prompt: \"Analyze this request\"\n        providers:\n          - provider: gemini\n      - name: review\n        prompt: \"Review the analysis\"\n        providers:\n          - provider: claude\n```\n\nUse `/providers` to see available providers.".to_string()
                    ))
                }
            } else {
                let providers: Vec<String> = arg.split(',').map(|s| s.trim().to_string()).collect();
                Some(CommandResult::Pipeline(providers))
            }
        }
        Some(c) if c.name == "quit" => Some(CommandResult::Quit),
        _ => Some(CommandResult::Display(format!(
            "Unknown command: /{cmd_part}\nType /help for available commands."
        ))),
    }
}

/// Format help text showing all available commands
fn format_help() -> String {
    let mut text = "# Available Commands\n\n".to_string();

    for cmd in COMMANDS {
        let aliases = if cmd.aliases.is_empty() {
            String::new()
        } else {
            format!(" *({})*", cmd.aliases.join(", "))
        };
        text.push_str(&format!(
            "- **/{name}**{aliases} — {desc}\n",
            name = cmd.name,
            aliases = aliases,
            desc = cmd.description,
        ));
    }

    text
}

/// Format cost summary
fn format_cost(runner: &ShellRunner) -> String {
    let metrics = runner.session_metrics();

    let mut text = "# Session Cost Summary\n\n".to_string();
    text.push_str(&format!("- **Turns:** {}\n", metrics.turn_count));
    text.push_str(&format!(
        "- **Tokens in:** {} *(~estimated)*\n",
        metrics.total_tokens_in
    ));
    text.push_str(&format!(
        "- **Tokens out:** {} *(~estimated)*\n",
        metrics.total_tokens_out
    ));
    text.push_str(&format!(
        "- **Est. cost:** ${:.6}\n",
        metrics.total_cost_estimate
    ));
    text
}

/// Format agents list with orchestration organization
fn format_agents() -> String {
    let mut text = String::new();

    // Try to read orchestration config for team structure
    let config_content = std::fs::read_to_string(".armadai/config.yaml")
        .or_else(|_| std::fs::read_to_string("armadai.yaml"))
        .unwrap_or_default();

    let coordinator = parse_field_from_yaml(&config_content, "coordinator");
    let teams = parse_teams_from_yaml(&config_content);
    let agents = list_agents_from_config();

    if agents.is_empty() {
        text.push_str("(no agents found in project config)\n");
        return text;
    }

    // If we have orchestration info, show the org chart
    if let Some(ref coord) = coordinator {
        let pattern = parse_field_from_yaml(&config_content, "pattern")
            .unwrap_or_else(|| "hierarchical".to_string());
        text.push_str("# Orchestration\n\n");
        text.push_str(&format!("- **Pattern:** {}\n\n", pattern));
        text.push_str(&format!("### 🎯 Coordinator: {}\n\n", coord));

        if !teams.is_empty() {
            for (i, team) in teams.iter().enumerate() {
                if let Some(ref lead) = team.lead {
                    text.push_str(&format!("### 📋 Team {} — Lead: {}\n", i + 1, lead));
                } else {
                    text.push_str(&format!("### 👥 Team {} — Direct reports\n", i + 1));
                }
                for agent in &team.agents {
                    text.push_str(&format!("- 🔧 {}\n", agent));
                }
                text.push('\n');
            }
        } else {
            text.push_str("### 👥 Agents\n");
            for agent in &agents {
                if Some(agent) != coordinator.as_ref() {
                    text.push_str(&format!("- 🔧 {}\n", agent));
                }
            }
        }

        text.push_str(&format!("**Total:** {} agent(s)\n", agents.len()));
    } else {
        // No orchestration — flat list
        text.push_str("# Available Agents\n\n");
        for agent in &agents {
            text.push_str(&format!("- {}\n", agent));
        }
        text.push_str(&format!("\n**Total:** {} agent(s)\n", agents.len()));
    }

    text
}

/// Simple YAML field parser
fn parse_field_from_yaml(content: &str, field: &str) -> Option<String> {
    let prefix = format!("{}:", field);
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&prefix) {
            let value = trimmed[prefix.len()..]
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Parse teams from orchestration YAML config
fn parse_teams_from_yaml(content: &str) -> Vec<TeamInfo> {
    let mut teams = Vec::new();
    let mut in_teams = false;
    let mut current_lead: Option<String> = None;
    let mut current_agents: Vec<String> = Vec::new();
    let mut in_agents_list = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("teams:") {
            in_teams = true;
            continue;
        }

        if !in_teams {
            continue;
        }

        // Exit teams on next top-level key
        if !trimmed.is_empty()
            && !trimmed.starts_with('-')
            && !trimmed.starts_with(' ')
            && !trimmed.starts_with('\t')
            && !trimmed.starts_with("lead:")
            && !trimmed.starts_with("agents:")
        {
            break;
        }

        if trimmed.starts_with("- lead:") || trimmed.starts_with("- agents:") {
            // Save previous team
            if !current_agents.is_empty() || current_lead.is_some() {
                teams.push(TeamInfo {
                    lead: current_lead.take(),
                    agents: std::mem::take(&mut current_agents),
                });
            }
            in_agents_list = false;
        }

        if trimmed.starts_with("- lead:") {
            let val = trimmed
                .strip_prefix("- lead:")
                .unwrap()
                .trim()
                .trim_matches('"');
            if !val.is_empty() {
                current_lead = Some(val.to_string());
            }
        } else if trimmed.contains("agents:") {
            in_agents_list = true;
        } else if in_agents_list && trimmed.starts_with("- ") {
            let agent = trimmed.strip_prefix("- ").unwrap().trim().trim_matches('"');
            if !agent.is_empty() && !agent.contains(':') {
                current_agents.push(agent.to_string());
            }
        }
    }

    // Save last team
    if !current_agents.is_empty() || current_lead.is_some() {
        teams.push(TeamInfo {
            lead: current_lead,
            agents: current_agents,
        });
    }

    teams
}

struct TeamInfo {
    lead: Option<String>,
    agents: Vec<String>,
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
    let model_display = if model.is_empty() { "(not set)" } else { model };

    let pricing = match (
        provider.to_lowercase().as_str(),
        model.to_lowercase().as_str(),
    ) {
        ("gemini", m) if m.contains("flash") => "$0.075/1M in, $0.30/1M out",
        ("gemini", m) if m.contains("pro") => "$1.25/1M in, $10.00/1M out",
        ("claude", m) if m.contains("sonnet") => "$3.00/1M in, $15.00/1M out",
        ("claude", m) if m.contains("opus") => "$15.00/1M in, $75.00/1M out",
        ("claude", m) if m.contains("haiku") => "$0.80/1M in, $4.00/1M out",
        ("openai", m) if m.contains("gpt-4o") => "$2.50/1M in, $10.00/1M out",
        _ => "(unknown)",
    };

    let mut text = "# Current Model\n\n".to_string();
    text.push_str(&format!("- **Provider:** {}\n", provider));
    text.push_str(&format!("- **Model:** {}\n", model_display));
    text.push_str(&format!("- **Pricing:** {}\n", pricing));
    text
}

/// Format providers list
fn format_providers() -> String {
    use super::detect::list_providers;

    let providers = list_providers();
    let mut text = "# Available Providers\n\n".to_string();

    if providers.is_empty() {
        text.push_str("*(no providers found)*\n");
        return text;
    }

    for provider in &providers {
        let status = if provider.available {
            "✓ available"
        } else {
            "✗ not installed"
        };

        text.push_str(&format!(
            "- **{}** ({}) — {}\n",
            provider.display_name, provider.command, status
        ));

        if provider.available && !provider.model_name.is_empty() {
            text.push_str(&format!("  Model: {}\n", provider.model_name));
        }
    }

    text.push_str("\nUse `/switch <provider>` to change provider (e.g., `/switch claude`)\n");
    text
}

/// Format conversation history
fn format_history(runner: &ShellRunner) -> String {
    let history = runner.history();

    let mut text = "# Conversation History\n\n".to_string();

    if history.is_empty() {
        text.push_str("*(no messages yet)*\n");
        return text;
    }

    let mut turn = 1;
    for msg in history {
        match msg.role {
            MessageRole::User => {
                let preview = if msg.content.len() > 80 {
                    format!("{}…", &msg.content[..80])
                } else {
                    msg.content.clone()
                };
                text.push_str(&format!("**Turn {}** — {}\n", turn, preview));
                turn += 1;
            }
            MessageRole::Assistant => {
                let preview = if msg.content.len() > 80 {
                    format!("{}…", &msg.content[..80])
                } else {
                    msg.content.clone()
                };
                text.push_str(&format!("- → {}\n", preview));
            }
            MessageRole::System => {}
        }
    }

    text
}

/// Format sessions list
fn format_sessions() -> String {
    use super::session::{format_relative_time, list_sessions};

    let sessions = list_sessions();
    let mut text = "# Saved Sessions\n\n".to_string();

    if sessions.is_empty() {
        text.push_str("*(no saved sessions)*\n\n");
        text.push_str("Sessions are automatically saved as you chat.\n");
        text.push_str("Use `/resume <session-id>` to restore a previous conversation.\n");
        return text;
    }

    for session in &sessions {
        let project = session
            .project_dir
            .split('/')
            .next_back()
            .unwrap_or(&session.project_dir);
        let relative = format_relative_time(&session.updated_at);

        text.push_str(&format!(
            "- **{}** — {} ({} turns, ${:.4}) — {}\n",
            session.id, project, session.turn_count, session.total_cost, relative
        ));
    }

    text.push_str("\nUse `/resume <session-id>` to restore a session.\n");
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
        let shell_config = crate::shell::config::ShellConfig::default();
        let result = try_execute("/unknown", &runner, "Gemini", "gemini-2.5-flash", &shell_config);
        assert!(result.is_some());
        match result.unwrap() {
            CommandResult::Display(text) => {
                assert!(text.contains("Unknown command"));
            }
            _ => panic!("Expected Display"),
        }
    }
}
