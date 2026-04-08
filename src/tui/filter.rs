use crate::core::agent::Agent;
use crate::core::prompt::Prompt;
use crate::core::skill::Skill;
use crate::core::starter::StarterPack;
use crate::model_registry::ModelEntry;
#[cfg(feature = "storage")]
use crate::tui::app::OrchestrationEntry;
use crate::tui::app::{RunEntry, SortMode};

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
