//! Reads ArmadAI-format agents (`# H1` + `## Metadata` + `## System Prompt`).
//!
//! **No format parsing happens here.** `armadai_core::parser::parse_agent_file`
//! is the product's own reader for this format — the same one `run`, `link`,
//! `list` and the TUI go through — and it stays the only one. This module is
//! the *adapter*: it maps one `Agent` onto the [`ImportedAgent`] the rules
//! already consume, and decides which of those fields this format can honestly
//! fill. A second reader would be a second answer to "what does this file
//! say", and the audit's whole value is saying what the product sees.
//!
//! Three mappings carry a decision, and each is measured on the 77-agent
//! library that motivated issue #391:
//!
//! - **name** is the *file stem*, not the H1. `project::resolve_agent` looks
//!   agents up as `<dir>/<name>.md`, so the stem is what `armadai run <name>`
//!   accepts and what a `@mention` in an instructions file resolves against.
//!   (Measured: on 6 of the 77, `slugify(H1)` and the stem differ —
//!   `gravitee-am-app-manager` is titled *Gravitee AM Application Manager* —
//!   so the two are not interchangeable.)
//! - **description** comes from [`LinkAgent`], not from a heuristic of our
//!   own. `AgentMetadata` has no `description` field; what ArmadAI *publishes*
//!   as one is the first non-empty line of the system prompt, because that is
//!   exactly what `impl From<&Agent> for LinkAgent` derives and what
//!   `armadai link` writes as `description:` into the generated
//!   `.claude/agents/<slug>.md`. Deriving it here again — "first sentence
//!   after the H1", say — would audit a description no router ever sees
//!   (measured: every one of the 77 files goes straight from its H1 to
//!   `## Metadata`, so that heuristic yields nothing at all).
//! - **tools** stays `None` *and* the format is recorded, because this format
//!   cannot express a tool restriction at all. See [`AgentFormat`].
//!
//! [`LinkAgent`]: crate::linker::LinkAgent
//! [`AgentFormat`]: super::AgentFormat

use std::path::{Path, PathBuf};

use armadai_core::agent::Agent;
use armadai_core::parser::parse_agent_file;

use super::{AgentFormat, ImportedAgent, ParseIssue, PartialMetadata};

/// Read every ArmadAI-format agent directly under `dir`, sorted by name.
///
/// Deliberately **flat**, where the Claude Code reader recurses: an ArmadAI
/// library resolves an agent as `<dir>/<name>.md`
/// (`armadai_core::project::resolve_agent`), so a file in a subdirectory is
/// not an agent this library can run, and reporting findings against it would
/// be reporting on something unreachable.
pub(crate) fn parse_agents(dir: &Path) -> Vec<ImportedAgent> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.sort();
    let mut agents: Vec<ImportedAgent> = files.iter().map(|p| parse_one(p)).collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

/// The stem is the agent's routable name — see the module doc.
fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_one(path: &Path) -> ImportedAgent {
    match parse_agent_file(path) {
        Ok(agent) => imported(path, &agent),
        // Every refusal of the product parser is a real defect in the file:
        // it means `armadai run` cannot load it either. `{e:#}` keeps the
        // anyhow context chain ("reading <path>", "Missing ## Metadata
        // section"), which is what makes the A01 message actionable.
        Err(e) => ImportedAgent {
            name: stem(path),
            source_path: path.to_path_buf(),
            metadata: PartialMetadata::default(),
            system_prompt: String::new(),
            issues: vec![ParseIssue {
                file: path.to_path_buf(),
                message: format!("{e:#}"),
            }],
            format: AgentFormat::Armadai,
        },
    }
}

fn imported(path: &Path, agent: &Agent) -> ImportedAgent {
    // The single existing implementation of "what description does ArmadAI
    // publish for this agent", reused rather than restated: `link` writes
    // this exact string into the native config a router then reads.
    let description = crate::linker::LinkAgent::from(agent).description;
    ImportedAgent {
        name: stem(path),
        source_path: path.to_path_buf(),
        metadata: PartialMetadata {
            description,
            model: agent.metadata.model.clone(),
            // Not "no restriction declared" — *no restriction declarable*.
            // `AgentFormat::Armadai` is what tells `A08` the difference.
            tools: None,
            // Left empty on purpose. `extra` is Claude Code frontmatter this
            // audit does not type, and `A12` reports its keys as
            // non-standard *frontmatter*. ArmadAI's `## Metadata` keys are a
            // different, product-owned vocabulary: routing them through
            // `extra` would make `A12` announce `provider (76), model (76),
            // tags (76)` on a healthy library. The same reasoning keeps
            // `metadata.scope` out of `extra["paths"]`, which `C02`/`C05`
            // read — measured on the 77-agent library, `scope` yields 149
            // overlapping pairs across agents belonging to unrelated
            // repositories (`src/main/java/**` for a Java service,
            // `src/tui/` for ArmadAI itself), i.e. one cluster naming 31
            // agents and no conflict at all.
            extra: Default::default(),
        },
        system_prompt: prompt_text(agent),
        issues: Vec::new(),
        format: AgentFormat::Armadai,
    }
}

