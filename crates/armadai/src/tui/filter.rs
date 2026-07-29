#[cfg(feature = "storage")]
use crate::tui::app::OrchestrationEntry;
use crate::tui::app::{CostEntry, RunEntry, SortMode};
use armadai_core::agent::Agent;
use armadai_core::prompt::Prompt;
use armadai_core::skill::Skill;
use armadai_core::starter::StarterPack;
use armadai_providers::model_registry::ModelEntry;

/// Filter items by search query (case-insensitive substring match on name + metadata).
pub fn filter_agents(agents: &[Agent], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..agents.len()).collect();
    }
    let query = query.to_lowercase();
    agents
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            a.name.to_lowercase().contains(&query)
                || a.metadata.provider.to_lowercase().contains(&query)
                || a.metadata
                    .tags
                    .iter()
                    .any(|t| t.to_lowercase().contains(&query))
        })
        .map(|(i, _)| i)
        .collect()
}

pub fn filter_prompts(prompts: &[Prompt], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..prompts.len()).collect();
    }
    let query = query.to_lowercase();
    prompts
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.name.to_lowercase().contains(&query)
                || p.description
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}

pub fn filter_skills(skills: &[Skill], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..skills.len()).collect();
    }
    let query = query.to_lowercase();
    skills
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.name.to_lowercase().contains(&query)
                || s.description
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}

pub fn filter_starters(starters: &[StarterPack], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..starters.len()).collect();
    }
    let query = query.to_lowercase();
    starters
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.name.to_lowercase().contains(&query) || s.description.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}

pub fn filter_history(history: &[RunEntry], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..history.len()).collect();
    }
    let query = query.to_lowercase();
    history
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.agent.to_lowercase().contains(&query)
                || r.provider.to_lowercase().contains(&query)
                || r.model.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}

pub fn filter_costs(costs: &[CostEntry], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..costs.len()).collect();
    }
    let query = query.to_lowercase();
    costs
        .iter()
        .enumerate()
        .filter(|(_, c)| c.agent.to_lowercase().contains(&query))
        .map(|(i, _)| i)
        .collect()
}

