//! The `.armadai/agents.yaml` format: agents declared rather than written as
//! Markdown files.
//!
//! This module only *reads* the format. Turning a declaration into an `Agent`
//! (defaults merge, fragment composition) is the rest of this file's job, and
//! the dispatch between declared and file-backed agents lives in
//! `agent_source`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::agent::{AgentMetadata, default_temperature};

/// One entry of an agent's `prompt:` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptStep {
    /// `- specialist-base`
    Plain(String),
    /// `- { armadai-architecture: { module: core } }`
    Parameterised {
        fragment: String,
        vars: BTreeMap<String, String>,
    },
}

/// Values every agent inherits unless it says otherwise.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentDefaults {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub timeout: Option<u64>,
    pub model_fallback: Vec<String>,
    pub tags: Vec<String>,
    pub stacks: Vec<String>,
}

/// One declared agent.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDecl {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub model_fallback: Option<Vec<String>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub stacks: Option<Vec<String>>,
    #[serde(default)]
    pub scope: Option<Vec<String>>,
    #[serde(default, deserialize_with = "de_prompt")]
    pub prompt: Vec<PromptStep>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeclaredAgents {
    pub defaults: AgentDefaults,
    pub agents: Vec<AgentDecl>,
}

/// `Vec<serde_yaml_ng::Value>` rather than a `#[serde(untagged)]` enum: an
/// untagged enum's "did not match any variant" error is raised inside
/// `Vec::deserialize` itself, before this function ever runs, so it cannot be
/// improved from here. Discriminating by hand lets every rejection say what
/// shape was found and what shapes are valid.
fn de_prompt<'de, D>(d: D) -> Result<Vec<PromptStep>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw: Vec<serde_yaml_ng::Value> = Vec::deserialize(d)?;
    raw.into_iter()
        .map(|step| match step {
            serde_yaml_ng::Value::String(name) => Ok(PromptStep::Plain(name)),
            serde_yaml_ng::Value::Mapping(map) => {
                // One key means one fragment. Zero or several keys leave no
                // way to know which fragment the entry is about.
                let mut it = map.into_iter();
                match (it.next(), it.next()) {
                    (Some((key, value)), None) => {
                        let fragment = key.as_str().map(str::to_string).ok_or_else(|| {
                            D::Error::custom("a prompt step's fragment name must be a string")
                        })?;
                        let vars: BTreeMap<String, String> =
                            serde_yaml_ng::from_value(value).map_err(|e| {
                                D::Error::custom(format!(
                                    "a prompt step's variables for `{fragment}` must be a map of strings: {e}"
                                ))
                            })?;
                        Ok(PromptStep::Parameterised { fragment, vars })
                    }
                    _ => Err(D::Error::custom(
                        "a prompt step must name exactly one fragment",
                    )),
                }
            }
            other => Err(D::Error::custom(format!(
                "a prompt step must be a fragment name or a single-key map of a fragment name to its variables, got {}",
                describe_yaml_shape(&other)
            ))),
        })
        .collect()
}

/// A short, human-readable name for the kind of YAML value found, used to
/// make a rejection actionable (e.g. "got a sequence" rather than a generic
/// "invalid type" message).
fn describe_yaml_shape(v: &serde_yaml_ng::Value) -> &'static str {
    match v {
        serde_yaml_ng::Value::Null => "null",
        serde_yaml_ng::Value::Bool(_) => "a boolean",
        serde_yaml_ng::Value::Number(_) => "a number",
        serde_yaml_ng::Value::String(_) => "a string",
        serde_yaml_ng::Value::Sequence(_) => "a sequence",
        serde_yaml_ng::Value::Mapping(_) => "a mapping",
        serde_yaml_ng::Value::Tagged(_) => "a tagged value",
        #[allow(unreachable_patterns)]
        _ => "an unrecognised value",
    }
}

/// Read and validate an `agents.yaml`.
///
/// A missing file is an error, not an empty set: treating it as empty would
/// hide a mistyped path behind a fleet that silently has no agents.
pub fn load(path: &Path) -> anyhow::Result<DeclaredAgents> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    serde_yaml_ng::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("cannot parse {}: {e}", path.display()))
}

