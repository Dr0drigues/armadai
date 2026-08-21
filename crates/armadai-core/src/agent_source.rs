//! Loading an `Agent`, whatever its origin.
//!
//! `project::resolve_agent` returns a **path**, which serves callers that
//! manipulate files — `model_updater` and `pack_validation` rewrite deprecated
//! models in place. A declared agent has no file of its own, so it is right
//! that `resolve_agent` fails for one. This module is for callers that want the
//! agent, not its file.

use std::path::{Path, PathBuf};

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

/// Refuse a name that exists both as a declaration and as a library file.
///
/// The obvious alternative is a precedence rule — local wins, as everywhere
/// else. It is rejected on purpose: a silent precedence recreates the very
/// duplicated truth this format exists to remove, and you would edit a `.md`
/// with no effect and nothing to tell you. Failing forces a choice.
///
/// Two refinements over a naive `library.join(name + ".md")` existence
/// check, both load-bearing:
///
/// - Names are compared as **slugs** (`agent::slugify`), not as raw
///   strings. The linker projects every agent name through the same
///   `slugify` to name its output file, so a declaration `Core-Specialist`
///   and a file `core-specialist.md` — or a declaration `Agent (v2.0)` and
///   a file `agent-v20.md` — land on the *same* filename at link time even
///   though they are not byte-for-byte equal. Comparing raw names would
///   pass both cases and let one silently overwrite the other later; only
///   comparing slugs matches what actually happens on disk.
/// - `libraries` takes **every** directory a file-backed agent can resolve
///   from (see `project::resolve_agent`'s `Named` arm: `.armadai/agents/`,
///   legacy `agents/`, and the user library), not just one. A caller that
///   checks a single directory can believe the refusal is complete when it
///   is not — and a later resolution step that tries file-backed agents
///   first and declarations second, with no precedence rule, is only safe
///   if a name shared with *any* of them has already failed here.
///
/// A library directory that does not exist is not an error — a project
/// with no local `agents/` directory is normal. A directory that exists
/// but cannot be read (permissions, not a directory after all) *is*
/// propagated as an error: unlike "there is nothing there", "it would not
/// tell me" is exactly the kind of silent gap this function exists to
/// close.
///
/// Rule `C01` of the audit already reports name collisions; loading must
/// refuse them.
pub fn check_no_shadowing(project_root: &Path, libraries: &[PathBuf]) -> anyhow::Result<()> {
    let decls_path = declarations_path(project_root);
    if !decls_path.is_file() {
        return Ok(());
    }
    let decls = agent_decl::load(&decls_path)?;
    for decl in &decls.agents {
        let decl_slug = crate::agent::slugify(&decl.name);
        for library in libraries {
            if !library.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(library)? {
                let path = entry?.path();
                if !path.extension().is_some_and(|ext| ext == "md") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if crate::agent::slugify(stem) == decl_slug {
                    anyhow::bail!(
                        "agent '{}' is declared in {} and also written as {} — \
                         remove one; there is deliberately no precedence between them",
                        decl.name,
                        decls_path.display(),
                        path.display()
                    );
                }
            }
        }
    }
    Ok(())
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

    #[test]
    fn a_name_both_declared_and_written_as_a_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path()); // declares `core-specialist`
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(
            lib.path().join("core-specialist.md"),
            "# Core\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi",
        )
        .unwrap();

        let err = check_no_shadowing(dir.path(), &[lib.path().to_path_buf()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("core-specialist"), "must name it: {err}");
        // The point is that neither wins — say so, or the reader will assume
        // one does.
        assert!(
            err.contains("agents.yaml") && err.contains(".md"),
            "must name both sources so the reader can pick one: {err}"
        );
    }

    /// A declaration and a library file that only match once both are
    /// lowercased — on a case-insensitive filesystem (macOS/Windows
    /// default) a raw-filename existence check already caught this by
    /// accident; on a case-sensitive one (Linux CI) it would not, and the
    /// two would silently collide at link time instead. Comparing slugs
    /// makes the outcome the same on every platform.
    #[test]
    fn a_declared_name_collides_with_a_file_that_only_differs_in_case() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path()); // declares `core-specialist`
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("Core-Specialist.md"), "# Core").unwrap();

        let err = check_no_shadowing(dir.path(), &[lib.path().to_path_buf()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("core-specialist"), "must name it: {err}");
        assert!(
            err.contains("agents.yaml") && err.contains(".md"),
            "must name both sources so the reader can pick one: {err}"
        );
    }

    /// A declaration and a library file that only collide once punctuation
    /// and whitespace are folded to `-`, exactly as the linker's `slugify`
    /// does when it names the output file. Neither a raw-name comparison
    /// nor a case-insensitive one would catch this.
    #[test]
    fn a_declared_name_collides_with_a_file_only_after_slug_folding() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  \
             - name: \"Agent (v2.0)\"\n    description: d\n    prompt: [base]\n",
        )
        .unwrap();
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("agent-v20.md"), "# Agent").unwrap();

        let err = check_no_shadowing(dir.path(), &[lib.path().to_path_buf()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("Agent (v2.0)"), "must name it: {err}");
        assert!(
            err.contains("agents.yaml") && err.contains(".md"),
            "must name both sources so the reader can pick one: {err}"
        );
    }

    #[test]
    fn distinct_names_are_fine() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("other.md"), "# Other").unwrap();
        assert!(check_no_shadowing(dir.path(), &[lib.path().to_path_buf()]).is_ok());
    }

    #[test]
    fn a_project_without_declarations_never_shadows() {
        let dir = tempfile::tempdir().unwrap();
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("a.md"), "# A").unwrap();
        assert!(check_no_shadowing(dir.path(), &[lib.path().to_path_buf()]).is_ok());
    }

    /// A directory that simply does not exist (no local `agents/` in this
    /// project) must not be treated as an error — it is the common case.
    #[test]
    fn a_missing_library_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());
        let missing = dir.path().join("does-not-exist");
        assert!(check_no_shadowing(dir.path(), &[missing]).is_ok());
    }

    /// The whole point of taking every candidate directory instead of one:
    /// a fix that only checked `libraries[0]` would pass this test right up
    /// until the collision moved to the second or third directory, which is
    /// exactly the gap a name-resolution step with no precedence rule
    /// cannot tolerate.
    #[test]
    fn a_collision_in_the_second_of_several_library_directories_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path()); // declares `core-specialist`
        let lib1 = tempfile::tempdir().unwrap();
        std::fs::write(lib1.path().join("other.md"), "# Other").unwrap();
        let lib2 = tempfile::tempdir().unwrap();
        std::fs::write(lib2.path().join("core-specialist.md"), "# Core").unwrap();
        let lib3 = tempfile::tempdir().unwrap();
        std::fs::write(lib3.path().join("another.md"), "# Another").unwrap();

        let err = check_no_shadowing(
            dir.path(),
            &[
                lib1.path().to_path_buf(),
                lib2.path().to_path_buf(),
                lib3.path().to_path_buf(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("core-specialist"), "must name it: {err}");
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
