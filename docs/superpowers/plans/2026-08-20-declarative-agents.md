# Agents déclaratifs — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Declare agents in `.armadai/agents.yaml` — metadata inherited from defaults, system prompt composed from named fragments with substituted variables — producing an `Agent` in memory with no intermediate `.md` on disk.

**Architecture:** `AgentRef` gains a `Declared` variant; a new `load_agent()` returns an `Agent` where `resolve_agent()` returns a path. The variable substitution currently inline in `cli/new.rs` is extracted into `armadai-core` so templates and fragments behave identically. The linker is untouched — it consumes `Agent`, and the origin is indifferent to it.

**Tech Stack:** Rust edition 2024, `serde` / `serde_yaml_ng` (both already dependencies), `anyhow`.

**Spec:** `docs/superpowers/specs/2026-08-20-declarative-agents-design.md`

## Global Constraints

- No new dependency, no new feature flag, nothing feature-gated.
- Clippy must pass in the **4 CI modes**, each `--all-targets ... -- -D warnings`: `tui` / `tui,providers-api` / `tui,web,storage` / `tui,storage,e2e-fake`.
- Tests: `cargo test --no-default-features --features tui` and `--features tui,storage,e2e-fake`.
- `cargo fmt --all` before every commit.
- Code, comments and commit messages in **English**. Conventional Commits, **a single type** per message.
- Commit trailer: `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`
- **The 77 existing `.md` agents keep working.** YAML is an additional input, never a replacement.
- **No intermediate artefact.** Nothing in this plan writes a generated `.md` to disk.
- **Composition fails hard**: a missing fragment or an unsubstituted `{{var}}` is an error, not a degradation. This is the deliberate inverse of the policy gate's degrade-to-allow — degrading here ships an agent with amputated instructions that hallucinates rather than complains.
- rust-analyzer is unreliable in this repo (false "mismatched ABI", stale snapshots). **Always verify at the compiler.**

## File structure

| File | Responsibility |
|---|---|
| `crates/armadai-core/src/template.rs` — **new** | Variable substitution, extracted from `cli/new.rs`. One job: `{{var}}` → value, error on leftovers. |
| `crates/armadai-core/src/agent_decl.rs` — **new** | The `agents.yaml` format: deserialisation, defaults merge, fragment composition → `Agent`. |
| `crates/armadai-core/src/agent_source.rs` — **new** | `load_agent()`: dispatch between file-backed refs and declared ones. |
| `crates/armadai-core/src/project.rs` — modify | `AgentRef::Declared` variant. |
| `crates/armadai-core/src/model_updater.rs` — modify | Deprecated-model detection and rewrite for YAML-declared agents. |
| `crates/armadai/src/cli/new.rs` — modify | Consume `template.rs` instead of its inline `replace` chain. |

Three new files rather than one: substitution is reusable on its own, the YAML format is the bulk of the logic, and the dispatch is a thin seam. Keeping them apart means Task 1 is testable before the format exists.

---

### Task 1: Extract variable substitution into `armadai-core`

`cli/new.rs:55-78` substitutes `{{name}}`, `{{description}}`, `{{stack}}`, `{{model}}` with a chain of `replace` calls, then computes the leftover placeholders into a `remaining` vector **and does nothing with it**. Fragments need the same substitution, and the spec requires leftovers to be an error.

**Files:**
- Create: `crates/armadai-core/src/template.rs`
- Modify: `crates/armadai-core/src/lib.rs` (declare the module), `crates/armadai/src/cli/new.rs:55-78`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub fn render(template: &str, vars: &BTreeMap<String, String>) -> Result<String, TemplateError>`
  - `pub fn unused_vars(template: &str, vars: &BTreeMap<String, String>) -> Vec<String>`
  - `pub enum TemplateError { Unsubstituted(Vec<String>) }` (implements `std::error::Error`)

- [ ] **Step 1: Write the failing tests**

Create `crates/armadai-core/src/template.rs` with only its test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn substitutes_every_occurrence() {
        let out = render("{{a}} then {{b}} then {{a}}", &vars(&[("a", "1"), ("b", "2")])).unwrap();
        assert_eq!(out, "1 then 2 then 1");
    }

    #[test]
    fn a_template_without_variables_is_returned_as_is() {
        assert_eq!(render("plain text", &vars(&[])).unwrap(), "plain text");
    }

    /// A prompt still containing `{{module}}` is text the model will try to
    /// interpret. Refusing to build the agent is the whole point.
    #[test]
    fn an_unsubstituted_variable_is_an_error_naming_it() {
        let err = render("hello {{who}} and {{whom}}", &vars(&[])).unwrap_err();
        match err {
            TemplateError::Unsubstituted(names) => {
                assert_eq!(names, vec!["who".to_string(), "whom".to_string()])
            }
        }
    }

    #[test]
    fn a_provided_but_unused_variable_is_reported_not_fatal() {
        // Almost always a typo — worth saying, never worth failing.
        assert!(render("nothing here", &vars(&[("spare", "x")])).is_ok());
        assert_eq!(
            unused_vars("nothing here", &vars(&[("spare", "x")])),
            vec!["spare".to_string()]
        );
    }

    #[test]
    fn an_unterminated_placeholder_is_left_alone() {
        // `{{` with no closing `}}` is not a placeholder, just text.
        assert_eq!(render("a {{ b", &vars(&[])).unwrap(), "a {{ b");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armadai-core template`
