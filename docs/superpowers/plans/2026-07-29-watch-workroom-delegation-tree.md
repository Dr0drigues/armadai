# Workroom — arbre de délégation pour `armadai watch` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Faire apparaître les sous-agents délégués comme nœuds enfants dans le Workroom quand il est piloté par `armadai watch` (aujourd'hui seul le nœud racine `claude` s'affiche ; les délégations tombent en timeline).

**Architecture:** L'arbre « hierarchical » du Workroom est une indentation **par rôle** (`AgentRole::{Coordinator=0, Lead=1, Agent=2}`, via `tree_prefix`), sans champ parent. Deux changements : (A) le Workroom ajoute un nœud (rôle `Agent`) pour un agent inconnu vu en cours de run ; (B) `armadai watch` seed la racine en `Coordinator` via une config synthétique, pour que les sous-agents (rôle `Agent`) s'indentent dessous.

**Tech Stack:** Rust edition 2024, ratatui (Workroom, feature `tui`).

## Global Constraints

- **`armadai-core` NE CHANGE PAS** ; travail uniquement dans `crates/armadai/src/shell/workroom.rs` et `crates/armadai/src/cli/watch.rs`. Aucun changement de `RunEvent`.
- **Changement VISUEL du Workroom** → validation visuelle par Dimitri après merge (revalider `armadai watch --session mini` et `--session demo`).
- **Ne pas régresser le chemin normal `armadai run`** : les agents y sont pré-seedés par `init_from_config` avec leurs vrais rôles ; l'ajout-si-inconnu ne doit s'appliquer qu'aux agents RÉELLEMENT absents (un agent déjà présent garde son rôle).
- **Branche** : master-only, une branche pour l'enhancement, squash-merge, CI verte (confirmer 6/6 `pass`) + revue indé avant merge.
- **Gate CI** (workspace-wide) : fmt + clippy 3 combos `-D warnings` + tests 3 modes.
- `rust-analyzer` non fiable (ABI stale) → vérifier au compilateur. NE PAS `git add -A`. Commits/commentaires anglais, scope `plugin`/`tui`. Finir par `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

## Références (existant, à consommer verbatim)

- `crates/armadai/src/shell/workroom.rs` :
  - `enum AgentRole { Coordinator, Lead, Agent }` ; rang via `role_rank` (Coordinator=0, Lead=1, Agent=2) ; `tree_prefix(i)` indente les `Agent` sous un `Coordinator`.
  - `struct TrackedAgent { name, state, role, started_at, finished_at, spinner_frame, last_action, transitions }`.
  - `fn mark_working(&mut self, name, now)` / `mark_done` : **no-op si `name` inconnu** (`if let Some(a) = self.agents.iter_mut().find(...)`).
  - `on_run_event_at` : arme `RunStart{agents}` crée les nœuds (rôle `Agent`) ; `AgentStart{agent}` → `mark_working(agent)` ; `Delegate{from,to}` → `transition(from, Delegating)` + `current_agent = to`.
  - `fn init_from_config(&mut self, config_yaml: &str)` : parse `coordinator:` (→ rôle `Coordinator`) et les agents des teams (→ `Lead`/`Agent`).
- `crates/armadai/src/cli/watch.rs::execute` appelle `run_orchestration_tui(move |sink| async move { drive_session(picked, sink, true).await }, None, None)` — le 2ᵉ arg est `config_yaml: Option<String>`.

## File Structure

```
crates/armadai/src/shell/workroom.rs   # MODIFY — ajouter les nœuds inconnus (helper ensure_agent) ; AgentStart/Delegate
crates/armadai/src/cli/watch.rs        # MODIFY — passer une config synthétique `coordinator: claude`
```

---

## Task 1: Le Workroom matérialise les agents inconnus

**Files:**
- Modify: `crates/armadai/src/shell/workroom.rs`

**Interfaces:**
- Produces: comportement — après `AgentStart{agent}` ou `Delegate{from,to}`, tout agent absent du roster est ajouté comme `TrackedAgent { role: AgentRole::Agent, .. }`. Un agent déjà présent est inchangé (rôle préservé).

- [ ] **Step 1: Écrire le test (échec attendu)** — dans le `mod tests` de `workroom.rs`, ajouter :
```rust
#[test]
fn dynamically_delegated_agents_become_nodes() {
    use armadai_core::events::RunEvent;
    let t = std::time::Instant::now();
    let mut wr = Workroom::new();
    // watch-style: RunStart seeds only the root.
    wr.on_run_event_at(&RunEvent::RunStart {
        run_id: "r".into(), v: 1, agents: vec!["claude".into()],
        prov: "claude".into(), model: "m".into(), in_chars: 0,
    }, t);
    assert_eq!(wr.agents.len(), 1);
    // A delegation to an unknown agent must create its node.
    wr.on_run_event_at(&RunEvent::Delegate { from: "claude".into(), to: "core-specialist".into() }, t);
    wr.on_run_event_at(&RunEvent::AgentStart { agent: "core-specialist".into(), prov: "claude".into(), model: "m".into() }, t);
    assert!(wr.agents.iter().any(|a| a.name == "core-specialist"),
        "delegated subagent should appear as a node");
    // AgentStart for another unknown agent also creates it.
    wr.on_run_event_at(&RunEvent::AgentStart { agent: "qa-specialist".into(), prov: "claude".into(), model: "m".into() }, t);
    assert_eq!(wr.agents.len(), 3, "claude + core-specialist + qa-specialist");
    // New nodes are role Agent (indented under a coordinator in the tree).
    let core = wr.agents.iter().find(|a| a.name == "core-specialist").unwrap();
    assert_eq!(core.role, AgentRole::Agent);
}
```

- [ ] **Step 2: Vérifier l'échec** — `cargo test -p armadai --features tui dynamically_delegated_agents_become_nodes` → FAIL (le sous-agent n'est pas ajouté ; `agents.len()` reste 1).

- [ ] **Step 3: Implémenter.** Dans `workroom.rs`, ajouter un helper qui crée le nœud si absent (rôle `Agent`) :
```rust
/// Ensure an agent node exists (role Agent for dynamically-appearing agents,
/// e.g. subagents delegated during an `armadai watch` run where no config
/// pre-seeded the roster). No-op if the agent already exists (its role is
/// preserved — the config path is unaffected).
fn ensure_agent(&mut self, name: &str) {
    if !self.agents.iter().any(|a| a.name == name) {
        self.agents.push(TrackedAgent {
            name: name.to_string(),
            state: AgentState::Idle,
            role: AgentRole::Agent,
            started_at: None,
            finished_at: None,
            spinner_frame: 0,
            last_action: None,
            transitions: Vec::new(),
        });
    }
}
```
Puis, dans `on_run_event_at`, appeler `ensure_agent` avant de traiter l'agent :
- Arme `AgentStart { agent, .. }` : ajouter `self.ensure_agent(agent);` en première ligne (avant `self.mark_working(agent, now);`).
- Arme `Delegate { from, to }` : ajouter `self.ensure_agent(to);` (le `to` peut n'avoir jamais eu d'`AgentStart` encore ; `from` existe déjà — c'est la racine seedée). Garder le reste (`transition(from, Delegating, now)`, `current_agent = Some(to.clone())`).
- Arme `AgentEnd { agent, .. }` : ajouter `self.ensure_agent(agent);` en première ligne (robustesse — un end sans start connu crée quand même le nœud).

- [ ] **Step 4: Vérifier le succès** — `cargo test -p armadai --features tui dynamically_delegated_agents_become_nodes` → PASS. Lancer aussi la suite workroom : `cargo test -p armadai --features tui shell::workroom::` → tout PASS (pas de régression sur les tests existants, notamment ceux qui vérifient le roster seedé par config).

- [ ] **Step 5: Commit**
```bash
git add crates/armadai/src/shell/workroom.rs
git commit -m "fix(tui): Workroom materializes dynamically-delegated agents as nodes"
```

---

## Task 2: `armadai watch` seed la racine en Coordinator (arbre à 2 niveaux)

**Files:**
- Modify: `crates/armadai/src/cli/watch.rs`

**Interfaces:**
- Consumes: `run_orchestration_tui(run, config_yaml, explicit_pattern)` (le 2ᵉ arg `config_yaml: Option<String>`), le helper `ensure_agent` (Task 1) via les événements.
- Produces: le nœud racine `claude` a le rôle `Coordinator` → les sous-agents (`Agent`, Task 1) s'indentent dessous dans la vue hiérarchique.

- [ ] **Step 1: Écrire le test (échec attendu)** — dans le `mod tests` de `watch.rs`, ajouter un test qui vérifie le seeding coordinator via un `Workroom` construit à partir de la même config synthétique que `execute` :
```rust
#[test]
fn root_agent_is_seeded_as_coordinator() {
    use crate::shell::workroom::{AgentRole, Workroom};
    let mut wr = Workroom::new();
    wr.init_from_config(WATCH_ROOT_CONFIG);
    let claude = wr.agents.iter().find(|a| a.name == "claude")
        .expect("synthetic config seeds the root agent");
    assert_eq!(claude.role, AgentRole::Coordinator);
}
```
> Requiert d'exposer la constante de config synthétique (voir Step 3) et que `Workroom`/`AgentRole` soient accessibles depuis le test (ils sont `pub` dans `shell::workroom`).

- [ ] **Step 2: Vérifier l'échec** — `cargo test -p armadai --features tui root_agent_is_seeded_as_coordinator` → FAIL (constante inexistante / claude non seedé Coordinator).

- [ ] **Step 3: Implémenter.** Dans `watch.rs` :
```rust
/// Minimal synthetic project config so the Workroom seeds the transcript's
/// root agent ("claude") as the Coordinator — the delegated subagents (added
/// dynamically as role Agent, see Workroom::ensure_agent) then indent beneath
/// it in the hierarchical tree. A watched Claude Code session has no
/// armadai.yaml, so we synthesize the minimum init_from_config needs.
const WATCH_ROOT_CONFIG: &str = "coordinator: claude\n";
```
Puis, dans `execute`, remplacer l'appel TUI :
```rust
    let (_run_id, _content) = crate::shell::run_view::run_orchestration_tui(
        move |sink| async move { drive_session(picked, sink, true).await },
        Some(WATCH_ROOT_CONFIG.to_string()),
        None,
    )
    .await?;
    Ok(())
