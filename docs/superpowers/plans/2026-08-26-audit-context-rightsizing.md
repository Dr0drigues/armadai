# Audit context rightsizing — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `R` rule family to `armadai audit` that measures whether agentic assets are *sized* correctly — the question none of the existing 26 rules asks.

**Architecture:** Three pure rule functions in a new `audit/rules/rightsizing.rs`, registered in the existing static registry. One field added to `ImportedSkill` so `R01` stays a pure function of the pre-loaded context, as every other rule is. No new plumbing in `report.rs`.

**Tech Stack:** Rust edition 2024, `regex` (already a dependency), `serde_yaml_ng` for the settings section.

**Spec:** `docs/superpowers/specs/2026-08-26-audit-context-rightsizing-design.md`

> **Superseded, post-review (measured):** every `3000` below is the threshold this plan was
> written with. It is **4000** as shipped. The plan's derivation was wrong by ~36 %: the real
> ratio is **1.843 token/word** (median of the same 460-skill corpus), so the p90 of 2224 words
> is ~4090 tokens, not 3000. Measured through the real binary on those 460 skills: 3000 flags
> **54 (11.7 %)**, 4000 flags **20 (4.3 %)** — and 4.3 % is what the spec promised. The spec
> carries the correction; `docs/wiki/audit.md` and the code carry the right number. This file is
> a dated execution log, kept as written apart from this note.

## Global Constraints

