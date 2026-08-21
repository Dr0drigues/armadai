mod claude;
mod codex;
mod copilot;
mod gemini;
pub mod model_resolution;
mod opencode;

pub use claude::ClaudeLinker;
pub use codex::CodexLinker;
pub use copilot::CopilotLinker;
pub use gemini::GeminiLinker;
pub use opencode::OpencodeLinker;

use std::path::PathBuf;

use armadai_core::agent::Agent;

/// Supported link targets for autocompletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LinkTarget {
    Claude,
    Codex,
    Copilot,
    Gemini,
    Opencode,
}

impl LinkTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Gemini => "gemini",
            Self::Opencode => "opencode",
        }
    }
}

impl std::fmt::Display for LinkTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A resolved agent ready for linking.
#[allow(dead_code)]
pub struct LinkAgent {
    pub name: String,
    pub system_prompt: String,
    pub instructions: Option<String>,
    pub output_format: Option<String>,
    pub context: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub stacks: Vec<String>,
    pub scope: Vec<String>,
    pub model: Option<String>,
    pub model_fallback: Vec<String>,
    pub temperature: f32,
    pub provider: Option<String>,
}

/// A file to be written by a linker.
pub struct OutputFile {
    pub path: PathBuf,
    pub content: String,
}

/// Trait for generating target-specific config files.
#[allow(dead_code)]
pub trait Linker: Send + Sync {
    fn name(&self) -> &str;
    fn default_output_dir(&self) -> &str;
    fn generate(
        &self,
        agents: &[LinkAgent],
        coordinator: Option<&LinkAgent>,
        sources: &[String],
    ) -> Vec<OutputFile>;
}

/// Create a linker for the given target name.
pub fn create_linker(target: &str) -> anyhow::Result<Box<dyn Linker>> {
    match target {
        "claude" => Ok(Box::new(ClaudeLinker)),
        "codex" => Ok(Box::new(CodexLinker)),
        "copilot" => Ok(Box::new(CopilotLinker)),
        "gemini" => Ok(Box::new(GeminiLinker)),
        "opencode" => Ok(Box::new(OpencodeLinker)),
        _ => anyhow::bail!(
            "Unknown link target: '{target}'. Supported targets: claude, codex, copilot, gemini, opencode"
        ),
    }
}

/// Convert an agent name to a kebab-case slug suitable for filenames.
///
/// Moved to `armadai_core::agent` — normalising a name into a filename-safe
/// slug is agent-domain-model territory, and `agent_source::check_no_shadowing`
/// needs the exact same normalisation to catch a declaration/file collision
/// that only matches after folding. Re-exported here so no call site in this
/// crate has to change.
pub use armadai_core::agent::slugify;

impl From<&Agent> for LinkAgent {
    fn from(agent: &Agent) -> Self {
        let description = agent
            .system_prompt
            .lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string());

        Self {
            name: agent.name.clone(),
            system_prompt: agent.system_prompt.clone(),
            instructions: agent.instructions.clone(),
            output_format: agent.output_format.clone(),
            context: agent.context.clone(),
            description,
            tags: agent.metadata.tags.clone(),
            stacks: agent.metadata.stacks.clone(),
            scope: agent.metadata.scope.clone(),
            model: agent.metadata.model.clone(),
            model_fallback: agent.metadata.model_fallback.clone(),
            temperature: agent.metadata.temperature,
            provider: Some(agent.metadata.provider.clone()),
        }
    }
}

/// Protocol block appended to linked config files for ArmadAI shell parsing.
pub fn armadai_protocol_block() -> &'static str {
    r"

## ArmadAI Response Protocol

Follow this protocol for all responses:

1. When finished responding, end with this marker on its own line:
   <!--ARMADAI_END-->

2. When delegating to a sub-agent, prefix with:
   <!--ARMADAI_DELEGATE:agent-name-->

3. Before the END marker, include metadata:
   <!--ARMADAI_META:status=complete-->
