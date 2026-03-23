//! Orchestration context injection into agent system prompts.
//!
//! When orchestration is active, each agent receives an `## Orchestration Protocol`
//! block appended to its system prompt, describing its role, available contacts,
//! delegation syntax, and communication rules.

use std::collections::HashMap;

use super::{OrchestrationConfig, OrchestrationPattern, TeamConfig};

/// Agent description used for context injection.
pub struct AgentInfo {
    pub name: String,
    pub description: Option<String>,
}

/// Role of an agent in the hierarchical topology.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AgentRole {
    Coordinator,
    Lead { team_index: usize },
    Agent { team_index: usize },
    DirectAgent, // agent in a leadless team, reports to coordinator
}

/// Build the orchestration protocol block to inject into an agent's system prompt.
///
/// Returns `None` if orchestration is not enabled or pattern is not hierarchical.
pub fn build_orchestration_prompt(
    agent_name: &str,
    config: &OrchestrationConfig,
    agents_info: &HashMap<String, AgentInfo>,
) -> Option<String> {
    if !config.enabled || config.pattern != OrchestrationPattern::Hierarchical {
        return None;
    }

    let coordinator = config.coordinator.as_deref()?;
    let role = determine_role(agent_name, coordinator, &config.teams);

    let mut prompt = String::new();
    prompt.push_str("\n\n## Orchestration Protocol\n\n");

    match role {
        AgentRole::Coordinator => {
            build_coordinator_prompt(&mut prompt, config, agents_info);
        }
        AgentRole::Lead { team_index } => {
            build_lead_prompt(
                &mut prompt,
                coordinator,
                &config.teams[team_index],
                agents_info,
            );
        }
        AgentRole::Agent { team_index } => {
            let team = &config.teams[team_index];
            build_agent_prompt(&mut prompt, agent_name, team, agents_info);
        }
        AgentRole::DirectAgent => {
            build_direct_agent_prompt(&mut prompt, agent_name, coordinator, config, agents_info);
        }
    }

    Some(prompt)
}

/// Determine the role of an agent in the hierarchy.
fn determine_role(agent_name: &str, coordinator: &str, teams: &[TeamConfig]) -> AgentRole {
    if agent_name == coordinator {
        return AgentRole::Coordinator;
    }

    for (i, team) in teams.iter().enumerate() {
        if team.lead.as_deref() == Some(agent_name) {
            return AgentRole::Lead { team_index: i };
        }
        if team.agents.iter().any(|a| a == agent_name) {
            if team.lead.is_some() {
                return AgentRole::Agent { team_index: i };
            } else {
                return AgentRole::DirectAgent;
            }
        }
    }

    // Unknown agent — treat as direct agent
    AgentRole::DirectAgent
}

fn build_coordinator_prompt(
    prompt: &mut String,
    config: &OrchestrationConfig,
    agents_info: &HashMap<String, AgentInfo>,
) {
    prompt.push_str(
        "You are the **coordinator** of this agent team. The user speaks directly to you.\n\n",
    );

    // Team roster
    prompt.push_str("### Your team\n\n");
    prompt.push_str("| Agent | Role | Type |\n");
    prompt.push_str("|-------|------|------|\n");

    for team in &config.teams {
        if let Some(ref lead) = team.lead {
            let desc = agent_description(lead, agents_info);
            prompt.push_str(&format!("| {lead} | {desc} | Lead |\n"));
        }
        for agent in &team.agents {
            if team.lead.is_none() {
                let desc = agent_description(agent, agents_info);
                prompt.push_str(&format!("| {agent} | {desc} | Direct agent |\n"));
            }
        }
    }

    // Delegation syntax
    push_delegation_syntax(prompt);

    // Rules
    prompt.push_str("### Rules\n\n");
    prompt.push_str("1. Analyze the user's request before delegating\n");
    prompt.push_str(
        "2. If the request is unclear, ask clarifying questions FIRST (do not delegate yet)\n",
    );
    prompt.push_str("3. Delegate to the most appropriate agent(s) based on their specialization\n");

    for team in &config.teams {
        if let Some(ref lead) = team.lead {
            let team_agents = team.agents.join(", ");
            prompt.push_str(&format!(
                "4. For tasks involving {team_agents}, delegate to @{lead} (not directly to agents)\n"
            ));
        }
    }

    prompt
        .push_str("5. When all subtask results are collected, synthesize a final answer (no @)\n");
    prompt.push_str("6. Never delegate the same subtask to multiple agents\n");
}

