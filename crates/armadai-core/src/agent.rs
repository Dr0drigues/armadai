use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::orchestration::OrchestrationPattern;

// Re-export from orchestration module (canonical location)
pub use super::orchestration::{AgentRingConfig, TriggerConfig};

/// Agent interaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Guided,
    #[default]
    Autonomous,
}

/// An agent loaded from a Markdown definition file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Agent name (from H1 heading)
    pub name: String,
    /// Source file path
    pub source: PathBuf,
    /// Technical configuration
    pub metadata: AgentMetadata,
    /// System prompt sent to the model
    pub system_prompt: String,
    /// Execution instructions
    pub instructions: Option<String>,
    /// Expected output format
    pub output_format: Option<String>,
    /// Pipeline configuration
    pub pipeline: Option<PipelineConfig>,
    /// Additional context to inject
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    /// Provider name: anthropic, openai, google, cli, proxy
    pub provider: String,
    /// Model identifier (for API providers)
    pub model: Option<String>,
    /// CLI command (for cli provider)
    pub command: Option<String>,
    /// CLI arguments (for cli provider)
    pub args: Option<Vec<String>>,
    /// Sampling temperature
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Max output tokens
    pub max_tokens: Option<u32>,
    /// CLI provider timeout in seconds.
    ///
    /// Since #270, this bounds *inactivity* — the longest gap between two
    /// consecutive lines of subprocess output — not the call's total
    /// duration: a `CliProvider` call that keeps producing output survives
    /// past this many seconds; one that goes fully silent for this long is
    /// killed (see `armadai_providers::cli::CliProvider::complete`). Only
    /// applies to CLI-backed providers (`provider: cli`, or a unified name
    /// like `claude`/`gemini` resolving to its CLI); API providers ignore
    /// this field.
    pub timeout: Option<u64>,
    /// Tags for filtering
    #[serde(default)]
    pub tags: Vec<String>,
    /// Supported tech stacks
    #[serde(default)]
    pub stacks: Vec<String>,
    /// File/directory scope patterns (e.g. ["src/**/*.rs", "tests/"])
    #[serde(default)]
    pub scope: Vec<String>,
    /// Fallback models to try if the primary model is unavailable
    #[serde(default)]
    pub model_fallback: Vec<String>,
    /// Cost limit per execution (USD)
    pub cost_limit: Option<f64>,
    /// Rate limit (e.g. "10/min")
    pub rate_limit: Option<String>,
    /// Context window size override
    pub context_window: Option<u32>,
    /// Interaction mode (guided asks clarifying questions first)
    pub mode: Option<AgentMode>,
    /// Orchestration pattern this agent participates in
    pub orchestration: Option<OrchestrationPattern>,
    /// Blackboard trigger configuration (parsed from ## Triggers section)
    pub triggers: Option<TriggerConfig>,
    /// Ring configuration (parsed from ## Ring Config section)
    pub ring_config: Option<AgentRingConfig>,
}

/// The sampling temperature an agent gets when none is specified anywhere
/// (Markdown frontmatter default, and the declarative format's fallback
/// once neither the declaration nor its defaults set one).
pub fn default_temperature() -> f32 {
    0.7
}