Expected: FAIL — `render`, `unused_vars` and `TemplateError` do not exist.

- [ ] **Step 3: Write the implementation**

Above the test module in `crates/armadai-core/src/template.rs`:

```rust
//! `{{var}}` substitution, shared by agent templates and prompt fragments.
//!
//! Extracted from `cli/new.rs`, which substituted inline and computed the
//! leftover placeholders without acting on them. Leftovers are an error here:
//! a prompt containing a literal `{{module}}` is text the model will try to
//! interpret, and an agent built from it is quietly wrong.

use std::collections::BTreeMap;

/// A template could not be fully rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// Placeholder names with no value supplied, in order of appearance.
    Unsubstituted(Vec<String>),
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsubstituted(names) => write!(
                f,
                "no value supplied for {}",
                names
                    .iter()
                    .map(|n| format!("{{{{{n}}}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl std::error::Error for TemplateError {}

/// Every `{{name}}` placeholder in `template`, in order, without duplicates.
fn placeholders(template: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break; // unterminated: not a placeholder
        };
        let name = after[..end].trim().to_string();
        if !name.is_empty() && !found.contains(&name) {
            found.push(name);
        }
        rest = &after[end + 2..];
    }
    found
}

/// Substitute every `{{name}}` with its value.
///
/// Errors when a placeholder has no value: see the module docs.
pub fn render(
    template: &str,
    vars: &BTreeMap<String, String>,
) -> Result<String, TemplateError> {
    let missing: Vec<String> = placeholders(template)
        .into_iter()
        .filter(|n| !vars.contains_key(n))
        .collect();
    if !missing.is_empty() {
        return Err(TemplateError::Unsubstituted(missing));
    }
    let mut out = template.to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    Ok(out)
}

/// Values supplied that the template never uses — almost always a typo.
pub fn unused_vars(template: &str, vars: &BTreeMap<String, String>) -> Vec<String> {
    let used = placeholders(template);
    vars.keys()
        .filter(|k| !used.contains(k))
        .cloned()
        .collect()
}
```

Declare the module in `crates/armadai-core/src/lib.rs`, in alphabetical order among the existing `pub mod` lines.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armadai-core template`
Expected: PASS (5 tests).

- [ ] **Step 5: Make `cli/new.rs` consume it**

Replace the `replace` chain at `crates/armadai/src/cli/new.rs:55-78` with a `vars` map plus one `render` call. Keep `new.rs`'s current behaviour: a missing `--description` or `--stack` means those placeholders stay unsupplied, so a template using them would now **error** instead of silently keeping `{{description}}`. That is the intended improvement; confirm the existing `new.rs` tests still pass, and if one relied on the silent behaviour, report it rather than weakening it.

- [ ] **Step 6: Run the affected tests**

Run: `cargo test --no-default-features --features tui new`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/armadai-core/src/template.rs crates/armadai-core/src/lib.rs crates/armadai/src/cli/new.rs
git commit -m "refactor(core): extract {{var}} substitution into armadai-core

cli/new.rs substituted inline and computed leftover placeholders without
acting on them. Prompt fragments need the same substitution, and a leftover
is now an error: a prompt containing a literal {{module}} is text the model
will try to interpret."
```

---

### Task 2: Deserialise `agents.yaml`

**Files:**
- Create: `crates/armadai-core/src/agent_decl.rs`
- Modify: `crates/armadai-core/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub struct DeclaredAgents { pub defaults: AgentDefaults, pub agents: Vec<AgentDecl> }`
  - `pub struct AgentDefaults` — every field `Option`/`Vec`, mirroring `AgentMetadata`'s scalar subset
  - `pub struct AgentDecl { pub name: String, pub description: Option<String>, pub prompt: Vec<PromptStep>, … }`
  - `pub enum PromptStep { Plain(String), Parameterised { fragment: String, vars: BTreeMap<String, String> } }`
  - `pub fn load(path: &Path) -> anyhow::Result<DeclaredAgents>`

- [ ] **Step 1: Write the failing tests**

```rust
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
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armadai-core agent_decl`
Expected: FAIL — nothing exists yet.

- [ ] **Step 3: Write the implementation**