- Rules are **pure functions of `AuditContext`**. No rule reads the filesystem — the reverse pass does. The single `read_to_string` in the rules tree is `AuditSettings::from_project` loading config.
- `crates/armadai` is **binary-only**: `cargo test --lib` returns `0 passed` with **no error**. Use `cargo test --bin armadai` for unit tests. Always `--no-fail-fast`.
- Every test is verified by mutation: break what it protects, confirm red, restore, report the observed output. A test still green under mutation does not count.
- Gate before pushing: `cargo fmt --all`, clippy `-D warnings` in 5 feature modes, tests in 4 modes, gaveldrop must stay **13 cases · 83/83**.
- Conventional Commits, one type per subject. Trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Code, comments, commits in English.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/armadai/src/audit/reverse/mod.rs` | **Modify** — add `body_tokens: usize` to `ImportedSkill` |
| `crates/armadai/src/audit/reverse/claude.rs` | **Modify** — fill it in `parse_skill_dir`, which already reads the file (`:126`) |
| `crates/armadai/src/audit/rules/mod.rs` | **Modify** — `skill_token_threshold` on `AuditSettings`, 3 registry entries, `estimate_tokens` visibility, a `skill()` test helper |
| `crates/armadai/src/audit/rules/rightsizing.rs` | **Create** — `r01`, `r02`, `r04` and their unit tests |
| `crates/armadai/tests/audit_rightsizing.rs` | **Create** — black-box cases on the real binary |
| `docs/wiki/audit.md` | **Modify** — document the three rules and the new setting |

**Corrected during Task 1, measured:** 12 `ImportedSkill { .. }` literals, **10 in tests** (`rules/assets.rs` ×6, `rules/collisions.rs` ×2, `rules/usage_rules.rs` ×2) and **2 in production** — both inside `parse_skill_dir`, its early return for an unreadable file *and* its main return. `claude.rs`'s own tests build no `ImportedSkill` by hand; they go through `parse_skill_dir` on tempdirs. The 10 test sites get `body_tokens: 0`; the early return gets `0` too (absent file, no size to guess).

---

### Task 1: `body_tokens` on `ImportedSkill`

**Files:**
- Modify: `crates/armadai/src/audit/reverse/mod.rs:56-66`
- Modify: `crates/armadai/src/audit/reverse/claude.rs:120-160`
- Modify: `crates/armadai/src/audit/rules/mod.rs` (helper + `estimate_tokens` visibility)

**Interfaces:**
- Produces: `ImportedSkill.body_tokens: usize` — estimated tokens of the whole `SKILL.md`, including frontmatter. `0` when the file is unreadable or absent.
- Produces: `rules::test_support::skill(name, body_tokens) -> ImportedSkill`.
- Consumes: `rules::estimate_tokens`, which must become reachable from `reverse::claude` (it is `pub(crate)` in `rules/mod.rs`; `reverse` is a sibling module of `rules` under `audit`, so `pub(crate)` already suffices — verify at the compiler rather than assuming).

- [ ] **Step 1: Write the failing test**

In `crates/armadai/src/audit/reverse/claude.rs`, in its existing `mod tests`:

```rust
#[test]
fn a_parsed_skill_carries_its_body_size() {
    let dir = tempfile::tempdir().unwrap();
    let skills = dir.path().join(".claude/skills/big");
    std::fs::create_dir_all(&skills).unwrap();
    // 400 chars of body -> 100 estimated tokens (chars/4).
    let body = "x".repeat(400);
    std::fs::write(
        skills.join("SKILL.md"),
        format!("---\nname: big\ndescription: d\n---\n{body}"),
    )
    .unwrap();

    let parsed = parse_skill_dir(&skills);

    // Exact, not `>= 100`: the body alone is exactly 400 chars = 100 tokens, so
    // a `>= 100` assertion stays GREEN under the mutation that counts the body
    // instead of the file — measured during Task 1. 33 chars of frontmatter +
    // 400 = 433 -> 108 tokens at chars/4.
    assert_eq!(
        parsed.body_tokens, 108,
        "the whole file must be counted, frontmatter included"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test --bin armadai --no-fail-fast a_parsed_skill_carries_its_body_size`
Expected: compile error — no field `body_tokens` on `ImportedSkill`.

- [ ] **Step 3: Add the field and fill it**

In `reverse/mod.rs`, inside `pub struct ImportedSkill`, after `has_frontmatter`:

```rust
    /// Estimated tokens of the whole `SKILL.md`, frontmatter included. `0`
    /// when the file is absent or unreadable.
    ///
    /// A count, not the body: `R01` only asks how big the file is, and
    /// loading a whole SKILL.md into the audit context to answer that would be
    /// the very defect the R family exists to measure.
    pub body_tokens: usize,
```

In `reverse/claude.rs::parse_skill_dir`, the function already holds `content` from its
`read_to_string` at `:126`. Add to the returned literal:

```rust
        body_tokens: crate::audit::rules::estimate_tokens(&content),
```

- [ ] **Step 4: Fix the 13 test construction sites**

Add `body_tokens: 0,` to each `ImportedSkill { .. }` literal in `rules/assets.rs`,
`rules/collisions.rs`, `rules/usage_rules.rs` and `reverse/claude.rs`'s own tests. The
compiler lists them all; none of them tests size.

- [ ] **Step 5: Add the test helper**

In `rules/mod.rs`, inside `pub(crate) mod test_support`:

```rust
    pub fn skill(name: &str, body_tokens: usize) -> ImportedSkill {
        ImportedSkill {
            name: name.to_string(),
            source_path: PathBuf::from(format!(".claude/skills/{name}/SKILL.md")),
            description: Some(format!("{name} description")),
            has_skill_md: true,
            has_frontmatter: true,
            body_tokens,
            issues: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
```

- [ ] **Step 6: Run the tests AND clippy**

Run: `cargo test --bin armadai --no-fail-fast`, then clippy in all 5 modes.

`cargo test` alone is not enough here and the first draft of this plan got it wrong: a
foundation task creates a producer with no consumer, so `-D warnings` fails on two
`dead_code` errors — the field (non-test build; `pub` does not exempt it, this crate is
binary-only) and the helper (test build).

Gate them, and pick the attribute deliberately: `#[expect(dead_code)]` on the **helper**, so
the gate *breaks* the moment Task 2 uses it and the scaffolding cannot rot; `#[allow(dead_code)]`
on the **field**, because `expect` there is `unfulfilled` — the field *is* read by `reverse`'s
own tests. Measured, not reasoned. Mark both with a `Scaffold:` comment so Task 2 can
`grep -n "Scaffold:"` them.

- [ ] **Step 7: Mutation check**

Replace the fill with `body_tokens: 0` in `parse_skill_dir`. Run
`cargo test --bin armadai --no-fail-fast a_parsed_skill_carries_its_body_size`.
Expected: FAIL with `the whole file must be counted, got 0 tokens`. Restore, re-run green.
Record the observed output.

- [ ] **Step 8: Commit**

```bash
git add crates/armadai/src/audit/
git commit -m "feat(audit): carry each skill's body size on ImportedSkill"
```

---

### Task 2: `R01` — oversized skill with no progressive disclosure

**Files:**
- Create: `crates/armadai/src/audit/rules/rightsizing.rs`
- Modify: `crates/armadai/src/audit/rules/mod.rs` (module declaration, `skill_token_threshold`, registry entry)

**Interfaces:**
- Consumes: `ImportedSkill.body_tokens` (Task 1), `rules::test_support::skill` (Task 1).
- Produces: `pub(super) fn r01_oversized_skill(ctx: &AuditContext) -> Vec<Finding>`.
- Produces: `AuditSettings.skill_token_threshold: usize`, default `3000`, overridable via the `audit:` section.

- [ ] **Step 1: Write the failing tests**

Create `crates/armadai/src/audit/rules/rightsizing.rs`:

```rust
use std::path::Path;

use super::{AuditContext, Finding, Severity};

/// R01 — a `SKILL.md` past the token threshold whose skill directory has no
/// `references/` at all.
///
/// Size alone is a bad signal, measured: across 460 real skills, 46% of those
/// above the p90 have a `references/` directory against 67% below it — large
/// skills are *more* often split. So both conditions are required, which is
/// also what keeps a correctly-structured 41795-word skill out of the report.
///
/// Counterpart of `A05` for skills (`A05` covers agents' `system_prompt`).
/// `A09` validates a skill's structure but never its size.
pub(super) fn r01_oversized_skill(ctx: &AuditContext) -> Vec<Finding> {
    ctx.config
        .skills
        .iter()
        // Anti-cascade, same as A05: a skill that failed to parse is A01/A09's
        // job — one root cause, one finding.
        .filter(|s| s.issues.is_empty() && s.has_skill_md)
        .filter(|s| s.body_tokens > ctx.settings.skill_token_threshold)
        .filter(|s| !has_references(&s.source_path))
        .map(|s| Finding {
            rule: "R01",
            severity: Severity::Warning,
            file: s.source_path.clone(),
            related: Vec::new(),
            message: format!(
                "skill '{}' is ~{} tokens (threshold {}) and has no references/, so all of it \
                 loads on every invocation",
                s.name, s.body_tokens, ctx.settings.skill_token_threshold
            ),
            suggestion: Some(
                "split the detail into references/ — the Agent Skills standard loads those on \
                 demand, and armadai already installs them (core/skill.rs)"
                    .to_string(),
            ),
        })
        .collect()
}

/// Whether the skill directory holding this `SKILL.md` has a non-empty
/// `references/`. An empty directory is not progressive disclosure.
fn has_references(skill_md: &Path) -> bool {
    skill_md
        .parent()
        .map(|dir| dir.join("references"))
        .and_then(|refs| std::fs::read_dir(refs).ok())
        .is_some_and(|mut entries| entries.next().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::reverse::ImportedConfig;
    use crate::audit::rules::{AuditSettings, test_support::skill};

    fn ctx_for<'a>(
        config: &'a ImportedConfig,
        settings: &'a AuditSettings,
    ) -> AuditContext<'a> {
        AuditContext { config, settings, usage: None }
    }

    #[test]
    fn r01_flags_a_big_skill_with_no_references() {
        let config = ImportedConfig {
            skills: vec![skill("heavy", 5000)],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = r01_oversized_skill(&ctx_for(&config, &settings));
        assert_eq!(f.len(), 1, "expected exactly one finding, got {f:?}");
        assert_eq!(f[0].rule, "R01");
        assert!(
            f[0].message.contains("5000"),
            "the message must carry the measured size: {}",
            f[0].message
        );
    }

    #[test]
    fn r01_leaves_a_small_skill_alone() {
        let config = ImportedConfig {
            skills: vec![skill("light", 700)],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        assert!(r01_oversized_skill(&ctx_for(&config, &settings)).is_empty());
    }

    #[test]
    fn r01_leaves_a_big_but_split_skill_alone() {
        // The `quality-playbook` case: 41795 words with 16 references. Big and
        // correctly structured must not be flagged.
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("split");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(skill_dir.join("references/detail.md"), "detail").unwrap();
        let mut s = skill("split", 40_000);
        s.source_path = skill_dir.join("SKILL.md");

        let config = ImportedConfig { skills: vec![s], ..Default::default() };
        let settings = AuditSettings::default();
        assert!(
            r01_oversized_skill(&ctx_for(&config, &settings)).is_empty(),
            "a split skill must never be flagged, however big"
        );
    }

    #[test]
    fn r01_treats_an_empty_references_dir_as_no_disclosure() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("hollow");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        let mut s = skill("hollow", 5000);
        s.source_path = skill_dir.join("SKILL.md");

        let config = ImportedConfig { skills: vec![s], ..Default::default() };
        let settings = AuditSettings::default();
        assert_eq!(
            r01_oversized_skill(&ctx_for(&config, &settings)).len(),
            1,
            "an empty references/ is not progressive disclosure"
        );
    }

    #[test]
    fn r01_does_not_stack_on_an_unparsable_skill() {
        let mut s = skill("broken", 9000);
        s.issues = vec![crate::audit::reverse::ParseIssue {
            file: s.source_path.clone(),
            message: "invalid yaml".into(),
        }];
        let config = ImportedConfig { skills: vec![s], ..Default::default() };
        let settings = AuditSettings::default();
        assert!(
            r01_oversized_skill(&ctx_for(&config, &settings)).is_empty(),
            "A01/A09 own parse failures — one root cause, one finding"
        );
    }
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --bin armadai --no-fail-fast rightsizing`
Expected: compile error — `rightsizing` module not declared, `skill_token_threshold` missing.

- [ ] **Step 3: Wire the module and the setting**

In `rules/mod.rs`: add `mod rightsizing;` next to the other module declarations. Add to
`AuditSettings`:

```rust
    /// R01: estimated token count above which a skill with no `references/`
    /// is flagged. Default derived from a measured distribution: 460 real
    /// SKILL.md files give a p90 of 2224 words, ~3000 tokens at chars/4.
    pub skill_token_threshold: usize,
```

Add `skill_token_threshold: 3000,` to `Default::default()`, `skill_token_threshold:
Option<usize>` to the private `AuditSection`, and the corresponding override in
`from_project`, mirroring `prompt_token_threshold` exactly.

Add to `registry()`. Position is cosmetic — **the plan's original claim that it groups the report is false**, measured: `run_rules` (`rules/mod.rs:184`) sorts by `(severity, file, rule)`, so the registry order cannot reach the report. Put it after the `A` block because R01 reads well next to A05, not because it changes output:

```rust
        rightsizing::r01_oversized_skill,
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --bin armadai --no-fail-fast rightsizing`
Expected: PASS (5 tests).

- [ ] **Step 5: Mutation checks — three, because three conditions carry this rule**

1. Remove the `!has_references(..)` filter → `r01_leaves_a_big_but_split_skill_alone` must FAIL.
2. Change `entries.next().is_some()` to `true` in `has_references` →
   `r01_treats_an_empty_references_dir_as_no_disclosure` must FAIL.
3. Remove the `s.issues.is_empty()` filter → `r01_does_not_stack_on_an_unparsable_skill` must FAIL.

Restore after each, re-run green, and record each observed output. A condition with no
mutation that kills it is a condition that can silently stop working.

- [ ] **Step 6: Commit**

```bash
git add crates/armadai/src/audit/
git commit -m "feat(audit): add R01, flagging an oversized skill with no references/"
```

---

---

## Lessons from Tasks 1-4 — apply to every remaining task

Measured by executing this plan, not by reading it. Each of these was a real gap in the task
as written, so assume the same gaps in yours.

**A rule needs a registry test.** Removing the `registry()` entry leaves the rule's own unit
tests green while the rule never runs. That is exactly the dead-call defect found in #374.
Every rule gets a test asserting `run_rules` emits it — R01's is
`r01_is_wired_into_the_registry`.

**Settings plumbing needs its own tests.** R01's positive test asserted only that the message
carried the size, so raising the default *or* deleting the `from_project` override left
everything green. Assert the default value **and** an override, in the existing
`from_project_*` tests.

**One mutation per condition, and count the conditions first.** The plan listed three
conditions for R01 and gave two mutations, both aimed at the same one — the threshold had
none, and a fourth condition (`has_skill_md`) had neither test nor mutation. Enumerate the
conditions in the rule body, then check each has a mutation that kills exactly one test.

**Never let a test's answer depend on the ambient filesystem.** `test_support::skill` returns a
*relative* `source_path`, and anything resolving it hits the process cwd — which is global to
the test binary and moved mid-suite by `IsolatedProjectDir`. Three of R01's five planned tests
needed a `false` from `has_references` and got it by luck. The grave case was that a test's
*mutation sensitivity* depended on it: had the ambient answer been `true`, removing a filter
would have left it green. Use real tempdirs; `rightsizing.rs` now has a `skill_on_disk` helper.

**The plan's own assertions can be unfalsifiable.** Task 1 found `assert!(body_tokens >= 100)`
staying green under the mutation it was written to catch, because the body alone is exactly 100
tokens. Prefer exact equality with the arithmetic in a comment.

**A negative fixture must contain a token that only the filter under test rejects.** Measured in
Task 3: `r02_ignores_paths_inside_a_code_fence` first cited a *bare* path inside the fence, and a
bare path is no candidate to begin with — removing the fence tracking left the test green. Same
shape as the Task 1 defect, one level up: the fixture, not the assertion. For every filter, ask
what makes the fixture a candidate *before* that filter runs.

**A filter can be redundant with another, and then its mutation kills nothing.** Task 3's
"a path must have a directory part" was fully shadowed by the host-shape rejection for
`armadai.yaml` (a dot in the first component). Only a dotfile with a real extension
(`.mcp.json`) reaches it. Two filters that never disagree are one filter plus dead code — find
the input that separates them, or drop one.

**Measure a heuristic rule against the real corpus before writing its tests.** Task 3's
implementation as sketched in the plan produced **23 findings on this repo's own `CLAUDE.md`, 22
of them false**, and 24 (9 false) on the pre-#382 one. The shipped filters were derived from that
run. Unit tests on invented fixtures would have shipped all 22: every one of them passed the
plan's filters *by design*.

**A "sanity read" step can carry a false expectation.** Task 3's Step 6 asserted R02 must report
nothing on this repo. It reports exactly one thing, and the finding is **true**: `CLAUDE.md:80`
still says the Provider trait is in `providers/traits.rs`, while it is in
`crates/armadai-core/src/provider.rs` and no `traits.rs` exists anywhere in the tree (`find`,
0 hits). #382 rewrote the module map and carried that line over. The rule's first real catch is a
one-day-old stale path — left in place deliberately, so the finding is visible rather than
quietly patched away. Fixing it is a one-line `docs:` change and belongs to whoever owns
`CLAUDE.md`.

**A skill's body does not load on every invocation, and the report must not say it does.** The
Agent Skills standard is three-level: metadata always, the `SKILL.md` body when the skill
triggers, bundled files on demand. R04's message was reworded accordingly (instructions "on every
invocation", skills "each loaded whole when it triggers"). **R01's message still reads "so all of
it loads on every invocation"** — same inaccuracy, shipped in Task 2, not corrected here because
it is outside Tasks 3-4's scope. Task 5 must not restate it in `docs/wiki/audit.md`; the accurate
claim is that a skill body is loaded *whole* the moment the skill triggers, so its size is a cost
the author commits to at that point.

---

### Task 3: `R02` — a path named in the instructions does not exist

**Files:**
- Modify: `crates/armadai/src/audit/rules/rightsizing.rs`
- Modify: `crates/armadai/src/audit/rules/mod.rs` (registry entry)

**Interfaces:**
- Consumes: `ImportedConfig.instructions: Option<ImportedInstructions>` (which carries `content: String` and `source_path`).
- Produces: `pub(super) fn r02_stale_path(ctx: &AuditContext) -> Vec<Finding>`.
- Consumes: the project root, **without needing a new context field**. Checked: `ReverseLinker::parse(&self, root)` builds every path from `root.join(..)` (`reverse/claude.rs:60-62`), so `instructions.source_path` is `root.join("CLAUDE.md")` and `source_path.parent()` *is* the root. Resolve cited paths against it — never against the cwd, which is the trap already tracked at `config.rs:303`.

- [ ] **Step 1: Write the failing tests**

Append to `rightsizing.rs`:

```rust
/// R02 — a repo path cited in the root instructions file that resolves to
/// nothing.
///
/// Counterpart of `A10`, which does this for `@agent` mentions. This is the
/// rule that would have caught our own stale map: `CLAUDE.md` placed five
/// modules at a path where `ls` showed none of them, and described two
/// providers as `todo!()` stubs one day after they were implemented. A stale
/// map is worse than no map — it is read as authoritative.
///
/// False positives are the whole difficulty, so the filters are deliberately
/// narrow. Each one has its own negative test; a filter with no test is a
/// filter that can silently stop working.
pub(super) fn r02_stale_path(ctx: &AuditContext) -> Vec<Finding> {
    let Some(instructions) = &ctx.config.instructions else {
        return Vec::new();
    };
    let Some(base) = instructions.source_path.parent() else {
        return Vec::new();
    };

    cited_paths(&instructions.content)
        .into_iter()
        .filter(|p| !base.join(p).exists())
        .map(|p| Finding {
            rule: "R02",
            severity: Severity::Warning,
            file: instructions.source_path.clone(),
            related: Vec::new(),
            message: format!("instructions cite `{p}`, which does not exist"),
            suggestion: Some(
                "fix or drop the reference — a stale map is read as authoritative".to_string(),
            ),
        })
        .collect()
}

/// Backticked strings from `text` that look like real repo paths, excluding
/// fenced code blocks and obvious placeholders.
fn cited_paths(text: &str) -> Vec<String> {
    const SOURCE_EXT: [&str; 8] = [".rs", ".toml", ".md", ".yaml", ".yml", ".json", ".sh", ".ts"];
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for candidate in backticked(line) {
            let looks_like_path =
                candidate.contains('/') || SOURCE_EXT.iter().any(|e| candidate.ends_with(e));
            let is_placeholder = candidate.starts_with("path/to")
                || candidate.contains('<')
                || candidate.contains('*')
                || candidate.contains(' ');
            if looks_like_path && !is_placeholder {
                out.push(candidate.trim_end_matches('/').to_string());
            }
        }
    }
    out
}

fn backticked(line: &str) -> Vec<String> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
```

And its tests, inside the same `mod tests`:

```rust
    use crate::audit::reverse::ImportedInstructions;

    fn instructions_saying(dir: &std::path::Path, body: &str) -> ImportedConfig {
        ImportedConfig {
            instructions: Some(ImportedInstructions {
                source_path: dir.join("CLAUDE.md"),
                content: body.to_string(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn r02_flags_a_path_that_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(dir.path(), "See `src/gone/mod.rs` for details.");
        let settings = AuditSettings::default();
        let f = r02_stale_path(&ctx_for(&config, &settings));
        assert_eq!(f.len(), 1, "got {f:?}");
        assert_eq!(f[0].rule, "R02");
        assert!(f[0].message.contains("src/gone/mod.rs"));
    }

    #[test]
    fn r02_leaves_an_existing_path_alone() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/here")).unwrap();
        std::fs::write(dir.path().join("src/here/mod.rs"), "").unwrap();
        let config = instructions_saying(dir.path(), "See `src/here/mod.rs`.");
        let settings = AuditSettings::default();
        assert!(r02_stale_path(&ctx_for(&config, &settings)).is_empty());
    }

    #[test]
    fn r02_ignores_paths_inside_a_code_fence() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "Run it:\n```bash\ncat `src/example/never.rs`\n```\n",
        );
        let settings = AuditSettings::default();
        assert!(
            r02_stale_path(&ctx_for(&config, &settings)).is_empty(),
            "a fenced block is an example, not a claim"
        );
    }

    #[test]
    fn r02_ignores_placeholders_and_globs() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "Use `path/to/thing.rs`, `<your>/file.rs`, `crates/*/src/lib.rs`.",
        );
        let settings = AuditSettings::default();
        assert!(
            r02_stale_path(&ctx_for(&config, &settings)).is_empty(),
            "placeholders and globs describe a shape, not a file"
        );
    }

    #[test]
    fn r02_ignores_backticked_prose_that_is_not_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(dir.path(), "The `Provider` trait and `complete()`.");
        let settings = AuditSettings::default();
        assert!(r02_stale_path(&ctx_for(&config, &settings)).is_empty());
    }

    #[test]
    fn r02_is_silent_without_an_instructions_file() {
        let settings = AuditSettings::default();
        let config = ImportedConfig::default();
        assert!(r02_stale_path(&ctx_for(&config, &settings)).is_empty());
    }
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --bin armadai --no-fail-fast r02`
Expected: FAIL — function not yet registered / not yet written, depending on order.

- [ ] **Step 3: Register the rule**

Add `rightsizing::r02_stale_path,` to `registry()`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --bin armadai --no-fail-fast r02`
Expected: PASS (6 tests).

- [ ] **Step 5: Mutation checks — one per filter**

1. Remove the fence tracking → `r02_ignores_paths_inside_a_code_fence` must FAIL.
2. Remove the `is_placeholder` guard → `r02_ignores_placeholders_and_globs` must FAIL.
3. Remove the `looks_like_path` guard → `r02_ignores_backticked_prose_that_is_not_a_path` must FAIL.
4. Invert the existence check to `base.join(p).exists()` → `r02_leaves_an_existing_path_alone` must FAIL.
5. **Remove the `registry()` entry** → a `r02_is_wired_into_the_registry` test must FAIL. Write
   that test; without it, forgetting the entry ships a rule that never runs with every unit
   test green.

Restore after each and record the outputs.

- [ ] **Step 6: Run it against this repo's own CLAUDE.md**

Not an assertion, a sanity read: `cargo run --bin armadai -- audit`.

**Measured outcome, correcting this step's original expectation** ("R02 reports nothing, #382
fixed the stale paths"): R02 reports **one** finding, and it is a **true positive** —
`CLAUDE.md:80` places the Provider trait at `providers/traits.rs`, which exists nowhere
(`find` over the tree, `.git`/`target` excluded: 0 hits); the trait is at
`crates/armadai-core/src/provider.rs:47`. #382 rewrote the module map and carried that one line
over. Verdict: the rule is right, the file is stale. Left unfixed on purpose so the finding stays
visible; the fix is one line and is not part of Tasks 3-4.

The same run also showed that **the `cited_paths` sketched in Step 1 is not shippable**: it
reports 23 paths on this `CLAUDE.md`, 22 false — crate-relative fragments (`cli/`, `web/`,
`parser/`, `test_support/`, …), user-config paths (`~/.config/armadai/`), a bare extension
(`.md`), a bare convention filename (`armadai.yaml`), a module directory
(`core/orchestration/es/`). See the shipped `rightsizing.rs` for what replaced it: a path must
carry a directory part *and* a real source extension (via `Path::extension`, so a bare `.md`
never qualifies) and must not be absolute, home-relative or URL-shaped; and resolution is
root-relative **or** a whole-component suffix anywhere in the tree, because a multi-crate repo
cites modules relative to their crate. Measured on the pre-#382 `CLAUDE.md`, suffix resolution
reports 16 stale paths with no crate-prefix false positive where root-only reports 24 with 9
false — and it catches the exact family #382 called INTROUVABLES (`core/*.rs`, `storage/*.rs`).

- [ ] **Step 7: Commit**

```bash
git add crates/armadai/src/audit/
git commit -m "feat(audit): add R02, flagging a cited path that does not exist"
```

---

### Task 4: `R04` — weight of the front-loaded context

> Titled "always-loaded" in the original plan. Renamed on measurement: only the instructions
> file is always loaded. See the last lesson above.

**Files:**
- Modify: `crates/armadai/src/audit/rules/rightsizing.rs`
- Modify: `crates/armadai/src/audit/rules/mod.rs` (registry entry)

**Interfaces:**
- Produces: `pub(super) fn r04_context_weight(ctx: &AuditContext) -> Vec<Finding>` — exactly one `Info` finding, or none when there is nothing to weigh.

- [ ] **Step 1: Write the failing tests**

```rust
/// R04 — how many tokens load by default, every time.
///
/// Info, no judgement, on the model of `U04`. Its value is making a cost
/// visible that nobody currently sees. `references/` are excluded on purpose:
/// they load on demand, which is the whole point of splitting.
pub(super) fn r04_context_weight(ctx: &AuditContext) -> Vec<Finding> {
    let instructions_tokens = ctx
        .config
        .instructions
        .as_ref()
        .map(|i| super::estimate_tokens(&i.content))
        .unwrap_or(0);
    let skills_tokens: usize = ctx.config.skills.iter().map(|s| s.body_tokens).sum();
    let total = instructions_tokens + skills_tokens;
    if total == 0 {
        return Vec::new();
    }

    let anchor = ctx
        .config
        .instructions
        .as_ref()
        .map(|i| i.source_path.clone())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    vec![Finding {
        rule: "R04",
        severity: Severity::Info,
        file: anchor,
        related: Vec::new(),
        message: format!(
            "~{total} tokens load on every invocation: {instructions_tokens} from the \
             instructions file, {skills_tokens} from {} skill(s). references/ excluded — \
             those load on demand",
            ctx.config.skills.len()
        ),
        suggestion: None,
    }]
}
```

Tests:

```rust
    #[test]
    fn r04_sums_instructions_and_skills() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = instructions_saying(dir.path(), &"x".repeat(400)); // 100 tokens
        config.skills = vec![skill("a", 300), skill("b", 200)];
        let settings = AuditSettings::default();
        let f = r04_context_weight(&ctx_for(&config, &settings));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Info);
        assert!(
            f[0].message.contains("600"),
            "total must be 100 + 300 + 200: {}",
            f[0].message
        );
    }

    #[test]
    fn r04_is_silent_on_an_empty_project() {
        let settings = AuditSettings::default();
        let config = ImportedConfig::default();
        assert!(r04_context_weight(&ctx_for(&config, &settings)).is_empty());
    }
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --bin armadai --no-fail-fast r04`
Expected: FAIL — `600` absent, or compile error before registration.

- [ ] **Step 3: Register**

Add `rightsizing::r04_context_weight,` to `registry()`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --bin armadai --no-fail-fast r04`
Expected: PASS.

- [ ] **Step 5: Mutation check**

Two, not one:

1. Drop `skills_tokens` from the sum (`let total = instructions_tokens;`) →
   `r04_sums_instructions_and_skills` must FAIL on the `600` assertion.
2. **Remove the `registry()` entry** → a `r04_is_wired_into_the_registry` test must FAIL.

Restore after each, re-run green, record both outputs.

- [ ] **Step 6: Commit**

```bash
git add crates/armadai/src/audit/
git commit -m "feat(audit): add R04, reporting the always-loaded context weight"
```

---

### Task 5: Black-box coverage and documentation

**Files:**
- Create: `crates/armadai/tests/audit_rightsizing.rs`
- Modify: `docs/wiki/audit.md`

**Interfaces:**
- Consumes: the real `armadai` binary via `env!("CARGO_BIN_EXE_armadai")`.

- [ ] **Step 1: Write the black-box test**

The substance of `R02` is filesystem resolution and of `R04` aggregation — neither is proven by
a unit test on a fixture. Follow the isolation pattern of `crates/armadai/tests/link_manifest.rs`:
redirect **both** `ARMADAI_CONFIG_DIR` and `XDG_DATA_HOME`, because the `#[cfg(test)]` guard in
`db.rs` does not protect a *spawned* binary.

```rust
//! Black-box coverage for the R (rightsizing) rules, on the real binary.

use std::process::Command;

fn run_audit(root: &std::path::Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_armadai"))
        .args(["audit"])
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env("ARMADAI_CONFIG_DIR", root.join(".cfg"))
        .env("XDG_DATA_HOME", root.join(".data"))
        .output()
        .expect("armadai must run");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn r02_names_a_stale_path_in_the_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".claude/agents")).unwrap();
    std::fs::write(
        root.join(".claude/agents/one.md"),
        "---\nname: one\ndescription: d\n---\nBody.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("CLAUDE.md"),
        "The engine lives in `src/vanished/engine.rs`.\n",
    )
    .unwrap();

    let (_ok, text) = run_audit(root);
    assert!(
        text.contains("R02") && text.contains("src/vanished/engine.rs"),
        "R02 must name the missing path; got:\n{text}"
    );
}

#[test]
fn r01_names_an_oversized_skill_without_references() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let skill = root.join(".claude/skills/heavy");
    std::fs::create_dir_all(&skill).unwrap();
    // Past the 3000-token default: 16k chars -> ~4000 tokens.
    std::fs::write(
        skill.join("SKILL.md"),
        format!("---\nname: heavy\ndescription: d\n---\n{}", "x".repeat(16_000)),
    )
    .unwrap();

    let (_ok, text) = run_audit(root);
    assert!(
        text.contains("R01") && text.contains("heavy"),
        "R01 must name the oversized skill; got:\n{text}"
    );
}
```

- [ ] **Step 2: Run and watch them fail, then pass**

Run: `cargo test --test audit_rightsizing --no-fail-fast`
If red for a reason other than the assertion (binary not found, audit refusing to run on a
bare project), fix that first — a test that fails for the wrong reason proves nothing.

- [ ] **Step 3: Mutation check**

Unregister `r02_stale_path` from `registry()` → `r02_names_a_stale_path_in_the_instructions`
must FAIL. Same for `r01`. This is what proves the rules are actually wired into the CLI path,
not just unit-tested. Restore both.

- [ ] **Step 4: Document the rules**

In `docs/wiki/audit.md`, add three rows to the rule table in the existing format, and document
`skill_token_threshold` alongside `prompt_token_threshold` in the `audit:` settings section.
State the derivation of the default (p90 of 460 measured skills) so the number does not read as
arbitrary.

- [ ] **Step 5: Full gate**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,storage,e2e-fake -- -D warnings
cargo test --no-default-features --features tui --no-fail-fast
cargo test --no-default-features --features tui,storage,e2e-fake,web --no-fail-fast
cargo test --no-default-features --features tui,providers-api --no-fail-fast
cargo test --no-default-features --features tui,web,storage,providers-api --no-fail-fast
cargo test --no-default-features --features tui,storage,e2e-fake --test gaveldrop
```

gaveldrop must still read **13 cases · 83/83**.

- [ ] **Step 6: Commit and open the PR**

```bash
git add crates/armadai/tests/ docs/wiki/audit.md
git commit -m "test(audit): cover the R rules on the real binary"
```

PR body in French, linking #384, with the mutation outputs for each rule. Do not merge.

---

## Task 5 — measured corrections

Six, in the same spirit as the lessons above: found by executing the task, not by reading it.

**The isolation list was short by one variable.** Step 1 redirects `ARMADAI_CONFIG_DIR` and
`XDG_DATA_HOME`, and both are needed. But `cli::audit::execute` also calls
`usage::scan(&root)` unconditionally, which reads the developer's real `~/.claude/projects`
unless `ARMADAI_CLAUDE_PROJECTS_DIR` says otherwise — machine-dependent, potentially hundreds of
megabytes, and any `U0x` finding it produces lands in the very stdout these tests assert on.
`audit_usage.rs` already sets it; the shipped helper sets all three.

**The proposed assertions were whole-output `contains`, the exact defect `audit_usage.rs`
documents.** `text.contains("R01") && text.contains("heavy")` can pass on two unrelated lines —
the report names the same file on its `A09`, `A12` and `R04` lines. Every assertion here is
same-line, through an `Output::line_with(rule, needles)` helper that also insists on **exactly
one** matching line, and each carries the measured number (`~4010 tokens`, `(threshold 3000)`,
`~300 tokens`) rather than the rule code alone.

**Step 4 pointed at a settings section that does not exist.** "document
`skill_token_threshold` alongside `prompt_token_threshold` in the `audit:` settings section" —
`grep -rn prompt_token_threshold docs/` finds it only in `declarative-agents.md` and in older
plans. `docs/wiki/audit.md` had no settings section at all, so one was created, documenting all
five keys.

**Unregistering a rule is necessary but not sufficient to prove CLI reachability.** Task 3-4
already added `rXX_is_wired_into_the_registry` unit tests, which go red on that same mutation —
so on its own it does not separate "the CLI reaches the rule" from "`run_rules` reaches the
rule". Two mutations that **only** the black-box tests catch were measured instead, and they are
the ones that answer the question:

1. `print_terminal`'s severity loop reduced to `[Critical, Warning]` — the R04 finding is still
   computed (the Summary line still says `1 info`, the Breakdown still says `R04×1`) and is
   never printed. **734 unit tests green, `r04_totals_the_front_loaded_context_from_the_real_files`
   red.**
2. `cli::audit::execute` passing `&AuditSettings::default()` instead of the loaded `&settings` —
   the project's `audit.skill_token_threshold` is read, parsed, and thrown away. **734 unit tests
   green, `the_project_config_threshold_reaches_r01_through_the_cli` red.**

**Two lessons above are now stale, and the fix landed before Task 5 started.** The last lesson
says "R01's message still reads 'so all of it loads on every invocation'"; `ce7fd30` reworded it
to "so the whole body is loaded as soon as the skill triggers". Task 3's Step 6 says the stale
`providers/traits.rs` in `CLAUDE.md` was left in place deliberately; `ce7fd30` fixed that too, and
`armadai audit --no-usage` on this repo now reports **zero** R02 findings (`R04  CLAUDE.md
~1423 tokens`, plus pre-existing `A06`/`A08`/`A10`).

**The R01 fixture's missing `.claude/agents/` is fine — do not "fix" it.** `ClaudeReverseLinker::detect`
(`reverse/claude.rs:52-56`) returns true on `.claude/skills` or `CLAUDE.md` alone, so a
skills-only project is detected and `execute` does not take its "nothing here" early return.
Verified by running it.