```
> Le `--json` path (headless) est inchangé (pas de Workroom, donc pas de rôle à seeder).

- [ ] **Step 4: Vérifier le succès** — `cargo test -p armadai --features tui watch::` → PASS (dont le nouveau test + les 2 existants). Vérifier que `init_from_config("coordinator: claude\n")` seede bien claude en Coordinator (le test le prouve).

- [ ] **Step 5: Commit**
```bash
git add crates/armadai/src/cli/watch.rs
git commit -m "feat(plugin): watch seeds the transcript root as Coordinator for the delegation tree"
```

---

## Invariant de fin

- `armadai watch --session <s>` : le Workroom affiche `claude` (Coordinator, racine) avec les sous-agents délégués indentés dessous (`core-specialist`, `qa-specialist`, … pour `mini` ; les 73 pour `demo`).
- Chemin `armadai run` inchangé (agents pré-seedés par config gardent leurs rôles ; `ensure_agent` no-op quand l'agent existe).
- `armadai-core` inchangé ; gate workspace-wide verte.
- **Validation visuelle Dimitri** sur `--session mini` (lisible) et `--session demo` (charge réelle).

## Hors périmètre

- Vraie profondeur d'arbre multi-niveaux (claude → dev-lead → specialist) : les sous-agents Claude Code vivent dans des fichiers séparés (`agent-<id>.jsonl`), non consommés en P1 → arbre à 2 niveaux (racine + délégués directs). Multi-niveaux = P2+.
- Tool calls individuels comme nœuds/lignes : hors périmètre (P1).

## Self-Review (rempli à l'écriture)

- **Couverture** : sous-agents en nœuds (Task 1) + racine Coordinator pour l'indentation (Task 2) = arbre à 2 niveaux, l'outcome validé.
- **Placeholders** : aucun ; code complet aux deux tâches.
- **Non-régression** : `ensure_agent` no-op si présent → chemin `run`/config intact (les tests workroom existants doivent rester verts — vérifié en Task 1 Step 4).
- **Cohérence** : `ensure_agent` (Task 1) + `WATCH_ROOT_CONFIG`/`init_from_config` (Task 2) ; `AgentRole::Coordinator/Agent` conformes à l'enum existant.
