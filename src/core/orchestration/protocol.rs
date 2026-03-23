//! Delegation protocol parsing.
//!
//! Parses LLM responses to extract `@agent-name: message` delegation directives.
//! The protocol is text-based and provider-agnostic.

use super::{AgentRelationship, OrchestrationConfig, classify_relationship};

/// A parsed delegation action from an LLM response.
#[derive(Debug, Clone, PartialEq)]
pub enum DelegationAction {
    /// Downward delegation: coordinator/lead → agent.
    Delegate { target: String, task: String },

    /// Lateral question: agent → peer in the same team.
    AskPeer { target: String, question: String },

    /// Upward escalation: agent → lead/coordinator.
    Escalate { target: String, message: String },

    /// Final answer with no delegation.
    FinalAnswer { content: String },
}

/// Try to parse a line as `@agent-name: message`.
///
/// Returns `Some((target, message))` if the line starts with `@`, followed by
/// a valid agent name (alphanumeric + hyphens), then `:`, then content.
fn parse_delegation_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('@') {
        return None;
    }
    let rest = &trimmed[1..];

    // Parse agent name: first char must be alphanumeric or underscore,
    // subsequent chars can be alphanumeric, underscore, or hyphen.
    let name_end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .unwrap_or(rest.len());

    if name_end == 0 {
        return None;
    }

    let name = &rest[..name_end];
    let after_name = &rest[name_end..];

    // Must be followed by `:` (with optional whitespace)
    let after_name = after_name.trim_start();
    if !after_name.starts_with(':') {
        return None;
    }
    let message = after_name[1..].trim();
    if message.is_empty() {
        return None;
    }

    Some((name.to_string(), message.to_string()))
}

/// Parse delegation directives from an LLM response.
///
/// Each line matching `@agent-name: message` is classified based on the
/// sender→target relationship in the hierarchy.
///
/// Lines that do not match are collected as narrative text. If no delegations
/// are found, the entire response is returned as a `FinalAnswer`.
pub fn parse_delegations(
    response: &str,
    sender: &str,
    config: &OrchestrationConfig,
) -> Vec<DelegationAction> {
    let mut actions = Vec::new();

    for line in response.lines() {
        if let Some((target, message)) = parse_delegation_line(line) {
            let relationship = classify_relationship(config, sender, &target);

            let action = match relationship {
                AgentRelationship::Superior => DelegationAction::Delegate {
                    target,
                    task: message,
                },
                AgentRelationship::Peer => DelegationAction::AskPeer {
                    target,
                    question: message,
                },
                AgentRelationship::Subordinate => DelegationAction::Escalate { target, message },
                AgentRelationship::Unknown => {
                    // Unknown target — treat as delegation attempt with warning
                    tracing::warn!("Agent '{sender}' tried to contact unknown agent '{target}'");
                    DelegationAction::Delegate {
                        target,
                        task: message,
                    }
                }
            };

            actions.push(action);
        }
    }

    if actions.is_empty() {
        vec![DelegationAction::FinalAnswer {
            content: response.to_string(),
        }]
    } else {
        actions
    }
}

