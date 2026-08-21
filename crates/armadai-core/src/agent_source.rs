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

/// One agent `load_all_agents` could not include, and why — the
/// distinction a caller that WRITES config (`link`) needs and a caller that
/// only DISPLAYS it (`list`) does not.
///
/// `link` must refuse to write a smaller fleet than the one declared, but
/// only when this chantier's declarative format is the reason: a dropped
/// declaration, or a shadowing collision. A failure that predates it
/// entirely — an unparseable `.md`, an `AgentRef` that does not resolve to
/// any file — keeps its exact old behaviour (warn, link what did load, exit
/// 0), because that behaviour was never wrong; only the new format's own
/// failures are. `list` prints every variant's `message()` alike and never
/// refuses anything, being read-only.
///
/// Deliberately structured rather than a flat `String`, so a caller decides
/// by matching a variant — never by inspecting message text, which is free
/// to reword without silently changing what refuses a write.
#[derive(Debug, Clone)]
pub enum LoadWarning {
    /// Unrelated to declarative agents: an unparseable `.md`, or a
    /// `path:`/`named`/`registry:` ref that did not resolve to a file.
    PreExisting(String),
    /// A single named declared agent this chantier's format failed to
    /// load — a broken declaration (metadata/composition error) or a name
    /// that collided with a file-backed agent. Named so a caller narrowing
    /// by `--agents` can tell whether the loss is even part of what it is
    /// about to write.
    Dropped { agent: String, message: String },
    /// The whole `.armadai/agents.yaml` failed to load (unreadable, or a
    /// YAML error) — an unknown number of declared agents, of unknown
    /// names, are lost. Cannot be scoped to a `--agents` filter the way
    /// `Dropped` can: any requested name might have been among them.
    DeclarationsUnreadable(String),
}

impl LoadWarning {
    /// The human-readable text every caller's warning loop prints, whatever
    /// the variant — `link`/`list` never format these three differently.
    pub fn message(&self) -> &str {
        match self {
            LoadWarning::PreExisting(m)
            | LoadWarning::Dropped { message: m, .. }
            | LoadWarning::DeclarationsUnreadable(m) => m,
        }
    }
}

/// Whether `warnings` contain a loss a write-side caller (`link`) must
/// refuse over, given the agents actually being requested: `None` for the
/// whole fleet, `Some(names)` for a `--agents` filter (compared
/// case-insensitively, matching `link`'s own filter).
///
/// A `PreExisting` loss never blocks — see [`LoadWarning`]'s own doc for
/// why. A `DeclarationsUnreadable` loss always blocks: it cannot be scoped
/// by name. A `Dropped` loss blocks only when its agent is among the ones
/// requested — `--agents good` must not refuse over a `bad` this chantier's
/// format dropped when `bad` was never going to be written anyway.
pub fn blocks_a_write(warnings: &[LoadWarning], requested: Option<&[String]>) -> bool {
    warnings.iter().any(|w| match w {
        LoadWarning::PreExisting(_) => false,
        LoadWarning::DeclarationsUnreadable(_) => true,
        LoadWarning::Dropped { agent, .. } => match requested {
            None => true,
            Some(names) => names
                .iter()
                .any(|n| n.to_lowercase() == agent.to_lowercase()),
        },
    })
}

