# Plugin Claude Code → Workroom, P1 (transcript adapter) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ajouter à ArmadAI une commande `armadai watch` qui reconstruit des `RunEvent` depuis le transcript JSONL d'une session Claude Code et les affiche dans le Workroom (live + replay), plus un plugin Claude Code minimal (hook `SessionStart`) qui enregistre les sessions dans un index.

**Architecture:** L'adaptateur vit **côté bin** (`crates/armadai/src/claude_adapter/`) et produit des `armadai_core::events::RunEvent` — le cœur `armadai-core` reste générique, aucune connaissance de Claude Code. Le Workroom est réutilisé tel quel via `shell::run_view::run_orchestration_tui`. Un hook `SessionStart` (dans un plugin versionné) invoque `armadai __claude-register-session` qui append `{session_id, transcript_path, cwd, started_at}` dans un index JSONL connu.

**Tech Stack:** Rust edition 2024 (workspace), serde/serde_json, tokio, clap, ratatui (Workroom via feature `tui`), le plugin Claude Code (JSON statique + le binaire `armadai`).

## Global Constraints

- **`armadai-core` NE CHANGE PAS** : tout le code P1 est dans le bin `crates/armadai/`. `RunEvent` n'est PAS étendu (mapping au niveau agent seulement).
- **Branche** : master-only, une PR par tâche, squash-merge, revue indé + CI verte (6 checks), confirmer 6/6 `pass` avant merge. Workspace virtuel → `cargo test`/`clippy` à la racine sont workspace-wide.
- **Gate CI** (racine) : `cargo fmt --all -- --check` ; clippy 3 combos `--all-targets -D warnings` (`tui` / `tui,providers-api` / `tui,web,storage`) ; `cargo test` 3 modes (`tui` / `tui,storage` / `tui,providers-api`). Le code `watch`/Workroom est gated `tui`.
- **`rust-analyzer` non fiable** (ABI stale) → vérifier au compilateur.
- **Parser transcript défensif** : jamais paniquer sur une ligne inconnue/malformée ; types d'entrées non pertinents ignorés ; jamais charger le fichier entier (lecture ligne à ligne).
- **Hook** : le binaire invoqué par le hook **exit 0 toujours**, **rien sur stdout** (interprété par Claude Code), échec d'écriture → warn silencieux.
- Code/commentaires/commits en anglais. Conventional Commits, scope `plugin`. Terminer chaque commit par `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- NE PAS `git add -A` (untracked pré-existant) — stager explicitement.

## Références (types existants, à consommer verbatim)

`armadai_core::events::RunEvent` (variantes + champs EXACTS) :
```rust
RunStart { run_id: String, v: u32, agents: Vec<String>, prov: String, model: String, in_chars: usize }
AgentStart { agent: String, prov: String, model: String }
AgentEnd { agent: String, tin: u32, tout: u32, cost: f64, content: String }
Delegate { from: String, to: String }
Result { content: String, tin: u32, tout: u32, cost: f64, agents: usize }
Error { code: String, msg: String }
```
`armadai_core::events::EventSink { fn emit(&self, ev: &RunEvent); }` (+ `make_sink(json: bool) -> Arc<dyn EventSink>`).
Harnais Workroom : `crate::shell::run_view::run_orchestration_tui(run: impl FnOnce(Arc<dyn EventSink>) -> F, config_yaml: Option<String>, explicit_pattern: Option<OrchestrationPattern>) -> anyhow::Result<(Option<String>, Option<String>)>` où `F: Future<Output = anyhow::Result<()>> + Send + 'static`.
Pattern CLI : `crates/armadai/src/cli/mod.rs` — enum `Command` (variante + handler `=> module::execute(...).await`).

## File Structure

```
crates/armadai/
  assets/claude-plugin/
    .claude-plugin/plugin.json         # NEW — manifeste plugin
    hooks/hooks.json                   # NEW — hook SessionStart -> armadai __claude-register-session
    README.md                          # NEW — install/usage
  src/
    main.rs                            # MODIFY — `mod claude_adapter;`
    claude_adapter/
      mod.rs                           # NEW — re-exports + `drive_session`
      session_index.rs                 # NEW — SessionRef, read/resolve/append de l'index
      transcript.rs                    # NEW — lecteur streaming + parse défensif -> RelevantEntry
      mapper.rs                        # NEW — RelevantEntry -> RunEvent (machine à états)
    cli/
      mod.rs                           # MODIFY — commandes `Watch` + `__claude-register-session`
      watch.rs                         # NEW — `armadai watch` (Workroom) + register handler
```

---

## Task 1: Index de sessions (`session_index.rs`)

**Files:**
- Create: `crates/armadai/src/claude_adapter/session_index.rs`
- Create: `crates/armadai/src/claude_adapter/mod.rs` (déclare `pub mod session_index;`)
- Modify: `crates/armadai/src/main.rs` (ajouter `mod claude_adapter;`)

