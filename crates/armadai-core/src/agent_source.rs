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
        if let Some(collision) = shadowing_conflict(&decl.name, libraries)? {
            anyhow::bail!(shadowing_message(&decl.name, &decls_path, &collision));
        }
    }
    Ok(())
}

/// Does `name` (compared as a slug) collide with a `.md` file in any of
/// `libraries`? Returns the colliding file's path if so, `None` otherwise.
///
/// Factored out of `check_no_shadowing` so a single-name check —
/// `load_agent_by_name` needs one for just the name it is resolving,
/// `load_all_agents` needs one per declaration so a single collision costs
/// only that declaration — and a whole-fleet check (`check_no_shadowing`
/// itself, still used as a fast go/no-op reusable check) share one
/// slug-comparison, one skip-missing-directory rule, and one skip-non-`.md`
/// rule, rather than three copies that could drift.
fn shadowing_conflict(name: &str, libraries: &[PathBuf]) -> anyhow::Result<Option<PathBuf>> {
    let slug = crate::agent::slugify(name);
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
            if crate::agent::slugify(stem) == slug {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

/// The message used whenever a declared name collides with a file-backed
/// agent — shared by `check_no_shadowing`, `load_all_agents`, and
/// `load_agent_by_name` so the wording (and the fact that neither side is
/// given precedence) can't drift across the three collision-detection call
/// sites.
fn shadowing_message(decl_name: &str, decls_path: &Path, file_path: &Path) -> String {
    format!(
        "agent '{decl_name}' is declared in {} and also written as {} — \
         remove one; there is deliberately no precedence between them",
        decls_path.display(),
        file_path.display()
    )
}

/// Every prompt fragment a project's declarations can reference, gathered
/// from the same three tiers `project::resolve_prompt` searches by name:
/// `.armadai/prompts/` (preferred), the legacy `prompts/`, then the user's
/// global library. A name defined in more than one tier keeps its most
/// local definition — the same "closer wins" rule the rest of resolution
/// uses.
///
/// Every caller that turns declared agents into something usable (`link`,
/// `list`, `run`, `inspect`) needs this fragment set, so it lives here
/// rather than being reimplemented at each call site.
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

/// Every agent a project has: those resolved from files, plus those declared.
///
/// Same `(values, non-fatal errors)` shape as `resolve_all_agents`, so callers
/// keep their existing warning loop. A broken declaration costs its own agent,
/// never the whole fleet — a fleet that vanishes because one agent has a typo
/// is worse than a fleet with a gap and a warning. **This is true of a
/// shadowing collision too**: it costs only the colliding declaration (with a
/// warning naming both sources, via `shadowing_message`), not every other
/// declared agent — the first cut of this function bailed out of the whole
/// declarations block on the first collision found, which meant one
/// accidental collision (plausibly against a file the project doesn't even
/// own, since the check spans the user's whole global library) silently
/// dropped every other declared agent while callers kept exiting 0.
pub fn load_all_agents(
    config: &crate::project::ProjectConfig,
    root: &Path,
    fragments: &[Prompt],
) -> (Vec<Agent>, Vec<String>) {
    let (paths, mut errors) = crate::project::resolve_all_agents(config, root);
    let mut agents = Vec::new();
    for path in &paths {
        match crate::parser::parse_agent_file(path) {
            Ok(a) => agents.push(a),
            Err(e) => errors.push(format!("failed to parse {}: {e}", path.display())),
        }
    }

    let decls_path = declarations_path(root);
    if decls_path.is_file() {
        let libraries = crate::project::library_dirs(root);
        match agent_decl::load(&decls_path) {
            Ok(decls) => {
                for decl in &decls.agents {
                    match shadowing_conflict(&decl.name, &libraries) {
                        Ok(Some(collision)) => {
                            errors.push(shadowing_message(&decl.name, &decls_path, &collision));
                            continue;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            errors.push(format!("{e}"));
                            continue;
                        }
                    }
                    match agent_decl::to_agent(decl, &decls.defaults, fragments, decls_path.clone())
                    {
                        Ok(a) => agents.push(a),
                        Err(e) => errors.push(format!("{e}")),
                    }
                }
            }
            Err(e) => errors.push(format!("{e}")),
        }
    }

    (agents, errors)
}

/// Find one agent by name, whether it is declared or written as a file.
///
/// Resolution order: file-backed first — a config `AgentRef` matching `name`
/// (mirroring `AgentResolution::Project`'s own ref-matching in `cli::run` and
/// `cli::inspect`, which this replaces), or failing that a bare `Named`
/// lookup — then the project's declarations. There is deliberately no
/// precedence rule for a name that resolves both ways: this function refuses
/// it outright (see the `shadowing_conflict` check below), the same refusal
/// `load_all_agents` applies per-declaration and `check_no_shadowing` applies
/// fleet-wide — a name ambiguous enough to fail `link`/`list` must fail
/// `run`/`inspect` too, not silently resolve to whichever side is tried
/// first.
///
/// A `config.agents` entry of `AgentRef::Declared` matching `name` is never
/// handed to `resolve_agent` (which always refuses that variant) — it is
/// left unmatched here so resolution falls through to the declarations
/// lookup below, which is what actually resolves it.
pub fn load_agent_by_name(
    name: &str,
    config: &crate::project::ProjectConfig,
    project_root: &Path,
    fragments: &[Prompt],
) -> anyhow::Result<Agent> {
    let matched_ref = config.agents.iter().find(|r| match r {
        AgentRef::Named { name: n } => n == name,
        AgentRef::Path { path } => path.file_stem().is_some_and(|s| s == name),
        AgentRef::Registry { registry } => registry.ends_with(name),
        AgentRef::Declared { .. } => false,
    });

    let file_result = match matched_ref {
        Some(agent_ref) => resolve_agent(agent_ref, project_root),
        None => resolve_agent(
            &AgentRef::Named {
                name: name.to_string(),
            },
            project_root,
        ),
    };

    let decls_path = declarations_path(project_root);
    let decls = if decls_path.is_file() {
        Some(agent_decl::load(&decls_path)?)
    } else {
        None
    };
    let declared = decls
        .as_ref()
        .and_then(|d| d.agents.iter().find(|a| a.name == name));

    // Collision check scoped to just this one name — no need to scan the
    // whole fleet the way `check_no_shadowing` does for `load_all_agents`.
    // Checked against `project::library_dirs` directly, not against whether
    // `file_result` happened to succeed: an explicit `path:`/`registry:` ref
    // pointing outside those library dirs is not the ambiguity
    // `check_no_shadowing` polices, and using `file_result` here would let
    // `run`/`inspect` accept a name `link`/`list` refuse (or the reverse).
    if declared.is_some()
        && let Some(collision) =
            shadowing_conflict(name, &crate::project::library_dirs(project_root))?
    {
        anyhow::bail!(shadowing_message(name, &decls_path, &collision));
    }

    match file_result {
        Ok(path) => crate::parser::parse_agent_file(&path),
        Err(file_err) => match declared {
            Some(decl) => agent_decl::to_agent(
                decl,
                &decls
                    .as_ref()
                    .expect("declared borrows from decls")
                    .defaults,
                fragments,
                decls_path,
            ),
            None => anyhow::bail!(
                "agent '{name}' not found: not resolvable as a file ({file_err}), \
                 and not declared in {}",
                decls_path.display()
            ),
        },
    }
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

    // -----------------------------------------------------------------
    // load_all_agents
    // -----------------------------------------------------------------

    /// Point the global agent library (`ARMADAI_CONFIG_DIR`) at an empty
    /// temp dir for the duration of `f`.
    ///
    /// `load_all_agents`'s real `library_dirs(root)` always includes
    /// `user_agents_dir()` — on a machine that has ever run `armadai
    /// extract`/`init` from this very repo, that is a REAL directory
    /// holding this project's own team agents (`core-specialist.md`
    /// included). A test exercising `load_all_agents` without isolating
    /// this can pass — or fail — for a reason that has nothing to do with
    /// its own fixture, and would behave differently on a machine that has
    /// never run those commands. Serialised via `ENV_MUTEX` since it
    /// mutates a process-global env var.
    fn with_isolated_global_library<T>(f: impl FnOnce() -> T) -> T {
        let _guard = crate::config::ENV_MUTEX.lock().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        let empty = tempfile::tempdir().unwrap();
        // SAFETY: serialised via ENV_MUTEX above.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", empty.path());
        }
        let result = f();
        // SAFETY: restoring original env state, still under the guard.
        unsafe {
            match &orig {
                Some(v) => std::env::set_var("ARMADAI_CONFIG_DIR", v),
                None => std::env::remove_var("ARMADAI_CONFIG_DIR"),
            }
        }
        result
    }

    /// Build a `ProjectConfig` whose agent list holds one `AgentRef::Path`,
    /// following `project.rs`'s own `test_resolve_all_agents` construction
    /// rather than inventing a second way to build one.
    fn config_listing_agent_path(
        _project_root: &std::path::Path,
        rel_path: &str,
    ) -> crate::project::ProjectConfig {
        crate::project::ProjectConfig {
            agents: vec![AgentRef::Path {
                path: std::path::PathBuf::from(rel_path),
            }],
            ..Default::default()
        }
    }

    /// A project with one `.md` agent and one declared agent must yield both.
    #[test]
    fn declared_and_file_agents_are_loaded_together() {
        with_isolated_global_library(|| {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
            std::fs::write(
                dir.path().join(".armadai/agents.yaml"),
                "defaults:\n  provider: claude\nagents:\n  - name: declared-one\n    prompt: [base]\n",
            )
            .unwrap();
            let md_dir = dir.path().join("agents");
            std::fs::create_dir_all(&md_dir).unwrap();
            std::fs::write(
                md_dir.join("file-one.md"),
                "# file-one\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi\n",
            )
            .unwrap();
            let config = config_listing_agent_path(dir.path(), "agents/file-one.md");

            let (agents, errors) = load_all_agents(&config, dir.path(), &fragments());
            let mut names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
            names.sort_unstable();
            assert_eq!(
                names,
                vec!["declared-one", "file-one"],
                "errors: {errors:?}"
            );
        });
    }

    /// A project with no declarations must behave exactly as before.
    #[test]
    fn a_project_without_declarations_loads_only_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let md_dir = dir.path().join("agents");
        std::fs::create_dir_all(&md_dir).unwrap();
        std::fs::write(
            md_dir.join("only.md"),
            "# only\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi\n",
        )
        .unwrap();
        let config = config_listing_agent_path(dir.path(), "agents/only.md");

        let (agents, _) = load_all_agents(&config, dir.path(), &[]);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "only");
    }

    /// One bad declaration must not silence the good agents — the existing
    /// callers print warnings and carry on, and that contract must hold.
    #[test]
    fn a_broken_declaration_becomes_an_error_string_not_a_lost_fleet() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        // `ghost` is not a known fragment -> composition fails for this agent.
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  - name: broken\n    prompt: [ghost]\n  \
             - name: fine\n    prompt: [base]\n",
        )
        .unwrap();
        let config = config_listing_agent_path(dir.path(), "agents/none.md");

        let (agents, errors) = load_all_agents(&config, dir.path(), &fragments());
        assert!(
            agents.iter().any(|a| a.name == "fine"),
            "the healthy agent must survive: {errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.contains("broken")),
            "the broken one must be reported by name: {errors:?}"
        );
    }

    /// A declared agent and a same-named file must still refuse to load at
    /// all via `load_all_agents` — the shadowing check now actually runs.
    /// The file that collides is local to the fixture (`agents/`), but the
    /// check spans `library_dirs`, which also includes the real global
    /// library — isolated here so THIS fixture is what makes the test pass,
    /// not whatever happens to be installed on the machine running it.
    #[test]
    fn load_all_agents_refuses_a_shadowed_declaration() {
        with_isolated_global_library(|| {
            let dir = tempfile::tempdir().unwrap();
            project(dir.path()); // declares `core-specialist`
            let md_dir = dir.path().join("agents");
            std::fs::create_dir_all(&md_dir).unwrap();
            std::fs::write(md_dir.join("core-specialist.md"), "# Core").unwrap();
            let config = crate::project::ProjectConfig::default();

            let (agents, errors) = load_all_agents(&config, dir.path(), &fragments());
            assert!(
                agents.iter().all(|a| a.name != "core-specialist"),
                "a shadowed name must not be silently loaded from either side: {agents:?}"
            );
            assert!(
                errors.iter().any(|e| e.contains("core-specialist")),
                "the collision must be reported: {errors:?}"
            );
        });
    }

    /// A collision must cost only the colliding declaration — every other
    /// declared agent, including ones declared alongside it in the same
    /// `agents.yaml`, must still load. This is the Finding-1 regression: the
    /// first cut of `load_all_agents` bailed on the whole declarations block
    /// at the first collision, dropping every declared agent, not just the
    /// colliding one.
    #[test]
    fn a_shadowing_collision_costs_only_its_own_declaration_not_the_fleet() {
        with_isolated_global_library(|| {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
            std::fs::write(
                dir.path().join(".armadai/agents.yaml"),
                "defaults:\n  provider: claude\nagents:\n  \
                 - name: shadowed-one\n    prompt: [base]\n  \
                 - name: healthy-one\n    prompt: [base]\n",
            )
            .unwrap();
            let md_dir = dir.path().join("agents");
            std::fs::create_dir_all(&md_dir).unwrap();
            std::fs::write(md_dir.join("shadowed-one.md"), "# Shadowed").unwrap();
            let config = crate::project::ProjectConfig::default();

            let (agents, errors) = load_all_agents(&config, dir.path(), &fragments());
            assert!(
                agents.iter().any(|a| a.name == "healthy-one"),
                "the un-shadowed declaration must survive: agents={agents:?} errors={errors:?}"
            );
            assert!(
                agents.iter().all(|a| a.name != "shadowed-one"),
                "the shadowed name must not load from either side: {agents:?}"
            );
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("shadowed-one") && e.contains("agents.yaml")),
                "the collision must be reported, naming both sources: {errors:?}"
            );
        });
    }

    // -----------------------------------------------------------------
    // load_agent_by_name
    // -----------------------------------------------------------------

    /// A declared agent must be reachable by name with no `AgentRef` at all
    /// listing it in `armadai.yaml` — the whole point of "every agent in
    /// `agents.yaml` is included automatically".
    ///
    /// Deliberately does NOT reuse the shared `project()`/`core-specialist`
    /// fixture: `load_agent_by_name` tries a file-backed lookup first, which
    /// walks the REAL `~/.config/armadai/agents/` on this machine (this very
    /// repo's own `core-specialist` team agent is routinely extracted there).
    /// A name that could plausibly exist globally would make this test pass
    /// for the wrong reason on a dev box while still passing in CI — so this
    /// uses a name no real global library will ever hold.
    #[test]
    fn load_agent_by_name_finds_a_declared_agent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  \
             - name: zzz-declared-only-agent\n    prompt: [base]\n",
        )
        .unwrap();
        let config = crate::project::ProjectConfig::default();

        let agent =
            load_agent_by_name("zzz-declared-only-agent", &config, dir.path(), &fragments())
                .unwrap();
        assert_eq!(agent.name, "zzz-declared-only-agent");
        assert_eq!(agent.system_prompt, "You are zzz-declared-only-agent.");
        assert_eq!(agent.metadata.provider, "claude");
    }

    /// Regression guard on `run`/`inspect`'s most common path: a plain
    /// `.md` agent, referenced by a `path:` entry, must resolve to the SAME
    /// agent it always did. Every assertion below is a value this test would
    /// catch going wrong (wrong file loaded, empty metadata, wrong prompt) —
    /// a bare "it loaded" assertion could not tell that apart from a bug
    /// that returns some other agent, or a half-populated one.
    #[test]
    fn load_agent_by_name_still_resolves_a_file_backed_agent_exactly_as_before() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("sentinel.md"),
            "# Sentinel Prime\n\n\
             ## Metadata\n\
             - provider: cli\n\
             - command: sentinel-cmd\n\
             - model: sentinel-model-x\n\
             - tags: alpha-tag\n\n\
             ## System Prompt\n\n\
             Guard the perimeter, sentinel-style.\n",
        )
        .unwrap();
        let config = crate::project::ProjectConfig {
            agents: vec![AgentRef::Path {
                path: std::path::PathBuf::from("sentinel.md"),
            }],
            ..Default::default()
        };

        let agent = load_agent_by_name("sentinel", &config, dir.path(), &[]).unwrap();
        assert_eq!(agent.name, "Sentinel Prime");
        assert_eq!(agent.metadata.provider, "cli");
        assert_eq!(agent.metadata.command.as_deref(), Some("sentinel-cmd"));
        assert_eq!(agent.metadata.model.as_deref(), Some("sentinel-model-x"));
        assert_eq!(agent.metadata.tags, vec!["alpha-tag".to_string()]);
        assert_eq!(
            agent.system_prompt.trim(),
            "Guard the perimeter, sentinel-style."
        );
    }

    /// An unknown name must error, naming both places that were searched: the
    /// file-backed resolution (via the underlying `resolve_agent` error) and
    /// `agents.yaml` — a mistyped declared agent's name must not be reported
    /// as though only one of the two had been consulted.
    #[test]
    fn an_unknown_name_errors_naming_both_places_searched() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path()); // .armadai/agents.yaml exists, declares core-specialist
        let config = crate::project::ProjectConfig::default();

        let err = load_agent_by_name("ghost-agent", &config, dir.path(), &fragments())
            .unwrap_err()
            .to_string();
        assert!(err.contains("ghost-agent"), "must name the agent: {err}");
        assert!(
            err.contains("agents.yaml"),
            "must say it looked in the declarations: {err}"
        );
        assert!(
            err.contains("not found") || err.contains("not resolvable"),
            "must say it looked for a file too: {err}"
        );
    }

    /// The reachability premise ("files first, then declarations, no
    /// precedence rule") only holds if a name shared by both has already
    /// been refused. Before Finding 2's fix, `load_agent_by_name` never
    /// checked, so `run`/`inspect` silently picked the file-backed side of
    /// a name that `link`/`list` (via `check_no_shadowing`) already refuse —
    /// two commands disagreeing about whether the exact same project is
    /// valid.
    #[test]
    fn load_agent_by_name_refuses_a_shadowed_name() {
        with_isolated_global_library(|| {
            let dir = tempfile::tempdir().unwrap();
            project(dir.path()); // declares `core-specialist`
            let md_dir = dir.path().join("agents");
            std::fs::create_dir_all(&md_dir).unwrap();
            std::fs::write(md_dir.join("core-specialist.md"), "# Core").unwrap();
            let config = crate::project::ProjectConfig::default();

            let err = load_agent_by_name("core-specialist", &config, dir.path(), &fragments())
                .unwrap_err()
                .to_string();
            assert!(err.contains("core-specialist"), "must name it: {err}");
            assert!(
                err.contains("agents.yaml") && err.contains(".md"),
                "must name both sources so the reader can pick one: {err}"
            );
        });
    }
}