/// Extract only the narrative (non-delegation) text from a response.
pub fn extract_narrative(response: &str) -> String {
    response
        .lines()
        .filter(|line| parse_delegation_line(line).is_none())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::orchestration::TeamConfig;

    fn sample_config() -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![
                TeamConfig {
                    lead: Some("java-lead".to_string()),
                    agents: vec![
                        "java-arch".to_string(),
                        "java-sec".to_string(),
                        "java-test".to_string(),
                    ],
                },
                TeamConfig {
                    lead: None,
                    agents: vec!["cloud-expert".to_string(), "ops-expert".to_string()],
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn test_simple_delegation() {
        let config = sample_config();
        let response = "@java-lead: audit the auth module";
        let actions = parse_delegations(response, "coordinator", &config);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            DelegationAction::Delegate {
                target: "java-lead".to_string(),
                task: "audit the auth module".to_string(),
            }
        );
    }

    #[test]
    fn test_multiple_delegations() {
        let config = sample_config();
        let response = "I'll split this task.\n\
                        @java-lead: audit security of auth module\n\
                        @cloud-expert: check cloud IAM config";
        let actions = parse_delegations(response, "coordinator", &config);
        assert_eq!(actions.len(), 2);
        assert!(
            matches!(&actions[0], DelegationAction::Delegate { target, .. } if target == "java-lead")
        );
        assert!(
            matches!(&actions[1], DelegationAction::Delegate { target, .. } if target == "cloud-expert")
        );
    }

    #[test]
    fn test_peer_question() {
        let config = sample_config();
        let response = "@java-test: which tests cover SQL injection?";
        let actions = parse_delegations(response, "java-sec", &config);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            DelegationAction::AskPeer {
                target: "java-test".to_string(),
                question: "which tests cover SQL injection?".to_string(),
            }
        );
    }

    #[test]
    fn test_escalation() {
        let config = sample_config();
        let response = "@java-lead: found 3 critical vulnerabilities in auth module";
        let actions = parse_delegations(response, "java-sec", &config);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            DelegationAction::Escalate {
                target: "java-lead".to_string(),
                message: "found 3 critical vulnerabilities in auth module".to_string(),
            }
        );
    }

    #[test]
    fn test_final_answer() {
        let config = sample_config();
        let response = "The auth module is secure. No issues found.";
        let actions = parse_delegations(response, "coordinator", &config);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            DelegationAction::FinalAnswer {
                content: response.to_string(),
            }
        );
    }

    #[test]
    fn test_no_false_positive_on_email() {
        let config = sample_config();
        // Email addresses should not trigger delegation
        let response = "Contact support@company.com for help.";
        let actions = parse_delegations(response, "coordinator", &config);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], DelegationAction::FinalAnswer { .. }));
    }

    #[test]
    fn test_no_false_positive_inline_mention() {
        let config = sample_config();
        // Inline @mention (not at line start) should not trigger
        let response = "I think @java-lead should handle this.";
        let actions = parse_delegations(response, "coordinator", &config);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], DelegationAction::FinalAnswer { .. }));
    }

    #[test]
    fn test_mixed_delegation_and_narrative() {
        let config = sample_config();
        let response = "Let me analyze this request.\n\
                        @java-lead: check auth module\n\
                        I'll also need infra review.\n\
                        @cloud-expert: verify IAM policies";
        let actions = parse_delegations(response, "coordinator", &config);
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn test_extract_narrative() {
        let response = "Analysis complete.\n\
                        @java-lead: check auth module\n\
                        All other areas look fine.";
        let narrative = extract_narrative(response);
        assert_eq!(narrative, "Analysis complete.\nAll other areas look fine.");
    }

    #[test]
    fn test_leadless_team_peer() {
        let config = sample_config();
        let response = "@ops-expert: can you check the monitoring setup?";
        let actions = parse_delegations(response, "cloud-expert", &config);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], DelegationAction::AskPeer { .. }));
    }

    #[test]
    fn test_unknown_agent_treated_as_delegation() {
        let config = sample_config();
        let response = "@unknown-agent: do something";
        let actions = parse_delegations(response, "coordinator", &config);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], DelegationAction::Delegate { .. }));
    }

    #[test]
    fn test_agent_to_coordinator_is_escalation() {
        let config = sample_config();
        let response = "@coordinator: here are my findings";
        let actions = parse_delegations(response, "java-lead", &config);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], DelegationAction::Escalate { .. }));
    }

    #[test]
    fn test_lead_delegates_to_team_agent() {
        let config = sample_config();
        let response = "@java-arch: review the architecture of the auth module";
        let actions = parse_delegations(response, "java-lead", &config);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], DelegationAction::Delegate { .. }));
    }
}