```rust
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

/// Raw shape accepted by serde before validation.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawPromptStep {
    Plain(String),
    Parameterised(BTreeMap<String, BTreeMap<String, String>>),
}

/// Values every agent inherits unless it says otherwise.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentDefaults {
    pub provider: Option<String>,
    pub model: Option<String>,
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

fn de_prompt<'de, D>(d: D) -> Result<Vec<PromptStep>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw: Vec<RawPromptStep> = Vec::deserialize(d)?;
    raw.into_iter()
        .map(|step| match step {
            RawPromptStep::Plain(name) => Ok(PromptStep::Plain(name)),
            RawPromptStep::Parameterised(map) => {
                // One key means one fragment. Two keys leave no way to know
                // which fragment the entry is about.
                let mut it = map.into_iter();
                match (it.next(), it.next()) {
                    (Some((fragment, vars)), None) => {
                        Ok(PromptStep::Parameterised { fragment, vars })
                    }
                    _ => Err(D::Error::custom(
                        "a prompt step must name exactly one fragment",
                    )),
                }
            }
        })
        .collect()
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
```

Declare `pub mod agent_decl;` in `lib.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armadai-core agent_decl`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai-core/src/agent_decl.rs crates/armadai-core/src/lib.rs
git commit -m "feat(core): read the .armadai/agents.yaml declaration format

deny_unknown_fields throughout: a mistyped key in a fleet declaration should
fail loudly, not be ignored. A prompt step naming several fragments is
rejected — there would be no way to know which one it means."
```

---

### Task 3: Merge defaults into `AgentMetadata`

**Files:**
- Modify: `crates/armadai-core/src/agent_decl.rs`

**Interfaces:**
- Consumes: Task 2 (`AgentDecl`, `AgentDefaults`).
- Produces: `pub fn merge_metadata(decl: &AgentDecl, defaults: &AgentDefaults) -> anyhow::Result<AgentMetadata>`

**Two facts read from the code, not assumed** — the whole task turns on them:
1. `AgentMetadata` has **no `Default`** (`agent.rs:40` derives only `Debug, Clone, Serialize, Deserialize`). Do not add one: `String::default()` for `provider` and `0.0` for `temperature` are both wrong values that would then propagate silently.
2. `provider` is **required** in the `.md` format — `parser/metadata.rs:83` does `provider.context("Missing 'provider' in Metadata")?`. YAML must be just as strict, hence the `Result`. Defaulting it to `"claude"` here would make the two formats disagree on what a valid agent is.

`temperature`'s real default is `0.7` (`agent.rs:85-87`, `fn default_temperature`, currently private). Make it `pub` and call it rather than writing `0.7` in a second place.

- [ ] **Step 1: Write the failing tests**

```rust
    fn decl(name: &str) -> AgentDecl {
        AgentDecl {
            name: name.into(),
            description: None,
            provider: None,
            model: None,
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armadai-core agent_decl::tests::an_agent_without_overrides`
Expected: FAIL — `merge_metadata` does not exist.

- [ ] **Step 3: Write the implementation**

First make the parser's own default reachable — in `crates/armadai-core/src/agent.rs:85`, change `fn default_temperature()` to `pub fn default_temperature()`. It stays the single definition of that value.

Then, in `agent_decl.rs`:

```rust
use crate::agent::{AgentMetadata, default_temperature};

/// Build an agent's metadata from its declaration and the shared defaults.
///
/// Shallow merge: a field absent from the declaration takes the default's
/// value. Lists are **replaced, never merged** — less expressive, but an agent
/// that declares a `scope` has exactly that scope, rather than silently
/// inheriting a wider one.
///
/// Fails when no `provider` is declared at either level. The `.md` parser
/// refuses that too (`parser/metadata.rs`), and the two formats must agree on
/// what a valid agent is. Every remaining field is written out explicitly:
/// `AgentMetadata` has no `Default`, and giving it one would mean `provider:
/// ""` and `temperature: 0.0` — two wrong values free to propagate.
pub fn merge_metadata(
    decl: &AgentDecl,
    defaults: &AgentDefaults,
) -> anyhow::Result<AgentMetadata> {
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
        command: None,
        args: None,
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
```

The fields left at `None` are the ones the format does not yet expose (`command`/`args` belong to the `cli` provider, `mode`/`orchestration`/`triggers`/`ring_config` to features out of this scope). The compiler will tell you if a field is missing — that is why the struct is written out in full rather than closed with `..Default::default()`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armadai-core agent_decl`
Expected: PASS (11 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai-core/src/agent_decl.rs crates/armadai-core/src/agent.rs
git commit -m "feat(core): merge declared agent metadata over shared defaults

Shallow merge, lists replaced rather than extended: an agent that declares a
scope has exactly that scope. Inheriting a wider one silently is how a
specialist ends up allowed everywhere.

Fails when no provider is declared at either level, as the .md parser already
does — the two formats must agree on what a valid agent is. default_temperature
becomes pub so the value stays defined in one place."
```

---

### Task 4: Compose the system prompt from fragments

**Files:**
- Modify: `crates/armadai-core/src/agent_decl.rs`

**Interfaces:**
- Consumes: Task 1 (`template::render`, `template::unused_vars`), Task 2 (`PromptStep`), and the existing `crate::prompt::{Prompt, load_all_prompts}`.
- Produces: `pub fn compose_prompt(steps: &[PromptStep], decl: &AgentDecl, fragments: &[Prompt]) -> anyhow::Result<String>`

- [ ] **Step 1: Write the failing tests**

```rust
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
        let steps = vec![
            PromptStep::Plain("b".into()),
            PromptStep::Plain("a".into()),
        ];
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
        let out = compose_prompt(
            &[PromptStep::Plain("greet".into())],
            &d,
            &frags,
        )
        .unwrap();
        assert_eq!(out, "I am core-specialist: the core");
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armadai-core agent_decl::tests::fragments_are_concatenated`
Expected: FAIL — `compose_prompt` does not exist.

- [ ] **Step 3: Write the implementation**

```rust
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
        let frag = fragments
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "agent '{}' references prompt fragment '{name}', which was not found",
                    decl.name
                )
            })?;

        // The agent's own fields, then the step's variables — a step may
        // override an implicit value deliberately.
        let mut vars: BTreeMap<String, String> = BTreeMap::new();
        vars.insert("name".into(), decl.name.clone());
        vars.insert(
            "description".into(),
            decl.description.clone().unwrap_or_default(),
        );
        vars.extend(extra);

        let rendered = crate::template::render(&frag.body, &vars).map_err(|e| {
            anyhow::anyhow!("agent '{}', fragment '{name}': {e}", decl.name)
        })?;
        parts.push(rendered);
    }
    Ok(parts.join("\n\n"))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armadai-core agent_decl`
Expected: PASS (17 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai-core/src/agent_decl.rs
git commit -m "feat(core): compose a declared agent's prompt from fragments

Fails on a missing fragment or an unsubstituted variable, naming both the
fragment and the agent — in a 77-agent fleet an error without the agent name
is a search. This hard failure is the inverse of the policy gate's
degrade-to-allow, and deliberately so: an agent with amputated instructions
hallucinates rather than complains."
```