**Interfaces:**
- Produces: `pub struct SessionRef { pub session_id: String, pub transcript_path: PathBuf, pub cwd: String, pub started_at: String }`
- Produces: `pub fn index_path() -> PathBuf` (env `ARMADAI_SESSION_INDEX` sinon `<config_dir>/claude-sessions.jsonl`)
- Produces: `pub fn append(entry: &SessionRef) -> anyhow::Result<()>`
- Produces: `pub fn load() -> anyhow::Result<Vec<SessionRef>>` (dédup par `session_id`, dernière occurrence gagne, ordre = ordre d'apparition de la dernière occurrence)
- Produces: `pub fn resolve(sessions: &[SessionRef], last: bool, session_id: Option<&str>) -> Option<SessionRef>`

- [ ] **Step 1: Écrire les tests (échec attendu)**

Dans `crates/armadai/src/claude_adapter/session_index.rs`, en bas :
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_index(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("idx.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        (dir, p)
    }

    #[test]
    fn load_parses_and_dedups_last_wins() {
        let (_d, p) = tmp_index(&[
            r#"{"session_id":"a","transcript_path":"/t/a.jsonl","cwd":"/c","started_at":"t1"}"#,
            r#"{"session_id":"b","transcript_path":"/t/b.jsonl","cwd":"/c","started_at":"t2"}"#,
            r#"{"session_id":"a","transcript_path":"/t/a2.jsonl","cwd":"/c","started_at":"t3"}"#,
            "not json — skipped",
        ]);
        let v = load_from(&p).unwrap();
        assert_eq!(v.len(), 2, "dedup by session_id");
        let a = v.iter().find(|s| s.session_id == "a").unwrap();
        assert_eq!(a.transcript_path, PathBuf::from("/t/a2.jsonl"), "last wins");
    }

    #[test]
    fn append_then_load_roundtrips() {
        let (_d, p) = tmp_index(&[]);
        append_to(&p, &SessionRef {
            session_id: "x".into(),
            transcript_path: "/t/x.jsonl".into(),
            cwd: "/c".into(),
            started_at: "t".into(),
        }).unwrap();
        let v = load_from(&p).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].session_id, "x");
    }

    #[test]
    fn resolve_last_and_by_id() {
        let (_d, p) = tmp_index(&[
            r#"{"session_id":"a","transcript_path":"/t/a.jsonl","cwd":"/c","started_at":"t1"}"#,
            r#"{"session_id":"b","transcript_path":"/t/b.jsonl","cwd":"/c","started_at":"t2"}"#,
        ]);
        let v = load_from(&p).unwrap();
        assert_eq!(resolve(&v, true, None).unwrap().session_id, "b", "--last = dernière");
        assert_eq!(resolve(&v, false, Some("a")).unwrap().session_id, "a");
        assert!(resolve(&v, false, Some("zzz")).is_none());
    }
}
```

- [ ] **Step 2: Vérifier l'échec** — `cargo test -p armadai --features tui session_index` → FAIL (fns absentes). *(le crate bin s'appelle `armadai` ; les tests bin nécessitent au moins `--features tui` si du code testé est gated ; ici Task 1 n'est pas gated mais on garde la commande cohérente)*

- [ ] **Step 3: Implémenter** `session_index.rs` :
```rust
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One registered Claude Code session, appended by the plugin's SessionStart
/// hook (via `armadai __claude-register-session`) and read by `armadai watch`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRef {
    pub session_id: String,
    pub transcript_path: PathBuf,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub started_at: String,
}

/// Resolved path of the session index (override with `ARMADAI_SESSION_INDEX`).
pub fn index_path() -> PathBuf {
    if let Ok(p) = std::env::var("ARMADAI_SESSION_INDEX") {
        return PathBuf::from(p);
    }
    armadai_core::config::config_dir().join("claude-sessions.jsonl")
}

/// Append one session entry to the index (creating parent dirs as needed).
pub fn append(entry: &SessionRef) -> anyhow::Result<()> {
    append_to(&index_path(), entry)
}

fn append_to(path: &std::path::Path, entry: &SessionRef) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry)?;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Load + dedup the index (last occurrence of a `session_id` wins).
pub fn load() -> anyhow::Result<Vec<SessionRef>> {
    load_from(&index_path())
}

fn load_from(path: &std::path::Path) -> anyhow::Result<Vec<SessionRef>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    // Dedup last-wins while preserving the order of last occurrences.
    let mut order: Vec<String> = Vec::new();
    let mut by_id: std::collections::HashMap<String, SessionRef> = std::collections::HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<SessionRef>(line) else {
            continue; // defensive: skip malformed lines
        };
        if !by_id.contains_key(&entry.session_id) {
            order.push(entry.session_id.clone());
        } else {
            order.retain(|id| id != &entry.session_id);
            order.push(entry.session_id.clone());
        }
        by_id.insert(entry.session_id.clone(), entry);
    }
    Ok(order.into_iter().filter_map(|id| by_id.remove(&id)).collect())
}