/// Build an agent's metadata from its declaration and the shared defaults.
///
/// Shallow merge: a field absent from the declaration takes the default's
/// value. Lists are **replaced, never merged** — less expressive, but an agent
/// that declares `tags` has exactly those tags, rather than silently
/// inheriting a wider set.
///
/// Fails when no `provider` is declared at either level. The `.md` parser
/// refuses that too (`parser/metadata.rs`), and the two formats must agree on
/// what a valid agent is. Every remaining field is written out explicitly:
/// `AgentMetadata` has no `Default`, and giving it one would mean `provider:
/// ""` and `temperature: 0.0` — two wrong values free to propagate.
pub fn merge_metadata(decl: &AgentDecl, defaults: &AgentDefaults) -> anyhow::Result<AgentMetadata> {
    let provider = decl
        .provider
        .clone()
        .or_else(|| defaults.provider.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "agent '{}': no provider declared, and none in defaults",
                decl.name
            )
        })?;

    Ok(AgentMetadata {
        provider,
        model: decl.model.clone().or_else(|| defaults.model.clone()),
        command: decl.command.clone().or_else(|| defaults.command.clone()),
        args: decl.args.clone().or_else(|| defaults.args.clone()),
        temperature: decl
            .temperature
            .or(defaults.temperature)
            .unwrap_or_else(default_temperature),
        max_tokens: decl.max_tokens.or(defaults.max_tokens),
        timeout: decl.timeout.or(defaults.timeout),
        tags: decl.tags.clone().unwrap_or_else(|| defaults.tags.clone()),
        stacks: decl
            .stacks
            .clone()
            .unwrap_or_else(|| defaults.stacks.clone()),
        scope: decl.scope.clone().unwrap_or_default(),
        model_fallback: decl
            .model_fallback
            .clone()
            .unwrap_or_else(|| defaults.model_fallback.clone()),
        cost_limit: None,
        rate_limit: None,
        context_window: None,
        mode: None,
        orchestration: None,
        triggers: None,
        ring_config: None,
    })
}