pub fn filter_models(models: &[(String, ModelEntry)], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..models.len()).collect();
    }
    let query = query.to_lowercase();
    models
        .iter()
        .enumerate()
        .filter(|(_, (provider, entry))| {
            provider.to_lowercase().contains(&query)
                || entry.id.to_lowercase().contains(&query)
                || entry
                    .name
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Sort filtered indices by the specified sort mode.
pub fn sort_by_name<T: AsRef<str>>(indices: Vec<usize>, names: &[T], mode: SortMode) -> Vec<usize> {
    match mode {
        SortMode::Default => indices,
        SortMode::NameAsc => {
            let mut sorted = indices;
            sorted.sort_by(|&a, &b| names[a].as_ref().cmp(names[b].as_ref()));
            sorted
        }
        SortMode::NameDesc => {
            let mut sorted = indices;
            sorted.sort_by(|&a, &b| names[b].as_ref().cmp(names[a].as_ref()));
            sorted
        }
    }
}

/// Apply filtering and sorting to get display indices for agents.
pub fn apply_filter_and_sort_agents(
    agents: &[Agent],
    query: &str,
    sort_mode: SortMode,
) -> Vec<usize> {
    let filtered = filter_agents(agents, query);
    let names: Vec<_> = filtered.iter().map(|&i| &agents[i].name).collect();
    sort_by_name(filtered, &names, sort_mode)
}

/// Apply filtering and sorting to get display indices for prompts.
pub fn apply_filter_and_sort_prompts(
    prompts: &[Prompt],
    query: &str,
    sort_mode: SortMode,
) -> Vec<usize> {
    let filtered = filter_prompts(prompts, query);
    let names: Vec<_> = filtered.iter().map(|&i| &prompts[i].name).collect();
    sort_by_name(filtered, &names, sort_mode)
}

/// Apply filtering and sorting to get display indices for skills.
pub fn apply_filter_and_sort_skills(
    skills: &[Skill],
    query: &str,
    sort_mode: SortMode,
) -> Vec<usize> {
    let filtered = filter_skills(skills, query);
    let names: Vec<_> = filtered.iter().map(|&i| &skills[i].name).collect();
    sort_by_name(filtered, &names, sort_mode)
}

/// Apply filtering and sorting to get display indices for starters.
pub fn apply_filter_and_sort_starters(
    starters: &[StarterPack],
    query: &str,
    sort_mode: SortMode,
) -> Vec<usize> {
    let filtered = filter_starters(starters, query);
    let names: Vec<_> = filtered.iter().map(|&i| &starters[i].name).collect();
    sort_by_name(filtered, &names, sort_mode)
}

/// Apply filtering and sorting to get display indices for history.
pub fn apply_filter_and_sort_history(
    history: &[RunEntry],
    query: &str,
    sort_mode: SortMode,
) -> Vec<usize> {
    let filtered = filter_history(history, query);
    let names: Vec<_> = filtered.iter().map(|&i| &history[i].agent).collect();
    sort_by_name(filtered, &names, sort_mode)
}

/// Apply filtering and sorting to get display indices for costs.
///
/// Unlike the other list views, `SortMode::Default` here means **cost
/// descending** (the most useful ordering for a cost summary) rather than
/// load order — costs have no inherent "natural" order to fall back to.
/// `NameAsc`/`NameDesc` still sort by agent name, same as everywhere else.
pub fn apply_filter_and_sort_costs(
    costs: &[CostEntry],
    query: &str,
    sort_mode: SortMode,
) -> Vec<usize> {
    let filtered = filter_costs(costs, query);
    match sort_mode {
        SortMode::Default => {
            let mut sorted = filtered;
            sorted.sort_by(|&a, &b| {
                costs[b]
                    .total_cost
                    .partial_cmp(&costs[a].total_cost)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            sorted
        }
        SortMode::NameAsc | SortMode::NameDesc => {
            let names: Vec<_> = filtered.iter().map(|&i| &costs[i].agent).collect();
            sort_by_name(filtered, &names, sort_mode)
        }
    }
}

/// Apply filtering and sorting to get display indices for models.
pub fn apply_filter_and_sort_models(
    models: &[(String, ModelEntry)],
    query: &str,
    sort_mode: SortMode,
) -> Vec<usize> {
    let filtered = filter_models(models, query);
    let names: Vec<_> = filtered.iter().map(|&i| &models[i].0).collect();
    sort_by_name(filtered, &names, sort_mode)
}

#[cfg(feature = "storage")]
pub fn filter_orchestration(orchestration: &[OrchestrationEntry], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..orchestration.len()).collect();
    }
    let query = query.to_lowercase();
    orchestration
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            o.run_id.to_lowercase().contains(&query)
                || o.pattern.to_lowercase().contains(&query)
                || o.halt_reason
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(feature = "storage")]
pub fn apply_filter_and_sort_orchestration(
    orchestration: &[OrchestrationEntry],
    query: &str,
    sort_mode: SortMode,
) -> Vec<usize> {
    let filtered = filter_orchestration(orchestration, query);
    let names: Vec<_> = filtered.iter().map(|&i| &orchestration[i].run_id).collect();
    sort_by_name(filtered, &names, sort_mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(agent: &str, total_cost: f64) -> CostEntry {
        CostEntry {
            agent: agent.to_string(),
            total_runs: 1,
            total_cost,
            total_tokens_in: 0,
            total_tokens_out: 0,
        }
    }

    #[test]
    fn costs_default_sort_is_cost_descending() {
        let costs = vec![cost("alpha", 0.5), cost("beta", 2.0), cost("gamma", 1.0)];
        let indices = apply_filter_and_sort_costs(&costs, "", SortMode::Default);
        let ordered: Vec<&str> = indices.iter().map(|&i| costs[i].agent.as_str()).collect();
        assert_eq!(ordered, vec!["beta", "gamma", "alpha"]);
    }

    #[test]
    fn costs_name_asc_sorts_alphabetically() {
        let costs = vec![cost("gamma", 1.0), cost("alpha", 0.5), cost("beta", 2.0)];
        let indices = apply_filter_and_sort_costs(&costs, "", SortMode::NameAsc);
        let ordered: Vec<&str> = indices.iter().map(|&i| costs[i].agent.as_str()).collect();
        assert_eq!(ordered, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn costs_filter_by_agent_name() {
        let costs = vec![cost("alpha", 0.5), cost("beta", 2.0)];
        let indices = apply_filter_and_sort_costs(&costs, "alp", SortMode::Default);
        assert_eq!(indices, vec![0]);
    }
}
