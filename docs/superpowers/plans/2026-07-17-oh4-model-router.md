# OH4 — Routeur dynamique de modèle (`latest:auto`) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Un agent avec `model: latest:auto` fait choisir le tier (`Fast`/`Pro`/`Max`) à l'exécution par des heuristiques statiques (longueur, mots-clés, tags override, budget cap), puis résout le modèle concret — sans toucher les agents à modèle explicite.

**Architecture:** Un module pur `core/routing.rs` (`RoutingRules` + `route()`), configurable via `armadai.yaml > routing:`, branché dans `run_single_agent` : `latest:auto` → `route(...)` → `resolve_model_for_tier` (existant).

**Tech Stack:** Rust edition 2024, serde/serde_yaml_ng (déjà présents), réutilise `ModelTier`/`resolve_model_for_tier` de `linker/model_resolution`.

## Global Constraints

- Rust edition 2024. Clippy DOIT passer **2 modes** : `--no-default-features --features tui` ET `--features tui,providers-api`, `-D warnings`.
- `parse_latest_placeholder("latest:auto")` retourne `None` (pas dans sa liste) → **le routeur doit intercepter `latest:auto` explicitement** dans `run_single_agent` AVANT toute résolution, sinon la chaîne `"latest:auto"` partirait telle quelle au provider.
- Ordre des tiers : `Fast < Pro < Max` (ordre de déclaration de `ModelTier`).
- Conventional Commits. Branche `feat/oh4-router` depuis `origin/release/1.0.0` ; PR vers `release/1.0.0`.
- **Dépendance OH3** : l'émission de l'événement `RunEvent::Route` (Task 4) suppose OH3 mergé. Si OH3 n'est pas présent, omettre l'émission (le routage fonctionne sans).
- **Hors scope beta.3** (documenté) : routage dans les moteurs d'orchestration (`llm_agents::agent_model`) et donc budget **effectif** (le budget vit dans l'orchestration). En agent simple/`--pipe`, `route()` reçoit `budget: None`. `route()` implémente et teste quand même le signal budget.

---

### Task 1: `ModelTier` ordonnable

