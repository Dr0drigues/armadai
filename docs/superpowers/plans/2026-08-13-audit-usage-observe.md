# Audit de l'usage observé — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mesurer ce que le projet fait réellement des orchestrateurs natifs Claude Code (sous-agents, skills, outils) en scannant les transcripts JSONL, puis nourrir `armadai audit --propose` avec ces faits.

**Architecture:** Un nouveau module `crates/armadai/src/audit/usage/` en miroir de `audit/reverse/` : `discovery.rs` trouve les transcripts, `scan.rs` les lit en streaming, `facts.rs` porte l'agrégat déterministe `UsageFacts`. Cet agrégat alimente quatre règles `U0x` puis `generate_proposal`, qui gagne les modèles observés, des tags de volumétrie, et un `armadai.yaml` déduit de l'arbre de délégation. Le seul jugement non déterministe — le nommage des routes — est isolé derrière `--deep`.

**Tech Stack:** Rust edition 2024, `serde_json` (déjà présent), `anyhow`, `tempfile` et `assert_cmd` pour les tests. Pas de gaveldrop : son adaptateur ne couvre que les runs orchestrés (voir Task 9).

**Spec:** `docs/superpowers/specs/2026-08-13-audit-usage-observe-design.md`

## Global Constraints

- Aucune nouvelle dépendance, aucune nouvelle feature flag. Le module reste non gated comme `audit/`.
- Clippy doit passer dans les **4 modes CI** : `tui` / `tui,providers-api` / `tui,web,storage` / `tui,storage,e2e-fake`, chacun en `--all-targets -- -D warnings`.
- Tests : `cargo test --no-default-features --features tui` et `--features tui,storage,e2e-fake`.
- `cargo fmt --all` avant chaque commit.
- Code, commentaires et messages de commit en **anglais**. Conventional Commits, **un seul type** par message.
- Trailer de commit : `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`
- Le scan ne doit **jamais** échouer sur une donnée absente ou malformée : un champ manquant dégrade la métrique correspondante, il ne casse pas l'audit.
- Aucun chemin machine (`--json`, `sink.emit`) ne doit être coloré ou modifié.

## Écart assumé par rapport au spec

Le spec liste « durées, échecs » parmi les métriques de `UsageFacts`. Elles ne sont **pas** implémentées : elles exigeraient d'exposer `is_error` et les timestamps par bloc dans `RelevantEntry`, ce qui élargit le changement sur `transcript.rs` bien au-delà du nécessaire, et aucun consommateur des lots 2 et 3 ne les lit. Le reste de `UsageFacts` est implémenté intégralement. Si un besoin réel apparaît, c'est un ajout additif à `AgentUsage`.

---

# LOT 1 — Observation (PR 1)

Livrable autonome : `armadai audit` rapporte l'usage réel et émet les findings `U01`–`U04`.

### Task 1: Exposer le nom des outils dans le parseur de transcript

`Block::Other` écrase le nom de tout `tool_use` non-`Agent`. Le mapper ignore cette variante, donc lui donner un nom ne change rien au Workroom mais rend la donnée disponible au scan.

**Files:**
- Modify: `crates/armadai/src/claude_adapter/transcript.rs:12` (variante), `:112` (construction), `:295` (test existant)
- Modify: `crates/armadai/src/claude_adapter/mapper.rs:89` (match arm)

**Interfaces:**
- Consumes: rien.
- Produces: `Block::Tool { name: String }` remplace `Block::Other`.

- [ ] **Step 1: Write the failing test**

Dans le module `tests` de `transcript.rs` :

```rust
#[test]
fn non_agent_tool_use_keeps_its_name() {
    let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
    match parse_line(line).expect("assistant entry") {
        RelevantEntry::Assistant { blocks, .. } => {
            assert_eq!(
                blocks.as_slice(),
                [Block::Tool { name: "Bash".to_string() }],
                "a non-Agent tool_use must carry its tool name"
            );
        }
        _ => panic!("expected Assistant"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui non_agent_tool_use_keeps_its_name`
Expected: FAIL — `Block::Tool` n'existe pas (erreur de compilation `E0599`/`E0433`).

- [ ] **Step 3: Write minimal implementation**

Dans `transcript.rs`, remplacer la variante `Other` :

```rust
pub enum Block {
    Text(String),
    AgentSpawn {
        tool_use_id: String,
        subagent_type: String,
        description: String,
    },
    /// Any other `tool_use`, keyed by its tool name (`Bash`, `Read`, `Skill`, …).
    Tool { name: String },
}
```

Et le site de construction (ligne ~112) :

```rust
            Some("tool_use") => {
                let name = b
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                blocks.push(Block::Tool { name });
            }
```

Dans `mapper.rs:89`, le match arm devient :

```rust
                        Block::Text(_) | Block::Tool { .. } => {}
```

Dans le test existant de `transcript.rs:295`, remplacer l'assertion :

```rust
                assert!(matches!(blocks.as_slice(), [Block::Tool { .. }]))
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui claude_adapter`
Expected: PASS, y compris tous les tests existants du mapper.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/claude_adapter/
git commit -m "refactor(transcript): keep the tool name on non-Agent tool_use blocks

Block::Other erased the tool name. The mapper ignores this variant either
way, so naming it leaves the Workroom unchanged while making tool usage
observable for the audit scan."
```

---

### Task 2: Extraire `parse_value` pour un seul parse JSON par ligne

Le scan a besoin de l'**enveloppe** de la ligne (`timestamp`, `isSidechain`, `attributionSkill`, `uuid`, `parentUuid`, `sessionId`, `cwd`) autant que du **message**. Parser la ligne deux fois doublerait le coût sur 287 Mo. On extrait donc la partie « à partir d'un `Value` déjà parsé ».

**Files:**
- Modify: `crates/armadai/src/claude_adapter/transcript.rs:49-60`

**Interfaces:**
- Consumes: Task 1 (`Block::Tool`).
- Produces: `pub fn parse_value(v: &serde_json::Value) -> Option<RelevantEntry>`. `parse_line` reste inchangé pour ses appelants existants.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parse_value_matches_parse_line_on_the_same_input() {
    let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":2,"output_tokens":3}}}"#;
    let v: Value = serde_json::from_str(line).unwrap();
    assert_eq!(
        parse_value(&v),
        parse_line(line),
        "parse_value must be the same parser, minus the string decoding step"
    );
    assert!(parse_value(&v).is_some(), "and it must actually parse");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui parse_value_matches_parse_line`
Expected: FAIL — `parse_value` n'existe pas.

- [ ] **Step 3: Write minimal implementation**

Remplacer `parse_line` par un mince wrapper et déplacer le corps dans `parse_value` :

```rust
/// Defensive parse of one transcript JSONL line. Returns `None` for malformed
/// lines and for any entry type the adapter does not model (ai-title, mode,
/// pr-link, system, attachment, …) — never panics.
pub fn parse_line(line: &str) -> Option<RelevantEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    parse_value(&v)
}

/// Same as [`parse_line`], for a `Value` the caller already parsed. The audit
/// scan reads the entry envelope (timestamp, isSidechain, attribution…) from
/// the same `Value`, so parsing the line twice would double its cost.
pub fn parse_value(v: &Value) -> Option<RelevantEntry> {
    match v.get("type")?.as_str()? {
        "assistant" => parse_assistant(v.get("message")?),
        "user" => parse_user_tool_result(v.get("message")?),
        _ => None,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui claude_adapter`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/claude_adapter/transcript.rs
git commit -m "refactor(transcript): split parse_value out of parse_line

The audit scan reads the entry envelope from the same JSON object as the
message, so it must not re-parse the line."
```

---

### Task 3: Découverte des transcripts (slug + repli par `cwd`)

**Files:**
- Create: `crates/armadai/src/audit/usage/mod.rs`
- Create: `crates/armadai/src/audit/usage/discovery.rs`
- Modify: `crates/armadai/src/audit/mod.rs` (ajouter `pub mod usage;`)

**Interfaces:**
- Consumes: rien.
- Produces:
  - `pub fn projects_root() -> Option<PathBuf>`
  - `pub fn slug_for(root: &Path) -> String`
  - `pub fn transcript_files(root: &Path) -> Vec<PathBuf>`

- [ ] **Step 1: Write the failing tests**

Créer `discovery.rs` avec uniquement le module de tests d'abord :

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises `ARMADAI_CLAUDE_PROJECTS_DIR` mutation across the crate,
    /// mirroring the SessionIndexEnvGuard pattern in `cli/watch.rs`.
    struct ProjectsDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ProjectsDirGuard {
        fn set(path: &Path) -> Self {
            let lock = armadai_core::config::ENV_MUTEX.lock().unwrap();
            // SAFETY: modifies the global environment; serialised via ENV_MUTEX.
            unsafe { std::env::set_var("ARMADAI_CLAUDE_PROJECTS_DIR", path) }
            Self { _lock: lock }
        }
    }

    impl Drop for ProjectsDirGuard {
        fn drop(&mut self) {
            // SAFETY: restoring env state at end of test scope.
            unsafe { std::env::remove_var("ARMADAI_CLAUDE_PROJECTS_DIR") }
        }
    }

    #[test]
    fn slug_replaces_path_separators_with_dashes() {
        assert_eq!(
            slug_for(Path::new("/Users/x/work/misc/armadai")),
            "-Users-x-work-misc-armadai"
        );
    }

    #[test]
    fn finds_transcripts_by_slug() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let project = Path::new("/Users/x/proj");
        let slug_dir = dir.path().join(slug_for(project));
        std::fs::create_dir_all(&slug_dir).unwrap();
        std::fs::write(slug_dir.join("a.jsonl"), "").unwrap();
        std::fs::write(slug_dir.join("ignored.txt"), "").unwrap();

        let found = transcript_files(project);
        assert_eq!(found.len(), 1, "only .jsonl files count: {found:?}");
        assert!(found[0].ends_with("a.jsonl"));
    }

    #[test]
    fn falls_back_to_cwd_matching_when_slug_misses() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        // A directory whose name does NOT match the slug rule, but whose
        // entries declare the audited root as their cwd.
        let odd = dir.path().join("some-unexpected-name");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(
            odd.join("s.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/Users/x/proj\"}\n",
        )
        .unwrap();

        let found = transcript_files(Path::new("/Users/x/proj"));
        assert_eq!(found.len(), 1, "cwd fallback must find it: {found:?}");
    }

    #[test]
    fn missing_projects_dir_yields_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(&dir.path().join("does-not-exist"));
        assert!(transcript_files(Path::new("/Users/x/proj")).is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui audit::usage::discovery`
Expected: FAIL — les fonctions n'existent pas.

- [ ] **Step 3: Write minimal implementation**

`crates/armadai/src/audit/usage/mod.rs` :

```rust
//! Observed usage of native agentic assets, read from Claude Code transcripts.
//!
//! Mirror of `audit::reverse` in the runtime direction: `reverse` reads what a
//! project *declares*, this module reads what it actually *ran*.
pub mod discovery;
```

`crates/armadai/src/audit/usage/discovery.rs` :