---

### Task 5: `load_agent` — the dispatch, and `AgentRef::Declared`

**Files:**
- Create: `crates/armadai-core/src/agent_source.rs`
- Modify: `crates/armadai-core/src/project.rs:152-156` (the `AgentRef` enum), `crates/armadai-core/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 2-4 (`load`, `merge_metadata`, `compose_prompt`), and the existing `project::resolve_agent`, `parser::parse_agent_file`, `prompt::load_all_prompts`.
- Produces: `pub fn load_agent(r: &AgentRef, project_root: &Path, fragments: &[Prompt]) -> anyhow::Result<Agent>`

- [ ] **Step 1: Write the failing tests**

```rust
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
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armadai-core agent_source`
Expected: FAIL — neither `load_agent` nor `AgentRef::Declared` exists.

- [ ] **Step 3: Add the enum variant**

In `crates/armadai-core/src/project.rs`, extend `AgentRef` (it is `#[serde(untagged)]`, so the new variant must have a distinct key — `declared` — to stay unambiguous):

```rust
pub enum AgentRef {
    Named { name: String },
    Registry { registry: String },
    Path { path: PathBuf },
    /// An agent declared in `.armadai/agents.yaml` rather than written as a file.
    Declared { declared: String },
}
```

- [ ] **Step 4: Write the dispatch**

```rust
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
            anyhow::anyhow!(
                "agent '{declared}' is not declared in {}",
                path.display()
            )
        })?;

    Ok(Agent {
        name: decl.name.clone(),
        source: path.clone(),
        metadata: agent_decl::merge_metadata(decl, &decls.defaults)?,
        system_prompt: agent_decl::compose_prompt(&decl.prompt, decl, fragments)?,
        instructions: None,
        output_format: None,
        pipeline: None,
        context: None,
    })
}
```

Declare `pub mod agent_source;` in `lib.rs`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p armadai-core agent_source`
Expected: PASS (3 tests). The compiler will also flag every exhaustive `match` on `AgentRef` — fix each by handling `Declared` explicitly rather than with a catch-all, so a future variant is caught too.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/armadai-core/src/agent_source.rs crates/armadai-core/src/project.rs crates/armadai-core/src/lib.rs
git commit -m "feat(core): load an agent from a declaration or from a file

resolve_agent returns a path, which is right for callers that rewrite files
in place. A declared agent has no file, so load_agent returns the Agent
itself and dispatches on the ref."
```

---

### Task 6: Reject a name declared twice

**Files:**
- Modify: `crates/armadai-core/src/agent_source.rs`

