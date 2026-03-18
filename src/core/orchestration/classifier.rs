use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::blackboard::BlackboardConfig;
use super::ring::RingConfig;
use super::{OrchestrationPattern, PatternConfig};
use crate::core::agent::Agent;

/// Result of task classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskClassification {
    pub pattern: OrchestrationPattern,
    pub agents: Vec<String>,
    pub config: PatternConfig,
    pub reasoning: String,
}

/// Classify a task and select the appropriate orchestration pattern.
///
/// Uses heuristics (no LLM call):
/// 1. Count how many agents are relevant to the task.
/// 2. If only one → Direct.
/// 3. If multiple with low domain overlap → Blackboard (parallel).
/// 4. If multiple with high domain overlap → Ring (cross-critique).
pub fn classify_task(task: &str, available_agents: &[Agent]) -> TaskClassification {
    let relevant: Vec<&Agent> = available_agents
        .iter()
        .filter(|a| agent_matches_task(a, task))
        .collect();

    match relevant.len() {
        0 => {
            // No matching agent — fall back to first available or report none
            if let Some(first) = available_agents.first() {
                TaskClassification {
                    pattern: OrchestrationPattern::Direct,
                    agents: vec![first.name.clone()],
                    config: PatternConfig::Direct {
                        agent: first.name.clone(),
                    },
                    reasoning: "No agent matches task keywords; using first available agent"
                        .to_string(),
                }
            } else {
                TaskClassification {
                    pattern: OrchestrationPattern::Direct,
                    agents: vec![],
                    config: PatternConfig::Direct {
                        agent: String::new(),
                    },
                    reasoning: "No agents available".to_string(),
                }
            }
        }
        1 => {
            let agent = relevant[0];
            TaskClassification {
                pattern: OrchestrationPattern::Direct,
                agents: vec![agent.name.clone()],
                config: PatternConfig::Direct {
                    agent: agent.name.clone(),
                },
                reasoning: format!(
                    "Single matching agent '{}'; direct execution",
                    agent.name
                ),
            }
        }
        _ => {
            let overlap = compute_domain_overlap(&relevant);
            let agent_names: Vec<String> = relevant.iter().map(|a| a.name.clone()).collect();

            if overlap < 0.3 {
                TaskClassification {
                    pattern: OrchestrationPattern::Blackboard,
                    agents: agent_names,
                    config: PatternConfig::Blackboard(BlackboardConfig::default()),
                    reasoning: format!(
                        "Multiple agents with low domain overlap ({overlap:.2}); \
                         parallel blackboard execution"
                    ),
                }
            } else {
                TaskClassification {
                    pattern: OrchestrationPattern::Ring,
                    agents: agent_names,
                    config: PatternConfig::Ring(RingConfig::default()),
                    reasoning: format!(
                        "Multiple agents with high domain overlap ({overlap:.2}); \
                         ring with cross-critique"
                    ),
                }
            }
        }
    }
}

/// Check if an agent is relevant to a task based on tags and name keywords.
fn agent_matches_task(agent: &Agent, task: &str) -> bool {
    let task_lower = task.to_lowercase();

    // Check if any tag appears in the task (tag normalized once)
    if agent
        .metadata
        .tags
        .iter()
        .any(|tag| task_lower.contains(&tag.to_lowercase()))
    {
        return true;
    }

    // Check if the agent name (words) appears in the task
    agent
        .name
        .split_whitespace()
        .any(|w| w.len() > 2 && task_lower.contains(&w.to_lowercase()))
}