```rust
use std::path::{Path, PathBuf};

/// Root holding Claude Code's per-project transcript directories.
/// `ARMADAI_CLAUDE_PROJECTS_DIR` overrides it (used by tests and the e2e suite).
pub fn projects_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ARMADAI_CLAUDE_PROJECTS_DIR") {
        return Some(PathBuf::from(dir));
    }
    Some(dirs::home_dir()?.join(".claude").join("projects"))
}

/// Claude Code's directory-name encoding for a project path: separators become
/// dashes (`/Users/x/proj` -> `-Users-x-proj`).
pub fn slug_for(root: &Path) -> String {
    root.to_string_lossy().replace(['/', '\\'], "-")
}

/// Every `.jsonl` transcript belonging to `root`.
///
/// Two-tier resolution: the slug is only an access shortcut, so when it misses
/// (its exact encoding of `.`, `_` and spaces is not publicly specified) we
/// scan every project directory and keep those whose entries declare `root` as
/// their `cwd` — that field is in the data and is authoritative.
pub fn transcript_files(root: &Path) -> Vec<PathBuf> {
    let Some(projects) = projects_root() else {
        return Vec::new();
    };
    let by_slug = projects.join(slug_for(root));
    if by_slug.is_dir() {
        return jsonl_in(&by_slug);
    }
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let files = jsonl_in(&entry.path());
        if files.iter().any(|f| declares_cwd(f, root)) {
            found.extend(files);
        }
    }
    found.sort();
    found
}

fn jsonl_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();
    files
}

/// True if any of the file's first lines declares `root` as its `cwd`.
/// Only the head is read: `cwd` is repeated on every entry, so a few lines
/// settle it without reading a multi-megabyte transcript.
fn declares_cwd(file: &Path, root: &Path) -> bool {
    use std::io::BufRead;
    let Ok(handle) = std::fs::File::open(file) else {
        return false;
    };
    let wanted = root.to_string_lossy();
    std::io::BufReader::new(handle)
        .lines()
        .map_while(Result::ok)
        .take(20)
        .any(|line| {
            serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .and_then(|v| {
                    v.get("cwd")
                        .and_then(serde_json::Value::as_str)
                        .map(|c| c == wanted)
                })
                .unwrap_or(false)
        })
}
```

Dans `crates/armadai/src/audit/mod.rs`, ajouter à la liste des modules :

```rust
pub mod usage;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit::usage::discovery`
Expected: PASS (4 tests).

Si `dirs` n'est pas déjà une dépendance de la crate `armadai`, vérifier avec `grep -n '^dirs' crates/armadai/Cargo.toml`. En cas d'absence, utiliser à la place le helper de config déjà présent : `armadai_core::config::config_dir()` expose le même socle, remonter d'un niveau n'est pas acceptable — préférer alors `std::env::var("HOME")` avec repli `None`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/
git commit -m "feat(audit): discover Claude Code transcripts for a project

Two-tier resolution: the documented-by-observation slug encoding first,
then a cwd-based scan, because the slug's handling of dots, underscores and
spaces is not specified. The cwd field is in the data and authoritative."
```

---

### Task 4: Le type `UsageFacts` et son agrégation pure

Toute la logique d'agrégation est testable sans I/O : `scan.rs` (Task 5) ne fait que lire des fichiers et pousser dans ce type.

**Files:**
- Create: `crates/armadai/src/audit/usage/facts.rs`
- Modify: `crates/armadai/src/audit/usage/mod.rs`

**Interfaces:**
- Consumes: rien.
- Produces:
  - `pub struct AgentUsage { pub invocations: u32, pub models: BTreeMap<String, u32> }`
  - `pub struct UsageFacts { pub sessions: u32, pub window: Option<(String, String)>, pub agents: BTreeMap<String, AgentUsage>, pub skills: BTreeMap<String, u32>, pub tools: BTreeMap<String, u32>, pub root_agent: String, pub edges: BTreeMap<String, BTreeSet<String>>, pub max_fanout: u32 }`
  - `impl UsageFacts { pub fn observe_timestamp(&mut self, ts: &str); pub fn record_delegation(&mut self, parent: &str, child: &str, model: &str); pub fn record_skill_turn(&mut self, skill: &str); pub fn record_tool(&mut self, tool: &str); pub fn depth(&self) -> u32; pub fn dominant_model(&self, agent: &str) -> Option<&str>; pub fn is_empty(&self) -> bool }`
  - `pub const ROOT_AGENT: &str = "claude";`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_tracks_oldest_and_newest_timestamp() {
        let mut f = UsageFacts::default();
        f.observe_timestamp("2026-08-02T10:00:00Z");
        f.observe_timestamp("2026-07-01T10:00:00Z");
        f.observe_timestamp("2026-08-13T10:00:00Z");
        assert_eq!(
            f.window,
            Some((
                "2026-07-01T10:00:00Z".to_string(),
                "2026-08-13T10:00:00Z".to_string()
            ))
        );
    }

    #[test]
    fn delegation_counts_invocations_and_models() {
        let mut f = UsageFacts::default();
        f.record_delegation(ROOT_AGENT, "qa", "claude-opus-5");
        f.record_delegation(ROOT_AGENT, "qa", "claude-opus-5");
        f.record_delegation(ROOT_AGENT, "qa", "claude-sonnet-5");
        let qa = &f.agents["qa"];
        assert_eq!(qa.invocations, 3);
        assert_eq!(f.dominant_model("qa"), Some("claude-opus-5"));
    }

    #[test]
    fn depth_is_one_for_a_flat_tree_and_two_when_nested() {
        let mut flat = UsageFacts::default();
        flat.record_delegation(ROOT_AGENT, "qa", "m");
        flat.record_delegation(ROOT_AGENT, "core", "m");
        assert_eq!(flat.depth(), 1, "root -> agents is depth 1");

        let mut nested = UsageFacts::default();
        nested.record_delegation(ROOT_AGENT, "lead", "m");
        nested.record_delegation("lead", "qa", "m");
        assert_eq!(nested.depth(), 2, "root -> lead -> agent is depth 2");
    }

    #[test]
    fn depth_is_zero_without_any_delegation() {
        assert_eq!(UsageFacts::default().depth(), 0);
        assert!(UsageFacts::default().is_empty());
    }

    #[test]
    fn depth_terminates_on_a_cyclic_edge_set() {
        // Defensive: a malformed transcript must never hang the audit.
        let mut f = UsageFacts::default();
        f.record_delegation(ROOT_AGENT, "a", "m");
        f.record_delegation("a", "b", "m");
        f.record_delegation("b", "a", "m");
        assert!(f.depth() >= 2, "cycle must not loop forever");
    }

    #[test]
    fn skill_turns_and_tools_accumulate() {
        let mut f = UsageFacts::default();
        f.record_skill_turn("armadai");
        f.record_skill_turn("armadai");
        f.record_tool("Bash");
        assert_eq!(f.skills["armadai"], 2);
        assert_eq!(f.tools["Bash"], 1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui audit::usage::facts`
Expected: FAIL — le type n'existe pas.

- [ ] **Step 3: Write minimal implementation**

```rust
use std::collections::{BTreeMap, BTreeSet};

/// The native CLI's main thread, i.e. the root of every observed delegation
/// tree. Claude Code's own turns are not a declared agent, so the tree needs a
/// stable name for them.
pub const ROOT_AGENT: &str = "claude";

/// What one agent was observed doing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentUsage {
    pub invocations: u32,
    /// Model name -> number of delegations seen on that model.
    pub models: BTreeMap<String, u32>,
}

/// Deterministic aggregate of everything the scan observed. Serialisable by
/// construction: no paths, no handles, only counted facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageFacts {
    pub sessions: u32,
    /// Oldest and newest timestamps encountered — a constat, not a filter.
    /// ISO-8601 UTC strings compare correctly lexicographically.
    pub window: Option<(String, String)>,
    pub agents: BTreeMap<String, AgentUsage>,
    /// Skill -> attributed turns (`attributionSkill`), the reliable metric.
    pub skills: BTreeMap<String, u32>,
    pub tools: BTreeMap<String, u32>,
    pub root_agent: String,
    /// Delegation edges: parent -> children.
    pub edges: BTreeMap<String, BTreeSet<String>>,
    /// Largest parallel fan-out seen in a single assistant message.
    pub max_fanout: u32,
}

impl UsageFacts {
    pub fn observe_timestamp(&mut self, ts: &str) {
        if ts.is_empty() {
            return;
        }
        self.window = Some(match self.window.take() {
            None => (ts.to_string(), ts.to_string()),
            Some((min, max)) => (
                if ts < min.as_str() { ts.to_string() } else { min },
                if ts > max.as_str() { ts.to_string() } else { max },
            ),
        });
    }

    pub fn record_delegation(&mut self, parent: &str, child: &str, model: &str) {
        let entry = self.agents.entry(child.to_string()).or_default();
        entry.invocations += 1;
        if !model.is_empty() {
            *entry.models.entry(model.to_string()).or_default() += 1;
        }
        self.edges
            .entry(parent.to_string())
            .or_default()
            .insert(child.to_string());
    }

    pub fn record_skill_turn(&mut self, skill: &str) {
        *self.skills.entry(skill.to_string()).or_default() += 1;
    }

    pub fn record_tool(&mut self, tool: &str) {
        if tool.is_empty() {
            return;
        }
        *self.tools.entry(tool.to_string()).or_default() += 1;
    }

    /// Most-used model for `agent`, ties broken by name for determinism.
    pub fn dominant_model(&self, agent: &str) -> Option<&str> {
        let usage = self.agents.get(agent)?;
        usage
            .models
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(name, _)| name.as_str())
    }

    /// Longest delegation chain from the root. Visited-set guarded: a
    /// malformed transcript must never hang the audit.
    pub fn depth(&self) -> u32 {
        fn walk(
            edges: &BTreeMap<String, BTreeSet<String>>,
            node: &str,
            seen: &mut BTreeSet<String>,
        ) -> u32 {
            if !seen.insert(node.to_string()) {
                return 0;
            }
            let deepest = edges
                .get(node)
                .into_iter()
                .flatten()
                .map(|child| walk(edges, child, seen))
                .max()
                .unwrap_or(0);
            deepest + 1
        }
        if self.edges.is_empty() {
            return 0;
        }
        let root = if self.root_agent.is_empty() {
            ROOT_AGENT
        } else {
            self.root_agent.as_str()
        };
        walk(self.edges, root, &mut BTreeSet::new()).saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.skills.is_empty() && self.tools.is_empty()
    }
}
```

Note pour l'implémenteur : `walk(self.edges, …)` doit être `walk(&self.edges, …)`. Corriger à la compilation.

Ajouter à `usage/mod.rs` :

```rust
pub mod facts;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit::usage::facts`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/usage/
git commit -m "feat(audit): add the UsageFacts deterministic aggregate