**Interfaces:**
- Consumes: Task 5 (`load_agent`, `declarations_path`).
- Produces: `pub fn check_no_shadowing(project_root: &Path, library: &Path) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

```rust
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

        let err = check_no_shadowing(dir.path(), lib.path())
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

    #[test]
    fn distinct_names_are_fine() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("other.md"), "# Other").unwrap();
        assert!(check_no_shadowing(dir.path(), lib.path()).is_ok());
    }

    #[test]
    fn a_project_without_declarations_never_shadows() {
        let dir = tempfile::tempdir().unwrap();
        let lib = tempfile::tempdir().unwrap();
        std::fs::write(lib.path().join("a.md"), "# A").unwrap();
        assert!(check_no_shadowing(dir.path(), lib.path()).is_ok());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armadai-core check_no_shadowing`
Expected: FAIL — the function does not exist.

- [ ] **Step 3: Write the implementation**

```rust
/// Refuse a name that exists both as a declaration and as a library file.
///
/// The obvious alternative is a precedence rule — local wins, as everywhere
/// else. It is rejected on purpose: a silent precedence recreates the very
/// duplicated truth this format exists to remove, and you would edit a `.md`
/// with no effect and nothing to tell you. Failing forces a choice.
///
/// Rule `C01` of the audit already reports name collisions; loading must
/// refuse them.
pub fn check_no_shadowing(project_root: &Path, library: &Path) -> anyhow::Result<()> {
    let decls_path = declarations_path(project_root);
    if !decls_path.is_file() {
        return Ok(());
    }
    let decls = agent_decl::load(&decls_path)?;
    for decl in &decls.agents {
        let file = library.join(format!("{}.md", decl.name));
        if file.is_file() {
            anyhow::bail!(
                "agent '{}' is declared in {} and also written as {} — \
                 remove one; there is deliberately no precedence between them",
                decl.name,
                decls_path.display(),
                file.display()
            );
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armadai-core agent_source`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai-core/src/agent_source.rs
git commit -m "feat(core): refuse an agent name that is both declared and a file

No precedence, deliberately. A silent local-wins rule recreates the
duplicated truth this format removes: you would edit a .md with no effect and
nothing would tell you."
```

---

### Task 7: The invariant — a declared agent projects like its `.md` twin

This is the task that proves there are not two divergent paths. It is also the property easiest to verify badly, so it gets its own test file.

**Files:**
- Modify: `crates/armadai/src/linker/mod.rs` — add to the existing `#[cfg(test)] mod tests` (the one holding `test_slugify_*`)

**Where the test lives, and why it is not in `tests/`:** `crates/armadai` is **bin-only** — no `src/lib.rs`, no `[lib]` section, two `[[bin]]` targets. An integration test under `tests/` therefore cannot `use armadai::linker`; the existing `tests/*.rs` drive the compiled binary instead. This test needs the trait directly, so it belongs in the crate as a unit test.

**Interfaces:**
- Consumes: Tasks 1-5, plus the real linker API read from `linker/mod.rs`:
  - `pub fn create_linker(target: &str) -> anyhow::Result<Box<dyn Linker>>`
  - `Linker::generate(&self, agents: &[LinkAgent], coordinator: Option<&LinkAgent>, sources: &[String]) -> Vec<OutputFile>`
  - `pub struct OutputFile { pub path: PathBuf, pub content: String }`
  - `impl From<&Agent> for LinkAgent` — note it derives `description` from the **first non-empty line of `system_prompt`**, not from any declared description. Two agents with the same prompt therefore get the same description, which is part of what this test pins.

- [ ] **Step 1: Write the failing test**

```rust
    /// A declared agent and its hand-written `.md` twin must produce the same
    /// native projection. If they diverge, the declaration format is a second
    /// source of truth rather than an alternative spelling of the first —
    /// exactly what it exists to avoid.
    ///
    /// Run against **every** target, not just claude: a divergence that only
    /// shows in the codex projection is still a divergence.
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
             scope: [src/core/**]\n    prompt: [base]\n",
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
             - scope: [src/core/**]\n\n## System Prompt\n\nYou own the core domain.\n",
        )
        .unwrap();
        let written = armadai_core::parser::parse_agent_file(&md).unwrap();

        // Sanity check before comparing projections: if the two agents are not
        // actually equivalent, an equal projection would prove nothing.
        assert_eq!(declared.system_prompt, written.system_prompt);
        assert_eq!(declared.metadata.model, written.metadata.model);
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --no-default-features --features tui a_declared_agent_and_its_md_twin`
Expected: FAIL. `tempfile` must be a dev-dependency of `crates/armadai` — check, and add it if absent.

- [ ] **Step 3: Make it pass**

Do **not** weaken the assertions, and do not drop a target from the loop to make it green. If the projections differ, the difference **is** the finding: report which field diverges, on which target, and why.

`Agent::source` differs by construction (one is a `.yaml`, one a `.md`), and the test does not compare it because `LinkAgent` does not carry it. Verify that rather than assume it: if `From<&Agent> for LinkAgent` does read `source`, say so — the invariant then cannot hold as written and the spec needs revisiting.

Any divergence in `metadata` or `system_prompt` is a real defect in Tasks 3-4 — fix it there, not here.

- [ ] **Step 4: Run the full suite**

Run: `cargo test --no-default-features --features tui`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/linker/mod.rs crates/armadai/Cargo.toml
git commit -m "test: a declared agent must project like its hand-written twin

The invariant that keeps the declaration format an alternative spelling of
the .md rather than a second source of truth."
```

---

### Task 8: Deprecated models in `agents.yaml`

`model_updater` fixes deprecated models in place and is called automatically by `run`, `link` and `init`. Without this, `agents.yaml` becomes the one place in a project where a dead model goes unnoticed.

**Files:**
- Modify: `crates/armadai-core/src/model_updater.rs` (add beside `check_agent_file:24` and `update_agent_file:78`)

**Interfaces:**
- Consumes: Task 2 (`agent_decl::load`), and the existing `DeprecationFinding { agent_path, agent_name, field, current, replacement }` plus `resolve_alias`.
- Produces:
  - `pub fn check_declarations(path: &Path) -> Vec<DeprecationFinding>`
  - `pub fn update_declarations(path: &Path, findings: &[DeprecationFinding]) -> anyhow::Result<usize>`

- [ ] **Step 1: Write the failing tests**

```rust
    /// Take a real deprecated model from the alias registry, so the test does
    /// not encode a value that may stop being deprecated.
    fn a_deprecated_model() -> (String, String) {
        // `resolve_alias` returns Some(replacement) for a deprecated name.
        for candidate in ["claude-3-sonnet-20240229", "claude-3-opus-20240229"] {
            if let Some(r) = resolve_alias(candidate) {
                return (candidate.to_string(), r);
            }
        }
        panic!("no known deprecated model in the alias registry — update this helper");
    }

    fn write_yaml(dir: &Path, body: &str) -> std::path::PathBuf {
        let p = dir.join("agents.yaml");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn a_deprecated_model_in_defaults_is_found_and_fixed() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!("defaults:\n  model: {old}\nagents:\n  - name: a\n"),
        );
        let findings = check_declarations(&p);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(update_declarations(&p, &findings).unwrap(), 1);
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains(&new) && !after.contains(&old));
    }

    /// A `.md` agent declares `model` once; `agents.yaml` carries it in
    /// `defaults` and in every agent that deviates. `replacen(.., 1)` would
    /// fix only the first.
    #[test]
    fn the_same_deprecated_model_is_fixed_at_every_occurrence() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!(
                "defaults:\n  model: {old}\nagents:\n  \
                 - name: a\n    model: {old}\n  - name: b\n    model: {old}\n"
            ),
        );
        let findings = check_declarations(&p);
        assert_eq!(findings.len(), 3, "one per occurrence: {findings:?}");
        update_declarations(&p, &findings).unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(!after.contains(&old), "every occurrence must be fixed:\n{after}");
        assert_eq!(after.matches(&new).count(), 3);
    }

    /// A raw `: <model>` pattern would also match inside prose. Correcting a
    /// configuration is one thing; rewriting a comment is another.
    #[test]
    fn a_deprecated_model_named_in_a_comment_or_description_is_left_alone() {
        let (old, _new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!(
                "# we used to run {old} here\nagents:\n  - name: a\n    \
                 description: migrated away from {old}\n"
            ),
        );
        assert!(
            check_declarations(&p).is_empty(),
            "no `model:` key carries it, so there is nothing to fix"
        );
        let before = std::fs::read_to_string(&p).unwrap();
        update_declarations(&p, &[]).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
    }

    #[test]
    fn comments_and_key_order_survive_the_rewrite() {
        let (old, _new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!(
                "# fleet defaults\ndefaults:\n  model: {old}\n  \
                 temperature: 0.3   # deliberately warm\nagents:\n  - name: a\n"
            ),
        );
        let findings = check_declarations(&p);
        update_declarations(&p, &findings).unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("# fleet defaults"), "comment lost:\n{after}");
        assert!(
            after.contains("# deliberately warm"),
            "inline comment lost:\n{after}"
        );
        assert!(
            after.find("model:").unwrap() < after.find("temperature:").unwrap(),
            "key order changed — a serde round-trip would do this:\n{after}"
        );
    }

    #[test]
    fn a_deprecated_model_in_model_fallback_is_fixed() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!("agents:\n  - name: a\n    model_fallback: [{old}]\n"),
        );
        let findings = check_declarations(&p);
        assert_eq!(findings.len(), 1, "{findings:?}");
        update_declarations(&p, &findings).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains(&new));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armadai-core check_declarations`
Expected: FAIL — the functions do not exist.

- [ ] **Step 3: Write the implementation**

Detection walks the parsed declaration, so it can only ever see real `model` / `model_fallback` values — never prose:

```rust
/// Deprecated models declared in an `agents.yaml`.
///
/// One finding **per occurrence**: unlike a `.md` agent, which declares
/// `model` once, a declaration file carries it in `defaults` and in every
/// agent that deviates. `field` distinguishes them (`defaults.model`,
/// `<agent>.model`, `<agent>.model_fallback[i]`) so the rewrite can target
/// each one.
pub fn check_declarations(path: &Path) -> Vec<DeprecationFinding> {
    let Ok(decls) = crate::agent_decl::load(path) else {
        return Vec::new(); // unreadable: not this function's problem
    };
    let mut out = Vec::new();
    let mut push = |field: String, agent_name: String, current: &str| {
        if let Some(replacement) = resolve_alias(current) {
            out.push(DeprecationFinding {
                agent_path: path.to_path_buf(),
                agent_name,
                field,
                current: current.to_string(),
                replacement,
            });
        }
    };
    if let Some(m) = &decls.defaults.model {
        push("defaults.model".into(), "defaults".into(), m);
    }
    for (i, fb) in decls.defaults.model_fallback.iter().enumerate() {
        push(format!("defaults.model_fallback[{i}]"), "defaults".into(), fb);
    }
    for a in &decls.agents {
        if let Some(m) = &a.model {
            push(format!("{}.model", a.name), a.name.clone(), m);
        }
        for (i, fb) in a.model_fallback.iter().flatten().enumerate() {
            push(
                format!("{}.model_fallback[{i}]", a.name),
                a.name.clone(),
                fb,
            );
        }
    }
    out
}