/// Pick a session: `--last` → last entry; else by `session_id`.
pub fn resolve(sessions: &[SessionRef], last: bool, session_id: Option<&str>) -> Option<SessionRef> {
    if last {
        return sessions.last().cloned();
    }
    if let Some(id) = session_id {
        return sessions.iter().find(|s| s.session_id == id).cloned();
    }
    None
}
```
Créer `crates/armadai/src/claude_adapter/mod.rs` :
```rust
pub mod session_index;
```
Dans `crates/armadai/src/main.rs`, ajouter (à côté des autres `mod`, non gated) :
```rust
mod claude_adapter;
```

- [ ] **Step 4: Vérifier le succès** — `cargo test -p armadai --features tui session_index` → 3 tests PASS.
- [ ] **Step 5: Commit**
```bash
git add crates/armadai/src/claude_adapter/mod.rs crates/armadai/src/claude_adapter/session_index.rs crates/armadai/src/main.rs
git commit -m "feat(plugin): session index for Claude Code transcript adapter (P1)"
```

---

## Task 2: Lecteur de transcript défensif (`transcript.rs`)

**Files:**
- Create: `crates/armadai/src/claude_adapter/transcript.rs`
- Modify: `crates/armadai/src/claude_adapter/mod.rs` (ajouter `pub mod transcript;`)

**Interfaces:**
- Consumes: rien des tâches précédentes.
- Produces:
```rust
pub enum Block { Text(String), AgentSpawn { tool_use_id: String, subagent_type: String }, Other }
pub struct Usage { pub input_tokens: u32, pub output_tokens: u32 }
pub enum RelevantEntry {
    Assistant { model: String, blocks: Vec<Block>, usage: Usage },
    ToolResult { tool_use_id: String, text: String },
}
pub fn parse_line(line: &str) -> Option<RelevantEntry>
```

Le parse est **défensif** : on désérialise en `serde_json::Value`, on regarde `type`, et on n'extrait que ce dont le mapper a besoin. Toute forme inattendue → `None` (ligne ignorée). Les sous-agents = `tool_use` de nom `"Agent"` (input `subagent_type`). Les résultats d'outil arrivent soit comme bloc `tool_result` dans un message `user`, soit via `toolUseResult` top-level ; on gère le bloc `tool_result` (Anthropic) ici.

- [ ] **Step 1: Écrire les tests (échec attendu)** dans `transcript.rs` :
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_app_specific_and_malformed() {
        assert!(parse_line(r#"{"type":"ai-title","aiTitle":"x"}"#).is_none());
        assert!(parse_line(r#"{"type":"mode","mode":"x"}"#).is_none());
        assert!(parse_line("not json").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn parses_assistant_text_and_usage() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-x","content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":10,"output_tokens":5}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { model, blocks, usage } => {
                assert_eq!(model, "claude-x");
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 5);
                assert!(matches!(blocks.as_slice(), [Block::Text(t)] if t == "hello"));
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn parses_agent_spawn_tool_use() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"tu1","name":"Agent","input":{"subagent_type":"core-specialist","prompt":"x"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { blocks, .. } => {
                assert!(matches!(blocks.as_slice(),
                    [Block::AgentSpawn { tool_use_id, subagent_type }]
                    if tool_use_id == "tu1" && subagent_type == "core-specialist"));
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn parses_tool_result_from_user() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":[{"type":"text","text":"done"}]}]}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::ToolResult { tool_use_id, text } => {
                assert_eq!(tool_use_id, "tu1");
                assert_eq!(text, "done");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn non_agent_tool_use_yields_other_block() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { blocks, .. } => assert!(matches!(blocks.as_slice(), [Block::Other])),
            _ => panic!("expected Assistant"),
        }
    }
}
```

- [ ] **Step 2: Vérifier l'échec** — `cargo test -p armadai --features tui transcript::` → FAIL.

- [ ] **Step 3: Implémenter** `transcript.rs` :
```rust
use serde_json::Value;

/// One content block we care about within an assistant message.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    AgentSpawn { tool_use_id: String, subagent_type: String },
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// A transcript entry the mapper acts on. Everything else is dropped.
#[derive(Debug, Clone, PartialEq)]
pub enum RelevantEntry {
    Assistant { model: String, blocks: Vec<Block>, usage: Usage },
    ToolResult { tool_use_id: String, text: String },
}

/// Defensive parse of one transcript JSONL line. Returns `None` for malformed
/// lines and for any entry type the adapter does not model (ai-title, mode,
/// pr-link, system, attachment, …) — never panics.
pub fn parse_line(line: &str) -> Option<RelevantEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "assistant" => parse_assistant(v.get("message")?),
        "user" => parse_user_tool_result(v.get("message")?),
        _ => None,
    }
}

fn parse_assistant(msg: &Value) -> Option<RelevantEntry> {
    let model = msg.get("model").and_then(Value::as_str).unwrap_or("").to_string();
    let usage = msg.get("usage").map(parse_usage).unwrap_or_default();
    let mut blocks = Vec::new();
    for b in msg.get("content").and_then(Value::as_array).into_iter().flatten() {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = b.get("text").and_then(Value::as_str) {
                    blocks.push(Block::Text(t.to_string()));
                }
            }
            Some("tool_use") if b.get("name").and_then(Value::as_str) == Some("Agent") => {
                let id = b.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let sub = b
                    .get("input")
                    .and_then(|i| i.get("subagent_type"))
                    .and_then(Value::as_str)
                    .unwrap_or("agent")
                    .to_string();
                blocks.push(Block::AgentSpawn { tool_use_id: id, subagent_type: sub });
            }
            Some("tool_use") => blocks.push(Block::Other),
            _ => {} // thinking, redacted_thinking, etc. — dropped
        }
    }
    Some(RelevantEntry::Assistant { model, blocks, usage })
}

fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
    }
}

fn parse_user_tool_result(msg: &Value) -> Option<RelevantEntry> {
    for b in msg.get("content").and_then(Value::as_array).into_iter().flatten() {
        if b.get("type").and_then(Value::as_str) == Some("tool_result") {
            let id = b.get("tool_use_id").and_then(Value::as_str)?.to_string();
            let text = tool_result_text(b.get("content"));
            return Some(RelevantEntry::ToolResult { tool_use_id: id, text });
        }
    }
    None
}

/// A tool_result `content` may be a string or an array of `{type:text,text}`.
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
```
Ajouter `pub mod transcript;` dans `crates/armadai/src/claude_adapter/mod.rs`.

- [ ] **Step 4: Vérifier le succès** — `cargo test -p armadai --features tui transcript::` → 5 tests PASS.
- [ ] **Step 5: Commit**
```bash
git add crates/armadai/src/claude_adapter/transcript.rs crates/armadai/src/claude_adapter/mod.rs
git commit -m "feat(plugin): defensive Claude Code transcript line parser (P1)"
```

