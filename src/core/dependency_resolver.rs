//! Resolves resource dependencies between agents and prompts.
//!
//! In ArmadAI, agents do not declare their prompts — prompts declare which
//! agents they apply to via the `apply_to:` frontmatter field. This module
//! walks the inverse relation: given a set of agents, find every prompt that
//! targets at least one of them (or `*`).
//!
//! Skills are intentionally NOT auto-resolved here: skills live at the pack
//! or project level, not the agent level. The `extract` CLI surfaces them as
//! a separate, explicit selection.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::core::agent::Agent;
use crate::core::prompt::{Prompt, matching_prompts};

/// Result of dependency resolution for a set of agents.
#[derive(Debug, Default)]
pub struct ResolvedDeps {
    /// Prompts that apply to at least one of the input agents.
    pub prompts: Vec<Prompt>,
}

/// Compute the prompt closure of a set of agents from the available pool.
///
/// Order is stable: prompts are returned in the order they appear in
/// `available_prompts`, with duplicates removed.
pub fn resolve_dependencies(agents: &[Agent], available_prompts: &[Prompt]) -> ResolvedDeps {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut prompts = Vec::new();

    for agent in agents {
        // `apply_to:` in prompts uses the file-stem (kebab-case), not the H1
        // display name. Fall back to `name` for in-memory agents without a path.
        let agent_id = agent
            .source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&agent.name);

        for p in matching_prompts(available_prompts, agent_id) {
            if seen.insert(p.source.clone()) {
                prompts.push(p.clone());
            }
        }
    }

    ResolvedDeps { prompts }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::agent::{Agent, AgentMetadata};

    fn agent(name: &str) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("agents/{name}.md")),
            metadata: AgentMetadata {
                provider: "claude".to_string(),
                model: Some("latest:pro".to_string()),
                command: None,
                args: None,
                temperature: 0.3,
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

    fn prompt(name: &str, apply_to: &[&str]) -> Prompt {
        Prompt {
            name: name.to_string(),
            description: None,
            apply_to: apply_to.iter().map(|s| s.to_string()).collect(),
            body: String::new(),
            source: PathBuf::from(format!("prompts/{name}.md")),
        }
    }

    #[test]
    fn resolves_named_apply_to_for_single_agent() {
        let agents = vec![agent("code-reviewer")];
        let pool = vec![
            prompt("rust-style", &["code-reviewer"]),
            prompt("test-style", &["test-writer"]),
        ];

        let deps = resolve_dependencies(&agents, &pool);

        assert_eq!(deps.prompts.len(), 1);
        assert_eq!(deps.prompts[0].name, "rust-style");
    }

    #[test]
    fn resolves_wildcard_apply_to() {
        let agents = vec![agent("anybody")];
        let pool = vec![prompt("global", &["*"])];

        let deps = resolve_dependencies(&agents, &pool);

        assert_eq!(deps.prompts.len(), 1);
        assert_eq!(deps.prompts[0].name, "global");
    }

    #[test]
    fn deduplicates_prompt_shared_by_multiple_agents() {
        let agents = vec![agent("reviewer"), agent("writer")];
        let pool = vec![prompt("conventions", &["reviewer", "writer"])];

        let deps = resolve_dependencies(&agents, &pool);

        assert_eq!(deps.prompts.len(), 1);
        assert_eq!(deps.prompts[0].name, "conventions");
    }

    #[test]
    fn ignores_unrelated_prompts() {
        let agents = vec![agent("code-reviewer")];
        let pool = vec![
            prompt("test-style", &["test-writer"]),
            prompt("doc-style", &["doc-writer"]),
        ];

        let deps = resolve_dependencies(&agents, &pool);

        assert!(deps.prompts.is_empty());
    }

    #[test]
    fn preserves_pool_order() {
        let agents = vec![agent("a")];
        let pool = vec![
            prompt("z-first", &["a"]),
            prompt("m-second", &["a"]),
            prompt("a-third", &["a"]),
        ];

        let deps = resolve_dependencies(&agents, &pool);

        assert_eq!(
            deps.prompts
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["z-first", "m-second", "a-third"]
        );
    }

    #[test]
    fn empty_inputs_yield_empty_deps() {
        let deps = resolve_dependencies(&[], &[]);
        assert!(deps.prompts.is_empty());
    }
}