fn build_lead_prompt(
    prompt: &mut String,
    coordinator: &str,
    team: &TeamConfig,
    agents_info: &HashMap<String, AgentInfo>,
) {
    let lead_name = team.lead.as_deref().unwrap_or("unknown");
    prompt.push_str(&format!(
        "You are the **lead** of your team. You report to @{coordinator}.\n\n"
    ));

    // Team roster
    prompt.push_str("### Your team\n\n");
    prompt.push_str("| Agent | Role |\n");
    prompt.push_str("|-------|------|\n");
    for agent in &team.agents {
        let desc = agent_description(agent, agents_info);
        prompt.push_str(&format!("| {agent} | {desc} |\n"));
    }

    // Delegation syntax
    push_delegation_syntax(prompt);

    prompt.push_str(&format!(
        "To report results back to the coordinator:\n```\n@{coordinator}: [results summary]\n```\n\n"
    ));

    // Rules
    prompt.push_str("### Rules\n\n");
    prompt.push_str(&format!("1. You receive tasks from @{coordinator}\n"));
    prompt.push_str("2. Decompose the task if needed and delegate to your agents\n");
    prompt.push_str("3. Your agents can talk to each other using @peer-name: ...\n");
    prompt.push_str("4. Synthesize results from your team before reporting back\n");
    prompt.push_str(&format!(
        "5. Escalate to @{coordinator} if the task is outside your team's scope\n"
    ));

    let _ = lead_name; // used for context
}

fn build_agent_prompt(
    prompt: &mut String,
    agent_name: &str,
    team: &TeamConfig,
    agents_info: &HashMap<String, AgentInfo>,
) {
    let lead = team.lead.as_deref().unwrap_or("coordinator");

    prompt.push_str(&format!(
        "You are a **specialist agent**. You report to @{lead}.\n\n"
    ));

    // Peers
    let peers: Vec<&String> = team
        .agents
        .iter()
        .filter(|a| a.as_str() != agent_name)
        .collect();
    if !peers.is_empty() {
        prompt.push_str("### Your peers (same team — you can ask them questions)\n\n");
        prompt.push_str("| Agent | Role |\n");
        prompt.push_str("|-------|------|\n");
        for peer in &peers {
            let desc = agent_description(peer, agents_info);
            prompt.push_str(&format!("| {peer} | {desc} |\n"));
        }
        prompt.push('\n');
    }

    // Communication syntax
    prompt.push_str("### Communication syntax\n\n");
    if !peers.is_empty() {
        prompt.push_str("To ask a peer for information:\n```\n@peer-name: your question\n```\n\n");
    }
    prompt.push_str(&format!(
        "To report your results back to your lead:\n```\n@{lead}: [your results]\n```\n\n"
    ));

    // Rules
    prompt.push_str("### Rules\n\n");
    prompt.push_str(&format!("1. You receive tasks from @{lead}\n"));
    prompt.push_str("2. Complete the task using your expertise\n");
    if !peers.is_empty() {
        prompt.push_str("3. If you need information from a peer, use @peer-name: question\n");
    }
    prompt.push_str(&format!("4. Always report results back to @{lead}\n"));
    prompt.push_str("5. Do NOT delegate tasks — you are a specialist, not a coordinator\n");
    prompt.push_str(&format!(
        "6. If the task is outside your expertise, escalate to @{lead}\n"
    ));
}

fn build_direct_agent_prompt(
    prompt: &mut String,
    agent_name: &str,
    coordinator: &str,
    config: &OrchestrationConfig,
    agents_info: &HashMap<String, AgentInfo>,
) {
    prompt.push_str(&format!(
        "You are a **specialist agent** reporting directly to @{coordinator}.\n\n"
    ));

    // Find peers (other agents in leadless teams)
    let mut peers = Vec::new();
    for team in &config.teams {
        if team.lead.is_none() {
            for agent in &team.agents {
                if agent != agent_name {
                    peers.push(agent.as_str());
                }
            }
        }
    }

    if !peers.is_empty() {
        prompt.push_str("### Your peers\n\n");
        prompt.push_str("| Agent | Role |\n");
        prompt.push_str("|-------|------|\n");
        for peer in &peers {
            let desc = agent_description(peer, agents_info);
            prompt.push_str(&format!("| {peer} | {desc} |\n"));
        }
        prompt.push('\n');
    }

    prompt.push_str("### Communication syntax\n\n");
    prompt.push_str(&format!(
        "To report results:\n```\n@{coordinator}: [your results]\n```\n\n"
    ));

    prompt.push_str("### Rules\n\n");
    prompt.push_str(&format!("1. You receive tasks from @{coordinator}\n"));
    prompt.push_str("2. Complete the task using your expertise\n");
    prompt.push_str(&format!(
        "3. Always report results back to @{coordinator}\n"
    ));
    prompt.push_str("4. Do NOT delegate tasks — you are a specialist\n");
}