/// Rewrite deprecated models in an `agents.yaml`.
///
/// Textual substitution, like `update_agent_file` — a `serde_yaml_ng`
/// round-trip would silently drop every comment and reorder keys.
///
/// Bounded to lines whose key is `model`/`model_fallback`, or which are list
/// items: a raw `: <model>` pattern would also match inside a comment or a
/// `description`, and correcting a configuration must not rewrite prose.
pub fn update_declarations(
    path: &Path,
    findings: &[DeprecationFinding],
) -> anyhow::Result<usize> {
    if findings.is_empty() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(path)?;
    let mut count = 0;
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        let code = line.split('#').next().unwrap_or(line);
        let trimmed = code.trim_start();
        let is_model_key = trimmed.starts_with("model:")
            || trimmed.starts_with("model_fallback:")
            || trimmed.starts_with("- ");
        let mut kept = line.to_string();
        if is_model_key {
            for f in findings {
                if kept.contains(&f.current) {
                    kept = kept.replace(&f.current, &f.replacement);
                    count += 1;
                }
            }
        }
        out.push_str(&kept);
        out.push('\n');
    }
    std::fs::write(path, out)?;
    Ok(count)
}
```

Note for the implementer: `model_fallback: [a, b]` on one line and `- a` on its own line are both handled by the `is_model_key` test above. A `- ` list item under some *other* key could be touched — if any of the tests shows that, narrow the check by tracking the enclosing key rather than loosening the test.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armadai-core model_updater`
Expected: PASS.

