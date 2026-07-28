//! C8 — deterministic declarative agent selection (named routes + capability tags).

use std::collections::{BTreeMap, HashMap};

/// Result of an agent selection.
#[derive(Debug, Clone)]
pub struct AgentSelection {
    pub agents: Vec<String>,
    pub reason: String,
}

/// Error selecting agents.
#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    #[error("unknown route '{name}' (known routes: {})", known.join(", "))]
    UnknownRoute { name: String, known: Vec<String> },
    #[error("no agent matches tags [{}] in roster [{}]", tags.join(", "), roster.join(", "))]
    NoMatch {
        tags: Vec<String>,
        roster: Vec<String>,
    },
}

/// True if `agent_tags` (already lowercased-comparable) intersects `wanted`,
/// case-insensitively.
fn tags_intersect(agent_tags: &[String], wanted: &[String]) -> bool {
    wanted.iter().any(|w| {
        let wl = w.to_lowercase();
        agent_tags.iter().any(|t| t.to_lowercase() == wl)
    })
}

/// Select the participating agents deterministically.
///
/// - neither `route` nor `tags` → full `roster`.
/// - `route` only → the route's agent list (`UnknownRoute` if absent).
/// - `tags` only → `roster` filtered by tag intersection (`NoMatch` if empty).
/// - `route` + `tags` → the route's list is the candidate pool, filtered by
///   tags (`NoMatch` if empty).
///
/// `agent_tags` maps an agent name to its (tags ∪ stacks). Deterministic:
/// order follows the input (`roster` order for tag filtering, route order for
/// route selection). No I/O.
pub fn select_agents(
    roster: &[String],
    route: Option<&str>,
    tags: &[String],
    routes: &BTreeMap<String, Vec<String>>,
    agent_tags: &HashMap<String, Vec<String>>,
) -> Result<AgentSelection, SelectionError> {
    let empty: Vec<String> = Vec::new();

    // Resolve the candidate pool (route list, or the roster).
    let (pool, via_route): (Vec<String>, Option<String>) = match route {
        Some(name) => match routes.get(name) {
            Some(list) => (list.clone(), Some(name.to_string())),
            None => {
                return Err(SelectionError::UnknownRoute {
                    name: name.to_string(),
                    known: routes.keys().cloned().collect(),
                });
            }
        },
        None => (roster.to_vec(), None),
    };

    // Filter the pool by tags if any were requested.
    if tags.is_empty() {
        let reason = match &via_route {
            Some(r) => format!("route '{r}' → {} agent(s)", pool.len()),
            None => format!("no routing (full roster, {} agent(s))", pool.len()),
        };
        return Ok(AgentSelection {
            agents: pool,
            reason,
        });
    }

    let filtered: Vec<String> = pool
        .iter()
        .filter(|name| {
            let t = agent_tags.get(*name).unwrap_or(&empty);
            tags_intersect(t, tags)
        })
        .cloned()
        .collect();

    if filtered.is_empty() {
        return Err(SelectionError::NoMatch {
            tags: tags.to_vec(),
            roster: pool,
        });
    }

    let reason = match &via_route {
        Some(r) => format!(
            "route '{r}' + tags [{}] → {}/{} agent(s)",
            tags.join(", "),
            filtered.len(),
            pool.len()
        ),
        None => format!(
            "tags [{}] matched {}/{} roster agent(s)",
            tags.join(", "),
            filtered.len(),
            pool.len()
        ),
    };
    Ok(AgentSelection {
        agents: filtered,
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    fn routes() -> BTreeMap<String, Vec<String>> {
        let mut m = BTreeMap::new();
        m.insert(
            "security-audit".to_string(),
            vec!["rust-security".to_string(), "qa".to_string()],
        );
        m
    }
    fn tags_map() -> HashMap<String, Vec<String>> {
        let mut m = HashMap::new();
        m.insert(
            "rust-security".to_string(),
            vec!["security".to_string(), "rust".to_string()],
        );
        m.insert("ui".to_string(), vec!["frontend".to_string()]);
        m.insert("qa".to_string(), vec!["testing".to_string()]);
        m
    }

    #[test]
    fn no_selectors_returns_full_roster() {
        let roster = vec!["rust-security".to_string(), "ui".to_string()];
        let sel = select_agents(&roster, None, &[], &routes(), &tags_map()).unwrap();
        assert_eq!(sel.agents, roster);
    }

    #[test]
    fn route_only_returns_route_agents() {
        let roster = vec!["ui".to_string()];
        let sel =
            select_agents(&roster, Some("security-audit"), &[], &routes(), &tags_map()).unwrap();
        assert_eq!(
            sel.agents,
            vec!["rust-security".to_string(), "qa".to_string()]
        );
    }

    #[test]
    fn unknown_route_errors() {
        let roster = vec!["ui".to_string()];
        let err = select_agents(&roster, Some("nope"), &[], &routes(), &tags_map()).unwrap_err();
        assert!(matches!(err, SelectionError::UnknownRoute { .. }));
    }

    #[test]
    fn tags_only_filters_roster_case_insensitive() {
        let roster = vec![
            "rust-security".to_string(),
            "ui".to_string(),
            "qa".to_string(),
        ];
        let sel = select_agents(
            &roster,
            None,
            &["Security".to_string()],
            &routes(),
            &tags_map(),
        )
        .unwrap();
        assert_eq!(sel.agents, vec!["rust-security".to_string()]); // only rust-security has "security"
    }

    #[test]
    fn tags_no_match_errors() {
        let roster = vec!["ui".to_string()];
        let err = select_agents(
            &roster,
            None,
            &["security".to_string()],
            &routes(),
            &tags_map(),
        )
        .unwrap_err();
        assert!(matches!(err, SelectionError::NoMatch { .. }));
    }

    #[test]
    fn route_then_tags_refines_within_pool() {
        // pool = [rust-security, qa]; tags [security] keeps only rust-security.
        let roster = vec!["ui".to_string()];
        let sel = select_agents(
            &roster,
            Some("security-audit"),
            &["security".to_string()],
            &routes(),
            &tags_map(),
        )
        .unwrap();
        assert_eq!(sel.agents, vec!["rust-security".to_string()]);
    }

    #[test]
    fn deterministic_same_input_same_output() {
        let roster = vec![
            "rust-security".to_string(),
            "qa".to_string(),
            "ui".to_string(),
        ];
        let a =
            select_agents(&roster, None, &["rust".to_string()], &routes(), &tags_map()).unwrap();
        let b =
            select_agents(&roster, None, &["rust".to_string()], &routes(), &tags_map()).unwrap();
        assert_eq!(a.agents, b.agents);
    }
}
