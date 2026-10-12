//! Reading a legacy project with the `armadai-core` loaders.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use armadai_core::agent::Agent;
use armadai_core::project::{AgentRef, ProjectConfig};
use armadai_core::prompt::Prompt;
use armadai_core::{agent_decl, agent_source, parser, project};

use super::leftover::{Leftover, leftover};

/// Legacy config files, in the order the legacy loader prefers them.
pub const CONFIG_FILES: [&str; 3] = [".armadai/config.yaml", "armadai.yaml", "armadai.yml"];

/// A legacy skill directory and the raw text of its `SKILL.md`.
#[derive(Debug, Clone)]
pub struct OldSkill {
    pub dir: PathBuf,
    pub text: String,
}

#[derive(Debug)]
pub struct OldProject {
    pub root: PathBuf,
    pub config: ProjectConfig,
    pub config_file: PathBuf,
    pub agents: Vec<Agent>,
    pub prompts: Vec<Prompt>,
    pub skills: Vec<OldSkill>,
    /// Declared agent name -> its `description:` in `agents.yaml`.
    pub declared_descriptions: BTreeMap<String, String>,
    /// Agent `.md` source -> the text the legacy parser left out of it.
    pub leftovers: BTreeMap<PathBuf, Leftover>,
    /// Prompt fragments composed into declared agents.
    pub fragment_sources: Vec<PathBuf>,
    pub load_warnings: Vec<String>,
}