---

## Task 3: Mapper `RelevantEntry` → `RunEvent` (`mapper.rs`)

**Files:**
- Create: `crates/armadai/src/claude_adapter/mapper.rs`
- Modify: `crates/armadai/src/claude_adapter/mod.rs` (ajouter `pub mod mapper;`)

**Interfaces:**
- Consumes: `transcript::{RelevantEntry, Block, Usage}`, `armadai_core::events::RunEvent`.
- Produces:
```rust
pub struct Mapper { /* private state */ }
impl Mapper {
    pub fn new(session_id: &str) -> Self
    /// Feed one entry; returns the RunEvents it produces (in order).
    pub fn push(&mut self, entry: RelevantEntry) -> Vec<RunEvent>
    /// Signal end-of-stream (EOF/replay or terminal stop); returns closing events.
    pub fn finish(&mut self) -> Vec<RunEvent>
}
```

Comportement (niveau agent, MVP) :
- Le 1er `Assistant` déclenche `RunStart { run_id: session_id, v: 1, agents: ["claude"], prov: "claude", model, in_chars: 0 }` **puis** `AgentStart { agent: "claude", prov: "claude", model }`.
- Chaque `Block::AgentSpawn { tool_use_id, subagent_type }` → `Delegate { from: "claude", to: subagent_type }` puis `AgentStart { agent: subagent_type, prov: "claude", model }` ; on mémorise `tool_use_id → subagent_type`.
- Un `ToolResult { tool_use_id, text }` dont l'id correspond à un spawn connu → `AgentEnd { agent: subagent_type, tin: 0, tout: 0, cost: 0.0, content: text tronqué à 2000 }`.
- `usage` accumulé (tin/tout) sur tous les `Assistant` ; le dernier bloc `Text` non vide est retenu comme contenu final.
- `finish()` → `AgentEnd { agent: "claude", tin, tout, cost: 0.0, content: last_text }` puis `Result { content: last_text, tin, tout, cost: 0.0, agents: <nb agents distincts vus> }`.
- Tokens/coût des sous-agents : 0 en P1 (leur usage vit dans `agent-<id>.jsonl`, hors P1) ; `cost` = 0.0 (enrichissement ultérieur).

