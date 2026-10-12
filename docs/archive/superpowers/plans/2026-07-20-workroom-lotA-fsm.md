# Workroom Lot A — FSM event-based (marqueurs ArmadAI) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Piloter les statuts de la Workroom du shell par les marqueurs du protocole ArmadAI (`DELEGATE`/`META`/`END`) au lieu du scan flou `detect_mentions`, avec des données enrichies (dernière action, transitions) stockées pour un futur drill-down.

**Architecture:** `TrackedAgent` gagne `last_action` + `transitions`. Une nouvelle méthode `Workroom::apply_stream_text(chunk)` bufferise le texte streamé, en extrait les marqueurs complets et applique une FSM (DELEGATE → Working / END → Done + agent courant). `parse_streaming_line` route vers la même extraction. `detect_mentions` reste en place en Lot A (retiré au Lot B via le recâblage `app.rs`).

**Tech Stack:** Rust edition 2024, ratatui (gated `tui`).

## Global Constraints

- Base = `origin/release/1.0.0` (@ `7f00266`). Branche `feat/workroom-lotA-fsm`, PR vers `release/1.0.0`.
- Le module est gated `tui` (`#![cfg(feature = "tui")]` en tête de `workroom.rs`). Clippy 2 modes CI `-D warnings` : `--no-default-features --features tui` ET `--features tui,providers-api`. `cargo fmt -- --check`. `cargo test --no-default-features --features tui`.
- **Compilation préservée** : `detect_mentions` n'est PAS supprimé dans ce lot (ses appels dans `app.rs` restent valides). On AJOUTE la FSM à côté.
- Marqueurs (protocole ArmadAI, cf. `.claude/CLAUDE.md`) : `<!--ARMADAI_DELEGATE:agent-name-->`, `<!--ARMADAI_META:status=...-->`, `<!--ARMADAI_END-->`.
- Spec : `docs/superpowers/specs/2026-07-20-workroom-event-based-design.md`.

---

### Task 1: Enrichir `TrackedAgent` (last_action + transitions)

**Files:**
- Modify: `src/shell/workroom.rs`

**Interfaces:**
- Produces : `TrackedAgent` gagne `pub last_action: Option<String>` et `pub transitions: Vec<(AgentState, std::time::Instant)>`. Un helper `TrackedAgent` doit avoir `AgentState: Clone` (déjà le cas).

- [ ] **Step 1: Write the failing test**

Dans le module `#[cfg(test)]` de `workroom.rs`, ajouter :

```rust
#[test]
fn test_tracked_agent_has_enriched_fields() {
    let mut wr = Workroom::new();
    wr.init_from_config("coordinator: dev-lead\nteams:\n  - agents: [core-specialist]\n");
    // Every tracked agent starts with no last action and an empty transition log.
    for a in wr.agents_for_test() {
        assert!(a.last_action.is_none());
        assert!(a.transitions.is_empty());
    }
}
```

Ajouter aussi un accesseur test-only si `agents` est privé :

```rust
#[cfg(test)]
impl Workroom {
    pub fn agents_for_test(&self) -> &[TrackedAgent] { &self.agents }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui -p armadai test_tracked_agent_has_enriched_fields`
Expected: FAIL (`last_action`/`transitions` unknown).

- [ ] **Step 3: Add the fields + update all constructors**

Dans `TrackedAgent` (≈ ligne 29), ajouter :

```rust
    /// Short excerpt of the agent's latest action/status (for drill-down).
    pub last_action: Option<String>,
    /// State-transition history with timestamps (for drill-down).
    pub transitions: Vec<(AgentState, std::time::Instant)>,
```

Mettre à jour TOUS les littéraux `TrackedAgent { ... }` (rechercher `TrackedAgent {` — sites ≈ 82, 107, 130, 181 dans `init_from_config`/`set_agents_from_init`) en ajoutant `last_action: None, transitions: Vec::new(),`. Vérifier par compilation.

Ajouter l'accesseur `agents_for_test` (Step 1) s'il n'existe pas.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --no-default-features --features tui -p armadai test_tracked_agent_has_enriched_fields`
Expected: PASS.