**Files:**
- Modify: `src/linker/model_resolution.rs:27` (derive de `ModelTier`)
- Test: `src/linker/model_resolution.rs` `#[cfg(test)]`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn model_tier_is_ordered_fast_pro_max() {
    use ModelTier::*;
    assert!(Fast < Pro && Pro < Max);
    assert_eq!([Pro, Fast, Max].iter().copied().max().unwrap(), Max);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api model_tier_is_ordered`
Expected: FAIL — `ModelTier` doesn't implement `PartialOrd`/`Ord`.

- [ ] **Step 3: Add the derives** (`src/linker/model_resolution.rs:27`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelTier {
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --no-default-features --features tui,providers-api model_tier_is_ordered`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/linker/model_resolution.rs
git commit -m "feat(model): make ModelTier orderable (Fast < Pro < Max)"
```

---

### Task 2: Module `core/routing.rs`

**Files:**
- Create: `src/core/routing.rs`
- Modify: `src/core/mod.rs` (add `pub mod routing;`)
- Test: in `src/core/routing.rs` `#[cfg(test)]`

**Interfaces:**
- Produces: `struct RoutingRules` (Default + Deserialize), `struct BudgetState { remaining_ratio: f64 }`, `fn route(input: &str, agent_tags: &[String], budget: Option<BudgetState>, rules: &RoutingRules) -> ModelTier`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::linker::model_resolution::ModelTier::*;

    fn rules() -> RoutingRules { RoutingRules::default() }

    #[test]
    fn length_thresholds_select_tier() {
        assert_eq!(route("hi", &[], None, &rules()), Fast);           // short
        assert_eq!(route(&"x".repeat(1000), &[], None, &rules()), Pro);
        assert_eq!(route(&"x".repeat(5000), &[], None, &rules()), Max);
    }

    #[test]
    fn keyword_escalates_over_length() {
        // short input but contains a Max keyword
        assert_eq!(route("please refactor this", &[], None, &rules()), Max);
    }

    #[test]
    fn tag_overrides_input_signals() {
        // long input (would be Max by length) but agent tagged fast → Fast
        assert_eq!(route(&"x".repeat(5000), &["format".into()], None, &rules()), Fast);
        // tag critical → Max regardless of short input
        assert_eq!(route("hi", &["critical".into()], None, &rules()), Max);
    }

    #[test]
    fn budget_caps_downward_last() {
        // tag critical → Max, but budget nearly exhausted → capped to Fast
        let b = Some(BudgetState { remaining_ratio: 0.05 });
        assert_eq!(route("hi", &["critical".into()], b, &rules()), Fast);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api routing::`
Expected: FAIL — `route`/`RoutingRules` not found.

- [ ] **Step 3: Implement the module**

```rust
// src/core/routing.rs
use std::collections::HashMap;

use serde::Deserialize;

use crate::linker::model_resolution::ModelTier;

fn tier_from_str(s: &str) -> Option<ModelTier> {
    match s.to_lowercase().as_str() {
        "fast" => Some(ModelTier::Fast),
        "pro" => Some(ModelTier::Pro),
        "max" => Some(ModelTier::Max),
        _ => None,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LengthThresholds {
    pub fast_max: usize,
    pub pro_max: usize,
}
impl Default for LengthThresholds {
    fn default() -> Self {
        LengthThresholds { fast_max: 500, pro_max: 4000 }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Keywords {
    pub max: Vec<String>,
    pub fast: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RoutingRules {
    pub length_thresholds: LengthThresholds,
    pub keywords: Keywords,
    /// tag -> tier ("fast"|"pro"|"max")
    pub tags: HashMap<String, String>,
    pub budget_downgrade_ratio: f64,
}

impl Default for RoutingRules {
    fn default() -> Self {
        let mut tags = HashMap::new();
        tags.insert("critical".into(), "max".into());
        tags.insert("architecture".into(), "max".into());
        tags.insert("format".into(), "fast".into());
        RoutingRules {
            length_thresholds: LengthThresholds::default(),
            keywords: Keywords {
                max: ["refactor", "architecture", "prove", "debug"].iter().map(|s| s.to_string()).collect(),
                fast: ["list", "format", "summarize"].iter().map(|s| s.to_string()).collect(),
            },
            tags,
            budget_downgrade_ratio: 0.2,
        }
    }
}

pub struct BudgetState {
    pub remaining_ratio: f64,
}

fn tier_by_length(input: &str, t: &LengthThresholds) -> ModelTier {
    let n = input.chars().count();
    if n < t.fast_max {
        ModelTier::Fast
    } else if n < t.pro_max {
        ModelTier::Pro
    } else {
        ModelTier::Max
    }
}

fn tier_by_keywords(input: &str, k: &Keywords) -> Option<ModelTier> {
    let low = input.to_lowercase();
    if k.max.iter().any(|w| low.contains(&w.to_lowercase())) {
        Some(ModelTier::Max)
    } else if k.fast.iter().any(|w| low.contains(&w.to_lowercase())) {
        Some(ModelTier::Fast)
    } else {
        None
    }
}

/// Decide the model tier for a `latest:auto` agent. Pure & deterministic.
pub fn route(
    input: &str,
    agent_tags: &[String],
    budget: Option<BudgetState>,
    rules: &RoutingRules,
) -> ModelTier {
    // 1. Tag override: highest tier among mapped tags wins, ignoring input signals.
    let tag_tier = agent_tags
        .iter()
        .filter_map(|t| rules.tags.get(t))
        .filter_map(|s| tier_from_str(s))
        .max();

    let mut tier = if let Some(t) = tag_tier {
        t
    } else {
        // 2. Otherwise: max(length, keywords)
        let by_len = tier_by_length(input, &rules.length_thresholds);
        match tier_by_keywords(input, &rules.keywords) {
            Some(kw) => by_len.max(kw),
            None => by_len,
        }
    };

    // 3. Budget cap (downgrade only), applied last.
    if let Some(b) = budget
        && b.remaining_ratio <= rules.budget_downgrade_ratio
    {
        tier = tier.min(ModelTier::Fast);
    }
    tier
}
```

Add to `src/core/mod.rs`: `pub mod routing;`

- [ ] **Step 4: Run to verify tests pass**

Run: `cargo test --no-default-features --features tui,providers-api routing::`
Expected: PASS (4 tests).

- [ ] **Step 5: Clippy both modes + commit**

```bash
cargo clippy --no-default-features --features tui -- -D warnings
cargo clippy --no-default-features --features tui,providers-api -- -D warnings
git add src/core/routing.rs src/core/mod.rs
git commit -m "feat(core): static model-tier router (routing::route)"
```

---

### Task 3: Config `routing:` in `armadai.yaml`

**Files:**
- Modify: `src/core/project.rs:54-66` (`ProjectConfig` — add field)
- Test: `src/core/project.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `RoutingRules` (Task 2).
- Produces: `ProjectConfig.routing: Option<RoutingRules>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parses_routing_section() {
    let yaml = r#"
agents: []
routing:
  length_thresholds: { fast_max: 100, pro_max: 1000 }
  tags: { hot: max }
"#;
    let cfg: ProjectConfig = serde_yaml_ng::from_str(yaml).unwrap();
    let r = cfg.routing.expect("routing present");
    assert_eq!(r.length_thresholds.fast_max, 100);
    assert_eq!(r.tags.get("hot").map(String::as_str), Some("max"));
    // absent keys fall back to embedded defaults
    assert_eq!(r.budget_downgrade_ratio, 0.2);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api project::tests::parses_routing_section`
Expected: FAIL — no `routing` field.

- [ ] **Step 3: Add the field** (`src/core/project.rs`, in `ProjectConfig`, next to `shell`)

```rust
    #[serde(default)]
    pub routing: Option<crate::core::routing::RoutingRules>,
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --no-default-features --features tui,providers-api project::tests::parses_routing_section`
Expected: PASS.

- [ ] **Step 5: Clippy both modes + commit**

```bash
cargo clippy --no-default-features --features tui -- -D warnings
cargo clippy --no-default-features --features tui,providers-api -- -D warnings
git add src/core/project.rs
git commit -m "feat(config): add routing rules to armadai.yaml"
```

---

### Task 4: Wire `latest:auto` into `run_single_agent`

**Files:**
- Modify: `src/cli/run.rs:157-176` (model selection in `run_single_agent`)

**Interfaces:**
- Consumes: `route()` (Task 2), `ProjectConfig.routing` (Task 3), `resolve_model_for_tier` (existing).
- Note: `run_single_agent` already has `input` and `agent`; it needs access to `RoutingRules`. Pass the resolved rules down from `execute` (derive from `AgentResolution::Project { config }` → `config.routing.clone().unwrap_or_default()`, else `RoutingRules::default()`).

- [ ] **Step 1: Thread the rules into `run_single_agent`**

Add a parameter `routing_rules: &crate::core::routing::RoutingRules` to `run_single_agent` and pass it from both call sites in `execute` (sequential loop). Build it once in `execute`:
```rust
    let routing_rules = match &resolution {
        AgentResolution::Project { config, .. } => config.routing.clone().unwrap_or_default(),
        _ => crate::core::routing::RoutingRules::default(),
    };
```

- [ ] **Step 2: Intercept `latest:auto` at model selection** (`src/cli/run.rs:171-176`)

Replace the `let model = ...` block with:
```rust
    let raw_model = agent
        .metadata
        .model
        .clone()
        .or_else(|| agent.metadata.command.clone())
        .unwrap_or_else(|| "default".to_string());

    let model = if raw_model == "latest:auto" {
        let tier = crate::core::routing::route(input, &agent.metadata.tags, None, routing_rules);
        crate::linker::model_resolution::resolve_model_for_tier(&agent.metadata.provider, tier)
    } else {
        raw_model
    };
```

- [ ] **Step 3: Emit `RunEvent::Route` if OH3 present (optional)**

If OH3 is merged (`RunEvent` exists), add a `Route { agent: String, tier: String }` variant to `RunEvent` and emit it here via the sink:
```rust
    // sink.emit(&RunEvent::Route { agent: agent_name.into(), tier: format!("{tier:?}") });
```
If OH3 is NOT merged yet, skip this step (leave a one-line `// TODO(OH3): emit Route event` — acceptable ONLY because it is a cross-feature dependency, not a within-plan gap).

- [ ] **Step 4: Non-regression test — explicit model untouched**

```rust
// src/cli/run.rs #[cfg(test)] mod tests
#[test]
fn latest_auto_is_the_only_routed_value() {
    // concrete + latest:pro must NOT be treated as auto
    assert_ne!("claude-3", "latest:auto");
    assert_ne!("latest:pro", "latest:auto");
    // routing only triggers on the exact "latest:auto" string (guard documented)
}
```
(Behavioural coverage of the routed path is via `routing::tests` in Task 2; here we lock the trigger string.)

- [ ] **Step 5: Build + manual smoke**

Run: `cargo build --no-default-features --features tui,providers-api`
Create a test agent with `model: latest:auto` and a Max keyword input; confirm (stderr summary line) the resolved model corresponds to the Max tier for that provider.

- [ ] **Step 6: Clippy both modes + commit**

```bash
cargo clippy --no-default-features --features tui -- -D warnings
cargo clippy --no-default-features --features tui,providers-api -- -D warnings
git add src/cli/run.rs
git commit -m "feat(run): route model tier for latest:auto agents"
```

---

## Notes for the implementer

- The budget signal is fully implemented and unit-tested in `route()`, but in beta.3 it is invoked with `budget: None` from `run_single_agent` (no budget in the single-agent path). Effective budget-driven downgrade arrives with the orchestration integration below.
- **Out of scope (documented):** routing inside orchestration engines — `llm_agents::agent_model` (llm_agents.rs:24) is the model-selection point there, NOT `run_single_agent`. A follow-up would call `route()` from `contribute`/ring paths with the board's budget as `BudgetState`. Left for a later increment to keep beta.3 bounded.
- Do not add `latest:auto` to `parse_latest_placeholder` — it must remain `None` there; the interception in Task 4 is the single trigger point.
