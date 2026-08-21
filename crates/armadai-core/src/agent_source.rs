//! Loading an `Agent`, whatever its origin.
//!
//! `project::resolve_agent` returns a **path**, which serves callers that
//! manipulate files — `model_updater` and `pack_validation` rewrite deprecated
//! models in place. A declared agent has no file of its own, so it is right
//! that `resolve_agent` fails for one. This module is for callers that want the
//! agent, not its file.

use std::path::Path;

use crate::agent::Agent;
use crate::agent_decl;
use crate::project::{AgentRef, resolve_agent};
use crate::prompt::Prompt;

/// Where a project's declarations live.
pub fn declarations_path(project_root: &Path) -> std::path::PathBuf {
    project_root.join(".armadai").join("agents.yaml")
}

/// Load an agent, from a file or from the project's declarations.
pub fn load_agent(
    r: &AgentRef,
    project_root: &Path,
    fragments: &[Prompt],
) -> anyhow::Result<Agent> {
    let AgentRef::Declared { declared } = r else {
        // Unchanged path: resolve to a file, then parse it.
        let path = resolve_agent(r, project_root)?;
        return crate::parser::parse_agent_file(&path);
    };

    let path = declarations_path(project_root);
    let decls = agent_decl::load(&path)?;
    let decl = decls
        .agents
        .iter()
        .find(|a| &a.name == declared)
        .ok_or_else(|| {
            anyhow::anyhow!("agent '{declared}' is not declared in {}", path.display())
        })?;

    agent_decl::to_agent(decl, &decls.defaults, fragments, path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project with one declared agent and one fragment on disk.
    fn project(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join(".armadai")).unwrap();
        std::fs::write(
            dir.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\n  model: latest:pro\nagents:\n  \
             - name: core-specialist\n    description: the core\n    \
             prompt: [base]\n",
        )
        .unwrap();
    }

    fn fragments() -> Vec<crate::prompt::Prompt> {
        vec![crate::prompt::Prompt {
            name: "base".into(),
            description: None,
            apply_to: vec![],
            body: "You are {{name}}.".into(),
            source: std::path::PathBuf::from("base.md"),
        }]
    }

    #[test]
    fn a_declared_ref_yields_an_agent_without_touching_the_disk_for_it() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());
        let r = AgentRef::Declared {
            declared: "core-specialist".into(),
        };
        let agent = load_agent(&r, dir.path(), &fragments()).unwrap();
        assert_eq!(agent.name, "core-specialist");
        assert_eq!(agent.system_prompt, "You are core-specialist.");
        assert_eq!(agent.metadata.provider, "claude");
        // `source` points at the declaration, which is where it came from.
        assert!(agent.source.ends_with("agents.yaml"));
    }

    #[test]
    fn a_declared_name_absent_from_the_yaml_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());
        let r = AgentRef::Declared {
            declared: "ghost".into(),
        };
        let err = load_agent(&r, dir.path(), &fragments())
            .unwrap_err()
            .to_string();
        assert!(err.contains("ghost"), "must name the missing agent: {err}");
    }

    #[test]
    fn a_declared_ref_without_any_agents_yaml_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let r = AgentRef::Declared {
            declared: "x".into(),
        };
        assert!(load_agent(&r, dir.path(), &[]).is_err());
    }

    /// Three declared agents with distinguishable metadata and prompts,
    /// looked up by the middle name. `load_agent`'s
    /// `.find(|a| &a.name == declared)` is correct by inspection against a
    /// single-agent fixture, but that can't tell "found the right one" apart
    /// from "returned the only one there was" — this fixture can, because
    /// alpha's and gamma's values would make the assertions below fail just
    /// as loudly as a completely wrong agent would.
    #[test]
    fn a_declared_ref_finds_the_middle_agent_among_several() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            r#"
defaults:
  provider: claude

agents:
  - name: alpha
    description: the first agent
    provider: claude
    prompt: [alpha-frag]
  - name: beta
    description: the middle agent
    provider: openai
    model: gpt-5
    prompt: [beta-frag]
  - name: gamma
    description: the last agent
    provider: gemini
    prompt: [gamma-frag]
"#,
        )
        .unwrap();

        let fragments = vec![
            crate::prompt::Prompt {
                name: "alpha-frag".into(),
                description: None,
                apply_to: vec![],
                body: "Alpha body.".into(),
                source: std::path::PathBuf::from("alpha.md"),
            },
            crate::prompt::Prompt {
                name: "beta-frag".into(),
                description: None,
                apply_to: vec![],
                body: "Beta body.".into(),
                source: std::path::PathBuf::from("beta.md"),
            },
            crate::prompt::Prompt {
                name: "gamma-frag".into(),
                description: None,
                apply_to: vec![],
                body: "Gamma body.".into(),
                source: std::path::PathBuf::from("gamma.md"),
            },
        ];

        let r = AgentRef::Declared {
            declared: "beta".into(),
        };
        let agent = load_agent(&r, dir.path(), &fragments).unwrap();

        assert_eq!(agent.name, "beta");
        // Neither alpha's nor gamma's provider/model/prompt — beta's own.
        assert_eq!(agent.metadata.provider, "openai");
        assert_eq!(agent.metadata.model, Some("gpt-5".to_string()));
        assert_eq!(agent.system_prompt, "Beta body.");
    }

    /// `AgentRef` is `#[serde(untagged)]`. Adding `Declared` must not shift
    /// how a plain `- name: x` entry resolves — untagged enums try each
    /// variant in order and a missing required field is what actually
    /// disambiguates them, but that is easy to get wrong when a fourth
    /// variant with its own single required field joins the set.
    #[test]
    fn a_named_entry_still_deserialises_as_named_not_declared() {
        let refs: Vec<AgentRef> = serde_yaml_ng::from_str("- name: core-specialist\n").unwrap();
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
        let refs: Vec<AgentRef> = serde_yaml_ng::from_str("- declared: core-specialist\n").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[0],
            AgentRef::Declared {
                declared: "core-specialist".to_string()
            }
        );
    }
}