- [ ] **Step 5: Clippy 2 modes + fmt**

Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/shell/workroom.rs
git commit -m "feat(shell): enrich TrackedAgent with last_action + transitions"
```

---

### Task 2: FSM `apply_stream_text` (extraction marqueurs + transitions)

**Files:**
- Modify: `src/shell/workroom.rs`

**Interfaces:**
- Consumes (Task 1) : `TrackedAgent.{last_action, transitions}`.
- Produces :
  - `pub fn apply_stream_text(&mut self, chunk: &str)` — bufferise `chunk`, extrait les marqueurs complets, applique la FSM, conserve un éventuel marqueur partiel en fin de buffer.
  - FSM interne : `DELEGATE:X` → coordinateur/lead courant `Delegating`, X `Working` + `current_agent = X` + transition ; `META:status=s` → `last_action` de l'agent courant = s ; `END` → agent courant `Done` (+ transition), `current_agent` repointé sur le coordinateur.
  - `parse_streaming_line` route vers `apply_stream_text`.
  - Un helper `fn set_state(&mut self, name, state)` qui pousse la transition (`(state.clone(), Instant::now())`) et met à jour `started_at`/`finished_at`.

- [ ] **Step 1: Write the failing tests**

**IMPORTANT — config de test** : `init_from_config` parse du **YAML bloc** (`agents:` puis lignes `- nom` SANS `:`), PAS l'inline `[x]`. Utiliser ce helper dans les tests :

```rust
#[cfg(test)]
fn wr_dev_lead_core() -> Workroom {
    let mut wr = Workroom::new();
    // Block-style YAML (the line-based parser does NOT expand inline `[...]`).
    wr.init_from_config("coordinator: dev-lead\nteams:\n  - agents:\n      - core-specialist\n");
    wr
}
```

```rust
#[test]
fn test_apply_stream_text_delegate_sets_working_and_records_transition() {
    let mut wr = wr_dev_lead_core();
    wr.apply_stream_text("<!--ARMADAI_DELEGATE:core-specialist-->");
    let a = wr.agents_for_test().iter().find(|a| a.name == "core-specialist").unwrap();
    assert_eq!(a.state, AgentState::Working);
    assert!(!a.transitions.is_empty());
    let coord = wr.agents_for_test().iter().find(|a| a.name == "dev-lead").unwrap();
    assert_eq!(coord.state, AgentState::Delegating);
}

#[test]
fn test_apply_stream_text_end_marks_current_done() {
    let mut wr = wr_dev_lead_core();
    wr.apply_stream_text("<!--ARMADAI_DELEGATE:core-specialist-->");
    wr.apply_stream_text("<!--ARMADAI_META:status=complete-->");
    wr.apply_stream_text("<!--ARMADAI_END-->");
    let a = wr.agents_for_test().iter().find(|a| a.name == "core-specialist").unwrap();
    assert_eq!(a.state, AgentState::Done);
    assert_eq!(a.last_action.as_deref(), Some("complete"));
    assert!(a.finished_at.is_some());
}

#[test]
fn test_apply_stream_text_handles_marker_split_across_chunks() {
    let mut wr = wr_dev_lead_core();
    wr.apply_stream_text("<!--ARMADAI_DELE");
    wr.apply_stream_text("GATE:core-specialist-->");
    let a = wr.agents_for_test().iter().find(|a| a.name == "core-specialist").unwrap();
    assert_eq!(a.state, AgentState::Working);
}

