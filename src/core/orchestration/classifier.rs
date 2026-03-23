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
/// **Phase 1 — keyword heuristic classifier.**
/// This is a lightweight, zero-cost classifier that runs entirely on string
/// matching.  It will be replaced by an LLM-based classifier in phase 2 once
/// the prompt/evaluation harness is ready.  Until then, every change here
/// should keep the logic simple and deterministic.
///
/// Algorithm:
/// 1. Count how many agents are relevant to the task (tag + name matching).
/// 2. If only one → Direct.
/// 3. If multiple with low domain overlap → Blackboard (parallel).
/// 4. If multiple with high domain overlap → Ring (cross-critique).
/// 5. Keyword hints can nudge the pattern choice (e.g. "review" → Ring).
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
                reasoning: format!("Single matching agent '{}'; direct execution", agent.name),
            }
        }
        _ => {
            let overlap = compute_domain_overlap(&relevant);
            let agent_names: Vec<String> = relevant.iter().map(|a| a.name.clone()).collect();
            let hint = keyword_pattern_hint(task);

            let use_ring = match hint {
                Some(PatternHint::Ring) => true,
                Some(PatternHint::Blackboard) => false,
                None => overlap >= 0.3,
            };

            if use_ring {
                TaskClassification {
                    pattern: OrchestrationPattern::Ring,
                    agents: agent_names,
                    config: PatternConfig::Ring(RingConfig::default()),
                    reasoning: format!(
                        "Multiple agents (overlap {overlap:.2}{}); \
                         ring with cross-critique",
                        hint.map_or(String::new(), |h| format!(", keyword hint: {h:?}"))
                    ),
                }
            } else {
                TaskClassification {
                    pattern: OrchestrationPattern::Blackboard,
                    agents: agent_names,
                    config: PatternConfig::Blackboard(BlackboardConfig::default()),
                    reasoning: format!(
                        "Multiple agents (overlap {overlap:.2}{}); \
                         parallel blackboard execution",
                        hint.map_or(String::new(), |h| format!(", keyword hint: {h:?}"))
                    ),
                }
            }
        }
    }
}

/// Keyword-based pattern hint.
///
/// Phase-1 heuristic: certain task keywords strongly suggest one pattern.
/// Returns `None` when no keyword matches — the overlap score decides.
#[derive(Debug, Clone, Copy)]
enum PatternHint {
    /// Keywords that suggest cross-critique / sequential review.
    Ring,
    /// Keywords that suggest parallel, independent work.
    Blackboard,
}

/// Scan the task description for pattern-hinting keywords (whole-word match).
fn keyword_pattern_hint(task: &str) -> Option<PatternHint> {
    let words = task_words(task);

    // Ring keywords: action verbs that benefit from sequential review / critique.
    const RING_KEYWORDS: &[&str] = &[
        "review", "audit", "critique", "evaluate", "assess", "validate",
    ];
    // Blackboard keywords: tasks that benefit from parallel generation.
    const BLACKBOARD_KEYWORDS: &[&str] = &[
        "generate",
        "build",
        "create",
        "implement",
        "draft",
        "produce",
        "write",
    ];

    if RING_KEYWORDS.iter().any(|kw| words.iter().any(|w| w == kw)) {
        return Some(PatternHint::Ring);
    }
    if BLACKBOARD_KEYWORDS
        .iter()
        .any(|kw| words.iter().any(|w| w == kw))
    {
        return Some(PatternHint::Blackboard);
    }
    None
}

/// Split task into normalised lowercase words, stripping punctuation.
fn task_words(task: &str) -> Vec<String> {
    task.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Check if an agent is relevant to a task based on tags and name keywords.
///
/// Phase-1 heuristic: prefix matching on whole words to catch derived forms
/// (e.g. tag "review" matches task word "reviewing", "infra" matches
/// "infrastructure") while avoiding pure substring false positives.
/// A future LLM-based classifier will use semantic similarity instead.
fn agent_matches_task(agent: &Agent, task: &str) -> bool {
    let words = task_words(task);

    // Check if any tag matches a task word (either direction prefix)
    if agent.metadata.tags.iter().any(|tag| {
        let tag_lower = tag.to_lowercase();
        words
            .iter()
            .any(|w| w.starts_with(&tag_lower) || tag_lower.starts_with(w.as_str()))
    }) {
        return true;
    }

    // Check if any word from the agent's name prefix-matches a task word
    agent.name.split_whitespace().any(|name_word| {
        let nw = name_word.to_lowercase();
        nw.len() > 2
            && words
                .iter()
                .any(|w| w.starts_with(&nw) || nw.starts_with(w.as_str()))
    })
}

/// Compute domain overlap ratio between agents based on their tags.
/// Returns 0.0 (completely independent) to 1.0 (identical tags).
///
/// Uses pairwise Jaccard similarity averaged over all agent pairs.
/// Phase-1 heuristic: tag-only; a future version may use embedding similarity.
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
            make_agent("Security Agent", &["security"]),
            make_agent("Performance Agent", &["performance"]),
            make_agent("Style Agent", &["style"]),
        ];
        // Task matches all three agents but uses no ring/blackboard keyword hints
        let result = classify_task("check security performance style", &agents);
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
        assert!(agent_matches_task(&agent, "reviewer needed for this code"));
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

    #[test]
    fn test_keyword_hint_review_suggests_ring() {
        let agents = vec![
            make_agent("Agent A", &["code"]),
            make_agent("Agent B", &["code"]),
        ];
        let result = classify_task("review the code changes", &agents);
        assert_eq!(result.pattern, OrchestrationPattern::Ring);
    }

    #[test]
    fn test_keyword_hint_generate_suggests_blackboard() {
        let agents = vec![
            make_agent("Agent A", &["code"]),
            make_agent("Agent B", &["code"]),
        ];
        let result = classify_task("generate code for the API", &agents);
        assert_eq!(result.pattern, OrchestrationPattern::Blackboard);
    }

    #[test]
    fn test_keyword_hint_audit_suggests_ring() {
        let agents = vec![
            make_agent("Agent A", &["infra"]),
            make_agent("Agent B", &["infra"]),
        ];
        // "infra" prefix-matches "infrastructure"
        let result = classify_task("audit infrastructure setup", &agents);
        assert_eq!(result.pattern, OrchestrationPattern::Ring);
    }

    #[test]
    fn test_keyword_hint_none_falls_back_to_overlap() {
        assert!(keyword_pattern_hint("do something").is_none());
    }

    #[test]
    fn test_agent_matches_task_prefix_derived_forms() {
        // "review" tag matches "reviewing" task word
        let agent = make_agent("Test", &["review"]);
        assert!(agent_matches_task(&agent, "start reviewing the code"));

        // "infra" tag matches "infrastructure" task word
        let agent = make_agent("Test", &["infra"]);
        assert!(agent_matches_task(&agent, "infrastructure audit"));

        // Reverse: long tag, short task word
        let agent = make_agent("Test", &["infrastructure"]);
        assert!(agent_matches_task(&agent, "infra audit"));
    }

    #[test]
    fn test_agent_matches_task_no_false_positive_prefix() {
        // "view" should NOT match "reviewing" (too short, but len > 2 so it will
        // prefix-match — this is acceptable for phase-1 heuristic)
        let agent = make_agent("Test", &["secure"]);
        // "secure" should NOT match "insecure" (prefix doesn't match)
        assert!(!agent_matches_task(&agent, "insecure connection"));
    }
}
