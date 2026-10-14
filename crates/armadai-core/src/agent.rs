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
    /// Legacy `timeout` metadata, in seconds: bounded the inactivity of a CLI
    /// provider call when ArmadAI still executed agents. Nothing enforces it
    /// any more; it is only parsed so `migrate` can read old agent files.
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
/// the linker uses to name a file on disk.
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

/// Compose an agent's declared sections into the single prompt body that IS
/// "the prompt of an agent".
///
/// The parser splits an agent file into `system_prompt` / `instructions` /
/// `output_format` / `context`, dropping the `##` headings on the way in.
/// Every consumer that writes an agent's prompt (`migrate`, and the
/// Codex/Copilot/OpenCode linkers) has to put them back together.
///
/// This is that single definition (#395). Measured before hoisting it here:
/// the linkers of the time each carried their own copy
/// of this loop and produced byte-identical bodies for the same agent
/// (md5 `15461e18a2577665048e85fa3c5667b9` on a four-section fixture) — only
/// the surrounding wrapper (YAML frontmatter, TOML quoting) differed, which
/// is why the shareable part is exactly this function and not more.
///
/// They agreed on every four-section agent, not on every input: an *empty*
/// `## System Prompt` made codex and copilot open the body with two blank
/// lines and the other three with none. A single definition has to pick one,
/// and picks "no leading blank line" — see
/// `an_empty_system_prompt_does_not_open_the_prompt_with_a_blank_line`. On
/// the 77-agent library this repository links against, every generated file
/// is byte-identical to what the per-linker copies produced.
///
/// A section present but empty still contributes its heading: that is what
/// the linkers did before, and staying byte-identical with them matters more
/// than trimming a stray heading.
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
    // Which sections make up a prompt, and in which order, is stated once —
    // here. A heading cannot be added without its body: the tuple demands
    // both, where a second array zipped against a list of headings let one
    // be added alone and silently dropped.
    for (heading, body) in [
        ("## Instructions", instructions),
        ("## Output Format", output_format),
        ("## Context", context),
    ] {
        let Some(body) = body else { continue };
        // One blank line before the heading — but never open the prompt with
        // one, which is where the linkers of the time disagreed (see above).
        if !out.is_empty() && !out.ends_with("\n\n") {
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
    /// `migrate` and the linkers share this function, so a change here moves
    /// them all at once and no test comparing them would notice. This is what
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

    /// An agent may declare `## System Prompt` and leave its body empty — the
    /// parser accepts it. The linkers of the time disagreed on that input:
    /// codex and copilot opened the composed body with two blank lines, the
    /// others with none. Folding them into one definition forces a
    /// choice, and this pins it: no leading blank line, because a prompt that
    /// opens on whitespace spends the model's first tokens on nothing.
    ///
    /// Nothing else pins it. No agent in the 77-agent library exercises an
    /// empty system prompt, so both forms passed the whole suite — which is
    /// precisely why the chosen one has to be stated here rather than left to
    /// whichever linker's copy happened to be hoisted.
    #[test]
    fn an_empty_system_prompt_does_not_open_the_prompt_with_a_blank_line() {
        assert_eq!(
            compose_agent_prompt("", Some("INST"), None, None),
            "## Instructions\n\nINST"
        );
        assert_eq!(
            compose_agent_prompt("", None, Some("OUT"), Some("CTX")),
            "## Output Format\n\nOUT\n\n## Context\n\nCTX"
        );
    }
}