- [ ] **Step 1: Écrire les tests (échec attendu)** dans `mapper.rs` :
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_adapter::transcript::{Block, RelevantEntry, Usage};
    use armadai_core::events::RunEvent;

    fn assistant(blocks: Vec<Block>, tin: u32, tout: u32) -> RelevantEntry {
        RelevantEntry::Assistant { model: "m".into(), blocks, usage: Usage { input_tokens: tin, output_tokens: tout } }
    }

    #[test]
    fn simple_session_emits_runstart_agentstart_result() {
        let mut m = Mapper::new("s1");
        let mut evs = m.push(assistant(vec![Block::Text("hi".into())], 10, 3));
        evs.extend(m.finish());
        assert!(matches!(&evs[0], RunEvent::RunStart { run_id, agents, prov, .. }
            if run_id == "s1" && agents == &vec!["claude".to_string()] && prov == "claude"));
        assert!(matches!(&evs[1], RunEvent::AgentStart { agent, .. } if agent == "claude"));
        // AgentEnd(claude) then Result
        assert!(matches!(evs[evs.len()-2], RunEvent::AgentEnd { .. }));
        match evs.last().unwrap() {
            RunEvent::Result { content, tin, tout, agents, .. } => {
                assert_eq!(content, "hi");
                assert_eq!(*tin, 10);
                assert_eq!(*tout, 3);
                assert_eq!(*agents, 1);
            }
            other => panic!("expected Result, got {other:?}"),
        }
    }

    #[test]
    fn subagent_spawn_and_result_emit_delegate_start_end() {
        let mut m = Mapper::new("s2");
        let mut evs = m.push(assistant(vec![Block::Text("start".into())], 5, 1));
        evs.extend(m.push(assistant(vec![Block::AgentSpawn { tool_use_id: "tu1".into(), subagent_type: "core".into() }], 2, 1)));
        evs.extend(m.push(RelevantEntry::ToolResult { tool_use_id: "tu1".into(), text: "sub done".into() }));
        evs.extend(m.push(assistant(vec![Block::Text("final".into())], 1, 1)));
        evs.extend(m.finish());
        assert!(evs.iter().any(|e| matches!(e, RunEvent::Delegate { from, to } if from == "claude" && to == "core")));
        assert!(evs.iter().any(|e| matches!(e, RunEvent::AgentStart { agent, .. } if agent == "core")));
        assert!(evs.iter().any(|e| matches!(e, RunEvent::AgentEnd { agent, content, .. } if agent == "core" && content == "sub done")));
        // agents count in Result = claude + core = 2
        match evs.last().unwrap() {
            RunEvent::Result { agents, content, tin, .. } => {
                assert_eq!(*agents, 2);
                assert_eq!(content, "final");
                assert_eq!(*tin, 8, "5+2+1 input tokens accumulated");
            }
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn unknown_tool_result_id_is_ignored() {
        let mut m = Mapper::new("s3");
        let _ = m.push(assistant(vec![Block::Text("x".into())], 1, 1));
        let evs = m.push(RelevantEntry::ToolResult { tool_use_id: "nope".into(), text: "y".into() });
        assert!(evs.is_empty(), "no AgentEnd for an unknown tool_use_id");
    }
}
```

- [ ] **Step 2: Vérifier l'échec** — `cargo test -p armadai --features tui mapper::` → FAIL.

- [ ] **Step 3: Implémenter** `mapper.rs` :
```rust
use std::collections::HashMap;

use armadai_core::events::RunEvent;

use crate::claude_adapter::transcript::{Block, RelevantEntry};

const ROOT: &str = "claude";
const PROV: &str = "claude";
const MAX_CONTENT: usize = 2000;

/// Reconstructs agent-level `RunEvent`s from a stream of `RelevantEntry`.
pub struct Mapper {
    session_id: String,
    started: bool,
    model: String,
    tin: u32,
    tout: u32,
    last_text: String,
    spawns: HashMap<String, String>, // tool_use_id -> subagent_type
    agents_seen: std::collections::HashSet<String>,
    finished: bool,
}

impl Mapper {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            started: false,
            model: String::new(),
            tin: 0,
            tout: 0,
            last_text: String::new(),
            spawns: HashMap::new(),
            agents_seen: std::collections::HashSet::new(),
            finished: false,
        }
    }

    pub fn push(&mut self, entry: RelevantEntry) -> Vec<RunEvent> {
        let mut out = Vec::new();
        match entry {
            RelevantEntry::Assistant { model, blocks, usage } => {
                if !self.started {
                    self.started = true;
                    self.model = model.clone();
                    self.agents_seen.insert(ROOT.to_string());
                    out.push(RunEvent::RunStart {
                        run_id: self.session_id.clone(),
                        v: 1,
                        agents: vec![ROOT.to_string()],
                        prov: PROV.to_string(),
                        model: model.clone(),
                        in_chars: 0,
                    });
                    out.push(RunEvent::AgentStart {
                        agent: ROOT.to_string(),
                        prov: PROV.to_string(),
                        model: model.clone(),
                    });
                }
                self.tin = self.tin.saturating_add(usage.input_tokens);
                self.tout = self.tout.saturating_add(usage.output_tokens);
                for b in blocks {
                    match b {
                        Block::Text(t) if !t.trim().is_empty() => self.last_text = t,
                        Block::Text(_) | Block::Other => {}
                        Block::AgentSpawn { tool_use_id, subagent_type } => {
                            self.spawns.insert(tool_use_id, subagent_type.clone());
                            self.agents_seen.insert(subagent_type.clone());
                            out.push(RunEvent::Delegate { from: ROOT.to_string(), to: subagent_type.clone() });
                            out.push(RunEvent::AgentStart {
                                agent: subagent_type,
                                prov: PROV.to_string(),
                                model: self.model.clone(),
                            });
                        }
                    }
                }
            }
            RelevantEntry::ToolResult { tool_use_id, text } => {
                if let Some(agent) = self.spawns.remove(&tool_use_id) {
                    let mut content = text;
                    content.truncate(MAX_CONTENT);
                    out.push(RunEvent::AgentEnd { agent, tin: 0, tout: 0, cost: 0.0, content });
                }
            }
        }
        out
    }

    pub fn finish(&mut self) -> Vec<RunEvent> {
        if self.finished || !self.started {
            return Vec::new();
        }
        self.finished = true;
        let mut content = self.last_text.clone();
        content.truncate(MAX_CONTENT);
        vec![
            RunEvent::AgentEnd {
                agent: ROOT.to_string(),
                tin: self.tin,
                tout: self.tout,
                cost: 0.0,
                content: content.clone(),
            },
            RunEvent::Result {
                content,
                tin: self.tin,
                tout: self.tout,
                cost: 0.0,
                agents: self.agents_seen.len(),
            },
        ]
    }
}
```
Ajouter `pub mod mapper;` dans `mod.rs`.

- [ ] **Step 4: Vérifier le succès** — `cargo test -p armadai --features tui mapper::` → 3 tests PASS.
- [ ] **Step 5: Commit**
```bash
git add crates/armadai/src/claude_adapter/mapper.rs crates/armadai/src/claude_adapter/mod.rs
git commit -m "feat(plugin): map Claude Code transcript entries to RunEvents (P1)"
```

---

## Task 4: `drive_session` + sous-commande cachée `__claude-register-session`

**Files:**
- Modify: `crates/armadai/src/claude_adapter/mod.rs` (ajouter `drive_session` + `register_from_stdin`)
- Modify: `crates/armadai/src/cli/mod.rs` (variante `ClaudeRegisterSession` cachée + dispatch)

**Interfaces:**
- Consumes: `session_index::SessionRef`, `transcript::parse_line`, `mapper::Mapper`, `armadai_core::events::EventSink`.
- Produces:
```rust
// in claude_adapter/mod.rs
pub async fn drive_session(session: session_index::SessionRef, sink: std::sync::Arc<dyn armadai_core::events::EventSink>, follow: bool) -> anyhow::Result<()>
pub fn register_from_stdin() -> anyhow::Result<()>
```
`drive_session` : lit le transcript ligne à ligne (replay), passe chaque `parse_line` non-`None` au `Mapper`, `emit` chaque `RunEvent`. En mode `follow` (live) : après EOF, re-poll les octets ajoutés (boucle `sleep(200ms)` + lecture depuis l'offset) jusqu'à voir un `RunEvent::Result` **ou** absence de croissance prolongée ; sinon (replay) émet `finish()` à EOF. `register_from_stdin` : lit tout stdin, extrait `session_id`/`transcript_path`/`cwd`, append via `session_index::append`, **toujours `Ok(())`** (erreurs loggées en warn).

- [ ] **Step 1: Écrire le test (échec attendu)** dans `claude_adapter/mod.rs` :
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use armadai_core::events::RunEvent;
    use std::sync::{Arc, Mutex};

    struct CapSink(Arc<Mutex<Vec<RunEvent>>>);
    impl armadai_core::events::EventSink for CapSink {
        fn emit(&self, ev: &RunEvent) { self.0.lock().unwrap().push(ev.clone()); }
    }

    #[tokio::test]
    async fn drive_session_replays_a_transcript_to_events() {
        let dir = tempfile::tempdir().unwrap();
        let tp = dir.path().join("t.jsonl");
        std::fs::write(&tp, concat!(
            r#"{"type":"ai-title","aiTitle":"noise"}"#, "\n",
            r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":4,"output_tokens":2}}}"#, "\n",
        )).unwrap();
        let session = session_index::SessionRef {
            session_id: "s".into(), transcript_path: tp, cwd: "/c".into(), started_at: "t".into(),
        };
        let store = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn armadai_core::events::EventSink> = Arc::new(CapSink(store.clone()));
        drive_session(session, sink, false).await.unwrap();
        let evs = store.lock().unwrap();
        assert!(matches!(&evs[0], RunEvent::RunStart { run_id, .. } if run_id == "s"));
        assert!(matches!(evs.last().unwrap(), RunEvent::Result { content, .. } if content == "hello"));
    }

    #[test]
    fn register_from_reader_appends_index() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("idx.jsonl");
        // SAFETY: single-threaded test; serialise env via ENV_MUTEX in real cross-test setups.
        let _g = armadai_core::config::ENV_MUTEX.lock().unwrap();
        unsafe { std::env::set_var("ARMADAI_SESSION_INDEX", &idx); }
        let payload = r#"{"session_id":"z","transcript_path":"/t/z.jsonl","cwd":"/c"}"#;
        register_from_reader(payload.as_bytes()).unwrap();
        let v = session_index::load().unwrap();
        unsafe { std::env::remove_var("ARMADAI_SESSION_INDEX"); }
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].session_id, "z");
    }
}
```
> Note : `register_from_stdin()` délègue à `register_from_reader(std::io::stdin())` pour être testable. Le test env utilise `armadai_core::config::ENV_MUTEX` — le bin l'a déjà en `[dev-dependencies]` via la feature `test-support` (OH7 finition F3). Si `ENV_MUTEX` est indisponible dans ce contexte, remplacer par une écriture directe via `append_to` exposé — mais la voie `ARMADAI_SESSION_INDEX` reflète le vrai chemin.