/// Assemble an agent's system prompt from its declared fragments.
///
/// Fails on a missing fragment or an unsubstituted variable. That is the
/// deliberate inverse of the policy gate, where uncertainty allows: degrading
/// here would ship an agent whose instructions are amputated, and such an
/// agent fills the gaps instead of complaining.
pub fn compose_prompt(
    steps: &[PromptStep],
    decl: &AgentDecl,
    fragments: &[crate::prompt::Prompt],
) -> anyhow::Result<String> {
    let mut parts = Vec::new();
    for step in steps {
        let (name, extra) = match step {
            PromptStep::Plain(n) => (n.as_str(), BTreeMap::new()),
            PromptStep::Parameterised { fragment, vars } => (fragment.as_str(), vars.clone()),
        };
        let frag = fragments.iter().find(|f| f.name == name).ok_or_else(|| {
            anyhow::anyhow!(
                "agent '{}' references prompt fragment '{name}', which was not found",
                decl.name
            )
        })?;

        // The agent's own fields, then the step's variables — a step may
        // override an implicit value deliberately. `description` is inserted
        // only when the agent declares one: silently substituting an empty
        // string here would bypass the very failure this function exists to
        // enforce, producing exactly the amputated prompt the hard-failure
        // policy is meant to prevent. A step can still supply `description`
        // itself via `extra`, even when the agent has none.
        let mut vars: BTreeMap<String, String> = BTreeMap::new();
        vars.insert("name".into(), decl.name.clone());
        if let Some(d) = &decl.description {
            vars.insert("description".into(), d.clone());
        }
        vars.extend(extra);

        let rendered = crate::template::render(&frag.body, &vars)
            .map_err(|e| anyhow::anyhow!("agent '{}', fragment '{name}': {e}", decl.name))?;
        parts.push(rendered.trim_end().to_string());
    }
    Ok(parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
defaults:
  provider: claude
  model: latest:pro
  temperature: 0.3
  max_tokens: 8192

agents:
  - name: core-specialist
    description: Core domain and orchestration engine
    scope: [src/core/**, src/parser/**]
    tags: [rust, domain]
    prompt:
      - specialist-base
      - { armadai-architecture: { module: core } }

  - name: ui-specialist
    temperature: 0.4
    prompt: [specialist-base]
"#;

    fn parsed() -> DeclaredAgents {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents.yaml");
        std::fs::write(&p, SAMPLE).unwrap();
        load(&p).unwrap()
    }

    #[test]
    fn reads_defaults_and_agents() {
        let d = parsed();
        assert_eq!(d.defaults.provider.as_deref(), Some("claude"));
        assert_eq!(d.defaults.temperature, Some(0.3));
        assert_eq!(d.agents.len(), 2);
        assert_eq!(d.agents[0].name, "core-specialist");
    }

    #[test]
    fn a_plain_prompt_step_is_a_bare_fragment_name() {
        let d = parsed();
        assert_eq!(
            d.agents[0].prompt[0],
            PromptStep::Plain("specialist-base".into())
        );
    }

    #[test]
    fn a_parameterised_step_carries_its_fragment_and_vars() {
        let d = parsed();
        match &d.agents[0].prompt[1] {
            PromptStep::Parameterised { fragment, vars } => {
                assert_eq!(fragment, "armadai-architecture");
                assert_eq!(vars.get("module").map(String::as_str), Some("core"));
            }
            other => panic!("expected a parameterised step, got {other:?}"),
        }
    }

    #[test]
    fn a_step_with_several_keys_is_rejected() {
        // `{ a: {...}, b: {...} }` is ambiguous: which fragment is it?
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents.yaml");
        std::fs::write(
            &p,
            "agents:\n  - name: x\n    prompt:\n      - { a: {}, b: {} }\n",
        )
        .unwrap();
        let err = load(&p).unwrap_err().to_string();
        assert!(
            err.contains("exactly one fragment"),
            "the error must say what is wrong: {err}"
        );
    }

    #[test]
    fn a_malformed_file_reports_where() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents.yaml");
        std::fs::write(&p, "agents:\n  - name: [unclosed\n").unwrap();
        let err = load(&p).unwrap_err().to_string();
        assert!(
            err.contains("agents.yaml"),
            "the error must name the file: {err}"
        );
    }

    #[test]
    fn a_missing_file_is_an_error_not_an_empty_set() {
        // Silently treating it as empty would hide a typo'd path.
        assert!(load(std::path::Path::new("/nonexistent/agents.yaml")).is_err());
    }

    #[test]
    fn an_unknown_key_under_an_agent_is_rejected_and_named() {
        // `deny_unknown_fields` is deliberate on every declared struct: a
        // mistyped key in a fleet declaration must fail loudly, not be
        // silently dropped on the floor.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents.yaml");
        std::fs::write(&p, "agents:\n  - name: x\n    typo_field: oops\n").unwrap();
        let err = load(&p).unwrap_err().to_string();
        assert!(
            err.contains("typo_field"),
            "the error must name the offending key: {err}"
        );
    }

    #[test]
    fn an_unknown_key_under_defaults_is_rejected_and_named() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents.yaml");
        std::fs::write(&p, "defaults:\n  typo_default: oops\n").unwrap();
        let err = load(&p).unwrap_err().to_string();
        assert!(
            err.contains("unknown field `typo_default`"),
            "the error must name the offending key: {err}"
        );
    }

    #[test]
    fn an_unknown_key_at_the_top_level_is_rejected_and_named() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents.yaml");
        std::fs::write(&p, "typo_top: 1\n").unwrap();
        let err = load(&p).unwrap_err().to_string();
        assert!(
            err.contains("unknown field `typo_top`, expected `defaults` or `agents`"),
            "the error must name the offending key: {err}"
        );
    }

    #[test]
    fn a_number_prompt_step_is_rejected_with_actionable_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents.yaml");
        std::fs::write(&p, "agents:\n  - name: x\n    prompt:\n      - 42\n").unwrap();
        let err = load(&p).unwrap_err().to_string();
        assert!(
            err.contains("a fragment name")
                && err.contains("a single-key map of a fragment name to its variables")
                && err.contains("a number"),
            "the error must name the valid shapes and what was found: {err}"
        );
    }

    #[test]
    fn a_sequence_prompt_step_is_rejected_with_actionable_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents.yaml");
        std::fs::write(&p, "agents:\n  - name: x\n    prompt:\n      - [a, b]\n").unwrap();
        let err = load(&p).unwrap_err().to_string();
        assert!(
            err.contains("a fragment name")
                && err.contains("a single-key map of a fragment name to its variables")
                && err.contains("a sequence"),
            "the error must name the valid shapes and what was found: {err}"
        );
    }

    fn decl(name: &str) -> AgentDecl {
        AgentDecl {
            name: name.into(),
            description: None,
            provider: None,
            model: None,
            command: None,
            args: None,
            temperature: None,
            max_tokens: None,
            timeout: None,
            model_fallback: None,
            tags: None,
            stacks: None,
            scope: None,
            prompt: vec![],
        }
    }

    fn defaults() -> AgentDefaults {
        AgentDefaults {
            provider: Some("claude".into()),
            model: Some("latest:pro".into()),
            command: None,
            args: None,
            temperature: Some(0.3),
            max_tokens: Some(8192),
            timeout: None,
            model_fallback: vec!["latest:fast".into()],
            tags: vec!["shared".into()],
            stacks: vec![],
        }
    }

    #[test]
    fn an_agent_without_overrides_takes_every_default() {
        let m = merge_metadata(&decl("a"), &defaults()).unwrap();
        assert_eq!(m.provider, "claude");
        assert_eq!(m.model.as_deref(), Some("latest:pro"));
        assert_eq!(m.temperature, 0.3);
        assert_eq!(m.max_tokens, Some(8192));
        assert_eq!(m.tags, vec!["shared".to_string()]);
    }

    #[test]
    fn a_scalar_override_wins() {
        let mut d = decl("a");
        d.temperature = Some(0.9);
        assert_eq!(merge_metadata(&d, &defaults()).unwrap().temperature, 0.9);
    }

    /// Lists are replaced, never merged: an agent that declares its tags has
    /// exactly those, and does not silently inherit a wider set.
    #[test]
    fn a_declared_list_replaces_the_default_rather_than_extending_it() {
        let mut d = decl("a");
        d.tags = Some(vec!["own".into()]);
        let m = merge_metadata(&d, &defaults()).unwrap();
        assert_eq!(m.tags, vec!["own".to_string()]);
        assert!(!m.tags.contains(&"shared".to_string()));
    }

    #[test]
    fn an_empty_declared_list_means_empty_not_inherit() {
        let mut d = decl("a");
        d.tags = Some(vec![]);
        assert!(merge_metadata(&d, &defaults()).unwrap().tags.is_empty());
    }

    /// The `.md` parser refuses an agent with no provider
    /// (`parser/metadata.rs:83`). YAML must refuse it too, or the two formats
    /// disagree on what a valid agent is.
    #[test]
    fn a_provider_declared_nowhere_is_an_error() {
        let err = merge_metadata(&decl("a"), &AgentDefaults::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("provider"), "must say what is missing: {err}");
        assert!(err.contains("a"), "must name the agent: {err}");
    }

    #[test]
    fn temperature_falls_back_to_the_parsers_own_default() {
        let mut d = decl("a");
        d.provider = Some("claude".into());
        let m = merge_metadata(&d, &AgentDefaults::default()).unwrap();
        // Call the shared function rather than writing 0.7 in a second place:
        // this test must follow the default, not pin a copy of it.
        assert_eq!(m.temperature, crate::agent::default_temperature());
    }

    /// An agent may supply the provider itself with no defaults block at all.
    #[test]
    fn an_agent_can_carry_the_provider_alone() {
        let mut d = decl("a");
        d.provider = Some("cli".into());
        assert_eq!(
            merge_metadata(&d, &AgentDefaults::default())
                .unwrap()
                .provider,
            "cli"
        );
    }

    /// `cli` is the provider armadai uses to drive claude/gemini/codex as
    /// subprocesses, and it needs `command` (and usually `args`) to know what
    /// to run. Without this, the declarative format could not express a whole
    /// class of agent the `.md` format can.
    #[test]
    fn a_declared_cli_provider_carries_its_command_and_args_through() {
        let mut d = decl("a");
        d.provider = Some("cli".into());
        d.command = Some("claude".into());
        d.args = Some(vec!["--dangerously-skip-permissions".into()]);
        let m = merge_metadata(&d, &AgentDefaults::default()).unwrap();
        assert_eq!(m.command.as_deref(), Some("claude"));
        assert_eq!(
            m.args,
            Some(vec!["--dangerously-skip-permissions".to_string()])
        );
    }

    /// Same list-replacement rule as `tags`: a declared `args: []` means the
    /// agent takes no arguments, not "inherit whatever the defaults set".
    #[test]
    fn declared_empty_args_means_empty_not_inherit() {
        let mut d = decl("a");
        d.provider = Some("cli".into());
        d.args = Some(vec![]);
        let defaults = AgentDefaults {
            args: Some(vec!["--from-defaults".into()]),
            ..AgentDefaults::default()
        };
        let m = merge_metadata(&d, &defaults).unwrap();
        assert_eq!(m.args, Some(vec![]));
    }

    /// Every field gets a distinct, recognisable value so a field-swap bug
    /// (e.g. assigning `timeout` from `max_tokens`, or `stacks` from `tags`)
    /// fails this test even where the values are "the same shape" — two
    /// numeric fields, or two string-list fields, that happen to share a
    /// value would not expose such a bug, so none of them do here.
    #[test]
    fn merge_metadata_maps_every_field_without_swapping_any() {
        let d = AgentDecl {
            name: "kraken".into(),
            description: Some("desc-from-decl".into()),
            provider: Some("provider-from-decl".into()),
            model: Some("model-from-decl".into()),
            command: Some("command-from-decl".into()),
            args: Some(vec!["arg-from-decl".into()]),
            temperature: Some(0.11),
            max_tokens: Some(1111),
            timeout: Some(2222),
            model_fallback: Some(vec!["fallback-from-decl".into()]),
            tags: Some(vec!["tag-from-decl".into()]),
            stacks: Some(vec!["stack-from-decl".into()]),
            scope: Some(vec!["scope-from-decl".into()]),
            prompt: vec![],
        };
        let defaults = AgentDefaults {
            provider: Some("provider-from-defaults".into()),
            model: Some("model-from-defaults".into()),
            command: Some("command-from-defaults".into()),
            args: Some(vec!["arg-from-defaults".into()]),
            temperature: Some(0.99),
            max_tokens: Some(9999),
            timeout: Some(8888),
            model_fallback: vec!["fallback-from-defaults".into()],
            tags: vec!["tag-from-defaults".into()],
            stacks: vec!["stack-from-defaults".into()],
        };

        let m = merge_metadata(&d, &defaults).unwrap();

        assert_eq!(m.provider, "provider-from-decl");
        assert_eq!(m.model.as_deref(), Some("model-from-decl"));
        assert_eq!(m.command.as_deref(), Some("command-from-decl"));
        assert_eq!(m.args, Some(vec!["arg-from-decl".to_string()]));
        assert_eq!(m.temperature, 0.11);
        assert_eq!(m.max_tokens, Some(1111));
        assert_eq!(m.timeout, Some(2222));
        assert_eq!(m.tags, vec!["tag-from-decl".to_string()]);
        assert_eq!(m.stacks, vec!["stack-from-decl".to_string()]);
        assert_eq!(m.scope, vec!["scope-from-decl".to_string()]);
        assert_eq!(m.model_fallback, vec!["fallback-from-decl".to_string()]);
        assert_eq!(m.cost_limit, None);
        assert_eq!(m.rate_limit, None);
        assert_eq!(m.context_window, None);
        assert_eq!(m.mode, None);
        assert_eq!(m.orchestration, None);
        assert!(m.triggers.is_none());
        assert!(m.ring_config.is_none());
    }

    use crate::prompt::Prompt;

    fn fragment(name: &str, body: &str) -> Prompt {
        Prompt {
            name: name.into(),
            description: None,
            apply_to: vec![],
            body: body.into(),
            source: std::path::PathBuf::from(format!("{name}.md")),
        }
    }

    #[test]
    fn fragments_are_concatenated_in_declared_order() {
        let frags = vec![fragment("a", "first"), fragment("b", "second")];
        let steps = vec![PromptStep::Plain("b".into()), PromptStep::Plain("a".into())];
        let out = compose_prompt(&steps, &decl("x"), &frags).unwrap();
        assert_eq!(out, "second\n\nfirst");
    }

    #[test]
    fn a_parameterised_fragment_gets_its_variables() {
        let frags = vec![fragment("arch", "module is {{module}}")];
        let steps = vec![PromptStep::Parameterised {
            fragment: "arch".into(),
            vars: [("module".to_string(), "core".to_string())]
                .into_iter()
                .collect(),
        }];
        let out = compose_prompt(&steps, &decl("x"), &frags).unwrap();
        assert_eq!(out, "module is core");
    }

    /// The agent's own fields are available to every fragment, with the same
    /// names `cli/new.rs` uses for templates.
    #[test]
    fn the_agents_name_and_description_are_implicit_variables() {
        let frags = vec![fragment("greet", "I am {{name}}: {{description}}")];
        let mut d = decl("core-specialist");
        d.description = Some("the core".into());
        let out = compose_prompt(&[PromptStep::Plain("greet".into())], &d, &frags).unwrap();
        assert_eq!(out, "I am core-specialist: the core");
    }

    /// A step's own variables win over the implicit agent fields — a step may
    /// deliberately override `name` or `description` for its fragment.
    #[test]
    fn a_steps_own_variable_overrides_the_implicit_agent_name() {
        let frags = vec![fragment("greet", "I am {{name}}")];
        let steps = vec![PromptStep::Parameterised {
            fragment: "greet".into(),
            vars: [("name".to_string(), "something-else".to_string())]
                .into_iter()
                .collect(),
        }];
        let out = compose_prompt(&steps, &decl("core-specialist"), &frags).unwrap();
        assert_eq!(out, "I am something-else");
    }

    #[test]
    fn a_missing_fragment_is_an_error_naming_it_and_the_agent() {
        let err = compose_prompt(
            &[PromptStep::Plain("nope".into())],
            &decl("core-specialist"),
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("nope"), "must name the fragment: {err}");
        assert!(
            err.contains("core-specialist"),
            "must name the agent, or you cannot find it in a 77-agent fleet: {err}"
        );
    }

    /// An agent whose prompt still contains `{{module}}` is quietly wrong.
    #[test]
    fn an_unsubstituted_variable_fails_the_composition() {
        let frags = vec![fragment("arch", "module is {{module}}")];
        let err = compose_prompt(&[PromptStep::Plain("arch".into())], &decl("x"), &frags)
            .unwrap_err()
            .to_string();
        assert!(err.contains("module"), "must name the variable: {err}");
    }

    #[test]
    fn an_empty_prompt_list_yields_an_empty_system_prompt() {
        assert_eq!(compose_prompt(&[], &decl("x"), &[]).unwrap(), "");
    }

    /// Fragment bodies read from disk routinely end in their own trailing
    /// newline. Each rendered fragment is trimmed before joining, so the
    /// `"\n\n"` separator is the only blank line between fragments — not
    /// three or more. Pinned as the exact literal, not just "contains".
    #[test]
    fn trailing_newlines_in_fragment_bodies_do_not_pile_up_at_the_join() {
        let frags = vec![
            fragment("a", "first line\n"),
            fragment("b", "second line\n"),
        ];
        let steps = vec![PromptStep::Plain("a".into()), PromptStep::Plain("b".into())];
        let out = compose_prompt(&steps, &decl("x"), &frags).unwrap();
        assert_eq!(out, "first line\n\nsecond line");
    }

    /// The hard-failure principle this module is built on exists precisely so
    /// an agent with amputated instructions cannot ship silently. Letting
    /// `description: None` become an empty string would bypass that
    /// principle for the one field it is most likely to matter for.
    #[test]
    fn a_missing_description_fails_the_composition_naming_both_agent_and_field() {
        let frags = vec![fragment("greet", "Role: {{description}}")];
        let err = compose_prompt(
            &[PromptStep::Plain("greet".into())],
            &decl("core-specialist"),
            &frags,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("core-specialist"),
            "must name the agent: {err}"
        );
        assert!(err.contains("description"), "must name the field: {err}");
    }

    /// The escape hatch that keeps the above strictness usable: a step can
    /// supply `description` itself even when the agent declares none.
    #[test]
    fn a_step_can_supply_description_when_the_agent_has_none() {
        let frags = vec![fragment("greet", "Role: {{description}}")];
        let steps = vec![PromptStep::Parameterised {
            fragment: "greet".into(),
            vars: [("description".to_string(), "from the step".to_string())]
                .into_iter()
                .collect(),
        }];
        let out = compose_prompt(&steps, &decl("core-specialist"), &frags).unwrap();
        assert_eq!(out, "Role: from the step");
    }
}