Pure counting over delegations, skills and tools, with a cycle-guarded
depth walk so a malformed transcript can never hang the audit."
```

---

### Task 5: Le scan en streaming

**Files:**
- Create: `crates/armadai/src/audit/usage/scan.rs`
- Modify: `crates/armadai/src/audit/usage/mod.rs`

**Interfaces:**
- Consumes: Task 2 (`transcript::parse_value`), Task 3 (`transcript_files`), Task 4 (`UsageFacts`).
- Produces: `pub fn scan(root: &Path) -> UsageFacts`

Règles de rattachement, dans l'ordre :
1. une délégation vue dans une entrée **non**-sidechain a pour parent `ROOT_AGENT` ;
2. dans une entrée sidechain, le parent est l'agent ouvert par le `tool_use` le plus proche en remontant la chaîne `parentUuid` ;
3. si la remontée n'aboutit pas, rattachement à `ROOT_AGENT` (dégradation documentée, jamais une erreur).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, lines: &[&str]) {
        std::fs::write(dir.join(name), format!("{}\n", lines.join("\n"))).unwrap();
    }

    /// Same env-guard shape as discovery's tests.
    struct ProjectsDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl ProjectsDirGuard {
        fn set(path: &Path) -> Self {
            let lock = armadai_core::config::ENV_MUTEX.lock().unwrap();
            // SAFETY: modifies the global environment; serialised via ENV_MUTEX.
            unsafe { std::env::set_var("ARMADAI_CLAUDE_PROJECTS_DIR", path) }
            Self { _lock: lock }
        }
    }
    impl Drop for ProjectsDirGuard {
        fn drop(&mut self) {
            // SAFETY: restoring env state at end of test scope.
            unsafe { std::env::remove_var("ARMADAI_CLAUDE_PROJECTS_DIR") }
        }
    }

    fn fixture(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let project = PathBuf::from("/Users/x/proj");
        let slug = dir
            .path()
            .join(crate::audit::usage::discovery::slug_for(&project));
        std::fs::create_dir_all(&slug).unwrap();
        write(&slug, "s1.jsonl", lines);
        (dir, project)
    }

    #[test]
    fn counts_delegations_skills_tools_and_sessions() {
        let (dir, project) = fixture(&[
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","message":{"model":"claude-opus-5","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa","description":"run gate"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"{"type":"assistant","timestamp":"2026-08-02T00:00:00Z","isSidechain":false,"uuid":"u2","attributionSkill":"armadai","message":{"model":"claude-opus-5","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        let f = scan(&project);
        assert_eq!(f.sessions, 1, "one transcript file = one session");
        assert_eq!(f.agents["qa"].invocations, 1);
        assert_eq!(f.dominant_model("qa"), Some("claude-opus-5"));
        assert_eq!(f.skills["armadai"], 1);
        assert_eq!(f.tools["Bash"], 1);
        assert_eq!(
            f.window,
            Some((
                "2026-08-01T00:00:00Z".to_string(),
                "2026-08-02T00:00:00Z".to_string()
            ))
        );
        assert_eq!(f.depth(), 1);
    }

    #[test]
    fn parallel_fanout_in_one_message_is_measured() {
        let (dir, project) = fixture(&[
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa","description":"a"}},{"type":"tool_use","id":"t2","name":"Agent","input":{"subagent_type":"core","description":"b"}},{"type":"tool_use","id":"t3","name":"Agent","input":{"subagent_type":"ui","description":"c"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());
        assert_eq!(scan(&project).max_fanout, 3);
    }

    #[test]
    fn sidechain_delegation_nests_under_its_parent_agent() {
        let (dir, project) = fixture(&[
            // Main thread spawns "lead" (tool_use id t1) from entry u1.
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"lead","description":"lead work"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            // Inside lead's sidechain (parent chain u2 -> u1), it spawns "qa".
            r#"{"type":"assistant","timestamp":"2026-08-01T00:01:00Z","isSidechain":true,"uuid":"u2","parentUuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t2","name":"Agent","input":{"subagent_type":"qa","description":"sub work"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        let f = scan(&project);
        assert_eq!(f.depth(), 2, "qa must nest under lead, not under the root");
        assert!(f.edges["lead"].contains("qa"), "edges: {:?}", f.edges);
    }

    #[test]
    fn malformed_and_unknown_lines_are_skipped_without_failing() {
        let (dir, project) = fixture(&[
            "not json at all",
            r#"{"type":"ai-title","aiTitle":"x"}"#,
            r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        let f = scan(&project);
        assert_eq!(f.agents["qa"].invocations, 1, "the valid line still counts");
        assert_eq!(f.window, None, "no timestamp anywhere -> no window");
    }

    #[test]
    fn no_transcripts_yields_empty_facts() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let f = scan(Path::new("/Users/x/nothing-here"));
        assert!(f.is_empty());
        assert_eq!(f.sessions, 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui audit::usage::scan`
Expected: FAIL — `scan` n'existe pas.

- [ ] **Step 3: Write minimal implementation**

```rust
use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;

use serde_json::Value;

use crate::claude_adapter::transcript::{Block, RelevantEntry, parse_value};

use super::discovery::transcript_files;
use super::facts::{ROOT_AGENT, UsageFacts};

/// Aggregate every transcript belonging to `root`.
///
/// Streams line by line: transcripts reach hundreds of megabytes, so no file
/// is ever held in memory. Unreadable files and malformed lines are skipped —
/// a partial transcript still yields usable facts.
pub fn scan(root: &Path) -> UsageFacts {
    let mut facts = UsageFacts {
        root_agent: ROOT_AGENT.to_string(),
        ..Default::default()
    };
    for file in transcript_files(root) {
        let Ok(handle) = std::fs::File::open(&file) else {
            continue;
        };
        facts.sessions += 1;
        // Per-file state: which agent a given entry uuid belongs to, and each
        // entry's parent, so a sidechain delegation can be attributed.
        let mut parent_of: HashMap<String, String> = HashMap::new();
        let mut agent_at: HashMap<String, String> = HashMap::new();
        for line in std::io::BufReader::new(handle).lines().map_while(Result::ok) {
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            scan_entry(&v, &mut facts, &mut parent_of, &mut agent_at);
        }
    }
    facts
}

fn str_field<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

fn scan_entry(
    v: &Value,
    facts: &mut UsageFacts,
    parent_of: &mut HashMap<String, String>,
    agent_at: &mut HashMap<String, String>,
) {
    if let Some(ts) = str_field(v, "timestamp") {
        facts.observe_timestamp(ts);
    }
    if let Some(skill) = str_field(v, "attributionSkill") {
        facts.record_skill_turn(skill);
    }
    let uuid = str_field(v, "uuid").unwrap_or("").to_string();
    if let Some(parent) = str_field(v, "parentUuid") {
        parent_of.insert(uuid.clone(), parent.to_string());
    }
    let is_sidechain = v
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let Some(RelevantEntry::Assistant { model, blocks, .. }) = parse_value(v) else {
        return;
    };

    // Who is delegating in this entry: the root on the main thread, else the
    // nearest enclosing agent found by walking the parentUuid chain.
    let delegator = if is_sidechain {
        enclosing_agent(&uuid, parent_of, agent_at).unwrap_or_else(|| ROOT_AGENT.to_string())
    } else {
        ROOT_AGENT.to_string()
    };

    let mut fanout = 0;
    for block in &blocks {
        match block {
            Block::AgentSpawn {
                subagent_type,
                description,
                ..
            } => {
                // Same labelling rule as the Workroom mapper: a non-empty
                // description wins, so parallel same-type spawns stay distinct
                // there — but usage counts the reusable identity, the type.
                let _ = description;
                fanout += 1;
                facts.record_delegation(&delegator, subagent_type, &model);
                if !uuid.is_empty() {
                    agent_at.insert(uuid.clone(), subagent_type.clone());
                }
            }
            Block::Tool { name } => facts.record_tool(name),
            Block::Text(_) => {}
        }
    }
    facts.max_fanout = facts.max_fanout.max(fanout);
}

/// Walk up the parentUuid chain until an entry known to have opened an agent
/// is found. Bounded by the chain itself and by a visited set.
fn enclosing_agent(
    uuid: &str,
    parent_of: &HashMap<String, String>,
    agent_at: &HashMap<String, String>,
) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let mut cursor = uuid.to_string();
    while seen.insert(cursor.clone()) {
        if let Some(agent) = agent_at.get(&cursor) {
            return Some(agent.clone());
        }
        cursor = parent_of.get(&cursor)?.clone();
    }
    None
}
```

Ajouter à `usage/mod.rs` :

```rust
pub mod scan;

pub use facts::UsageFacts;
pub use scan::scan;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit::usage`
Expected: PASS (tous les tests de discovery, facts et scan).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/usage/
git commit -m "feat(audit): stream-scan transcripts into UsageFacts

