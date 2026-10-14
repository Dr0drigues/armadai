//! Where a legacy project's declared agents (`.armadai/agents.yaml`) and the
//! prompt fragments they reference are read from. Only `migrate` reads them.

use std::path::Path;

use crate::prompt::Prompt;

/// Where a project's declarations live.
pub fn declarations_path(project_root: &Path) -> std::path::PathBuf {
    project_root.join(".armadai").join("agents.yaml")
}

/// Every prompt fragment a project's declarations can reference, gathered
/// from the same three tiers `project::resolve_prompt` searches by name:
/// `.armadai/prompts/` (preferred), the legacy `prompts/`, then the user's
/// global library. A name defined in more than one tier keeps its most
/// local definition — the same "closer wins" rule the rest of resolution
/// uses.
///
pub fn project_fragments(project_root: &Path) -> Vec<Prompt> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in crate::project::prompt_dirs(project_root) {
        for p in crate::prompt::load_all_prompts(&dir) {
            if seen.insert(p.name.clone()) {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::project::AgentRef;

    /// `AgentRef` is `#[serde(untagged)]`. Adding `Declared` must not shift
    /// how a plain `- name: x` entry resolves — untagged enums try each
    /// variant in order and a missing required field is what actually
    /// disambiguates them, but that is easy to get wrong when a fourth
    /// variant with its own single required field joins the set.
    #[test]
    fn a_named_entry_still_deserialises_as_named_not_declared() {
        let refs: Vec<AgentRef> =
            armadai_model::yaml::from_str("- name: core-specialist\n").unwrap();
        assert_eq!(refs.len(), 1);
        assert!(
            matches!(refs[0], AgentRef::Named { .. }),
            "expected Named, got {:?}",
            refs[0]
        );
    }

    /// Symmetric check: a `- declared: x` entry must resolve to `Declared`,
    /// not to one of the other three variants.
    #[test]
    fn a_declared_entry_deserialises_as_declared() {
        let refs: Vec<AgentRef> =
            armadai_model::yaml::from_str("- declared: core-specialist\n").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[0],
            AgentRef::Declared {
                declared: "core-specialist".to_string()
            }
        );
    }
}