/// Every agent a project has: those resolved from files, plus those declared.
///
/// Same `(values, warnings)` shape as `resolve_all_agents`, so callers keep
/// their existing warning loop — just typed as [`LoadWarning`] instead of a
/// flat `String`, so a write-side caller can tell a pre-existing failure
/// from a loss this chantier's format is responsible for (see
/// [`blocks_a_write`]). A broken declaration or a shadowing collision costs
/// only its own agent, never the whole fleet — a fleet that vanishes
/// because one agent has a typo, or because one collision happens to fall
/// against a file the project doesn't even own (the check spans the user's
/// whole global library), is worse than a fleet with a gap and a warning.
pub fn load_all_agents(
    config: &crate::project::ProjectConfig,
    root: &Path,
    fragments: &[Prompt],
) -> (Vec<Agent>, Vec<LoadWarning>) {
    let mut agents = Vec::new();
    let mut warnings = Vec::new();

    // File-backed agents. `AgentRef::Declared` is skipped here rather than
    // handed to `resolve_agent` (which always refuses that variant, by
    // design, with a message naming `.armadai/agents.yaml`): the agent it
    // names loads fine below, via the declarations block, regardless of
    // whether `config.agents` names it at all — every declared agent is
    // included automatically. Passing it through would report a real,
    // loadable agent as an unresolvable file, every time.
    for agent_ref in config
        .agents
        .iter()
        .filter(|r| !matches!(r, AgentRef::Declared { .. }))
    {
        match resolve_agent(agent_ref, root) {
            Ok(path) => match crate::parser::parse_agent_file(&path) {
                Ok(a) => agents.push(a),
                Err(e) => warnings.push(LoadWarning::PreExisting(format!(
                    "failed to parse {}: {e}",
                    path.display()
                ))),
            },
            Err(e) => warnings.push(LoadWarning::PreExisting(format!("{e}"))),
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
                            warnings.push(LoadWarning::Dropped {
                                agent: decl.name.clone(),
                                message: shadowing_message(&decl.name, &decls_path, &collision),
                            });
                            continue;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warnings.push(LoadWarning::Dropped {
                                agent: decl.name.clone(),
                                message: format!("{e}"),
                            });
                            continue;
                        }
                    }
                    match agent_decl::to_agent(decl, &decls.defaults, fragments, decls_path.clone())
                    {
                        Ok(a) => agents.push(a),
                        Err(e) => warnings.push(LoadWarning::Dropped {
                            agent: decl.name.clone(),
                            message: format!("{e}"),
                        }),
                    }
                }
            }
            // The whole file failed (unreadable, or a top-level YAML
            // error): every agent it would have declared is lost, none of
            // them nameable, so this cannot be scoped to a `--agents`
            // filter the way one bad declaration can — see
            // `blocks_a_write`.
            Err(e) => warnings.push(LoadWarning::DeclarationsUnreadable(format!("{e}"))),
        }
    }

    (agents, warnings)
}