/// Read the legacy project at `root` (not walking up: `root` is the project).
pub fn read_old(root: &Path) -> anyhow::Result<OldProject> {
    let root = std::fs::canonicalize(root)
        .with_context(|| format!("No legacy project found at {}", root.display()))?;
    let config_file = CONFIG_FILES
        .iter()
        .map(|f| root.join(f))
        .find(|p| p.is_file())
        .with_context(|| {
            format!(
                "No legacy project found at {} (none of {})",
                root.display(),
                CONFIG_FILES.join(", ")
            )
        })?;
    let config = ProjectConfig::load(&config_file)
        .with_context(|| format!("cannot read {}", config_file.display()))?;
    let mut warnings = Vec::new();

    // File-backed agents, each file once even when referenced twice.
    let mut paths: Vec<PathBuf> = Vec::new();
    for r in config
        .agents
        .iter()
        .filter(|r| !matches!(r, AgentRef::Declared { .. }))
    {
        match project::resolve_agent(r, &root) {
            Ok(p) if !paths.contains(&p) => paths.push(p),
            Ok(_) => {}
            Err(e) => warnings.push(format!("agent not migrated: {e}")),
        }
    }
    let mut agents = Vec::new();
    let mut leftovers = BTreeMap::new();
    for p in &paths {
        match parser::parse_agent_file(p) {
            Ok(a) => {
                // the parser read it a moment ago: a failure here is a real I/O error
                let raw = std::fs::read_to_string(p)
                    .with_context(|| format!("cannot read {}", p.display()))?;
                leftovers.insert(p.clone(), leftover(&raw));
                agents.push(a)
            }
            Err(e) => warnings.push(format!(
                "agent not migrated: failed to parse {}: {e:#}",
                p.display()
            )),
        }
    }

    // Declared agents. Unlike `agent_source::load_all_agents`, a slug collision does
    // not drop an agent here: `convert` renames it instead, so nothing is lost.
    let mut declared_descriptions = BTreeMap::new();
    let mut fragment_sources: Vec<PathBuf> = Vec::new();
    let decls_path = agent_source::declarations_path(&root);
    if decls_path.is_file() {
        let decls = agent_decl::load(&decls_path)?;
        let fragments = agent_source::project_fragments(&root);
        for d in &decls.agents {
            let built = agent_decl::merge_metadata(d, &decls.defaults).and_then(|metadata| {
                Ok(Agent {
                    name: d.name.clone(),
                    source: decls_path.clone(),
                    metadata,
                    system_prompt: agent_decl::compose_prompt(&d.prompt, d, &fragments)?,
                    instructions: None,
                    output_format: None,
                    pipeline: None,
                    context: None,
                })
            });
            match built {
                Ok(a) => {
                    for step in &d.prompt {
                        let name = match step {
                            agent_decl::PromptStep::Plain(n) => n,
                            agent_decl::PromptStep::Parameterised { fragment, .. } => fragment,
                        };
                        let used = fragments.iter().find(|f| &f.name == name);
                        if let Some(f) = used.filter(|f| !fragment_sources.contains(&f.source)) {
                            fragment_sources.push(f.source.clone());
                        }
                    }
                    if let Some(desc) = &d.description {
                        declared_descriptions.insert(d.name.clone(), desc.clone());
                    }
                    agents.push(a);
                }
                Err(e) => warnings.push(format!("declared agent `{}` not migrated: {e}", d.name)),
            }
        }
    }

    let (prompt_paths, errs) = project::resolve_all_prompts(&config, &root);
    warnings.extend(
        errs.into_iter()
            .map(|e| format!("prompt not migrated: {e}")),
    );
    let mut prompts = Vec::new();
    for p in prompt_paths {
        match Prompt::load(&p) {
            Ok(pr) => prompts.push(pr),
            Err(e) => warnings.push(format!(
                "prompt not migrated: failed to parse {}: {e}",
                p.display()
            )),
        }
    }

    let (skill_dirs, errs) = project::resolve_all_skills(&config, &root);
    warnings.extend(errs.into_iter().map(|e| format!("skill not migrated: {e}")));
    let mut skills = Vec::new();
    for dir in skill_dirs {
        match std::fs::read_to_string(dir.join("SKILL.md")) {
            Ok(text) => skills.push(OldSkill { dir, text }),
            Err(e) => warnings.push(format!(
                "skill not migrated: cannot read {}: {e}",
                dir.join("SKILL.md").display()
            )),
        }
    }

    Ok(OldProject {
        root,
        config,
        config_file,
        agents,
        prompts,
        skills,
        declared_descriptions,
        leftovers,
        fragment_sources,
        load_warnings: warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        dir
    }

    const AGENT: &str =
        "# Dev\n\n## Metadata\n- provider: anthropic\n\n## System Prompt\n\nYou code.\n";

    #[test]
    fn no_config_is_an_error() {
        let dir = tree(&[("README.md", "x")]);
        let e = read_old(dir.path()).unwrap_err().to_string();
        assert!(e.contains("No legacy project found"), "{e}");
    }

    #[test]
    fn file_referenced_twice_is_read_once_and_missing_ref_warns() {
        let dir = tree(&[
            (
                ".armadai/config.yaml",
                "agents:\n  - name: dev\n  - path: .armadai/agents/dev.md\n  - name: ghost\n",
            ),
            (".armadai/agents/dev.md", AGENT),
        ]);
        let _env = armadai_core::test_support::IsolatedConfigDir::enter();
        let old = read_old(dir.path()).unwrap();
        assert_eq!(old.agents.len(), 1);
        assert_eq!(old.load_warnings.len(), 1, "{:?}", old.load_warnings);
        assert!(old.load_warnings[0].contains("ghost"));
    }

    #[test]
    fn declared_agents_with_colliding_slugs_are_both_kept() {
        let dir = tree(&[
            (".armadai/config.yaml", "agents: []\n"),
            (
                ".armadai/agents.yaml",
                "defaults: { provider: claude }\nagents:\n  - name: Core Dev\n    description: one\n    prompt: [base]\n  - name: core-dev\n    prompt: [base]\n",
            ),
            (".armadai/prompts/base.md", "Base text.\n"),
        ]);
        let _env = armadai_core::test_support::IsolatedConfigDir::enter();
        let old = read_old(dir.path()).unwrap();
        assert_eq!(old.agents.len(), 2, "{:?}", old.load_warnings);
        assert_eq!(old.declared_descriptions["Core Dev"], "one");
        assert_eq!(old.agents[0].system_prompt, "Base text.");
    }
}