/// Convert an agent name to a kebab-case slug suitable for filenames.
///
/// Two names that differ only by case, or by which punctuation/whitespace
/// they use as a separator, project to the same slug — which is exactly what
/// the linker uses to name a file on disk. `agent_source::shadowing_conflict`
/// compares slugs rather than raw names for the same reason: a declaration
/// and a library file that fold to the same slug will overwrite each other
/// at link time regardless of how differently they are spelled.
pub fn slugify(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else if c == ' ' || c == '_' {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|c| *c != '\0')
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Does the config reference `reference` designate the agent named
/// `agent_name`?
///
/// Configs name agents both ways: an agent's own name is its H1 title
/// (`Dev Lead`), while `link.coordinator` — like every path the linker
/// writes — is usually spelled as the slug (`dev-lead`). Reconciling the
/// two is exactly what [`slugify`] is for, so every command that resolves a
/// configured reference against a loaded roster must use this one criterion:
/// when `link` and `unlink` disagreed on it, `link` wrote a root context
/// file for a coordinator `unlink` did not recognise, leaving it on disk
/// with no message even naming it (issue #341).
///
/// The two arguments are NOT interchangeable: `reference` is matched
/// verbatim (case-insensitively) or against the slug of `agent_name`, never
/// the other way round — `name_matches_reference("Dev Lead", "dev-lead")`
/// holds while the swapped call does not.
pub fn name_matches_reference(agent_name: &str, reference: &str) -> bool {
    agent_name.eq_ignore_ascii_case(reference)
        || slugify(agent_name).eq_ignore_ascii_case(reference)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Agents to chain after this one
    pub next: Vec<String>,
}

/// The optional sections [`compose_agent_prompt`] appends, each with the
/// heading it is reintroduced under — the declaration order of an agent file.
///
/// Kept as one list so "which sections make up a prompt, in which order" is
/// stated once. Adding a fifth section to [`Agent`] means adding one line
/// here, and both `run` and `link` gain it in the same gesture.
const OPTIONAL_SECTION_HEADINGS: [&str; 3] = ["## Instructions", "## Output Format", "## Context"];

/// Compose an agent's declared sections into the single prompt body that IS
/// "the prompt of an agent".
///
/// The parser splits an agent file into `system_prompt` / `instructions` /
/// `output_format` / `context`, dropping the `##` headings on the way in.
/// Every consumer that hands an agent to a model has to put them back
/// together, and until #395 only the linkers did: `run` sent `system_prompt`
/// alone, so an agent whose output rules lived in `## Output Format` obeyed
/// them when linked into a native CLI and ignored them when run directly,
/// with nothing saying so.
///
/// This is that single definition. Measured before hoisting it here: the five
/// linkers (claude/codex/copilot/gemini/opencode) each carried their own copy
/// of this loop and produced byte-identical bodies for the same agent
/// (md5 `15461e18a2577665048e85fa3c5667b9` on a four-section fixture) — only
/// the surrounding wrapper (YAML frontmatter, TOML quoting) differed, which
/// is why the shareable part is exactly this function and not more.
///
/// A section present but empty still contributes its heading: that is what
/// the linkers did before, and `run`/`link` staying byte-identical matters
/// more than trimming a stray heading.
///
/// No trailing newline is appended — callers that need one normalise
/// afterwards, as the linkers already do for the file as a whole.
pub fn compose_agent_prompt(
    system_prompt: &str,
    instructions: Option<&str>,
    output_format: Option<&str>,
    context: Option<&str>,
) -> String {
    let mut out = String::from(system_prompt);
    for (heading, body) in
        OPTIONAL_SECTION_HEADINGS
            .iter()
            .zip([instructions, output_format, context])
    {
        let Some(body) = body else { continue };
        // Blank line before the heading, exactly as every linker did.
        if !out.ends_with("\n\n") {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str(heading);
        out.push_str("\n\n");
        out.push_str(body);
    }
    out
}

impl Agent {
    /// This agent's four sections composed into the prompt a model receives.
    ///
    /// The one entry point every provider-request construction site must use
    /// — reading `self.system_prompt` directly is what issue #395 was.
    pub fn composed_prompt(&self) -> String {
        compose_agent_prompt(
            &self.system_prompt,
            self.instructions.as_deref(),
            self.output_format.as_deref(),
            self.context.as_deref(),
        )
    }

    /// Replace every section with a single, already-composed prompt.
    ///
    /// For the callers that must append something *after* the whole prompt
    /// (`run`'s guided mode) while the composition itself happens further
    /// down, inside an engine: folding here and clearing the folded sections
    /// is what keeps the engine from composing them a second time, so
    /// [`Agent::composed_prompt`] stays idempotent across the hand-off.
    pub fn set_composed_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
        self.instructions = None;
        self.output_format = None;
        self.context = None;
    }

    /// Load all agents from the given directory (recursively).
    pub fn load_all(agents_dir: &std::path::Path) -> anyhow::Result<Vec<Agent>> {
        let mut agents = Vec::new();
        let mut skipped = Vec::new();
        Self::load_from_dir(agents_dir, &mut agents, &mut skipped)?;
        agents.sort_by_key(|a| a.name.to_lowercase());
        for msg in &skipped {
            tracing::debug!("{msg}");
        }
        Ok(agents)
    }

    /// Load all agents, returning both the agents and any skipped-file messages.
    pub fn load_all_with_skipped(
        agents_dir: &std::path::Path,
    ) -> anyhow::Result<(Vec<Agent>, Vec<String>)> {
        let mut agents = Vec::new();
        let mut skipped = Vec::new();
        Self::load_from_dir(agents_dir, &mut agents, &mut skipped)?;
        agents.sort_by_key(|a| a.name.to_lowercase());
        Ok((agents, skipped))
    }

    fn load_from_dir(
        dir: &std::path::Path,
        agents: &mut Vec<Agent>,
        skipped: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::load_from_dir(&path, agents, skipped)?;
            } else if path.extension().is_some_and(|ext| ext == "md") {
                match crate::parser::parse_agent_file(&path) {
                    Ok(agent) => agents.push(agent),
                    Err(e) => {
                        skipped.push(format!(
                            "Skipping {}: {e} (fix the file or remove it)",
                            path.display()
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Find an agent .md file by name (stem) in the agents directory tree.
    pub fn find_file(agents_dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
        let direct = agents_dir.join(format!("{name}.md"));
        if direct.exists() {
            return Some(direct);
        }
        Self::find_file_in_dir(agents_dir, name)
    }

    fn find_file_in_dir(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = Self::find_file_in_dir(&path, name) {
                    return Some(found);
                }
            } else if path.file_stem().is_some_and(|s| s == name)
                && path.extension().is_some_and(|e| e == "md")
            {
                return Some(path);
            }
        }
        None
    }

    /// Display string for the model/command column.
    pub fn model_display(&self) -> String {
        if let Some(ref model) = self.metadata.model {
            model.clone()
        } else if let Some(ref command) = self.metadata.command {
            format!("$ {command}")
        } else {
            "-".to_string()
        }
    }

    /// Filter agents by tags (all tags must match).
    pub fn matches_tags(&self, tags: &[String]) -> bool {
        tags.iter()
            .all(|t| self.metadata.tags.iter().any(|at| at == t))
    }

    /// Filter agents by stack.
    pub fn matches_stack(&self, stack: &str) -> bool {
        self.metadata
            .stacks
            .iter()
            .any(|s| s.eq_ignore_ascii_case(stack))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_matches_a_name_spelled_identically() {
        assert!(name_matches_reference("dev-lead", "dev-lead"));
        assert!(name_matches_reference("Dev Lead", "dev lead"));
    }

    /// The case issue #341 is about: `coordinator: dev-lead` beside an
    /// agent whose H1 title is `Dev Lead`.
    #[test]
    fn a_slug_reference_matches_a_title_cased_name() {
        assert!(name_matches_reference("Dev Lead", "dev-lead"));
        assert!(name_matches_reference("My_Test Agent", "my-test-agent"));
    }

    #[test]
    fn an_unrelated_reference_never_matches() {
        assert!(!name_matches_reference("Dev Lead", "qa-lead"));
        assert!(!name_matches_reference("Dev Lead", "dev"));
    }

    /// The arguments are directional — pinned so a swapped call site is a
    /// behaviour change, not a silent no-op.
    #[test]
    fn the_two_arguments_are_not_interchangeable() {
        assert!(name_matches_reference("Dev Lead", "dev-lead"));
        assert!(!name_matches_reference("dev-lead", "Dev Lead"));
    }

    /// Order and separators, pinned byte for byte.
    ///
    /// `run` and `link` now share this function, so a change here moves both
    /// at once and the wiring tests comparing them (`--test
    /// run_sends_every_section`) would stay green through it. This is what
    /// makes reordering the sections, or changing how they are separated, a
    /// deliberate act rather than a silent one.
    #[test]
    fn the_sections_compose_in_declaration_order_with_a_blank_line_between() {
        let composed = compose_agent_prompt("SYS", Some("INST"), Some("OUT"), Some("CTX"));
        assert_eq!(
            composed,
            "SYS\n\n## Instructions\n\nINST\n\n## Output Format\n\nOUT\n\n## Context\n\nCTX"
        );
    }

    /// An absent section contributes nothing — not an empty heading.
    #[test]
    fn absent_sections_are_skipped_entirely() {
        assert_eq!(compose_agent_prompt("SYS", None, None, None), "SYS");
        assert_eq!(
            compose_agent_prompt("SYS", None, Some("OUT"), None),
            "SYS\n\n## Output Format\n\nOUT"
        );
    }

    /// A system prompt already ending in a newline must not gain a third one:
    /// the parser trims sections inconsistently across formats, and `link`
    /// normalised this before the composition moved here.
    #[test]
    fn a_trailing_newline_does_not_double_the_separator() {
        assert_eq!(
            compose_agent_prompt("SYS\n", Some("INST"), None, None),
            "SYS\n\n## Instructions\n\nINST"
        );
        assert_eq!(
            compose_agent_prompt("SYS\n\n", Some("INST"), None, None),
            "SYS\n\n## Instructions\n\nINST"
        );
    }

    /// `set_composed_prompt` must make `composed_prompt` idempotent: it is
    /// what stops `run`'s guided mode from having its sections composed a
    /// second time inside the engine.
    #[test]
    fn set_composed_prompt_makes_composition_idempotent() {
        let mut agent = Agent {
            name: "a".to_string(),
            source: PathBuf::new(),
            metadata: AgentMetadata {
                provider: "cli".to_string(),
                model: None,
                command: None,
                args: None,
                temperature: default_temperature(),
                max_tokens: None,
                timeout: None,
                tags: Vec::new(),
                stacks: Vec::new(),
                scope: Vec::new(),
                model_fallback: Vec::new(),
                cost_limit: None,
                rate_limit: None,
                context_window: None,
                mode: None,
                orchestration: None,
                triggers: None,
                ring_config: None,
            },
            system_prompt: "SYS".to_string(),
            instructions: Some("INST".to_string()),
            output_format: None,
            pipeline: None,
            context: None,
        };
        let once = agent.composed_prompt();
        agent.set_composed_prompt(once.clone());
        assert_eq!(agent.composed_prompt(), once);
    }
}