Reads the entry envelope and the message from a single parsed Value, walks
parentUuid to attribute sidechain delegations to their enclosing agent, and
skips malformed lines rather than failing the audit."
```

---

### Task 6: Brancher l'usage dans `AuditContext` (migration mécanique)

`AuditContext` est construit **39 fois** dans les tests des règles existantes. L'ajout du champ les casse toutes ; le compilateur les liste (`E0063: missing field`), donc la migration est mécanique et sûre.

**Files:**
- Modify: `crates/armadai/src/audit/rules/mod.rs:133-136` (struct), `:37-51` de `audit/mod.rs` (`run_audit`)
- Modify: `crates/armadai/src/audit/rules/{assets,collisions,models,references,similarity}.rs` (sites de test)

**Interfaces:**
- Consumes: Task 4 (`UsageFacts`).
- Produces: `AuditContext { config, settings, usage: Option<&'a UsageFacts> }` et `run_audit(root, settings, usage: Option<&UsageFacts>) -> AuditReport`.

- [ ] **Step 1: Write the failing test**

Dans `audit/mod.rs`, module `tests` :

```rust
#[test]
fn run_audit_accepts_observed_usage() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join(".claude/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("a.md"),
        "---\nname: a\ndescription: d\n---\nBody",
    )
    .unwrap();
    let mut usage = usage::UsageFacts::default();
    usage.record_delegation(usage::facts::ROOT_AGENT, "a", "claude-opus-5");

    let report = run_audit(dir.path(), &rules::AuditSettings::default(), Some(&usage));
    assert_eq!(report.agent_count, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui run_audit_accepts_observed_usage`
Expected: FAIL — `run_audit` prend deux arguments.

- [ ] **Step 3: Write minimal implementation**

Dans `rules/mod.rs` :

```rust
pub struct AuditContext<'a> {
    pub config: &'a ImportedConfig,
    pub settings: &'a AuditSettings,
    /// Observed usage, when transcripts were found. `None` means the static
    /// rules run alone — usage never becomes a prerequisite.
    pub usage: Option<&'a crate::audit::usage::UsageFacts>,
}
```

Dans `audit/mod.rs`, `run_audit` :

```rust
pub fn run_audit(
    root: &Path,
    settings: &rules::AuditSettings,
    usage: Option<&usage::UsageFacts>,
) -> AuditReport {
    let (detected, config) = import_surfaces(root);
    let ctx = rules::AuditContext {
        config: &config,
        settings,
        usage,
    };
    AuditReport {
        root: root.to_path_buf(),
        detected,
        agent_count: config.agents.len(),
        skill_count: config.skills.len(),
        findings: rules::run_rules(&ctx),
        deep_raw: None,
    }
}
```

Puis compiler et ajouter `usage: None,` à chaque littéral signalé :

```bash
cargo test --no-default-features --features tui --no-run 2>&1 | grep -c "missing field \`usage\`"
```

Traiter les erreurs jusqu'à zéro. Ne **pas** utiliser `sed` : les littéraux sont multi-lignes et le compilateur donne les positions exactes.

Dans `cli/audit.rs`, l'unique appel de production devient — pour l'instant — `run_audit(&root, &settings, None)`. Task 8 le branchera sur le vrai scan.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit`
Expected: PASS. Vérifier aussi `cargo clippy --all-targets --no-default-features --features tui -- -D warnings`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/ crates/armadai/src/cli/audit.rs
git commit -m "feat(audit): thread optional observed usage through AuditContext

Option, not a requirement: with no transcript the static rules run exactly
as before."
```

---

### Task 7: Les règles `U01`–`U04`

**Files:**
- Create: `crates/armadai/src/audit/rules/usage_rules.rs`
- Modify: `crates/armadai/src/audit/rules/mod.rs` (déclaration + `registry()`)

**Interfaces:**
- Consumes: Task 6 (`ctx.usage`).
- Produces: `u01_declared_never_used`, `u02_used_but_undeclared`, `u03_coordinator_bypassed`, `u04_session_coverage`, chacune `fn(&AuditContext) -> Vec<Finding>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::usage::UsageFacts;
    use crate::audit::usage::facts::ROOT_AGENT;

    fn ctx<'a>(
        config: &'a crate::audit::reverse::ImportedConfig,
        settings: &'a AuditSettings,
        usage: &'a UsageFacts,
    ) -> AuditContext<'a> {
        AuditContext {
            config,
            settings,
            usage: Some(usage),
        }
    }

    #[test]
    fn u01_flags_a_declared_agent_that_never_ran() {
        let config = config_with(vec![agent("ghost", "prompt"), agent("qa", "prompt")]);
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        usage.sessions = 3;
        usage.record_delegation(ROOT_AGENT, "qa", "m");

        let f = u01_declared_never_used(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "only the unused one: {f:?}");
        assert!(f[0].message.contains("ghost"));
        assert_eq!(f[0].severity, Severity::Warning);
    }

    #[test]
    fn u01_is_silent_without_usage() {
        let config = config_with(vec![agent("ghost", "p")]);
        let settings = AuditSettings::default();
        let f = u01_declared_never_used(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert!(f.is_empty(), "no observation means no claim");
    }

    #[test]
    fn u01_is_silent_when_nothing_was_observed_at_all() {
        let config = config_with(vec![agent("ghost", "p")]);
        let settings = AuditSettings::default();
        let usage = UsageFacts::default();
        let f = u01_declared_never_used(&ctx(&config, &settings, &usage));
        assert!(
            f.is_empty(),
            "empty facts prove nothing about the declared assets"
        );
    }

    #[test]
    fn u02_flags_an_agent_used_but_not_declared() {
        let config = config_with(vec![agent("qa", "p")]);
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        usage.record_delegation(ROOT_AGENT, "qa", "m");
        usage.record_delegation(ROOT_AGENT, "general-purpose", "m");

        let f = u02_used_but_undeclared(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("general-purpose"));
        assert_eq!(f[0].severity, Severity::Info);
        assert!(
            f[0].suggestion.is_some(),
            "the fix (materialise it as an agent) must be spelled out"
        );
    }

    #[test]
    fn u03_flags_a_bypassed_declared_coordinator() {
        let mut config = config_with(vec![agent("dev-lead", "p"), agent("qa", "p")]);
        config.instructions = Some(crate::audit::reverse::ImportedInstructions {
            source_path: std::path::PathBuf::from("CLAUDE.md"),
            content: "delegate to @dev-lead so that he can delegate".to_string(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        for _ in 0..40 {
            usage.record_delegation(ROOT_AGENT, "qa", "m");
        }
        usage.record_delegation(ROOT_AGENT, "dev-lead", "m");

        let f = u03_coordinator_bypassed(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("dev-lead"));
        assert_eq!(f[0].severity, Severity::Warning);
    }

    #[test]
    fn u03_silent_when_the_declared_coordinator_leads() {
        let mut config = config_with(vec![agent("dev-lead", "p"), agent("qa", "p")]);
        config.instructions = Some(crate::audit::reverse::ImportedInstructions {
            source_path: std::path::PathBuf::from("CLAUDE.md"),
            content: "delegate to dev-lead".to_string(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        for _ in 0..10 {
            usage.record_delegation(ROOT_AGENT, "dev-lead", "m");
        }
        usage.record_delegation(ROOT_AGENT, "qa", "m");

        assert!(u03_coordinator_bypassed(&ctx(&config, &settings, &usage)).is_empty());
    }

    #[test]
    fn u04_reports_session_coverage_of_a_declared_skill() {
        let mut config = config_with(vec![]);
        config.skills.push(crate::audit::reverse::ImportedSkill {
            name: "armadai".to_string(),
            source_path: std::path::PathBuf::from(".claude/skills/armadai/SKILL.md"),
            description: Some("project skill".to_string()),
            has_skill_md: true,
            has_frontmatter: true,
            issues: vec![],
            extra: Default::default(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        usage.sessions = 59;
        usage.record_skill_turn("armadai");

        let f = u04_session_coverage(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, Severity::Info);
        assert!(
            f[0].message.contains("59"),
            "coverage must state the denominator: {}",
            f[0].message
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui usage_rules`
Expected: FAIL — les fonctions n'existent pas.

- [ ] **Step 3: Write minimal implementation**

```rust
//! U0x — rules over observed usage. Every rule is silent when no usage was
//! observed: absence of measurement is never evidence of absence of use.

use std::path::PathBuf;

use super::{AuditContext, Finding, Severity};
use crate::audit::usage::UsageFacts;

/// Sub-agents Claude Code provides itself. They are legitimately used without
/// ever appearing in `.claude/agents/`, which is exactly why U02 reports them:
/// ArmadAI has no implicit equivalent, so a migration must materialise them.
const BUILTIN_AGENTS: &[&str] = &["general-purpose", "Explore", "Plan", "claude"];

/// Share of delegations below which a declared coordinator counts as bypassed.
const COORDINATOR_SHARE: f64 = 0.5;

fn observed(ctx: &AuditContext) -> Option<&UsageFacts> {
    ctx.usage.filter(|u| !u.is_empty())
}

/// U01 — a declared asset that never ran over the observed sessions.
pub fn u01_declared_never_used(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for agent in &ctx.config.agents {
        if usage.agents.contains_key(&agent.name) {
            continue;
        }
        findings.push(Finding {
            rule: "U01",
            severity: Severity::Warning,
            file: agent.source_path.clone(),
            related: vec![],
            message: format!(
                "agent '{}' is declared but was never invoked across {} observed session(s)",
                agent.name, usage.sessions
            ),
            suggestion: Some(
                "remove it, or exclude it from the generated pack (--propose tags it `unused`)"
                    .to_string(),
            ),
        });
    }
    findings
}

/// U02 — a sub-agent that ran without being declared anywhere.
pub fn u02_used_but_undeclared(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    let declared: Vec<&str> = ctx.config.agents.iter().map(|a| a.name.as_str()).collect();
    let mut findings = Vec::new();
    for (name, stats) in &usage.agents {
        if declared.contains(&name.as_str()) {
            continue;
        }
        let builtin = BUILTIN_AGENTS.contains(&name.as_str());
        findings.push(Finding {
            rule: "U02",
            severity: Severity::Info,
            file: ctx
                .config
                .instructions
                .as_ref()
                .map(|i| i.source_path.clone())
                .unwrap_or_else(|| PathBuf::from(".")),
            related: vec![],
            message: format!(
                "sub-agent '{}' ran {} time(s) but is declared nowhere{}",
                name,
                stats.invocations,
                if builtin {
                    " (it is built into Claude Code)"
                } else {
                    ""
                }
            ),
            suggestion: Some(
                "ArmadAI has no implicit equivalent — materialise it as an explicit agent \
                 so a migrated fleet keeps the same workers"
                    .to_string(),
            ),
        });
    }
    findings
}

/// U03 — the root instructions name a coordinator that delegations bypass.
pub fn u03_coordinator_bypassed(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    let Some(instructions) = ctx.config.instructions.as_ref() else {
        return Vec::new();
    };
    let total: u32 = usage.agents.values().map(|a| a.invocations).sum();
    if total == 0 {
        return Vec::new();
    }
    let haystack = instructions.content.to_lowercase();
    let mut findings = Vec::new();
    for agent in &ctx.config.agents {
        // Only agents the instructions actually single out as coordinating.
        let named = haystack.contains(&format!("@{}", agent.name.to_lowercase()))
            || haystack.contains(&format!("delegate to {}", agent.name.to_lowercase()));
        if !named {
            continue;
        }
        let own = usage
            .agents
            .get(&agent.name)
            .map(|a| a.invocations)
            .unwrap_or(0);
        let share = f64::from(own) / f64::from(total);
        if share >= COORDINATOR_SHARE {
            continue;
        }
        findings.push(Finding {
            rule: "U03",
            severity: Severity::Warning,
            file: instructions.source_path.clone(),
            related: vec![agent.source_path.clone()],
            message: format!(
                "'{}' is named as coordinator but received {}/{} delegation(s) ({:.0}%)",
                agent.name,
                own,
                total,
                share * 100.0
            ),
            suggestion: Some(
                "an explicit orchestrator cannot be bypassed like prose can — \
                 --propose emits the observed root, with this one kept as a comment"
                    .to_string(),
            ),
        });
    }
    findings
}

/// U04 — session coverage of a declared skill, reported without judgement.
pub fn u04_session_coverage(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    if usage.sessions == 0 {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for skill in &ctx.config.skills {
        let turns = usage.skills.get(&skill.name).copied().unwrap_or(0);
        if turns == 0 {
            continue; // U01's territory, not a coverage report.
        }
        findings.push(Finding {
            rule: "U04",
            severity: Severity::Info,
            file: skill.source_path.clone(),
            related: vec![],
            message: format!(
                "skill '{}' governed {} turn(s) across {} observed session(s)",
                skill.name, turns, usage.sessions
            ),
            suggestion: None,
        });
    }
    findings
}
```

Dans `rules/mod.rs`, déclarer le module et enregistrer les règles à la fin de `registry()` :

```rust
mod usage_rules;
```

```rust
        collisions::c05_inconsistent_tools,
        usage_rules::u01_declared_never_used,
        usage_rules::u02_used_but_undeclared,
        usage_rules::u03_coordinator_bypassed,
        usage_rules::u04_session_coverage,
    ]
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui usage_rules`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/rules/
git commit -m "feat(audit): add U01-U04 rules over observed usage

Every rule stays silent without observation: absence of measurement is not
evidence of absence of use. U02 is the migration-critical one — Claude
Code's built-in sub-agents run constantly and are declared nowhere, and
ArmadAI has no implicit equivalent."
```

---

### Task 8: Section « Usage observé » dans le rapport et branchement CLI

**Files:**
- Modify: `crates/armadai/src/audit/report.rs` (champ + rendu terminal/markdown)
- Modify: `crates/armadai/src/audit/mod.rs` (`run_audit` peuple le champ)
- Modify: `crates/armadai/src/cli/audit.rs` (appelle le scan)

**Interfaces:**
- Consumes: Tasks 5, 6, 7.
- Produces: `AuditReport.usage: Option<UsageFacts>`.

- [ ] **Step 1: Write the failing test**

Dans `report.rs`, module `tests` :

```rust
#[test]
fn markdown_reports_the_observed_window_and_top_agents() {
    let mut usage = crate::audit::usage::UsageFacts::default();
    usage.sessions = 2;
    usage.observe_timestamp("2026-07-01T00:00:00Z");
    usage.observe_timestamp("2026-08-13T00:00:00Z");
    usage.record_delegation(crate::audit::usage::facts::ROOT_AGENT, "qa", "m");

    let report = AuditReport {
        root: std::path::PathBuf::from("/p"),
        detected: vec!["claude".to_string()],
        agent_count: 1,
        skill_count: 0,
        findings: vec![],
        deep_raw: None,
        usage: Some(usage),
    };
    let md = report.to_markdown();
    assert!(md.contains("Observed usage"), "{md}");
    assert!(md.contains("2026-07-01T00:00:00Z"), "window start: {md}");
    assert!(md.contains("qa"), "top agents: {md}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui markdown_reports_the_observed_window`
Expected: FAIL — `AuditReport` n'a pas de champ `usage`.

- [ ] **Step 3: Write minimal implementation**

Ajouter le champ à `AuditReport` :

```rust
    /// Observed usage, when transcripts were found for this project.
    pub usage: Option<crate::audit::usage::UsageFacts>,
```

Ajouter une méthode de rendu partagée et l'appeler depuis `to_markdown` (avant la liste des findings) :

```rust
    /// Markdown block describing what was observed. Empty when nothing was.
    fn usage_markdown(&self) -> String {
        use std::fmt::Write;
        let Some(usage) = self.usage.as_ref().filter(|u| !u.is_empty()) else {
            return String::new();
        };
        let mut out = String::new();
        let _ = writeln!(out, "\n## Observed usage\n");
        let _ = writeln!(out, "- Sessions scanned: {}", usage.sessions);
        if let Some((from, to)) = &usage.window {
            let _ = writeln!(out, "- Window: {from} → {to}");
        }
        let mut agents: Vec<_> = usage.agents.iter().collect();
        agents.sort_by(|a, b| b.1.invocations.cmp(&a.1.invocations).then(a.0.cmp(b.0)));
        if !agents.is_empty() {
            let _ = writeln!(out, "\n### Agents by invocation\n");
            for (name, stats) in agents.iter().take(10) {
                let _ = writeln!(out, "- `{}` — {} invocation(s)", name, stats.invocations);
            }
        }
        let mut skills: Vec<_> = usage.skills.iter().collect();
        skills.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        if !skills.is_empty() {
            let _ = writeln!(out, "\n### Skills by attributed turns\n");
            for (name, turns) in skills.iter().take(10) {
                let _ = writeln!(out, "- `{name}` — {turns} turn(s)");
            }
        }
        out
    }
```

Insérer `md.push_str(&self.usage_markdown());` dans `to_markdown()` juste après la ligne de résumé, et le rendu terminal équivalent dans `print_terminal` en utilisant `crate::cli::style::{header, muted}` avec `anstream::println!` (jamais de couleur sur un chemin machine).

Dans `audit/mod.rs`, `run_audit` renseigne le champ :

```rust
        findings: rules::run_rules(&ctx),
        deep_raw: None,
        usage: usage.cloned(),
```

Dans `cli/audit.rs`, remplacer l'appel :

```rust
    let settings = AuditSettings::from_project(&root);
    let observed = crate::audit::usage::scan(&root);
    let usage = (!observed.is_empty()).then_some(observed);
    let mut audit = run_audit(&root, &settings, usage.as_ref());
```

Mettre à jour les autres constructions de `AuditReport` signalées par le compilateur avec `usage: None`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit`
Expected: PASS.

- [ ] **Step 5: Verify the 4 clippy modes**

```bash
for f in "tui" "tui,providers-api" "tui,web,storage" "tui,storage,e2e-fake"; do
  cargo clippy --all-targets --no-default-features --features "$f" -- -D warnings || break
done
```
Expected: aucune erreur.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/ crates/armadai/src/cli/audit.rs
git commit -m "feat(audit): report observed usage and wire the scan into the CLI

armadai audit now states what it measured — sessions, window, agents by
invocation, skills by attributed turns — before listing findings."
```

---

### Task 9: Test d'intégration boîte noire sur le binaire

**Pas** un cas gaveldrop. L'adaptateur gaveldrop est spécialisé pour les runs orchestrés : son `claims()` ne retient un cas que s'il porte un `pattern`, `build_command` produit toujours `run`, et toutes les assertions portent sur des **events** JSON. `armadai audit` n'émet pas d'events et n'est pas un run — l'y faire entrer demanderait d'élargir l'adaptateur à des commandes arbitraires, un chantier sans rapport avec ce lot. Le modèle correct est déjà dans le dépôt : `crates/armadai/tests/hook_stdout.rs`, qui lance le binaire compilé via `assert_cmd`.

**Files:**
- Create: `crates/armadai/tests/audit_usage.rs`

**Interfaces:**
- Consumes: Task 8 (surface CLI complète).
- Produces: la régression boîte noire du lot 1.

- [ ] **Step 1: Write the failing test**

```rust
//! Black-box regression for the observed-usage audit pass: the compiled binary
//! must discover a transcript directory, aggregate it, and report U01/U02.
//!
//! Spawns the real binary (like `hook_stdout.rs`) because the pass is wired in
//! `cli::audit::execute` — the discovery + scan + rules + rendering chain is
//! only exercised end to end through `main()`.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// A project declaring one agent that never ran, plus a transcript in which
    /// Claude Code's built-in `general-purpose` did all the work.
    fn scenario() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        let agents = project.join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("ghost.md"),
            "---\nname: ghost\ndescription: never invoked\n---\nBody",
        )
        .unwrap();

        // The transcript lives in a directory whose name does NOT follow the
        // slug rule, so this also covers the cwd-based fallback from Task 3.
        let projects = dir.path().join("claude-projects");
        let session_dir = projects.join("unexpected-name");
        std::fs::create_dir_all(&session_dir).unwrap();
        let cwd = project.to_string_lossy().to_string();
        let lines = [
            format!(
                r#"{{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","cwd":"{cwd}","message":{{"model":"claude-opus-5","content":[{{"type":"tool_use","id":"t1","name":"Agent","input":{{"subagent_type":"general-purpose","description":"do work"}}}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","timestamp":"2026-08-02T00:00:00Z","isSidechain":false,"uuid":"u2","cwd":"{cwd}","attributionSkill":"armadai","message":{{"model":"claude-opus-5","content":[{{"type":"tool_use","id":"t2","name":"Bash","input":{{}}}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
            ),
        ];
        std::fs::write(session_dir.join("s1.jsonl"), lines.join("\n") + "\n").unwrap();

        (dir, project, projects)
    }

    #[test]
    fn audit_reports_observed_usage_and_usage_findings() {
        let (_dir, project, projects) = scenario();

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("audit")
            .arg(&project)
            .env("ARMADAI_CLAUDE_PROJECTS_DIR", &projects)
            .env("NO_COLOR", "1");
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            stdout.contains("U01") && stdout.contains("ghost"),
            "a declared-but-unused agent must be flagged:\n{stdout}"
        );
        assert!(
            stdout.contains("U02") && stdout.contains("general-purpose"),
            "the built-in worker must be reported as undeclared:\n{stdout}"
        );
        assert!(
            stdout.contains("2026-08-01T00:00:00Z"),
            "the observed window must be stated:\n{stdout}"
        );
    }

    #[test]
    fn audit_without_any_transcript_still_succeeds_and_claims_nothing() {
        let (_dir, project, _projects) = scenario();
        let empty = _dir.path().join("no-transcripts-here");
        std::fs::create_dir_all(&empty).unwrap();

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.arg("audit")
            .arg(&project)
            .env("ARMADAI_CLAUDE_PROJECTS_DIR", &empty)
            .env("NO_COLOR", "1");
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success(), "audit must not fail: {stdout}");
        assert!(
            !stdout.contains("U01") && !stdout.contains("U02"),
            "with nothing observed, no usage claim may be made:\n{stdout}"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui --test audit_usage`
Expected: FAIL avant les Tasks 1–8 ; après elles, les deux tests doivent passer. Si `assert_cmd` n'est pas déclaré en `dev-dependencies` de la crate `armadai`, vérifier avec `grep -n "assert_cmd" crates/armadai/Cargo.toml` — `hook_stdout.rs` l'utilise déjà, donc il doit être présent.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui --test audit_usage`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/armadai/tests/audit_usage.rs
git commit -m "test: cover the observed-usage audit through the real binary

Black-box regression over a fixture transcript directory: U01 on a
declared-but-unused agent, U02 on Claude Code's built-in general-purpose,
and silence when no transcript exists. Deliberately not a gaveldrop case —
that adapter only claims orchestrated runs and asserts on events."
```

---

### Task 10: Documentation du lot 1

**Files:**
- Modify: `docs/wiki/` (la page couvrant `audit` — la localiser avec `grep -rln "armadai audit" docs/wiki/`)
- Modify: `CLAUDE.md` (ligne décrivant `audit/`)

- [ ] **Step 1: Locate the audit documentation**

```bash
grep -rln "armadai audit" docs/wiki/ README.md
```

- [ ] **Step 2: Document the usage pass**

Ajouter à la page d'audit une section décrivant : ce que le scan lit (`~/.claude/projects/`, repli par `cwd`), les règles `U01`–`U04`, le fait que l'absence de transcript n'est jamais une erreur, et la limite assumée (Ring/Blackboard non inférables — utile même au lot 1, pour cadrer les attentes).

Dans `CLAUDE.md`, la ligne `audit/` devient :

```markdown
- `audit/` — `armadai audit`: agentic-asset adoption/collision audit engine (collision matrix, frontmatter passthrough). `audit/reverse/` reads what a project declares; `audit/usage/` scans Claude Code transcripts for what it actually ran (rules `U01`–`U04`).
```

- [ ] **Step 3: Commit**

```bash
git add docs/ CLAUDE.md
git commit -m "docs(audit): document the observed-usage pass"
```

---

### Task 11: PR du lot 1

- [ ] **Step 1: Run the full local gate**

```bash
cargo fmt --all -- --check
for f in "tui" "tui,providers-api" "tui,web,storage" "tui,storage,e2e-fake"; do
  cargo clippy --all-targets --no-default-features --features "$f" -- -D warnings || break
done
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage,e2e-fake
```
Expected: tout vert. **Ne pas ouvrir la PR autrement.**

- [ ] **Step 2: Push and open the PR**

```bash
git push -u origin feat/audit-usage-observe
gh pr create --title "feat(audit): measure observed usage of native orchestrators" --body "$(cat <<'EOF'
## What

`armadai audit` only measured declared assets. This adds `audit/usage/`, the
runtime-direction mirror of `audit/reverse/`: it discovers a project's Claude
Code transcripts, stream-scans them, and aggregates deterministic `UsageFacts`.

Four new rules run over it: `U01` (declared, never invoked), `U02` (used but
declared nowhere), `U03` (declared coordinator bypassed), `U04` (session
coverage). Every rule is silent without observation.

## Why U02 matters

Claude Code's built-in sub-agents (`general-purpose`, `Explore`, `Plan`) run
constantly and exist in no `.claude/agents/` file. ArmadAI has no implicit
equivalent, so a migration that ignores them loses the actual workers.

## Notes

- No new dependency, no new feature flag.
- Missing or malformed transcript data degrades a metric, never fails the audit.
- Ring and Blackboard are documented as not inferable from a tree-shaped Task
  transcript rather than guessed.

Spec: `docs/superpowers/specs/2026-08-13-audit-usage-observe-design.md`
Plan: `docs/superpowers/plans/2026-08-13-audit-usage-observe.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Independent review gate**

Une CI verte ne suffit pas : demander une revue de code indépendante avant merge, puis squash-merge.

---

# LOT 2 — Pack enrichi (PR 2)

### Task 12: `generate_proposal` accepte l'usage et retient les modèles observés

**Files:**
- Modify: `crates/armadai/src/audit/proposal.rs:131-167` (`render_agent`), `:448+` (`generate_proposal`)
- Modify: `crates/armadai/src/cli/audit.rs` (site d'appel)

**Interfaces:**
- Consumes: Task 4 (`UsageFacts::dominant_model`).
- Produces: `pub fn generate_proposal(root: &Path, config: &ImportedConfig, usage: Option<&UsageFacts>) -> anyhow::Result<ProposalSummary>`.

- [ ] **Step 1: Write the failing test**

Dans le module `tests` de `proposal.rs` :

```rust
#[test]
fn observed_model_wins_over_the_static_mapping() {
    let mut usage = crate::audit::usage::UsageFacts::default();
    usage.record_delegation(
        crate::audit::usage::facts::ROOT_AGENT,
        "qa",
        "claude-opus-5",
    );
    let mut a = crate::audit::rules::test_support::agent("qa", "Body");
    a.metadata.model = Some("sonnet".to_string());

    let md = render_agent_with_usage(&a, Some(&usage));
    assert!(
        md.contains("claude-opus-5"),
        "the observed model must win: {md}"
    );
}

#[test]
fn static_mapping_is_kept_when_the_agent_was_never_observed() {
    let usage = crate::audit::usage::UsageFacts::default();
    let mut a = crate::audit::rules::test_support::agent("ghost", "Body");
    a.metadata.model = Some("sonnet".to_string());

    let with = render_agent_with_usage(&a, Some(&usage));
    let without = render_agent(&a);
    assert_eq!(with, without, "unobserved agents keep today's behaviour");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui observed_model_wins`
Expected: FAIL — `render_agent_with_usage` n'existe pas.

- [ ] **Step 3: Write minimal implementation**

Renommer l'actuel corps de `render_agent` en `render_agent_with_usage(agent, usage)` et garder `render_agent` comme wrapper `render_agent_with_usage(agent, None)` — les appelants et tests existants restent valides. Dans le corps, à la place de la ligne `- model:` actuelle :

```rust
    // The observed model beats the static native→tier mapping: it is what the
    // agent actually ran on, not what its file claimed.
    let observed = usage.and_then(|u| u.dominant_model(&agent.name));
    match observed {
        Some(model) => {
            let _ = writeln!(md, "- model: {model}");
        }
        None => { /* existing native_model_to_tier branch, unchanged */ }
    }
```

Propager `usage: Option<&UsageFacts>` dans la signature de `generate_proposal` et jusqu'à l'appel de `render_agent_with_usage`. Dans `cli/audit.rs`, l'appel devient `generate_proposal(&root, &config, usage.as_ref())?` (`usage` existe déjà depuis la Task 8).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui proposal`
Expected: PASS, tous les tests existants inclus.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/ crates/armadai/src/cli/audit.rs
git commit -m "feat(audit): use the observed model in the generated pack

Option-typed: with no observation the static native-to-tier mapping stays
exactly as it was."
```

---

### Task 13: Tags de volumétrie par tercile

**Files:**
- Create: `crates/armadai/src/audit/usage/tiers.rs`
- Modify: `crates/armadai/src/audit/usage/mod.rs`, `crates/armadai/src/audit/proposal.rs`

**Interfaces:**
- Consumes: Task 4.
- Produces: `pub fn volume_tag(value: u32, all_observed: &[u32]) -> &'static str`

Règle, sans constante arbitraire :
- `value == 0` → `"unused"` ;
- moins de 3 valeurs observées non nulles → `"hot"` (les terciles n'ont pas de sens) ;
- sinon tercile : `>= p66` → `"hot"`, `>= p33` → `"warm"`, sinon `"cold"`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_always_unused() {
        assert_eq!(volume_tag(0, &[5, 10, 20]), "unused");
        assert_eq!(volume_tag(0, &[]), "unused");
    }

    #[test]
    fn fewer_than_three_observed_values_skip_terciles() {
        assert_eq!(volume_tag(1, &[1]), "hot");
        assert_eq!(volume_tag(1, &[1, 99]), "hot");
    }

    #[test]
    fn terciles_split_hot_warm_cold() {
        let all = [1, 2, 3, 10, 20, 300];
        assert_eq!(volume_tag(300, &all), "hot");
        assert_eq!(volume_tag(1, &all), "cold");
        assert_eq!(volume_tag(10, &all), "warm");
    }

    #[test]
    fn a_flat_distribution_does_not_panic_and_stays_stable() {
        let all = [7, 7, 7, 7];
        assert_eq!(volume_tag(7, &all), "hot", "all equal -> all top tercile");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui audit::usage::tiers`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
//! Volume tags derived from quantiles rather than absolute thresholds, so the
//! same rule is fair on a 5-session project and on a 500-session one.

/// `unused` / `cold` / `warm` / `hot` for `value` within `all_observed`.
pub fn volume_tag(value: u32, all_observed: &[u32]) -> &'static str {
    if value == 0 {
        return "unused";
    }
    let mut sorted: Vec<u32> = all_observed.iter().copied().filter(|v| *v > 0).collect();
    if sorted.len() < 3 {
        return "hot";
    }
    sorted.sort_unstable();
    let p33 = sorted[sorted.len() / 3];
    let p66 = sorted[sorted.len() * 2 / 3];
    if value >= p66 {
        "hot"
    } else if value >= p33 {
        "warm"
    } else {
        "cold"
    }
}
```

Dans `proposal.rs`, la ligne de tags de `render_agent_with_usage` :

```rust
    let volume = usage.map(|u| {
        let all: Vec<u32> = u.agents.values().map(|a| a.invocations).collect();
        let own = u.agents.get(&agent.name).map(|a| a.invocations).unwrap_or(0);
        crate::audit::usage::tiers::volume_tag(own, &all)
    });
    match volume {
        Some(tag) => {
            let _ = writeln!(md, "- tags: [imported, {tag}]");
        }
        None => {
            let _ = writeln!(md, "- tags: [imported]");
        }
    }
```

Déclarer `pub mod tiers;` dans `usage/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/
git commit -m "feat(audit): tag generated agents by observed volume tercile

Quantiles, not absolute thresholds: no magic constant to justify, and the
rule stays fair regardless of project size."
```

---

### Task 14: Ordre par volumétrie, stubs `U02` et `USAGE.md`

**Files:**
- Modify: `crates/armadai/src/audit/proposal.rs` (`generate_proposal`)

**Interfaces:**
- Consumes: Tasks 12, 13.
- Produces: `.armadai-proposal/USAGE.md` et les stubs des agents observés-non-déclarés.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn proposal_writes_usage_md_and_a_stub_for_undeclared_agents() {
    let dir = tempfile::tempdir().unwrap();
    let mut usage = crate::audit::usage::UsageFacts::default();
    usage.sessions = 4;
    usage.observe_timestamp("2026-08-01T00:00:00Z");
    usage.record_delegation(
        crate::audit::usage::facts::ROOT_AGENT,
        "general-purpose",
        "claude-opus-5",
    );
    let config = crate::audit::rules::test_support::config_with(vec![]);

    generate_proposal(dir.path(), &config, Some(&usage)).unwrap();

    let usage_md =
        std::fs::read_to_string(dir.path().join(".armadai-proposal/USAGE.md")).unwrap();
    assert!(usage_md.contains("4"), "session count: {usage_md}");
    assert!(usage_md.contains("2026-08-01T00:00:00Z"), "window: {usage_md}");
    assert!(
        usage_md.contains("Blackboard") || usage_md.contains("blackboard"),
        "the assumed limit must be stated: {usage_md}"
    );
    assert!(
        dir.path()
            .join(".armadai-proposal/agents/general-purpose.md")
            .exists(),
        "an observed-but-undeclared agent must get a stub"
    );
}

#[test]
fn proposal_without_usage_writes_no_usage_md() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::audit::rules::test_support::config_with(vec![
        crate::audit::rules::test_support::agent("qa", "Body"),
    ]);
    generate_proposal(dir.path(), &config, None).unwrap();
    assert!(!dir.path().join(".armadai-proposal/USAGE.md").exists());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui proposal_writes_usage_md`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

Dans `generate_proposal`, après l'écriture des agents déclarés :

```rust
    // Agents that ran but are declared nowhere (rule U02). ArmadAI has no
    // implicit equivalent to Claude Code's built-ins, so the pack must
    // materialise them or the migrated fleet loses its actual workers.
    if let Some(u) = usage {
        let declared: Vec<&str> = config.agents.iter().map(|a| a.name.as_str()).collect();
        for (name, stats) in &u.agents {
            if declared.contains(&name.as_str()) {
                continue;
            }
            let slug = slugify(name);
            let file = agents_dir.join(format!("{slug}.md"));
            std::fs::write(&file, render_stub(name, stats, u))?;
            slugs.push(slug);
        }
    }
```

`slugify` et `slugs` réutilisent les helpers déjà présents dans la fonction (les nommer d'après ce que le code existant utilise). `render_stub` :

```rust
/// Minimal agent file for a worker observed in transcripts but declared in no
/// native file. Its system prompt cannot be recovered — only its identity and
/// how much work it did.
fn render_stub(name: &str, stats: &crate::audit::usage::AgentUsage, usage: &UsageFacts) -> String {
    use std::fmt::Write;
    let mut md = String::new();
    let _ = writeln!(md, "# {name}\n");
    let _ = writeln!(
        md,
        "> Observed worker: {} invocation(s) across {} session(s). Declared in no native \
         config file, so its prompt could not be recovered — write it.\n",
        stats.invocations, usage.sessions
    );
    let _ = writeln!(md, "## Metadata");
    let _ = writeln!(md, "- provider: claude");
    if let Some(model) = usage.dominant_model(name) {
        let _ = writeln!(md, "- model: {model}");
    }
    let _ = writeln!(md, "- description: Observed worker, prompt to be written");
    let _ = writeln!(md, "- tags: [imported, observed-only]");
    let _ = writeln!(md, "\n## System Prompt\n");
    let _ = writeln!(
        md,
        "TODO: this agent was reconstructed from observed usage only. Describe its role."
    );
    md
}
```

Le `TODO:` ici est du **contenu généré destiné à l'utilisateur** (un marqueur à remplir dans le fichier produit), pas un placeholder de plan.

Trier les agents par volumétrie avant l'écriture de `pack.yaml` (quand `usage` est présent), et écrire `USAGE.md` :

```rust
    if let Some(u) = usage {
        let mut doc = String::new();
        let _ = writeln!(doc, "# Observed usage behind this proposal\n");
        let _ = writeln!(doc, "- Sessions scanned: {}", u.sessions);
        if let Some((from, to)) = &u.window {
            let _ = writeln!(doc, "- Window observed: {from} → {to}");
        }
        let _ = writeln!(doc, "- Largest parallel fan-out: {}", u.max_fanout);
        let _ = writeln!(doc, "- Delegation depth: {}", u.depth());
        let _ = writeln!(
            doc,
            "\n## Decisions usage made\n\n\
             - `- model:` is the model each agent was actually observed running on.\n\
             - `- tags:` carry a volume tercile (`hot`/`warm`/`cold`), `unused` at zero.\n\
             - Agents tagged `observed-only` ran in transcripts but were declared in no \
             native file; their prompt could not be recovered.\n"
        );
        let _ = writeln!(
            doc,
            "## Assumed limits\n\n\
             Ring and Blackboard patterns are NOT inferable from a Claude Code transcript: \
             the Task model is a tree-shaped call/return, with no cycle and no shared \
             blackboard to observe. They are never proposed.\n"
        );
        std::fs::write(out_dir.join("USAGE.md"), doc)?;
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui proposal`
Expected: PASS.

- [ ] **Step 5: Commit + PR du lot 2**

```bash
cargo fmt --all
git add crates/armadai/src/audit/
git commit -m "feat(audit): order the pack by volume and document usage decisions

Stubs for observed-but-undeclared workers, plus a USAGE.md stating the
window, the decisions usage made, and the limits it refuses to guess."
```

Puis relancer la porte locale complète (4 modes clippy + 2 modes de test) et ouvrir la PR du lot 2 sur le même modèle que la Task 11.

---

# LOT 3 — Topologie et routes (PR 3)

### Task 15: Déduction du pattern d'orchestration

**Files:**
- Create: `crates/armadai/src/audit/usage/topology.rs`
- Modify: `crates/armadai/src/audit/usage/mod.rs`

**Interfaces:**
- Consumes: Task 4 (`UsageFacts::{depth, edges, agents, max_fanout}`).
- Produces:
  - `pub struct DeducedTopology { pub pattern: &'static str, pub coordinator: Option<String>, pub teams: Vec<(Option<String>, Vec<String>)>, pub max_concurrency: Option<u32> }`
  - `pub fn deduce(usage: &UsageFacts) -> Option<DeducedTopology>`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::usage::facts::ROOT_AGENT;

    #[test]
    fn no_delegation_deduces_direct() {
        let mut u = UsageFacts::default();
        u.record_tool("Bash");
        let t = deduce(&u).expect("some topology");
        assert_eq!(t.pattern, "direct");
        assert!(t.teams.is_empty());
    }

    #[test]
    fn flat_tree_deduces_hierarchical_without_teams() {
        let mut u = UsageFacts::default();
        u.root_agent = ROOT_AGENT.to_string();
        u.record_delegation(ROOT_AGENT, "qa", "m");
        u.record_delegation(ROOT_AGENT, "core", "m");
        let t = deduce(&u).unwrap();
        assert_eq!(t.pattern, "hierarchical");
        assert_eq!(t.coordinator.as_deref(), Some(ROOT_AGENT));
        assert!(t.teams.is_empty(), "flat means no teams: {:?}", t.teams);
    }

    #[test]
    fn nested_tree_deduces_teams_with_a_lead() {
        let mut u = UsageFacts::default();
        u.root_agent = ROOT_AGENT.to_string();
        u.record_delegation(ROOT_AGENT, "lead", "m");
        u.record_delegation("lead", "qa", "m");
        u.record_delegation("lead", "core", "m");
        let t = deduce(&u).unwrap();
        assert_eq!(t.pattern, "hierarchical");
        assert_eq!(t.teams.len(), 1, "{:?}", t.teams);
        assert_eq!(t.teams[0].0.as_deref(), Some("lead"));
        assert_eq!(t.teams[0].1, vec!["core".to_string(), "qa".to_string()]);
    }

    #[test]
    fn max_concurrency_comes_from_the_observed_fanout() {
        let mut u = UsageFacts::default();
        u.record_delegation(ROOT_AGENT, "qa", "m");
        u.max_fanout = 3;
        assert_eq!(deduce(&u).unwrap().max_concurrency, Some(3));
    }

    #[test]
    fn a_single_observed_fanout_writes_no_concurrency_key() {
        let mut u = UsageFacts::default();
        u.record_delegation(ROOT_AGENT, "qa", "m");
        u.max_fanout = 1;
        assert_eq!(
            deduce(&u).unwrap().max_concurrency,
            None,
            "no parallelism observed means no key at all"
        );
    }

    #[test]
    fn empty_facts_deduce_nothing() {
        assert!(deduce(&UsageFacts::default()).is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui audit::usage::topology`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
//! Project the observed delegation tree onto ArmadAI's orchestration schema.
//!
//! Ring and Blackboard are deliberately absent: Claude Code's Task model is a
//! tree-shaped call/return, so neither a cycle nor a shared blackboard is ever
//! observable. Guessing them from a tree's shape would be invention dressed up
//! as measurement.

use super::facts::{ROOT_AGENT, UsageFacts};

/// What the observed tree implies, ready to render as `armadai.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeducedTopology {
    pub pattern: &'static str,
    pub coordinator: Option<String>,
    /// (lead, agents) pairs — empty when the tree is flat.
    pub teams: Vec<(Option<String>, Vec<String>)>,
    /// Only set when parallelism was actually observed (fan-out > 1).
    pub max_concurrency: Option<u32>,
}

pub fn deduce(usage: &UsageFacts) -> Option<DeducedTopology> {
    if usage.is_empty() {
        return None;
    }
    let root = if usage.root_agent.is_empty() {
        ROOT_AGENT
    } else {
        usage.root_agent.as_str()
    };
    let max_concurrency = (usage.max_fanout > 1).then_some(usage.max_fanout);

    if usage.agents.is_empty() {
        return Some(DeducedTopology {
            pattern: "direct",
            coordinator: None,
            teams: Vec::new(),
            max_concurrency,
        });
    }

    // Every non-root node that itself delegates is a team lead.
    let mut teams: Vec<(Option<String>, Vec<String>)> = usage
        .edges
        .iter()
        .filter(|(parent, _)| parent.as_str() != root)
        .map(|(lead, children)| {
            (
                Some(lead.clone()),
                children.iter().cloned().collect::<Vec<_>>(),
            )
        })
        .collect();
    teams.sort();

    Some(DeducedTopology {
        pattern: "hierarchical",
        coordinator: Some(root.to_string()),
        teams,
        max_concurrency,
    })
}
```

Déclarer `pub mod topology;` dans `usage/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit::usage::topology`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/usage/
git commit -m "feat(audit): deduce the orchestration topology from the observed tree

Direct when nothing delegates, hierarchical otherwise, with teams whenever a
non-root node delegates in turn. Ring and Blackboard are never proposed —
a tree-shaped transcript cannot evidence them."
```

---

### Task 16: Rendu de l'`armadai.yaml` avec l'arbitrage en commentaire

**Files:**
- Create: `crates/armadai/src/audit/usage/render_yaml.rs`
- Modify: `crates/armadai/src/audit/usage/mod.rs`, `crates/armadai/src/audit/proposal.rs`

**Interfaces:**
- Consumes: Task 15 (`DeducedTopology`).
- Produces: `pub fn render_orchestration(topo: &DeducedTopology, declared_coordinator: Option<&str>, declared_share: Option<(u32, u32)>) -> String`

Écrit par `writeln!`, comme `pack.yaml` : `serde_yaml` effacerait les commentaires qui portent tout l'arbitrage.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> DeducedTopology {
        DeducedTopology {
            pattern: "hierarchical",
            coordinator: Some("claude".to_string()),
            teams: vec![],
            max_concurrency: Some(3),
        }
    }

    #[test]
    fn renders_the_observed_coordinator_as_the_active_key() {
        let y = render_orchestration(&flat(), None, None);
        assert!(y.contains("pattern: hierarchical"), "{y}");
        assert!(y.contains("coordinator: claude"), "{y}");
        assert!(y.contains("max_concurrency: 3"), "{y}");
    }

    #[test]
    fn a_diverging_declared_coordinator_is_kept_as_a_commented_alternative() {
        let y = render_orchestration(&flat(), Some("dev-lead"), Some((9, 541)));
        assert!(y.contains("coordinator: claude"), "observed is active: {y}");
        assert!(
            y.contains("# coordinator: dev-lead"),
            "declared must survive as a comment: {y}"
        );
        assert!(y.contains("9"), "both counts must be visible: {y}");
        assert!(y.contains("541"), "both counts must be visible: {y}");
    }

    #[test]
    fn an_agreeing_declared_coordinator_adds_no_comment() {
        let y = render_orchestration(&flat(), Some("claude"), Some((500, 541)));
        assert!(
            !y.contains("# coordinator:"),
            "nothing to arbitrate, no comment: {y}"
        );
    }

    #[test]
    fn default_valued_keys_are_omitted() {
        let topo = DeducedTopology {
            pattern: "direct",
            coordinator: None,
            teams: vec![],
            max_concurrency: None,
        };
        let y = render_orchestration(&topo, None, None);
        assert!(!y.contains("max_concurrency"), "no observation, no key: {y}");
        assert!(!y.contains("max_depth"), "defaults are never frozen: {y}");
        assert!(!y.contains("timeout"), "defaults are never frozen: {y}");
    }

    #[test]
    fn teams_render_with_their_lead() {
        let topo = DeducedTopology {
            pattern: "hierarchical",
            coordinator: Some("claude".to_string()),
            teams: vec![(
                Some("dev-lead".to_string()),
                vec!["qa".to_string(), "core".to_string()],
            )],
            max_concurrency: None,
        };
        let y = render_orchestration(&topo, None, None);
        assert!(y.contains("teams:"), "{y}");
        assert!(y.contains("- lead: dev-lead"), "{y}");
        assert!(y.contains("qa"), "{y}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui audit::usage::render_yaml`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
//! Render a deduced topology as the `orchestration:` block of armadai.yaml.
//!
//! Hand-written with `writeln!`, exactly like `pack.yaml`: a serde round-trip
//! would erase the comments, and the comments are what carry the arbitration
//! between what the project declares and what it actually does.

use std::fmt::Write;

use super::topology::DeducedTopology;

pub fn render_orchestration(
    topo: &DeducedTopology,
    declared_coordinator: Option<&str>,
    declared_share: Option<(u32, u32)>,
) -> String {
    let mut y = String::new();
    let _ = writeln!(y, "orchestration:");
    let _ = writeln!(y, "  enabled: true");
    let _ = writeln!(y, "  pattern: {}", topo.pattern);

    if let Some(observed) = &topo.coordinator {
        let _ = writeln!(y, "  coordinator: {observed}        # observed");
        // Only an actual divergence is worth arbitrating.
        if let Some(declared) = declared_coordinator.filter(|d| *d != observed.as_str()) {
            let counts = match declared_share {
                Some((own, total)) => format!(" — {own}/{total} delegation(s)"),
                None => String::new(),
            };
            let _ = writeln!(y, "  # coordinator: {declared}    # declared{counts}");
            let _ = writeln!(
                y,
                "  #   ↑ uncomment to follow the declared intent (finding U03)"
            );
        }
    }
    if let Some(c) = topo.max_concurrency {
        let _ = writeln!(y, "  max_concurrency: {c}         # max observed fan-out");
    }
    if !topo.teams.is_empty() {
        let _ = writeln!(y, "  teams:");
        for (lead, agents) in &topo.teams {
            match lead {
                Some(l) => {
                    let _ = writeln!(y, "    - lead: {l}");
                }
                None => {
                    let _ = writeln!(y, "    -");
                }
            }
            let _ = writeln!(y, "      agents: [{}]", agents.join(", "));
        }
    }
    y
}
```

Dans `generate_proposal`, quand `usage` est présent et que `deduce` renvoie une topologie, écrire `out_dir.join("armadai.yaml")` avec ce bloc. Le coordinateur déclaré se lit avec la même détection que `u03_coordinator_bypassed` — extraire ce test dans une fonction partagée `pub(crate) fn declared_coordinator(config: &ImportedConfig) -> Option<String>` placée dans `rules/usage_rules.rs` et l'appeler des deux côtés, plutôt que de dupliquer la règle.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/armadai/src/audit/
git commit -m "feat(audit): emit a deduced orchestration block in the proposal

The observed coordinator is the active key; a diverging declared one is kept
as a commented alternative with both counts, so the arbitration stays with
the human who reads it."
```

---

### Task 17: Nommage des routes sous `--deep`

**Files:**
- Create: `crates/armadai/src/audit/route_namer.md`
- Create: `crates/armadai/src/audit/usage/routes.rs`
- Modify: `crates/armadai/src/audit/usage/mod.rs`, `crates/armadai/src/audit/proposal.rs`, `crates/armadai/src/cli/audit.rs`

**Interfaces:**
- Consumes: Task 4, Task 16, et `deep::sanitize_excerpt` (rendre `pub(crate)` si nécessaire).
- Produces:
  - `pub fn build_payload(usage: &UsageFacts, samples: &BTreeMap<String, Vec<String>>, truncation: usize) -> String`
  - `pub fn parse_routes(response: &str) -> BTreeMap<String, Vec<String>>`
  - `pub const ROUTE_NAMER_PROMPT: &str = include_str!("../route_namer.md");`

Prérequis : `UsageFacts` doit porter un échantillon de descriptions. Ajouter à `AgentUsage` :

```rust
    /// Up to SAMPLE_MAX task descriptions seen for this agent, kept in
    /// encounter order — the raw material for route naming.
    pub samples: Vec<String>,
```

et dans `record_delegation`, un paramètre `description: &str` poussé tant que `samples.len() < SAMPLE_MAX` (`pub const SAMPLE_MAX: usize = 8;`). Mettre à jour les appelants et tests des Tasks 4, 5, 7, 12, 13, 14, 15 (le compilateur les liste).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_carries_agents_volumes_and_sanitized_samples() {
        let mut u = UsageFacts::default();
        u.record_delegation(
            crate::audit::usage::facts::ROOT_AGENT,
            "qa",
            "m",
            "run the clippy gate",
        );
        let payload = build_payload(&u, 2000);
        assert!(payload.contains("qa"), "{payload}");
        assert!(payload.contains("clippy"), "{payload}");
        assert!(payload.contains("invocations"), "{payload}");
    }

    #[test]
    fn payload_redacts_secrets_from_samples() {
        let mut u = UsageFacts::default();
        u.record_delegation(
            crate::audit::usage::facts::ROOT_AGENT,
            "qa",
            "m",
            "use sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH to call the API",
        );
        let payload = build_payload(&u, 2000);
        assert!(
            !payload.contains("sk-ant-api03-AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH"),
            "a secret must never leave the process: {payload}"
        );
    }

    #[test]
    fn parses_a_well_formed_route_response() {
        let routes = parse_routes(
            r#"{"routes":{"tests":["qa"],"engine":["core","cli"]}}"#,
        );
        assert_eq!(routes["tests"], vec!["qa".to_string()]);
        assert_eq!(routes["engine"], vec!["core".to_string(), "cli".to_string()]);
    }

    #[test]
    fn an_unparsable_response_yields_no_routes_rather_than_an_error() {
        assert!(parse_routes("I think maybe tests?").is_empty());
        assert!(parse_routes("").is_empty());
    }

    #[test]
    fn routes_referencing_unknown_agents_are_dropped() {
        let routes = parse_routes(r#"{"routes":{"tests":["qa"],"ghost":[]}}"#);
        assert!(
            !routes.contains_key("ghost"),
            "an empty route is useless: {routes:?}"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui audit::usage::routes`
Expected: FAIL.

- [ ] **Step 3: Write the prompt file**

`crates/armadai/src/audit/route_namer.md` :

```markdown
You name agent routes for an ArmadAI migration. You receive a JSON payload
listing each observed sub-agent, how many times it ran, and a sample of the
task descriptions it was actually given.

Group the agents into named routes that a developer would plausibly select on
the command line (`armadai run --route <name>`). A route is a short, lowercase,
kebab-case noun phrase describing the KIND OF WORK, never an agent's name.

Rules:
- Only use agent names present in the payload. Never invent one.
- Every route must contain at least one agent.
- An agent may appear in several routes when its samples genuinely span them.
- Prefer 2 to 6 routes. Fewer is better than arbitrary ones.
- If the samples are too thin to group honestly, return `{"routes":{}}`.

Respond with JSON only, no prose:

{"routes":{"route-name":["agent-a","agent-b"]}}
```

- [ ] **Step 4: Write minimal implementation**

```rust
//! `--deep` route naming: the only non-deterministic step in the usage pass.

use std::collections::BTreeMap;

use super::facts::UsageFacts;

pub const ROUTE_NAMER_PROMPT: &str = include_str!("../route_namer.md");

/// JSON payload for the route namer. Samples go through the audit's own
/// secret redaction first: task descriptions are free text written mid-session,
/// which is exactly where a path or a credential can end up.
pub fn build_payload(usage: &UsageFacts, truncation: usize) -> String {
    let agents: Vec<serde_json::Value> = usage
        .agents
        .iter()
        .map(|(name, stats)| {
            serde_json::json!({
                "agent": name,
                "invocations": stats.invocations,
                "samples": stats
                    .samples
                    .iter()
                    .map(|s| crate::audit::deep::sanitize_excerpt(s, truncation))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "agents": agents }).to_string()
}

/// Parse the namer's response. Anything unexpected yields no routes: the
/// proposal must degrade, never fail.
pub fn parse_routes(response: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(start) = response.find('{') else {
        return out;
    };
    let Some(end) = response.rfind('}') else {
        return out;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&response[start..=end]) else {
        return out;
    };
    let Some(routes) = v.get("routes").and_then(|r| r.as_object()) else {
        return out;
    };
    for (name, agents) in routes {
        let list: Vec<String> = agents
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a.as_str().map(str::to_string))
            .collect();
        if list.is_empty() {
            continue; // an empty route selects nothing — drop it
        }
        out.insert(name.clone(), list);
    }
    out
}
```

Si `sanitize_excerpt` est privé dans `deep.rs`, le passer `pub(crate)` — sa réutilisation est explicitement voulue par le spec.

Dans `cli/audit.rs`, quand `propose && deep` et qu'un CLI est disponible, appeler le namer via `call_deep_auditor` avec `ROUTE_NAMER_PROMPT` en tête du payload, puis passer les routes obtenues à `generate_proposal`. Étendre sa signature d'un dernier paramètre `routes: &BTreeMap<String, Vec<String>>` (vide par défaut) et les rendre sous `orchestration:` :

```rust
    if !routes.is_empty() {
        let _ = writeln!(y, "  routes:");
        for (name, agents) in routes {
            let _ = writeln!(y, "    {name}: [{}]", agents.join(", "));
        }
    }
```

Sans `--deep`, sans CLI, ou sur réponse invalide : aucun bloc `routes`, un avertissement via `crate::cli::style::warn`, et le reste de la proposition intact.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui audit`
Expected: PASS.

- [ ] **Step 6: Commit + PR du lot 3**

```bash
cargo fmt --all
git add crates/armadai/src/audit/
git commit -m "feat(audit): name orchestration routes under --deep

The only speculative step in the usage pass, isolated behind the flag that
already carries the LLM guardrails. Samples are redacted before they leave
the process; an unparsable answer drops the routes instead of failing."
```

Relancer la porte locale complète, mettre à jour la documentation du lot 3 (routes + topologie dans `docs/wiki/`), puis ouvrir la PR.

---

## Self-Review

**Couverture du spec :**

| Exigence du spec | Tâche |
|---|---|
| `usage/discovery.rs`, slug + repli `cwd` | 3 |
| `usage/scan.rs` streaming | 5 |
| `usage/facts.rs` — `UsageFacts` | 4 |
| `Block::Other` → `Block::Tool { name }` | 1 |
| Un seul parse JSON par ligne | 2 |
| `AuditContext { usage }` | 6 |
| Findings `U01`–`U04` | 7 |
| Section « Usage observé » | 8 |
| Métriques distinctes skills (tours) / agents (invocations) | 4, 7 |
| `generate_proposal(…, Option<&UsageFacts>)` | 12 |
| Modèles observés | 12 |
| Tags par tercile, cas < 3 assets | 13 |
| Ordre par volumétrie, stubs `U02`, `USAGE.md` | 14 |
| Déduction du pattern, `teams`, `max_concurrency` | 15 |
| `armadai.yaml` + arbitrage en commentaire, clés par défaut omises | 16 |
| Routes sous `--deep`, `route_namer.md`, `sanitize_excerpt` | 17 |
| Ring/Blackboard non proposés, limite écrite | 14 (`USAGE.md`), 15 (doc du module) |
| E2E via `ARMADAI_CLAUDE_PROJECTS_DIR` | 9 — **en test d'intégration `assert_cmd`, pas en cas gaveldrop** (voir la Task 9 : l'adaptateur gaveldrop ne claim que les runs orchestrés et n'assert que sur des events) |
| Tolérance aux champs absents | 4, 5, 7 |
| 4 modes clippy | 8, 11 |

Non couvert, écart assumé et documenté en tête de plan : durées et échecs dans `UsageFacts`.

**Cohérence des types :** `record_delegation` gagne son 4ᵉ paramètre `description` à la Task 17, ce qui casse les appels des Tasks 4, 5, 7, 12, 13, 14, 15. C'est explicite dans la Task 17. L'alternative — l'introduire dès la Task 4 — ajouterait un paramètre inutilisé pendant six tâches, ce que le premier relecteur rejetterait à juste titre.

`ROOT_AGENT` est défini en Task 4 et utilisé partout ensuite. `UsageFacts` est réexporté depuis `usage/mod.rs` (Task 5) et référencé comme `crate::audit::usage::UsageFacts` ; `facts::ROOT_AGENT` et `tiers::volume_tag` gardent leur chemin de module complet.