- [ ] **Step 5: Wire it into the automatic check**

`auto_check_and_prompt` (`model_updater.rs:145`) and `check_project` (`:60`) walk agent files. Extend them to also call `check_declarations` on `.armadai/agents.yaml` when it exists, and `update_declarations` on acceptance. Verify by hand that `armadai validate` on a project whose `agents.yaml` holds a deprecated model reports it.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/armadai-core/src/model_updater.rs
git commit -m "feat(core): fix deprecated models in agents.yaml too

Otherwise the declaration file is the one place in a project where a dead
model goes unnoticed, while run/link/init keep fixing every .md.

One finding per occurrence: a .md declares model once, a declaration carries
it in defaults and in every deviating agent, so replacen(.., 1) would fix
only the first. Substitution is bounded to model/model_fallback lines — a raw
pattern would also match inside a comment or a description, and correcting a
configuration must not rewrite prose. Textual rewrite, so comments and key
order survive; a serde round-trip would drop both."
```

---

### Task 9: Documentation

**Files:**
- Create: `docs/wiki/declarative-agents.md`
- Modify: `docs/wiki/SUMMARY.md`, `CLAUDE.md` (the `core/` bullet), `docs/wiki/agent-format.md`

- [ ] **Step 1: Write the page**

Cover, in this order: what a declaration looks like (the spec's example); that the `.md` format stays fully valid and this is an additional input; the defaults merge with **lists replaced, not merged**; fragment composition and `{{var}}` substitution; that a missing fragment or unsubstituted variable **fails** rather than degrades, and why; that a name both declared and written as a file is an error with no precedence; and that `A05` flags an over-composed prompt.

State plainly what it does **not** buy — no token saving, no reduced context saturation, no guaranteed improvement in agent quality — with the spec's numbers. A reader who adopts it expecting a smaller context will be disappointed, and better disappointed by the doc than by the tool.

- [ ] **Step 2: Cross-reference the existing format page**

`docs/wiki/agent-format.md` documents the `.md` format. Add a short pointer to the declaration format there, so a reader landing on one finds the other.

- [ ] **Step 3: Update `CLAUDE.md`**

Add to the `core/` module list, in the file's terse style:

```markdown
- `core/agent_decl.rs` + `core/agent_source.rs` — declarative agents: `.armadai/agents.yaml` declares agents (defaults merge + prompt composed from fragments with `{{var}}` substitution); `load_agent()` returns an `Agent` where `resolve_agent()` returns a path. `core/template.rs` holds the substitution, shared with `cli/new.rs`.
```

- [ ] **Step 4: Commit**

```bash
git add docs/ CLAUDE.md
git commit -m "docs: document declarative agents"
```

---

### Task 10: Gate and PR

- [ ] **Step 1: Run the full local gate**

```bash
cargo fmt --all -- --check
for f in "tui" "tui,providers-api" "tui,web,storage" "tui,storage,e2e-fake"; do
  cargo clippy --all-targets --no-default-features --features "$f" -- -D warnings || break