"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_simple() {
        assert_eq!(slugify("Code Reviewer"), "code-reviewer");
    }

    #[test]
    fn test_slugify_already_kebab() {
        assert_eq!(slugify("code-reviewer"), "code-reviewer");
    }

    #[test]
    fn test_slugify_underscores() {
        assert_eq!(slugify("my_test_agent"), "my-test-agent");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("Agent (v2.0)"), "agent-v20");
    }

    #[test]
    fn test_slugify_multiple_separators() {
        assert_eq!(slugify("a--b__c  d"), "a-b-c-d");
    }

    #[test]
    fn test_create_linker_claude() {
        assert!(create_linker("claude").is_ok());
    }

    #[test]
    fn test_create_linker_copilot() {
        assert!(create_linker("copilot").is_ok());
    }

    #[test]
    fn test_create_linker_gemini() {
        assert!(create_linker("gemini").is_ok());
    }

    #[test]
    fn test_create_linker_codex() {
        assert!(create_linker("codex").is_ok());
    }

    #[test]
    fn test_create_linker_opencode() {
        assert!(create_linker("opencode").is_ok());
    }

    #[test]
    fn test_create_linker_unknown() {
        assert!(create_linker("unknown").is_err());
    }

    /// A declared agent and its hand-written `.md` twin must produce the same
    /// native projection. If they diverge, the declaration format is a second
    /// source of truth rather than an alternative spelling of the first —
    /// exactly what it exists to avoid.
    ///
    /// Run against **every** target, not just claude: a divergence that only
    /// shows in the codex projection is still a divergence.
    ///
    /// Both twins carry a model, a temperature, tags and a scope (beyond the
    /// bare minimum) so the comparison actually exercises the metadata path
    /// `LinkAgent::from(&Agent)` walks, not just the system prompt.
    #[test]
    fn a_declared_agent_and_its_md_twin_project_identically() {
        let dir = tempfile::tempdir().unwrap();

        // The fragment both versions share.
        let fragments = vec![armadai_core::prompt::Prompt {
            name: "base".into(),
            description: None,
            apply_to: vec![],
            body: "You own the core domain.".into(),
            source: std::path::PathBuf::from("base.md"),
        }];

        // Declared version.
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\n  model: latest:pro\n  temperature: 0.3\n\
             agents:\n  - name: core-specialist\n    description: Core domain\n    \
             tags: [rust, domain]\n    scope: [src/core/**]\n    prompt: [base]\n",
        )
        .unwrap();
        let declared = armadai_core::agent_source::load_agent(
            &armadai_core::project::AgentRef::Declared {
                declared: "core-specialist".into(),
            },
            dir.path(),
            &fragments,
        )
        .unwrap();

        // Hand-written twin, same values.
        let md = dir.path().join("core-specialist.md");
        std::fs::write(
            &md,
            "# core-specialist\n\n## Metadata\n\
             - provider: claude\n- model: latest:pro\n- temperature: 0.3\n\
             - tags: [rust, domain]\n- scope: [src/core/**]\n\n## System Prompt\n\nYou own the core domain.\n",
        )
        .unwrap();
        let written = armadai_core::parser::parse_agent_file(&md).unwrap();

        // Sanity check before comparing projections: if the two agents are not
        // actually equivalent, an equal projection would prove nothing.
        assert_eq!(declared.system_prompt, written.system_prompt);
        assert_eq!(declared.metadata.model, written.metadata.model);
        assert_eq!(declared.metadata.temperature, written.metadata.temperature);
        assert_eq!(declared.metadata.tags, written.metadata.tags);
        assert_eq!(declared.metadata.scope, written.metadata.scope);

        for target in ["claude", "codex", "copilot", "gemini", "opencode"] {
            let linker = create_linker(target).unwrap();
            let a = linker.generate(&[LinkAgent::from(&declared)], None, &[]);
            let b = linker.generate(&[LinkAgent::from(&written)], None, &[]);

            // An empty projection on both sides would satisfy every assertion
            // below without proving anything.
            assert!(
                !a.is_empty(),
                "target {target} produced no output file — the comparison \
                 below would be vacuous"
            );
            assert_eq!(a.len(), b.len(), "target {target}: file count differs");
            for (x, y) in a.iter().zip(b.iter()) {
                assert_eq!(x.path, y.path, "target {target}: paths differ");
                assert_eq!(
                    x.content, y.content,
                    "target {target}: projection diverged — the declaration is \
                     not an alternative spelling but a second source of truth"
                );
            }
        }
    }

    /// Same invariant as above, for a `provider: cli` agent carrying
    /// `command`/`args` — fields added to the declaration format specifically
    /// so this class of agent could be declared. Nothing had yet proven a
    /// declared CLI agent projects like its `.md` twin; this is that proof.
    #[test]
    fn a_declared_cli_agent_and_its_md_twin_project_identically() {
        let dir = tempfile::tempdir().unwrap();

        let fragments = vec![armadai_core::prompt::Prompt {
            name: "base".into(),
            description: None,
            apply_to: vec![],
            body: "You wrap a CLI tool for scripted tasks.".into(),
            source: std::path::PathBuf::from("base.md"),
        }];

        // Declared version.
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "defaults:\n  provider: cli\n  temperature: 0.5\n\
             agents:\n  - name: shell-runner\n    description: Wraps a CLI tool\n    \
             command: echo\n    args: [hello, world]\n    model: local-llm\n    \
             tags: [ops, shell]\n    scope: [scripts/**]\n    prompt: [base]\n",
        )
        .unwrap();
        let declared = armadai_core::agent_source::load_agent(
            &armadai_core::project::AgentRef::Declared {
                declared: "shell-runner".into(),
            },
            dir.path(),
            &fragments,
        )
        .unwrap();

        // Hand-written twin, same values.
        let md = dir.path().join("shell-runner.md");
        std::fs::write(
            &md,
            "# shell-runner\n\n## Metadata\n\
             - provider: cli\n- command: echo\n- args: [hello, world]\n\
             - model: local-llm\n- temperature: 0.5\n- tags: [ops, shell]\n\
             - scope: [scripts/**]\n\n## System Prompt\n\nYou wrap a CLI tool for scripted tasks.\n",
        )
        .unwrap();
        let written = armadai_core::parser::parse_agent_file(&md).unwrap();

        // Sanity check before comparing projections: if the two agents are not
        // actually equivalent, an equal projection would prove nothing.
        assert_eq!(declared.system_prompt, written.system_prompt);
        assert_eq!(declared.metadata.provider, written.metadata.provider);
        assert_eq!(declared.metadata.command, written.metadata.command);
        assert_eq!(declared.metadata.args, written.metadata.args);
        assert_eq!(declared.metadata.model, written.metadata.model);
        assert_eq!(declared.metadata.temperature, written.metadata.temperature);
        assert_eq!(declared.metadata.tags, written.metadata.tags);
        assert_eq!(declared.metadata.scope, written.metadata.scope);

        for target in ["claude", "codex", "copilot", "gemini", "opencode"] {
            let linker = create_linker(target).unwrap();
            let a = linker.generate(&[LinkAgent::from(&declared)], None, &[]);
            let b = linker.generate(&[LinkAgent::from(&written)], None, &[]);

            assert!(
                !a.is_empty(),
                "target {target} produced no output file — the comparison \
                 below would be vacuous"
            );
            assert_eq!(a.len(), b.len(), "target {target}: file count differs");
            for (x, y) in a.iter().zip(b.iter()) {
                assert_eq!(x.path, y.path, "target {target}: paths differ");
                assert_eq!(
                    x.content, y.content,
                    "target {target}: projection diverged — the declaration is \
                     not an alternative spelling but a second source of truth"
                );
            }
        }
    }
}