fn push_delegation_syntax(prompt: &mut String) {
    prompt.push_str("\n### Delegation syntax\n\n");
    prompt.push_str("To delegate a subtask, write on a new line:\n");
    prompt.push_str("```\n@agent-name: description of the subtask\n```\n\n");
    prompt.push_str("You can delegate to multiple agents in the same response.\n\n");
}

fn agent_description(name: &str, agents_info: &HashMap<String, AgentInfo>) -> String {
    agents_info
        .get(name)
        .and_then(|info| info.description.clone())
        .unwrap_or_else(|| "Specialist".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![
                TeamConfig {
                    lead: Some("java-lead".to_string()),
                    agents: vec!["java-arch".to_string(), "java-sec".to_string()],
                },
                TeamConfig {
                    lead: None,
                    agents: vec!["cloud-expert".to_string(), "ops-expert".to_string()],
                },
            ],
            ..Default::default()
        }
    }

    fn sample_agents_info() -> HashMap<String, AgentInfo> {
        let mut m = HashMap::new();
        m.insert(
            "java-lead".to_string(),
            AgentInfo {
                name: "java-lead".to_string(),
                description: Some("Java team coordinator".to_string()),
            },
        );
        m.insert(
            "java-arch".to_string(),
            AgentInfo {
                name: "java-arch".to_string(),
                description: Some("Architecture & design patterns".to_string()),
            },
        );
        m.insert(
            "java-sec".to_string(),
            AgentInfo {
                name: "java-sec".to_string(),
                description: Some("Security audits".to_string()),
            },
        );
        m.insert(
            "cloud-expert".to_string(),
            AgentInfo {
                name: "cloud-expert".to_string(),
                description: Some("Cloud infrastructure".to_string()),
            },
        );
        m.insert(
            "ops-expert".to_string(),
            AgentInfo {
                name: "ops-expert".to_string(),
                description: Some("Operations & monitoring".to_string()),
            },
        );
        m
    }

    #[test]
    fn test_coordinator_prompt_contains_team() {
        let config = sample_config();
        let info = sample_agents_info();
        let prompt = build_orchestration_prompt("coordinator", &config, &info).unwrap();
        assert!(prompt.contains("**coordinator**"));
        assert!(prompt.contains("java-lead"));
        assert!(prompt.contains("cloud-expert"));
        assert!(prompt.contains("### Delegation syntax"));
        assert!(prompt.contains("### Rules"));
    }

    #[test]
    fn test_lead_prompt_contains_team_agents() {
        let config = sample_config();
        let info = sample_agents_info();
        let prompt = build_orchestration_prompt("java-lead", &config, &info).unwrap();
        assert!(prompt.contains("**lead**"));
        assert!(prompt.contains("java-arch"));
        assert!(prompt.contains("java-sec"));
        assert!(prompt.contains("@coordinator"));
        // Should not contain agents from other teams
        assert!(!prompt.contains("cloud-expert"));
    }

    #[test]
    fn test_agent_prompt_contains_peers() {
        let config = sample_config();
        let info = sample_agents_info();
        let prompt = build_orchestration_prompt("java-arch", &config, &info).unwrap();
        assert!(prompt.contains("**specialist agent**"));
        assert!(prompt.contains("java-sec")); // peer
        assert!(prompt.contains("@java-lead")); // lead
        // Should not list itself as peer
        assert!(!prompt.contains("| java-arch |"));
    }

    #[test]
    fn test_direct_agent_prompt() {
        let config = sample_config();
        let info = sample_agents_info();
        let prompt = build_orchestration_prompt("cloud-expert", &config, &info).unwrap();
        assert!(prompt.contains("directly to @coordinator"));
        assert!(prompt.contains("ops-expert")); // peer
    }

    #[test]
    fn test_disabled_returns_none() {
        let mut config = sample_config();
        config.enabled = false;
        let info = sample_agents_info();
        assert!(build_orchestration_prompt("coordinator", &config, &info).is_none());
    }

    #[test]
    fn test_non_hierarchical_returns_none() {
        let mut config = sample_config();
        config.pattern = OrchestrationPattern::Blackboard;
        let info = sample_agents_info();
        assert!(build_orchestration_prompt("coordinator", &config, &info).is_none());
    }

    #[test]
    fn test_agent_descriptions_used() {
        let config = sample_config();
        let info = sample_agents_info();
        let prompt = build_orchestration_prompt("coordinator", &config, &info).unwrap();
        assert!(prompt.contains("Java team coordinator"));
        assert!(prompt.contains("Cloud infrastructure"));
    }

    #[test]
    fn test_missing_agent_info_uses_default() {
        let config = sample_config();
        let info = HashMap::new(); // empty — no descriptions
        let prompt = build_orchestration_prompt("coordinator", &config, &info).unwrap();
        assert!(prompt.contains("Specialist")); // default description
    }
}