- [ ] **Step 2: Vérifier l'échec** — `cargo test -p armadai --features tui claude_adapter::tests` → FAIL.

- [ ] **Step 3: Implémenter** dans `crates/armadai/src/claude_adapter/mod.rs` :
```rust
pub mod mapper;
pub mod session_index;
pub mod transcript;

use std::io::{BufRead, Read};
use std::sync::Arc;

use armadai_core::events::{EventSink, RunEvent};

use mapper::Mapper;
use session_index::SessionRef;

/// Read `session`'s transcript and emit reconstructed `RunEvent`s to `sink`.
/// `follow=false` → replay to EOF then `finish()`. `follow=true` → after EOF,
/// keep polling appended bytes until a terminal `Result` is produced.
pub async fn drive_session(session: SessionRef, sink: Arc<dyn EventSink>, follow: bool) -> anyhow::Result<()> {
    let mut mapper = Mapper::new(&session.session_id);
    let mut offset: u64 = 0;
    let mut done = false;
    loop {
        let file = match std::fs::File::open(&session.transcript_path) {
            Ok(f) => f,
            Err(e) => {
                sink.emit(&RunEvent::Error {
                    code: "transcript_unreadable".into(),
                    msg: format!("{}: {e}", session.transcript_path.display()),
                });
                return Ok(());
            }
        };
        use std::io::Seek;
        let mut reader = std::io::BufReader::new(file);
        reader.seek(std::io::SeekFrom::Start(offset))?;
        let mut consumed = 0u64;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break; // EOF
            }
            // Only advance past complete (newline-terminated) lines, so a
            // partially-written trailing line is re-read next poll.
            if !line.ends_with('\n') {
                break;
            }
            consumed += n as u64;
            if let Some(entry) = transcript::parse_line(&line) {
                for ev in mapper.push(entry) {
                    if matches!(ev, RunEvent::Result { .. }) {
                        done = true;
                    }
                    sink.emit(&ev);
                }
            }
        }
        offset += consumed;
        if done {
            return Ok(());
        }
        if !follow {
            for ev in mapper.finish() {
                sink.emit(&ev);
            }
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// Hook entrypoint: read the SessionStart payload from stdin, append the
/// session to the index. Always returns `Ok(())` (errors warned), so the hook
/// never disturbs Claude Code.
pub fn register_from_stdin() -> anyhow::Result<()> {
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() {
        return Ok(());
    }
    let _ = register_from_reader(&buf[..]);
    Ok(())
}

fn register_from_reader(mut r: impl Read) -> anyhow::Result<()> {
    let mut buf = String::new();
    r.read_to_string(&mut buf)?;
    let v: serde_json::Value = serde_json::from_str(buf.trim())?;
    let get = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    let session_id = get("session_id");
    let transcript_path = get("transcript_path");
    if session_id.is_empty() || transcript_path.is_empty() {
        return Ok(()); // nothing usable; do not error
    }
    let entry = SessionRef {
        session_id,
        transcript_path: transcript_path.into(),
        cwd: get("cwd"),
        started_at: get("timestamp"),
    };
    if let Err(e) = session_index::append(&entry) {
        tracing::warn!("failed to register Claude Code session: {e}");
    }
    Ok(())
}
```
> `let _ = BufRead;` — retirer l'import `BufRead` s'il est inutile (le `read_line` vient de `BufRead` — le garder). Réconcilier les imports au compilateur.