/// Every prose section this agent carries, in the order a linked config lays
/// them out — because that is the text that reaches a model.
///
/// `A05` asks how big the prompt is, `A06` which line windows two agents
/// share and `A11` whether a secret is sitting in the prose; all three are
/// questions about the whole body, and an ArmadAI agent's body is four
/// optional sections rather than one. Counting only `## System Prompt` would
/// understate 60 of the 77 measured agents (which carry `## Instructions`)
/// and 44 (which carry `## Output Format`).
///
/// Not byte-identical to what any linker emits, and it does not need to be:
/// the five `linker::*::generate_agent_file` functions render a *file* (target
/// frontmatter, blank-line normalisation, per-target heading names), while
/// this renders *prose to measure*. Token counts and line windows are
/// insensitive to that difference.
///
/// It is however bounded by what the shared parser exposes, and that is
/// what `link` and `run` see — which is the point of reusing the product
/// parser instead of writing a second one.
///
/// Until #392 was fixed (by #394), that was less than the files hold:
/// `parse_agent_content` ended an H2 section at the next heading of *any*
/// level, so everything after a `###` was dropped before the audit saw it.
/// Measured after the fix, on the same 77 agents: the rules read 3.01x more
/// text (16205 -> 48778 estimated tokens), and `A06` goes from 0 to 2 real
/// duplication clusters — the number predicted here before the fix landed.
fn prompt_text(agent: &Agent) -> String {
    let mut out = agent.system_prompt.clone();
    for (heading, section) in [
        ("## Instructions", &agent.instructions),
        ("## Output Format", &agent.output_format),
        ("## Context", &agent.context),
    ] {
        if let Some(body) = section {
            out.push_str("\n\n");
            out.push_str(heading);
            out.push_str("\n\n");
            out.push_str(body);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One real-shaped ArmadAI agent: no frontmatter anywhere, four prose
    /// sections, and metadata as `- key: value` lines.
    const FULL: &str = "\
# Agent Builder

## Metadata
- provider: claude
- model: latest:pro
- temperature: 0.3
- tags: [authoring, agent]
- scope: [src/**/*.rs, docs/]

## System Prompt

You are an expert ArmadAI agent author.

Second paragraph of the prompt.

## Instructions

Ask for the purpose first.

## Output Format

A code block, ready to save.
";

    fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn an_armadai_agent_is_read_through_the_product_parser() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "agents/agent-builder.md", FULL);

        let agents = parse_agents(&dir.path().join("agents"));

        assert_eq!(agents.len(), 1, "{agents:?}");
        let a = &agents[0];
        assert!(
            a.issues.is_empty(),
            "a well-formed ArmadAI agent must not carry a parse issue \
             (the whole point of #391: read through this pass it used to yield \
             `missing YAML frontmatter`): {:?}",
            a.issues
        );
        assert_eq!(a.format, AgentFormat::Armadai);
        assert_eq!(a.metadata.model.as_deref(), Some("latest:pro"));
    }

    /// The name is the file stem, because that is the only spelling that
    /// resolves: `project::resolve_agent` looks for `<dir>/<name>.md`. The H1
    /// here is deliberately a title that slugifies to something *else*, the
    /// shape 6 of the 77 measured agents actually have.
    #[test]
    fn the_name_is_the_stem_the_library_resolves_and_not_the_h1() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "agents/gravitee-am-app-manager.md",
            "# Gravitee AM Application Manager\n\n## Metadata\n- provider: claude\n\n\
             ## System Prompt\n\nYou manage applications.",
        );

        let agents = parse_agents(&dir.path().join("agents"));

        assert_eq!(
            agents[0].name, "gravitee-am-app-manager",
            "the routable name is the stem; the H1 slugifies to \
             `gravitee-am-application-manager`, which resolves to nothing"
        );
    }

    /// `AgentMetadata` has no `description`. The one ArmadAI actually
    /// publishes is the first non-empty line of the system prompt — what
    /// `LinkAgent` derives and `armadai link` writes as `description:` into
    /// the generated native config. This asserts the audit reads that same
    /// string, not a fresh guess.
    #[test]
    fn the_description_is_the_one_link_publishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "agents/agent-builder.md", FULL);

        let agents = parse_agents(&dir.path().join("agents"));

        let published =
            crate::linker::LinkAgent::from(&armadai_core::parser::parse_agent_file(&path).unwrap())
                .description;
        assert_eq!(
            agents[0].metadata.description, published,
            "the audit must judge the description the linker publishes"
        );
        assert_eq!(
            agents[0].metadata.description.as_deref(),
            Some("You are an expert ArmadAI agent author."),
        );
    }

    /// `A05`, `A06` and `A11` all ask about the whole body. An ArmadAI agent's
    /// body is up to four sections, and 60 of the 77 measured agents carry
    /// `## Instructions` — counting only `## System Prompt` would understate
    /// them.
    #[test]
    fn the_prompt_text_carries_every_prose_section() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "agents/agent-builder.md", FULL);

        let agents = parse_agents(&dir.path().join("agents"));

        let prompt = &agents[0].system_prompt;
        for needle in [
            "You are an expert ArmadAI agent author.",
            "Ask for the purpose first.",
            "A code block, ready to save.",
        ] {
            assert!(prompt.contains(needle), "missing {needle:?} in:\n{prompt}");
        }
        assert!(
            !prompt.contains("provider: claude"),
            "metadata is configuration, not prompt text:\n{prompt}"
        );
    }

    /// Whatever the product parser refuses, `armadai run` refuses too — so it
    /// is a real defect, reported once, with the parser's own message chain.
    #[test]
    fn a_file_the_product_parser_rejects_becomes_one_parse_issue() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "agents/my-agent.md", "# My Agent\n");

        let agents = parse_agents(&dir.path().join("agents"));

        assert_eq!(agents.len(), 1);
        let a = &agents[0];
        assert_eq!(a.name, "my-agent", "the stem is still the name");
        assert_eq!(a.issues.len(), 1, "{:?}", a.issues);
        assert!(
            a.issues[0].message.contains("Metadata"),
            "the message must name what is missing, got: {}",
            a.issues[0].message
        );
        assert_eq!(a.format, AgentFormat::Armadai);
    }

    /// Flat, unlike the Claude Code reader: `resolve_agent` only ever looks at
    /// `<dir>/<name>.md`, so a nested file is not an agent this library can
    /// run and findings about it would be findings about nothing.
    #[test]
    fn the_scan_is_flat_because_that_is_how_the_library_resolves_agents() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "agents/top.md", FULL);
        write(dir.path(), "agents/team/nested.md", FULL);
        write(dir.path(), "agents/notes.txt", FULL);

        let agents = parse_agents(&dir.path().join("agents"));
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();

        assert_eq!(names, vec!["top"]);
    }

    /// Two rules read fields this format cannot fill, and both must stay
    /// silent rather than report the reader:
    ///
    /// - `A08` (permissive tools): the format has no tool list, hence
    ///   [`AgentFormat::declares_tools`];
    /// - `A12` (non-standard frontmatter fields) and `C02`/`C05` (path scopes
    ///   from `extra["paths"]`): `extra` is Claude Code frontmatter, and
    ///   routing `## Metadata` keys into it would make `A12` announce
    ///   `provider (76), model (76), tags (76)` on a healthy library, while
    ///   `scope` would make `C02` cluster 31 agents from unrelated
    ///   repositories.
    #[test]
    fn the_fields_this_format_cannot_express_stay_empty() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "agents/agent-builder.md", FULL);

        let a = parse_agents(&dir.path().join("agents")).remove(0);

        assert!(a.metadata.tools.is_none());
        assert!(!a.format.declares_tools());
        assert!(
            a.metadata.extra.is_empty(),
            "ArmadAI `## Metadata` keys are not Claude Code frontmatter: {:?}",
            a.metadata.extra
        );
        assert!(
            a.metadata.scope_globs().is_empty(),
            "the fixture declares `- scope:`, and it must not reach C02/C05"
        );
    }

    /// The one thing `A02` can still catch in this format: a `## System Prompt`
    /// section that exists but is empty parses fine, and then there is no
    /// description for a router to match on.
    #[test]
    fn an_empty_system_prompt_leaves_no_description_for_a02_to_find() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "agents/hollow.md",
            "# Hollow\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\n\
             ## Instructions\n\nDo things.",
        );

        let a = parse_agents(&dir.path().join("agents")).remove(0);

        assert!(a.issues.is_empty(), "{:?}", a.issues);
        assert!(
            a.metadata.description.is_none(),
            "got {:?}",
            a.metadata.description
        );
    }

    #[test]
    fn an_absent_directory_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(parse_agents(&dir.path().join("nope")).is_empty());
    }
}