/// Compute domain overlap ratio between agents based on their tags.
/// Returns 0.0 (completely independent) to 1.0 (identical tags).
fn compute_domain_overlap(agents: &[&Agent]) -> f32 {
    if agents.len() < 2 {
        return 0.0;
    }

    let tag_sets: Vec<HashSet<&str>> = agents
        .iter()
        .map(|a| a.metadata.tags.iter().map(|t| t.as_str()).collect())
        .collect();

    let mut total_pairs = 0u32;
    let mut overlap_sum = 0.0f32;

    for i in 0..tag_sets.len() {
        for j in (i + 1)..tag_sets.len() {
            total_pairs += 1;
            let intersection = tag_sets[i].intersection(&tag_sets[j]).count();
            let union = tag_sets[i].union(&tag_sets[j]).count();
            if union > 0 {
                overlap_sum += intersection as f32 / union as f32;
            }
        }
    }

    if total_pairs == 0 {
        0.0
    } else {
        overlap_sum / total_pairs as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent::{Agent, AgentMetadata};
    use std::path::PathBuf;

    fn make_agent(name: &str, tags: &[&str]) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("{name}.md")),
            metadata: AgentMetadata {
                provider: "anthropic".to_string(),
                model: Some("claude-sonnet-4-5-20250929".to_string()),
                command: None,
                args: None,
                temperature: 0.7,
                max_tokens: None,
                timeout: None,
                tags: tags.iter().map(|s| s.to_string()).collect(),
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
            system_prompt: "test".to_string(),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    #[test]
    fn test_classify_no_agents() {
        let result = classify_task("do something", &[]);
        assert_eq!(result.pattern, OrchestrationPattern::Direct);
        assert!(result.agents.is_empty());
    }

    #[test]
    fn test_classify_single_match() {
        let agents = vec![make_agent("Security Reviewer", &["security", "review"])];
        let result = classify_task("review security issues", &agents);
        assert_eq!(result.pattern, OrchestrationPattern::Direct);
        assert_eq!(result.agents, vec!["Security Reviewer"]);
    }

    #[test]
    fn test_classify_no_match_uses_first() {
        let agents = vec![make_agent("Code Formatter", &["format", "style"])];
        let result = classify_task("deploy to production", &agents);
        assert_eq!(result.pattern, OrchestrationPattern::Direct);
        assert_eq!(result.agents, vec!["Code Formatter"]);
    }

    #[test]
    fn test_classify_multiple_independent_blackboard() {
        let agents = vec![
            make_agent("Security Reviewer", &["security"]),
            make_agent("Performance Reviewer", &["performance"]),
            make_agent("Style Checker", &["style"]),
        ];
        let result = classify_task("review security performance style", &agents);
        assert_eq!(result.pattern, OrchestrationPattern::Blackboard);
        assert_eq!(result.agents.len(), 3);
    }

    #[test]
    fn test_classify_multiple_overlapping_ring() {
        let agents = vec![
            make_agent("Frontend Review", &["review", "frontend", "quality"]),
            make_agent("Backend Review", &["review", "backend", "quality"]),
            make_agent("Full Review", &["review", "quality", "security"]),
        ];
        let result = classify_task("review quality", &agents);
        assert_eq!(result.pattern, OrchestrationPattern::Ring);
        assert_eq!(result.agents.len(), 3);
    }

    #[test]
    fn test_compute_domain_overlap_none() {
        let a1 = make_agent("A", &["security"]);
        let a2 = make_agent("B", &["performance"]);
        let overlap = compute_domain_overlap(&[&a1, &a2]);
        assert!((overlap - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compute_domain_overlap_full() {
        let a1 = make_agent("A", &["review", "code"]);
        let a2 = make_agent("B", &["review", "code"]);
        let overlap = compute_domain_overlap(&[&a1, &a2]);
        assert!((overlap - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compute_domain_overlap_partial() {
        let a1 = make_agent("A", &["review", "security"]);
        let a2 = make_agent("B", &["review", "performance"]);
        let overlap = compute_domain_overlap(&[&a1, &a2]);
        // intersection = {"review"}, union = {"review", "security", "performance"}
        // Jaccard = 1/3 ≈ 0.333
        assert!(overlap > 0.3 && overlap < 0.4);
    }

    #[test]
    fn test_compute_domain_overlap_single_agent() {
        let a1 = make_agent("A", &["security"]);
        let overlap = compute_domain_overlap(&[&a1]);
        assert!((overlap - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_agent_matches_task_by_tag() {
        let agent = make_agent("Test Agent", &["security", "review"]);
        assert!(agent_matches_task(&agent, "review this code"));
        assert!(agent_matches_task(&agent, "security audit needed"));
        assert!(!agent_matches_task(&agent, "deploy to production"));
    }

    #[test]
    fn test_agent_matches_task_by_name() {
        let agent = make_agent("Security Reviewer", &[]);
        assert!(agent_matches_task(&agent, "need security help"));
        assert!(agent_matches_task(
            &agent,
            "reviewer needed for this code"
        ));
    }

    #[test]
    fn test_agent_matches_task_case_insensitive() {
        let agent = make_agent("Test Agent", &["SECURITY"]);
        assert!(agent_matches_task(&agent, "security review"));
    }

    #[test]
    fn test_classify_reasoning_contains_info() {
        let agents = vec![make_agent("Agent", &["test"])];
        let result = classify_task("test something", &agents);
        assert!(!result.reasoning.is_empty());
    }
}