Dans `crates/armadai/src/cli/mod.rs`, ajouter la variante cachée + le dispatch :
```rust
/// Internal: called by the Claude Code plugin's SessionStart hook. Reads the
/// hook JSON from stdin and registers the session. Hidden from help.
#[command(hide = true, name = "__claude-register-session")]
ClaudeRegisterSession,
```
et dans le `match` de dispatch :
```rust
Command::ClaudeRegisterSession => {
    crate::claude_adapter::register_from_stdin()
}
```
*(adapter à la signature réelle du dispatch : si les handlers renvoient `anyhow::Result<()>` et sont `.await`és, envelopper en `async`/`Ok` selon le pattern local ; `register_from_stdin` est sync → `Command::ClaudeRegisterSession => crate::claude_adapter::register_from_stdin(),` si le bras n'est pas `.await`é, sinon `=> { crate::claude_adapter::register_from_stdin() }`.)*

- [ ] **Step 4: Vérifier le succès** — `cargo test -p armadai --features tui claude_adapter::tests` → 2 tests PASS. Vérifier aussi `armadai __claude-register-session` n'apparaît PAS dans `cargo run -q -- --help`.
- [ ] **Step 5: Commit**
```bash
git add crates/armadai/src/claude_adapter/mod.rs crates/armadai/src/cli/mod.rs
git commit -m "feat(plugin): drive_session + hidden __claude-register-session subcommand (P1)"
```

---

## Task 5: Commande `armadai watch` + plugin Claude Code (assets)

**Files:**
- Create: `crates/armadai/src/cli/watch.rs`
- Modify: `crates/armadai/src/cli/mod.rs` (variante `Watch` + dispatch, gated `tui`)
- Create: `crates/armadai/assets/claude-plugin/.claude-plugin/plugin.json`
- Create: `crates/armadai/assets/claude-plugin/hooks/hooks.json`
- Create: `crates/armadai/assets/claude-plugin/README.md`

**Interfaces:**
- Consumes: `claude_adapter::{drive_session, session_index}`, `shell::run_view::run_orchestration_tui`, `armadai_core::events::make_sink`.
- Produces: `pub async fn execute(last: bool, session: Option<String>, json: bool) -> anyhow::Result<()>`

- [ ] **Step 1: Écrire le test (échec attendu)** dans `crates/armadai/src/cli/watch.rs` — test de résolution + mode `--json` headless (pas de TUI) :
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn json_mode_replays_without_tui() {
        let dir = tempfile::tempdir().unwrap();
        let tp = dir.path().join("t.jsonl");
        std::fs::write(&tp, concat!(
            r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#, "\n",
        )).unwrap();
        let idx = dir.path().join("idx.jsonl");
        let _g = armadai_core::config::ENV_MUTEX.lock().unwrap();
        unsafe { std::env::set_var("ARMADAI_SESSION_INDEX", &idx); }
        crate::claude_adapter::session_index::append(
            &crate::claude_adapter::session_index::SessionRef {
                session_id: "s".into(), transcript_path: tp, cwd: "/c".into(), started_at: "t".into(),
            }).unwrap();
        // json=true → no TUI; must resolve --last and complete without error.
        let r = execute(true, None, true).await;
        unsafe { std::env::remove_var("ARMADAI_SESSION_INDEX"); }
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn errors_when_no_session_found() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("empty.jsonl");
        let _g = armadai_core::config::ENV_MUTEX.lock().unwrap();
        unsafe { std::env::set_var("ARMADAI_SESSION_INDEX", &idx); }
        let r = execute(false, Some("does-not-exist".into()), true).await;
        unsafe { std::env::remove_var("ARMADAI_SESSION_INDEX"); }
        assert!(r.is_err());
    }
}
```

- [ ] **Step 2: Vérifier l'échec** — `cargo test -p armadai --features tui watch::` → FAIL.

- [ ] **Step 3: Implémenter** `crates/armadai/src/cli/watch.rs` :
```rust
use crate::claude_adapter::{drive_session, session_index};

/// `armadai watch` — attach the Workroom to a Claude Code session (from the
/// index the plugin populates) and stream reconstructed RunEvents.
pub async fn execute(last: bool, session: Option<String>, json: bool) -> anyhow::Result<()> {
    let sessions = session_index::load()?;
    if sessions.is_empty() {
        anyhow::bail!(
            "no Claude Code sessions registered — install the armadai-workroom plugin \
             (see crates/armadai/assets/claude-plugin) and start a Claude Code session"
        );
    }
    // Default (no --last, no --session): pick the most recent.
    let picked = session_index::resolve(&sessions, last || session.is_none(), session.as_deref())
        .ok_or_else(|| anyhow::anyhow!("no matching session (use --last or --session <id>)"))?;

    if json {
        // Headless: replay to JSONL on stdout (no TUI).
        let sink = armadai_core::events::make_sink(true);
        return drive_session(picked, sink, false).await;
    }

    // Live Workroom TUI, fed by the transcript adapter. `follow=true` tails.
    let (_run_id, _content) = crate::shell::run_view::run_orchestration_tui(
        move |sink| async move { drive_session(picked, sink, true).await },
        None,
        None,
    )
    .await?;
    Ok(())
}
```
Dans `crates/armadai/src/cli/mod.rs`, ajouter la variante (gated `tui` car elle tire le Workroom) :
```rust
/// Watch a Claude Code session live in the Workroom (via the armadai plugin).
#[cfg(feature = "tui")]
Watch {
    /// Attach to the most recently registered session.
    #[arg(long)]
    last: bool,
    /// Attach to a specific session id.
    #[arg(long)]
    session: Option<String>,
    /// Emit reconstructed RunEvents as JSONL to stdout instead of the TUI.
    #[arg(long)]
    json: bool,
},
```
et le dispatch :
```rust
#[cfg(feature = "tui")]
Command::Watch { last, session, json } => watch::execute(last, session, json).await,
```
Ajouter `#[cfg(feature = "tui")] mod watch;` en tête de `cli/mod.rs` (près des autres `mod`).

