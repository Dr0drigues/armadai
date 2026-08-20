//! Delegation policy: turn a declared topology into an enforced rule.
//!
//! Prose in `CLAUDE.md` does not constrain delegation — measured on this
//! project, the model quotes the instruction verbatim and routes elsewhere
//! anyway. This module is the decision half of the fix: a pure function over
//! `OrchestrationConfig`, called from a Claude Code `PreToolUse` hook.
//!
//! Design rule: **a gate that refuses because it did not understand is a gate
//! that gets uninstalled.** Every uncertainty degrades to `Ok` — the refusal
//! must come from an established violation, never from a doubt.

use serde::{Deserialize, Serialize};

use super::{OrchestrationConfig, TeamConfig};

/// How strictly the declared topology is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolicyMode {
    /// Declared topology is advisory (today's behaviour). Default, so that
    /// upgrading ArmadAI never changes an existing project's behaviour.
    #[default]
    Off,
    /// Anything not declared is refused.
    Strict,
}

/// A delegation the declared topology does not allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyViolation {
    /// The sub-agent the model tried to reach.
    pub target: String,
    /// What it was allowed to reach from where it stood.
    pub allowed: Vec<String>,
    /// Actionable message, surfaced to the model. Naming the permitted target
    /// is what makes it rewrite the call correctly rather than retry blindly.
    pub reason: String,
}

/// Everything the coordinator may reach: every team lead, plus the agents of
/// lead-less teams (who report to the coordinator directly).
fn coordinator_targets(teams: &[TeamConfig]) -> Vec<String> {
    let mut out = Vec::new();
    for team in teams {
        match &team.lead {
            Some(lead) => out.push(lead.clone()),
            None => out.extend(team.agents.iter().cloned()),
        }
    }
    out
}

/// Is `target` allowed from `caller` under `config`?
///
/// `caller` is `None` for the main thread (Claude Code sends an empty
/// `agent_type` there), `Some(name)` when a sub-agent is sub-delegating.
pub fn check_delegation(
    caller: Option<&str>,
    target: &str,
    config: &OrchestrationConfig,
) -> Result<(), PolicyViolation> {
    // Silent unless explicitly enabled.
    if config.policy != PolicyMode::Strict {
        return Ok(());
    }
    // No declared coordinator means no topology to violate (direct,
    // blackboard and ring patterns land here).
    let Some(coordinator) = config.coordinator.as_deref() else {
        return Ok(());
    };
    if target.is_empty() {
        return Ok(());
    }
    // Assistance agents are reachable from anywhere.
    if config.free_agents.iter().any(|a| a == target) {
        return Ok(());
    }

    let deny = |allowed: Vec<String>, from: &str| {
        let reason = if allowed.is_empty() {
            format!(
                "the declared topology allows no delegation from {from}; \
                 declare '{target}' in orchestration.teams or \
                 orchestration.free_agents to permit it"
            )
        } else {
            format!(
                "the declared topology allows only [{}] from {from}; \
                 hand the work to one of those, or declare '{target}' in \
                 orchestration.teams or orchestration.free_agents",
                allowed.join(", ")
            )
        };
        Err(PolicyViolation {
            target: target.to_string(),
            allowed,
            reason,
        })
    };

    match caller {
        // Main thread: the coordinator is the only door.
        None => {
            if target == coordinator {
                Ok(())
            } else {
                deny(vec![coordinator.to_string()], "the main thread")
            }
        }
        // The coordinator fans out to leads and to lead-less teams.
        Some(c) if c == coordinator => {
            let allowed = coordinator_targets(&config.teams);
            if allowed.iter().any(|a| a == target) {
                Ok(())
            } else {
                deny(allowed, coordinator)
            }
        }
        // A lead fans out inside its own team only.
        Some(c) => {
            if let Some(team) = config.teams.iter().find(|t| t.lead.as_deref() == Some(c)) {
                if team.agents.iter().any(|a| a == target) {
                    return Ok(());
                }
                return deny(team.agents.clone(), c);
            }
            // A plain specialist does not sub-delegate.
            deny(Vec::new(), c)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(policy: PolicyMode, teams: Vec<TeamConfig>) -> OrchestrationConfig {
        OrchestrationConfig {
            policy,
            coordinator: Some("dev-lead".into()),
            teams,
            free_agents: vec!["Explore".into()],
            ..Default::default()
        }
    }

    fn flat() -> Vec<TeamConfig> {
        vec![TeamConfig {
            lead: None,
            agents: vec!["qa-specialist".into(), "core-specialist".into()],
            ..Default::default()
        }]
    }

    #[test]
    fn off_allows_everything() {
        let c = cfg(PolicyMode::Off, flat());
        assert!(check_delegation(None, "anything", &c).is_ok());
    }

    #[test]
    fn main_thread_reaches_only_the_coordinator() {
        let c = cfg(PolicyMode::Strict, flat());
        assert!(check_delegation(None, "dev-lead", &c).is_ok());
        let v = check_delegation(None, "qa-specialist", &c).unwrap_err();
        assert_eq!(v.allowed, vec!["dev-lead".to_string()]);
        assert!(v.reason.contains("dev-lead"), "{}", v.reason);
    }

    #[test]
    fn coordinator_reaches_leadless_team_agents() {
        let c = cfg(PolicyMode::Strict, flat());
        assert!(check_delegation(Some("dev-lead"), "qa-specialist", &c).is_ok());
        assert!(check_delegation(Some("dev-lead"), "unknown", &c).is_err());
    }

    #[test]
    fn a_lead_reaches_its_own_team_only() {
        let teams = vec![
            TeamConfig {
                lead: Some("ui-lead".into()),
                agents: vec!["ui-specialist".into()],
                ..Default::default()
            },
            TeamConfig {
                lead: Some("back-lead".into()),
                agents: vec!["core-specialist".into()],
                ..Default::default()
            },
        ];
        let c = cfg(PolicyMode::Strict, teams);
        // The coordinator reaches leads, not their members directly.
        assert!(check_delegation(Some("dev-lead"), "ui-lead", &c).is_ok());
        assert!(check_delegation(Some("dev-lead"), "ui-specialist", &c).is_err());
        // A lead reaches its own team, not the other one.
        assert!(check_delegation(Some("ui-lead"), "ui-specialist", &c).is_ok());
        assert!(check_delegation(Some("ui-lead"), "core-specialist", &c).is_err());
    }

    #[test]
    fn a_plain_specialist_does_not_sub_delegate() {
        let c = cfg(PolicyMode::Strict, flat());
        let v = check_delegation(Some("qa-specialist"), "core-specialist", &c).unwrap_err();
        assert!(v.allowed.is_empty());
    }

    #[test]
    fn free_agents_are_reachable_from_anywhere() {
        let c = cfg(PolicyMode::Strict, flat());
        assert!(check_delegation(None, "Explore", &c).is_ok());
        assert!(check_delegation(Some("qa-specialist"), "Explore", &c).is_ok());
    }

    #[test]
    fn no_coordinator_means_no_topology_to_violate() {
        let c = OrchestrationConfig {
            policy: PolicyMode::Strict,
            coordinator: None,
            ..Default::default()
        };
        assert!(check_delegation(None, "whatever", &c).is_ok());
    }

    #[test]
    fn an_empty_target_degrades_to_allow() {
        let c = cfg(PolicyMode::Strict, flat());
        assert!(check_delegation(None, "", &c).is_ok());
    }
}