/// Find one agent by name, whether it is declared or written as a file.
///
/// Resolution order: file-backed first — a config `AgentRef` matching `name`
/// (mirroring `AgentResolution::Project`'s own ref-matching in `cli::run` and
/// `cli::inspect`, which this replaces), or failing that a bare `Named`
/// lookup — then the project's declarations. There is deliberately no
/// precedence rule for a name that resolves both ways: this function refuses
/// it outright (see the `shadowing_conflict` check below), the same refusal
/// `load_all_agents` applies per-declaration — a name ambiguous enough to
/// fail `link`/`list` must fail `run`/`inspect` too, not silently resolve to
/// whichever side is tried first.
///
/// A `config.agents` entry of `AgentRef::Declared` matching `name` is never
/// handed to `resolve_agent` (which always refuses that variant) — it is
/// left unmatched here so resolution falls through to the declarations
/// lookup below, which is what actually resolves it.
///
/// An unreadable `.armadai/agents.yaml` does not fail this function on its
/// own: it cannot declare `name`, so it cannot collide with a file-backed
/// `name` either. When the file-backed lookup ALSO succeeds, that `.md` is
/// unambiguous and is served, with a warning (`tracing::warn!`) so the
/// broken yaml is not silently invisible. Only when the file-backed lookup
/// fails too does the yaml error become the reason to fail — a working
/// declaration could have provided this name, so its parse error, not a
/// generic "not found", is what the caller needs to see.
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
    let (decls, decls_err) = if decls_path.is_file() {
        match agent_decl::load(&decls_path) {
            Ok(d) => (Some(d), None),
            Err(e) => (None, Some(e)),
        }
    } else {
        (None, None)
    };

    if let Some(decls_err) = decls_err {
        return match file_result {
            Ok(path) => {
                tracing::warn!(
                    "ignoring unparsable {}: {decls_err} (serving the file-backed agent \
                     '{name}' instead)",
                    decls_path.display()
                );
                crate::parser::parse_agent_file(&path)
            }
            Err(file_err) => Err(anyhow::anyhow!(
                "agent '{name}' not found as a file ({file_err}), and {} could not be read: \
                 {decls_err}",
                decls_path.display()
            )),
        };
    }

    let declared = decls
        .as_ref()
        .and_then(|d| d.agents.iter().find(|a| a.name == name));

    // Collision check scoped to just this one name — no need to scan the
    // whole fleet the way `check_no_shadowing` does for a full project
    // validation. Checked against `project::library_dirs` directly, not
    // against whether `file_result` happened to succeed: an explicit
    // `path:`/`registry:` ref pointing outside those library dirs is not
    // the ambiguity this collision check polices, and using `file_result`
    // here would let `run`/`inspect` accept a name `link`/`list` refuse (or
    // the reverse).
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
    fn shadowing_conflict_finds_a_file_matching_a_declared_name() {
        let lib = tempfile::tempdir().unwrap();
        let file = lib.path().join("core-specialist.md");
        std::fs::write(
            &file,
            "# Core\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi",
        )
        .unwrap();

        let found = shadowing_conflict("core-specialist", &[lib.path().to_path_buf()]).unwrap();
        assert_eq!(found, Some(file));
    }

    /// A declaration and a library file that only match once both are
    /// lowercased — on a case-insensitive filesystem (macOS/Windows
    /// default) a raw-filename existence check already caught this by
    /// accident; on a case-sensitive one (Linux CI) it would not, and the
    /// two would silently collide at link time instead. Comparing slugs
    /// makes the outcome the same on every platform.
    #[test]
    fn shadowing_conflict_matches_case_insensitively() {
        let lib = tempfile::tempdir().unwrap();
        let file = lib.path().join("Core-Specialist.md");
        std::fs::write(&file, "# Core").unwrap();

        let found = shadowing_conflict("core-specialist", &[lib.path().to_path_buf()]).unwrap();
        assert_eq!(found, Some(file));
    }

    /// A declaration and a library file that only collide once punctuation
    /// and whitespace are folded to `-`, exactly as the linker's `slugify`
    /// does when it names the output file. Neither a raw-name comparison
    /// nor a case-insensitive one would catch this.
    #[test]
    fn shadowing_conflict_matches_after_slug_folding() {
        let lib = tempfile::tempdir().unwrap();
        let file = lib.path().join("agent-v20.md");
        std::fs::write(&file, "# Agent").unwrap();

        let found = shadowing_conflict("Agent (v2.0)", &[lib.path().to_path_buf()]).unwrap();
        assert_eq!(found, Some(file));
    }

    #[test]
    fn shadowing_conflict_is_none_for_distinct_names() {
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("other.md"), "# Other").unwrap();
        assert_eq!(
            shadowing_conflict("core-specialist", &[lib.path().to_path_buf()]).unwrap(),
            None
        );
    }

    /// A directory that simply does not exist (no local `agents/` in this
    /// project) must not be treated as an error — it is the common case.
    #[test]
    fn a_missing_library_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert_eq!(
            shadowing_conflict("core-specialist", &[missing]).unwrap(),
            None
        );
    }

    /// The whole point of taking every candidate directory instead of one:
    /// a fix that only checked `libraries[0]` would pass this test right up
    /// until the collision moved to the second or third directory, which is
    /// exactly the gap a name-resolution step with no precedence rule
    /// cannot tolerate.
    #[test]
    fn a_collision_in_the_second_of_several_library_directories_is_caught() {
        let lib1 = tempfile::tempdir().unwrap();
        std::fs::write(lib1.path().join("other.md"), "# Other").unwrap();
        let lib2 = tempfile::tempdir().unwrap();
        let file = lib2.path().join("core-specialist.md");
        std::fs::write(&file, "# Core").unwrap();
        let lib3 = tempfile::tempdir().unwrap();
        std::fs::write(lib3.path().join("another.md"), "# Another").unwrap();

        let found = shadowing_conflict(
            "core-specialist",
            &[
                lib1.path().to_path_buf(),
                lib2.path().to_path_buf(),
                lib3.path().to_path_buf(),
            ],
        )
        .unwrap();
        assert_eq!(found, Some(file));
    }

    /// Symmetric check: a `- declared: x` entry must resolve to `Declared`,
    /// not to one of the other three variants.
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
            errors.iter().any(|e| e.message().contains("broken")),
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
                errors
                    .iter()
                    .any(|e| e.message().contains("core-specialist")),
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
                    .any(|e| e.message().contains("shadowed-one")
                        && e.message().contains("agents.yaml")),
                "the collision must be reported, naming both sources: {errors:?}"
            );
        });
    }

    /// Task 7b review, Regression 2: `armadai.yaml`'s `agents:` may list a
    /// declared agent explicitly via `- declared: x` (for routes/teams to
    /// point at deliberately) — that must never make the agent unloadable.
    /// The first cut fed every `AgentRef`, `Declared` included, straight to
    /// `resolve_all_agents`, which always refuses that variant by design;
    /// the resulting "error" then permanently blocked `link` once Regression
    /// 1 was fixed, even though the agent loads fine below via the
    /// declarations block regardless of what `config.agents` names.
    #[test]
    fn a_declared_agentref_in_config_does_not_block_its_own_agent() {
        with_isolated_global_library(|| {
            let dir = tempfile::tempdir().unwrap();
            project(dir.path()); // declares `core-specialist`, prompt: [base]
            let config = crate::project::ProjectConfig {
                agents: vec![AgentRef::Declared {
                    declared: "core-specialist".into(),
                }],
                ..Default::default()
            };

            let (agents, warnings) = load_all_agents(&config, dir.path(), &fragments());
            assert!(
                agents.iter().any(|a| a.name == "core-specialist"),
                "the declared agent must still load: warnings={warnings:?}"
            );
            assert!(
                warnings.is_empty(),
                "an explicit `declared:` ref naming a real declaration must not \
                 produce any warning at all: {warnings:?}"
            );
        });
    }

    /// A whole-file YAML failure (as opposed to one bad declaration) cannot
    /// be pinned on a single agent name, so it is classified
    /// `DeclarationsUnreadable`, not `Dropped`.
    #[test]
    fn an_unparseable_agents_yaml_is_declarations_unreadable_not_dropped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "agents:\n  - name: [unclosed\n",
        )
        .unwrap();
        let config = crate::project::ProjectConfig::default();

        let (agents, warnings) = load_all_agents(&config, dir.path(), &[]);
        assert!(
            agents.is_empty(),
            "nothing could have been declared: {agents:?}"
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            matches!(warnings[0], LoadWarning::DeclarationsUnreadable(_)),
            "must be DeclarationsUnreadable, not Dropped/PreExisting: {warnings:?}"
        );
    }

    #[test]
    fn blocks_a_write_ignores_a_pre_existing_failure() {
        let warnings = vec![LoadWarning::PreExisting("some old .md broke".into())];
        assert!(!blocks_a_write(&warnings, None));
        assert!(!blocks_a_write(&warnings, Some(&["anything".to_string()])));
    }

    #[test]
    fn blocks_a_write_is_unconditional_for_an_unreadable_declarations_file() {
        let warnings = vec![LoadWarning::DeclarationsUnreadable("bad yaml".into())];
        assert!(blocks_a_write(&warnings, None));
        assert!(blocks_a_write(&warnings, Some(&["unrelated".to_string()])));
    }

    /// The Regression 1 fix itself: `--agents good` must not refuse a link
    /// over a `bad` agent this chantier's format dropped, when `bad` was
    /// never part of what is being written.
    #[test]
    fn blocks_a_write_scopes_a_dropped_agent_to_the_request() {
        let warnings = vec![LoadWarning::Dropped {
            agent: "bad".into(),
            message: "bad is broken".into(),
        }];
        assert!(
            blocks_a_write(&warnings, None),
            "no filter means the whole fleet, including `bad`, was requested"
        );
        assert!(
            !blocks_a_write(&warnings, Some(&["good".to_string()])),
            "`bad` was never requested, so its drop must not block `good`"
        );
        assert!(
            blocks_a_write(&warnings, Some(&["bad".to_string()])),
            "`bad` WAS requested, so its own drop must block"
        );
        assert!(
            blocks_a_write(&warnings, Some(&["BAD".to_string()])),
            "the request match must be case-insensitive, like link's own --agents filter"
        );
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

    /// Task 7b review, Regression 3: `load_agent_by_name` used to load the
    /// yaml with `?` before ever looking at whether the name resolved as a
    /// file, so a malformed `.armadai/agents.yaml` broke every file-backed
    /// agent in the project, not just declared ones. An unparseable yaml
    /// cannot declare `name`, so it cannot collide with a file-backed
    /// `name` either — the file is unambiguous and must still be served.
    #[test]
    fn a_malformed_agents_yaml_does_not_break_a_file_backed_agent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "agents:
  - name: [unclosed
",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("sentinel.md"),
            "# Sentinel Prime

## Metadata
- provider: cli
- command: sentinel-cmd

## System Prompt

Guard the perimeter.
",
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
        assert_eq!(agent.metadata.command.as_deref(), Some("sentinel-cmd"));
    }

    /// The other half of Regression 3's ruling: when the name does NOT
    /// resolve as a file either, the malformed yaml IS the reason to fail —
    /// a working declaration could have provided this name — so its own
    /// parse error, not a generic "not found", must be what the caller
    /// sees.
    #[test]
    fn a_malformed_agents_yaml_fails_hard_when_the_name_is_not_a_file_either() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "agents:
  - name: [unclosed
",
        )
        .unwrap();
        let config = crate::project::ProjectConfig::default();

        let err = load_agent_by_name("nowhere", &config, dir.path(), &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("nowhere"), "must name the agent: {err}");
        assert!(
            err.contains("could not be read") && err.contains("agents.yaml"),
            "must name the yaml as the reason, not a generic not-found: {err}"
        );
        assert!(
            err.contains("cannot parse"),
            "must surface the actual yaml error, not swallow it: {err}"
        );
    }
}