// Regression: ArmadAI markers can be ECHOED in Claude Code recaps
// (e.g. `| Ceci est un recap ... <!--ARMADAI_END-->`). A stray END with no
// active agent must be a safe no-op — never mark an idle agent Done, never panic.
#[test]
fn test_apply_stream_text_stray_end_in_recap_is_noop() {
    let mut wr = wr_dev_lead_core();
    wr.apply_stream_text("| Ceci est un recap de Claude Code <!--ARMADAI_END-->");
    for a in wr.agents_for_test() {
        assert_ne!(a.state, AgentState::Done, "stray recap END must not mark '{}' done", a.name);
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui -p armadai apply_stream_text`
Expected: FAIL (`apply_stream_text` undefined).

- [ ] **Step 3: Implement the FSM**

Ajouter un champ buffer + agent courant à `Workroom` (struct ≈ ligne 51) :

```rust
    /// Buffer for streamed text, used to extract markers that may span chunks.
    marker_buf: String,
    /// Name of the agent currently holding the token (for END → Done).
    current_agent: Option<String>,
```

Les initialiser dans `Workroom::new()` (`marker_buf: String::new(), current_agent: None`).

Ajouter les méthodes :

```rust
    /// Push a state transition for an agent, updating timestamps + history.
    fn set_state(&mut self, name: &str, state: AgentState) {
        if let Some(agent) = self.agents.iter_mut().find(|a| a.name == name) {
            match state {
                AgentState::Working | AgentState::Delegating => {
                    if agent.started_at.is_none() {
                        agent.started_at = Some(Instant::now());
                    }
                }
                AgentState::Done => {
                    agent.finished_at = Some(Instant::now());
                }
                AgentState::Idle => {}
            }
            agent.transitions.push((state.clone(), Instant::now()));
            agent.state = state;
        }
    }

    fn coordinator_name(&self) -> Option<String> {
        self.agents
            .iter()
            .find(|a| a.role == AgentRole::Coordinator)
            .map(|a| a.name.clone())
    }

    /// Apply streamed text to the workroom FSM by extracting ArmadAI protocol
    /// markers. Buffers input so a marker split across chunks is handled.
    pub fn apply_stream_text(&mut self, chunk: &str) {
        self.marker_buf.push_str(chunk);

        // Extract complete markers `<!--ARMADAI_...-->` in order.
        loop {
            let Some(start) = self.marker_buf.find("<!--ARMADAI_") else {
                // No marker opener: keep only a possible partial opener tail.
                keep_tail(&mut self.marker_buf, "<!--ARMADAI_");
                break;
            };
            let Some(rel_end) = self.marker_buf[start..].find("-->") else {
                // Opener present but not yet closed: retain from `start`.
                self.marker_buf = self.marker_buf[start..].to_string();
                break;
            };
            let end = start + rel_end;
            let inner = self.marker_buf[start + 4..end].to_string(); // strip "<!--"
            self.apply_marker(inner.trim());
            self.marker_buf = self.marker_buf[end + 3..].to_string(); // strip "-->"
        }
    }

    /// Apply a single marker body (e.g. `ARMADAI_DELEGATE:core-specialist`).
    fn apply_marker(&mut self, body: &str) {
        if let Some(target) = body.strip_prefix("ARMADAI_DELEGATE:") {
            let target = target.trim().to_string();
            if let Some(coord) = self.coordinator_name() {
                self.set_state(&coord, AgentState::Delegating);
            }
            self.set_state(&target, AgentState::Working);
            self.current_agent = Some(target);
            self.visible = true;
        } else if let Some(status) = body.strip_prefix("ARMADAI_META:status=") {
            if let Some(cur) = self.current_agent.clone() {
                if let Some(agent) = self.agents.iter_mut().find(|a| a.name == cur) {
                    agent.last_action = Some(status.trim().to_string());
                }
            }
        } else if body.trim() == "ARMADAI_END" {
            // NOTE: ArmadAI markers can be echoed in Claude Code recaps
            // (e.g. `| ... recap ... <!--ARMADAI_END-->`), so a stray END is a
            // false positive. Only act when an agent is genuinely active; the
            // authoritative end-of-turn completion is `on_complete()`.
            if let Some(cur) = self.current_agent.clone()
                && let Some(agent) = self.agents.iter().find(|a| a.name == cur)
                && matches!(agent.state, AgentState::Working | AgentState::Delegating)
            {
                self.set_state(&cur, AgentState::Done);
                // Control returns to the coordinator.
                self.current_agent = self.coordinator_name();
            }
        }
    }
```

Ajouter la fonction libre `keep_tail` (retient un préfixe partiel du marqueur) dans le fichier :

```rust
/// Retain only a trailing partial-prefix of `needle` at the end of `buf`
/// (so a marker split across chunks can still be completed next call).
fn keep_tail(buf: &mut String, needle: &str) {
    let max = needle.len().min(buf.len());
    for k in (1..=max).rev() {
        if buf.is_char_boundary(buf.len() - k) && needle.starts_with(&buf[buf.len() - k..]) {
            *buf = buf[buf.len() - k..].to_string();
            return;
        }
    }
    buf.clear();
}
```

Router `parse_streaming_line` vers la FSM :

```rust
    /// Parse a streaming line for ArmadAI protocol markers.
    pub fn parse_streaming_line(&mut self, line: &str) {
        self.apply_stream_text(line);
    }
```

(La logique `on_delegate` reste utilisée par d'anciens chemins ; ne pas la supprimer en Lot A.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --no-default-features --features tui -p armadai apply_stream_text`
Expected: PASS (3 tests).

- [ ] **Step 5: Full workroom suite (regression) + clippy + fmt**

Run: `cargo test --no-default-features --features tui -p armadai workroom`
Expected: PASS (existing tests unaffected).
Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/shell/workroom.rs
git commit -m "feat(shell): marker-driven workroom FSM (apply_stream_text) with chunk buffering"
```

---

## Notes pour l'implémenteur

- Ne PAS toucher `src/shell/app.rs` en Lot A (le recâblage des appels + suppression de `detect_mentions` est le Lot B). `detect_mentions` reste présent et appelé — c'est voulu (compilation préservée, aucun changement de comportement observable tant que `app.rs` ne route pas vers `apply_stream_text`).
- La FSM est testée en isolation (feed de chunks). Le rendu (`render`) n'est pas modifié ici.
- `keep_tail` gère les marqueurs scindés entre chunks (`ARMADAI_DELE` + `GATE:...-->`). Vérifier les bornes de caractères UTF-8 (`is_char_boundary`).
- `AgentState` doit dériver `Clone` (déjà le cas) pour l'historique des transitions.