- [ ] **Step 4: Créer les assets du plugin.**
`crates/armadai/assets/claude-plugin/.claude-plugin/plugin.json` :
```json
{
  "name": "armadai-workroom",
  "displayName": "ArmadAI Workroom",
  "version": "0.1.0",
  "description": "Registers Claude Code sessions so `armadai watch` can visualize them in the ArmadAI Workroom.",
  "hooks": "./hooks/hooks.json"
}
```
`crates/armadai/assets/claude-plugin/hooks/hooks.json` :
```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          { "type": "command", "command": "armadai __claude-register-session", "async": true }
        ]
      }
    ]
  }
}
```
`crates/armadai/assets/claude-plugin/README.md` :
```markdown
# armadai-workroom (Claude Code plugin)

Registers each Claude Code session (session id + transcript path) into
`~/.config/armadai/claude-sessions.jsonl` via a `SessionStart` hook that calls
`armadai __claude-register-session`. Then run `armadai watch` to visualize the
session live (or replay it) in the ArmadAI Workroom.

## Install

Requires the `armadai` binary on your `PATH`.

    claude plugin install ./crates/armadai/assets/claude-plugin

## Use

    armadai watch            # pick the most recent session
    armadai watch --last     # most recent, no prompt
    armadai watch --session <id>
    armadai watch --json     # JSONL instead of the TUI
```

- [ ] **Step 5: Vérifier + valider** :
```bash
cargo test -p armadai --features tui watch::            # 2 tests PASS
python3 -c "import json; json.load(open('crates/armadai/assets/claude-plugin/.claude-plugin/plugin.json')); json.load(open('crates/armadai/assets/claude-plugin/hooks/hooks.json')); print('plugin JSON OK')"
cargo run -q --features tui -- --help | grep -q watch && echo "watch listed" || echo "!! watch absent"
cargo run -q --features tui -- --help | grep -c "__claude-register-session"   # attendu 0 (caché)
```

- [ ] **Step 6: Commit**
```bash
git add crates/armadai/src/cli/watch.rs crates/armadai/src/cli/mod.rs crates/armadai/assets/claude-plugin
git commit -m "feat(plugin): armadai watch command + Claude Code plugin assets (P1)"
```

---

## Invariant de fin de P1

- `armadai watch [--last|--session <id>|--json]` reconstruit et affiche une session Claude Code dans le Workroom (live + replay).
- Le plugin `crates/armadai/assets/claude-plugin/` s'installe (`claude plugin install`) et son hook `SessionStart` peuple l'index via `armadai __claude-register-session` (caché du `--help`).
- `armadai-core` **inchangé** (`cargo build -p armadai-core` identique ; aucune variante `RunEvent` ajoutée).
- Gate workspace-wide verte (fmt + clippy 3 combos + test 3 modes) ; parser défensif (lignes inconnues/malformées ignorées, jamais de panic, lecture streaming).

## Hors périmètre (rappel P1)

Couche hooks temps réel (P2) ; intégration relais armadai + retrait marqueurs (P3) ; drill-down `agent-<id>.jsonl` & tool calls individuels ; coût monétaire par modèle (tokens seulement en P1) ; publication marketplace.

## Self-Review (rempli à l'écriture)

- **Couverture spec** : plugin minimal + hook SessionStart (Task 5 assets + Task 4 register) ✓ ; index (Task 1) ✓ ; adaptateur streaming défensif (Task 2) + mapper agent-level (Task 3) ✓ ; `armadai watch` live+replay+json (Task 5) ✓ ; cœur inchangé / pas d'extension RunEvent ✓.
- **Placeholders** : aucun « TBD » ; deux notes de réconciliation-au-compilateur (imports `BufRead` ; forme exacte du bras de dispatch selon `.await`) — ce sont des ajustements mécaniques concrets, pas des trous.
- **Cohérence des types** : `SessionRef`, `RelevantEntry`/`Block`/`Usage`, `Mapper::{new,push,finish}`, `drive_session(session, sink, follow)`, `register_from_stdin/reader`, `watch::execute(last, session, json)` cohérents entre tâches ; champs `RunEvent` conformes à `core::events` (vérifiés verbatim).
- **Piège env-test** : les tests touchant `ARMADAI_SESSION_INDEX` prennent `armadai_core::config::ENV_MUTEX` (dispo via la dev-dep `test-support`, OH7 F3) et utilisent `unsafe { set_var }` (edition 2024).
- **Piège feature** : `watch` (Workroom) gated `tui` ; l'adaptateur + l'index + le register ne sont PAS gated (utilisables en `--json` sans TUI). Le hook/register ne dépend pas de `tui`.