done
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage,e2e-fake
```
Expected: all green. **Do not open the PR otherwise.**

- [ ] **Step 2: Verify against the real library**

Declare two of this project's own specialists in a throwaway `agents.yaml`, referencing the real fragments in `~/.config/armadai/prompts/`, and confirm `armadai link --target claude --dry-run` produces projections equivalent to today's. Capture the output. Every serious defect on this project's recent branches was found by running the real binary against real data, not by unit tests.

- [ ] **Step 3: Push and open the PR**

Body should carry: what it does; that the 77 `.md` agents keep working; the measured section of the spec on what it does *not* buy; the hard-failure rationale; and the limits (no `.md` → YAML converter, list-replacement semantics).

- [ ] **Step 4: Independent review**

Green CI is not sufficient on this project. Request an independent review before merge, and ask it explicitly to hunt for tautological tests — four separate reviews have found one on recent branches.

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task |
|---|---|
| `.armadai/agents.yaml` format, two prompt-step forms | 2 |
| Defaults merge, lists replaced not merged | 3 |
| Fragment composition, `{{var}}` substitution | 1, 4 |
| Substitution extracted from `cli/new.rs`, not rewritten | 1 |
| `AgentRef::Declared` + `load_agent()` returning an `Agent` | 5 |
| `resolve_agent` untouched for file-backed refs | 5 |
| Hard failure on missing fragment / unsubstituted variable | 1, 4 |
| Unused variable = warning, not error | 1 |
| Name collision = error, no precedence | 6 |
| Malformed YAML reports position | 2 |
| Orphan `Declared` reference errors | 5 |
| Identical projection invariant | 7 |
| Deprecated models in YAML, per occurrence, bounded to model keys, comments survive | 8 |
| `provider` as strict in YAML as in `.md` | 3 — derived from `parser/metadata.rs`, not from the spec; noted here because it tightens the format |
| `A05` measures composed prompts (no new limit) | — inherited: composed agents become `Agent`, so `A05` applies without code |
| No intermediate `.md` | 5 (constructs `Agent` directly) |
| 77 `.md` agents keep working | 5, 7 |
| Docs, including what it does not buy | 9 |

Two spec points deliberately carry no task: **the lot-2 extension to prompt fragments** and **the absence of a `.md` → YAML converter**, both listed as out of scope.

**Type consistency:** `PromptStep` is defined in Task 2 and consumed in Task 4 with the same variants. `AgentDefaults`/`AgentDecl` field names are identical across Tasks 2, 3 and 8. `load_agent`'s third parameter is `&[Prompt]` in Tasks 5, 6 and 7. `DeprecationFinding` is reused as-is in Task 8 — no new type.

**Signatures verified at the source, not assumed** — four of them changed the plan:

| Read | Consequence |
|---|---|
| `AgentMetadata` has no `Default` (`agent.rs:40`) | Task 3 writes every field out; adding a `Default` would mean `provider: ""` and `temperature: 0.0`. |
| `provider` is required (`parser/metadata.rs:83`) | `merge_metadata` returns `Result`; YAML is as strict as `.md`. |
| `crates/armadai` is bin-only (no `lib.rs`, no `[lib]`) | Task 7's test moved from `tests/` into the crate — `tests/` cannot `use armadai::linker`. |
| `Linker::generate(&[LinkAgent], Option<&LinkAgent>, &[String]) -> Vec<OutputFile>`, via `create_linker(target)` | Task 7 uses the trait across all five targets, not a fictional `linker::claude::generate`. |

**One risk the plan does not remove:** Task 7 asserts equal projections across five targets. If a target's projection legitimately depends on something a declared agent cannot carry, that assertion fails for a good reason. The step says to report it rather than drop the target — a green test bought by narrowing the loop proves nothing.
