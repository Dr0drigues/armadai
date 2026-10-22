//! Slots: an agent declares `slots: [name]`, a skill fills one with
//! `metadata: { armadai-slot: "name, other" }`.

use crate::load::{Project, Skill};

/// The slot names a skill fills (`armadai-slot`, split on `,`, trimmed).
pub(crate) fn filled_by(skill: &Skill) -> Vec<&str> {
    (skill.front.metadata.as_ref())
        .and_then(|m| m.get("armadai-slot"))
        .map_or_else(Vec::new, |v| v.split(',').map(str::trim).collect())
}

/// Ids of the skills that fill `slot`, sorted.
pub fn slot_fills(project: &Project, slot: &str) -> Vec<String> {
    // `skills` is a BTreeMap: already sorted by id
    (project.skills.iter())
        .filter(|(_, s)| filled_by(s).contains(&slot))
        .map(|(id, _)| id.clone())
        .collect()
}

/// The skills of `agent`: its `skills:` in declared order, then every skill filling one
/// of its slots, sorted by id, each skill once (first occurrence kept). Empty for an
/// unknown agent.
pub fn agent_skills(project: &Project, agent: &str) -> Vec<String> {
    let Some(a) = project.agents.get(agent) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    let fills: std::collections::BTreeSet<String> = a
        .front
        .slots
        .iter()
        .flat_map(|s| slot_fills(project, s))
        .collect();
    for s in a.front.skills.iter().cloned().chain(fills) {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}
