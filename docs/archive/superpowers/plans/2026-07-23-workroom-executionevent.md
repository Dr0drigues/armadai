# Workroom piloté par les événements cœur (agnostique provider) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Animer la Workroom depuis le flux cœur `RunEvent` d'ArmadAI (agnostique du provider) pendant `armadai run --orchestrate`, en réutilisant les layouts/drill-down de T3.

**Architecture:** Projection pure `Workroom::on_run_event_at(&RunEvent, Instant)` (mapping event→état, horloge injectée pour des tests déterministes) ; `WorkroomSink` (implémente `EventSink`) qui pousse les `RunEvent` clonés dans un `tokio::sync::mpsc::UnboundedSender` ; renderer live ratatui sur `armadai run --orchestrate` (feature `tui`) qui lance l'orchestration en tâche async, draine le channel, redessine, restaure le terminal et imprime le résultat.

**Tech Stack:** Rust edition 2024, ratatui/crossterm, tokio, feature `tui`.

## Global Constraints
- Gate à chaque tâche : clippy **3 modes** (`--no-default-features --features tui`, `--features tui,providers-api`, `--features tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test --no-default-features --features tui` (+ `--features tui,storage` pour l'e2e — voir Task 4).
- **Ne PAS toucher le chemin headless existant** : `--json` / `--quiet` / stdout non-TTY → comportement inchangé (sink stdout/json via `make_sink`, aucune TUI). Les e2e existants (`--json`) doivent rester verts.
- **Ne PAS toucher le mode relais du shell** (`src/shell/app.rs` marqueurs / `apply_stream_text` / `parse_streaming_line`) — hors périmètre.
- Projection **pure et déterministe** : `on_run_event_at` prend `now: Instant` en paramètre (jamais `Instant::now()` dans la projection). Tests sans accès env.
- Réutiliser l'API Workroom existante (`src/shell/workroom.rs`) : `AgentState {Working,Delegating,Done,Idle}`, `AgentRole {Coordinator,Lead,Agent}`, `TrackedAgent {name,state,role,started_at,finished_at,spinner_frame,last_action,transitions}`, `on_complete`, `set_state`, `coordinator_name`, `tick`, `init_from_config`. **Ne pas ajouter de variante `AgentState`** (pas d'état Error : `Error` → `last_action` sur l'agent courant).
- Branche depuis `release/1.0.0`. Une PR, revue indépendante + validation visuelle Dimitri avant merge.

## File Structure
- `src/core/events.rs` — `#[derive(Clone)]` sur `RunEvent` (Task 2).
- `src/shell/workroom.rs` — `on_run_event_at` + helpers privés + tests unitaires (Task 1).
- `src/shell/run_view.rs` *(nouveau)* — `WorkroomSink` (Task 2) + le renderer live `run_orchestration_tui(...)` (Task 3).
- `src/shell/mod.rs` — `pub mod run_view;` (Task 2).
- `src/cli/mod.rs` — flag `--no-tui` sur `Run` (Task 3).
- `src/cli/run.rs` — branchement TUI vs headless dans `execute` (Task 3).
- `tests/e2e/` — helper de rejeu de projection + cas (Task 4).

---

### Task 1: Projection pure `Workroom::on_run_event_at`

**Files:** Modify `src/shell/workroom.rs` (imports, `impl Workroom`, `mod tests`).

**Interfaces:**
- Consumes: `crate::core::events::RunEvent` (variantes ci-dessous), `std::time::Instant`.
- Produces: `pub fn on_run_event_at(&mut self, ev: &RunEvent, now: Instant)`.

- [ ] **Step 1 : Écrire les tests qui échouent** — dans `mod tests` de `src/shell/workroom.rs` :

```rust
    use crate::core::events::RunEvent;
    use std::time::Instant;

    fn rs(agents: &[&str]) -> RunEvent {
        RunEvent::RunStart {
            v: 1,
            agents: agents.iter().map(|s| s.to_string()).collect(),
            prov: "fake".into(),
            model: "m".into(),
            in_chars: 0,
        }
    }

    #[test]
    fn on_run_event_seeds_and_transitions() {
        let mut wr = Workroom::new();
        let t = Instant::now();
        wr.on_run_event_at(&rs(&["dev-lead", "core-specialist"]), t);
        assert_eq!(wr.agents.len(), 2);
        assert!(wr.is_visible());

        wr.on_run_event_at(&RunEvent::Delegate { from: "dev-lead".into(), to: "core-specialist".into() }, t);
        assert_eq!(wr.agents.iter().find(|a| a.name == "dev-lead").unwrap().state, AgentState::Delegating);

        wr.on_run_event_at(&RunEvent::AgentStart { agent: "core-specialist".into(), prov: "fake".into(), model: "m".into() }, t);
        assert_eq!(wr.agents.iter().find(|a| a.name == "core-specialist").unwrap().state, AgentState::Working);

        wr.on_run_event_at(&RunEvent::AgentEnd { agent: "core-specialist".into(), tin: 1, tout: 2, cost: 0.0, content: "done reticulating\nsplines".into() }, t);
        let a = wr.agents.iter().find(|a| a.name == "core-specialist").unwrap();
        assert_eq!(a.state, AgentState::Done);
        assert_eq!(a.last_action.as_deref(), Some("done reticulating"));
    }

    #[test]
    fn on_run_event_result_finalizes() {
        let mut wr = Workroom::new();
        let t = Instant::now();
        wr.on_run_event_at(&rs(&["a"]), t);
        wr.on_run_event_at(&RunEvent::AgentStart { agent: "a".into(), prov: "f".into(), model: "m".into() }, t);
        wr.on_run_event_at(&RunEvent::Result { content: "x".into(), tin: 0, tout: 0, cost: 0.0, agents: 1 }, t);
        // on_complete turns any Working agent into Done.
        assert_eq!(wr.agents.iter().find(|a| a.name == "a").unwrap().state, AgentState::Done);
    }

    #[test]
    fn on_run_event_unknown_variants_are_noops() {
        let mut wr = Workroom::new();
        let t = Instant::now();
        wr.on_run_event_at(&rs(&["a"]), t);
        wr.on_run_event_at(&RunEvent::Warning { code: "w".into(), from: None, to: None }, t);
        wr.on_run_event_at(&RunEvent::Route { agent: "a".into(), tier: "fast".into(), reason: "r".into() }, t);
        assert_eq!(wr.agents.iter().find(|a| a.name == "a").unwrap().last_action.as_deref(), Some("→ fast"));
    }
```

- [ ] **Step 2 : Lancer — échoue** (`no method on_run_event_at`).

Run: `cargo test --no-default-features --features tui workroom::tests::on_run_event 2>&1 | tail -5`
Expected: erreur de compilation.

- [ ] **Step 3 : Ajouter l'import** en tête de `src/shell/workroom.rs` (après les `use` existants) :

```rust
use crate::core::events::RunEvent;
```

- [ ] **Step 4 : Implémenter la projection + helpers** dans `impl Workroom` :

```rust
    /// Apply one core `RunEvent` to the workroom state (provider-agnostic
    /// projection). `now` is injected so timing is deterministic in tests;
    /// the live renderer passes `Instant::now()`.
    pub fn on_run_event_at(&mut self, ev: &RunEvent, now: Instant) {
        match ev {
            RunEvent::RunStart { agents, .. } => {
                for name in agents {
                    if !self.agents.iter().any(|a| a.name == *name) {
                        self.agents.push(TrackedAgent {
                            name: name.clone(),
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
                self.visible = true;
            }
            RunEvent::AgentStart { agent, .. } => self.mark_working(agent, now),
            RunEvent::AgentEnd { agent, content, .. } => {
                self.mark_done(agent, now);
                let first = content.lines().next().unwrap_or("").trim();
                if !first.is_empty() {
                    self.set_action(agent, first.to_string());
                }
            }
            RunEvent::Delegate { from, to } => {
                self.transition(from, AgentState::Delegating, now);
                self.current_agent = Some(to.clone());
            }
            RunEvent::NestedStart { team_lead, .. } => self.transition(team_lead, AgentState::Delegating, now),
            RunEvent::NestedEnd { team_lead } => self.transition(team_lead, AgentState::Done, now),
            RunEvent::AgentSelect { selected, .. } => {
                for a in selected {
                    self.mark_working(a, now);
                }
            }
            RunEvent::Vote { agent, conf } => self.set_action(agent, format!("vote {conf:.2}")),
            RunEvent::Board { agent, kind } => self.set_action(agent, format!("board {kind}")),
            RunEvent::Route { agent, tier, .. } => self.set_action(agent, format!("→ {tier}")),
            RunEvent::Result { .. } => self.on_complete(),
            RunEvent::Error { msg, .. } => {
                if let Some(cur) = self.current_agent.clone() {
                    self.set_action(&cur, format!("error: {msg}"));
                }
            }
            RunEvent::Warning { .. } => {}
        }
    }

    fn mark_working(&mut self, name: &str, now: Instant) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == name) {
            a.state = AgentState::Working;
            a.started_at.get_or_insert(now);
            a.transitions.push((AgentState::Working, now));
        }
    }

    fn mark_done(&mut self, name: &str, now: Instant) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == name) {
            a.state = AgentState::Done;
            a.finished_at = Some(now);
            a.transitions.push((AgentState::Done, now));
        }
    }

    fn transition(&mut self, name: &str, state: AgentState, now: Instant) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == name) {
            if state == AgentState::Delegating && a.started_at.is_none() {
                a.started_at = Some(now);
            }
            a.transitions.push((state.clone(), now));
            a.state = state;
        }
    }

    fn set_action(&mut self, name: &str, action: String) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.name == name) {
            a.last_action = Some(action);
        }
    }
```

Note : si `AgentState` ne dérive pas `Clone`, l'ajouter (`#[derive(Debug, Clone, PartialEq)]`) — vérifier la déclaration `enum AgentState` (~ligne 21). Réutiliser les champs de `TrackedAgent` exactement comme `init_from_config` les construit.

- [ ] **Step 5 : Lancer — passe**

Run: `cargo test --no-default-features --features tui workroom:: 2>&1 | tail -8`
Expected: nouveaux tests verts, existants inchangés.

- [ ] **Step 6 : Gate + commit**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
git add src/shell/workroom.rs
git commit -m "feat(workroom): pure projection of core RunEvent stream (on_run_event_at)"
```

---

### Task 2: `RunEvent: Clone` + `WorkroomSink`

**Files:** Modify `src/core/events.rs` ; Create `src/shell/run_view.rs` ; Modify `src/shell/mod.rs`.

**Interfaces:**
- Consumes: `RunEvent`, `EventSink` (`src/core/events.rs`), `Workroom::on_run_event_at` (Task 1).
- Produces: `pub struct WorkroomSink { tx: tokio::sync::mpsc::UnboundedSender<RunEvent> }` impl `EventSink` ; `WorkroomSink::new() -> (Self, UnboundedReceiver<RunEvent>)`.

- [ ] **Step 1 : Rendre `RunEvent` clonable.** Dans `src/core/events.rs`, sur `enum RunEvent` (~ligne 6) : `#[derive(Debug, Serialize)]` → `#[derive(Debug, Clone, Serialize)]`. (Tous les champs — String/Vec<String>/u32/f64/f32/usize — sont Clone.)

- [ ] **Step 2 : Écrire le test d'intégration qui échoue** — créer `src/shell/run_view.rs` avec, pour l'instant, seulement le module de test :

```rust
#![cfg(feature = "tui")]

// (implementation added in Step 4)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::{EventSink, RunEvent};
    use crate::shell::workroom::Workroom;
    use std::time::Instant;

    #[test]
    fn sink_forwards_events_to_projection() {
        let (sink, mut rx) = WorkroomSink::new();
        sink.emit(&RunEvent::RunStart { v: 1, agents: vec!["a".into(), "b".into()], prov: "f".into(), model: "m".into(), in_chars: 0 });
        sink.emit(&RunEvent::AgentStart { agent: "a".into(), prov: "f".into(), model: "m".into() });
        sink.emit(&RunEvent::AgentEnd { agent: "a".into(), tin: 0, tout: 0, cost: 0.0, content: "hi".into() });
        drop(sink);

        let mut wr = Workroom::new();
        let now = Instant::now();
        while let Ok(ev) = rx.try_recv() {
            wr.on_run_event_at(&ev, now);
        }
        let agents = wr.agents_for_test();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents.iter().find(|a| a.name == "a").unwrap().state, crate::shell::workroom::AgentState::Done);
    }
}
```

Note : rendre `Workroom::agents` accessible aux tests via `agents_for_test()` (déjà présent) et exporter `AgentState` (déjà `pub`).

- [ ] **Step 3 : Déclarer le module** — dans `src/shell/mod.rs`, ajouter `pub mod run_view;` (sous le même `#[cfg(feature = "tui")]` que les autres modules shell si applicable).

- [ ] **Step 4 : Implémenter `WorkroomSink`** en tête de `src/shell/run_view.rs` (avant le `mod tests`) :

```rust
use crate::core::events::{EventSink, RunEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// An `EventSink` that forwards a clone of every `RunEvent` into a channel,
/// so a TUI render loop can drain and project them onto a `Workroom`.
pub struct WorkroomSink {
    tx: UnboundedSender<RunEvent>,
}

impl WorkroomSink {
    pub fn new() -> (Self, UnboundedReceiver<RunEvent>) {
        let (tx, rx) = unbounded_channel();
        (Self { tx }, rx)
    }
}

impl EventSink for WorkroomSink {
    fn emit(&self, ev: &RunEvent) {
        // Receiver gone (TUI exited) → drop silently; the run still completes.
        let _ = self.tx.send(ev.clone());
    }
}
```

- [ ] **Step 5 : Lancer — passe**

Run: `cargo test --no-default-features --features tui shell::run_view 2>&1 | tail -6`
Expected: `sink_forwards_events_to_projection` vert.

- [ ] **Step 6 : Gate 3 modes + commit** (Clone sur RunEvent touche les 3 modes) :

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
git add src/core/events.rs src/shell/run_view.rs src/shell/mod.rs
git commit -m "feat(shell): WorkroomSink bridges the core event stream to the workroom"
```

---

### Task 3: Renderer live sur `armadai run --orchestrate` + flag `--no-tui`

**Files:** Modify `src/shell/run_view.rs` (renderer) ; `src/cli/mod.rs` (flag) ; `src/cli/run.rs` (branchement).

**Interfaces:**
- Consumes: `WorkroomSink` (Task 2), `Workroom` (rendu T3), `crate::core::events::EventSink`.
- Produces: `pub async fn run_orchestration_tui(run: impl FnOnce(std::sync::Arc<dyn EventSink>) -> F, config_yaml: Option<String>) -> anyhow::Result<()>` où `F: Future<Output = anyhow::Result<()>>` — lance `run` avec un `WorkroomSink`, affiche la Workroom live, restaure le terminal.

- [ ] **Step 1 : Ajouter le flag `--no-tui`** dans `src/cli/mod.rs`, variante `Run { … }`, après `quiet` :

```rust
        /// Disable the live orchestration TUI (force plain headless output)
        #[arg(long = "no-tui")]
        no_tui: bool,
```
et propager dans le handler `Command::Run { … }` (~ligne 478) jusqu'à `run::execute(...)` (ajouter le paramètre `no_tui`).

- [ ] **Step 2 : Implémenter le renderer** dans `src/shell/run_view.rs` :

```rust
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::shell::workroom::Workroom;

/// Run an orchestration (`run`) while showing a live Workroom TUI fed by its
/// event stream. Restores the terminal on exit (including on error).
pub async fn run_orchestration_tui<F>(
    run: impl FnOnce(Arc<dyn EventSink>) -> F,
    config_yaml: Option<String>,
) -> anyhow::Result<()>
where
    F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let (sink, mut rx) = WorkroomSink::new();
    let sink: Arc<dyn EventSink> = Arc::new(sink);

    // Seed roles from the orchestration config if available (RunStart carries
    // no roles); otherwise the flotte stays flat.
    let mut workroom = Workroom::new();
    if let Some(cfg) = config_yaml {
        workroom.init_from_config(&cfg);
    }
    workroom.set_visible(true);

    // Launch the orchestration in the background.
    let handle = tokio::spawn(run(sink));

    // Enter alternate screen (mirrors src/shell/app.rs).
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let render_result = run_loop(&mut terminal, &mut workroom, &mut rx, handle).await;

    // Always restore the terminal.
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    render_result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    workroom: &mut Workroom,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<RunEvent>,
    mut handle: tokio::task::JoinHandle<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let mut final_content: Option<String> = None;
    loop {
        // Drain all pending events.
        while let Ok(ev) = rx.try_recv() {
            if let RunEvent::Result { content, .. } = &ev {
                final_content = Some(content.clone());
            }
            workroom.on_run_event_at(&ev, Instant::now());
        }
        workroom.tick();
        terminal.draw(|f| workroom.render(f, f.area()))?;

        // Input: Ctrl+W focus/drill-down (already implemented), Ctrl+C/q quit.
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            match k.code {
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    handle.abort();
                    break;
                }
                KeyCode::Char('w') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    workroom.set_focused(!workroom.is_focused());
                }
                KeyCode::Up | KeyCode::Char('k') if workroom.is_focused() => workroom.select_prev(),
                KeyCode::Down | KeyCode::Char('j') if workroom.is_focused() => workroom.select_next(),
                KeyCode::Char('q') if !workroom.is_focused() => break,
                _ => {}
            }
        }

        // Exit once the orchestration finished and the channel is drained.
        if handle.is_finished() && rx.is_empty() {
            // Apply any last events.
            while let Ok(ev) = rx.try_recv() {
                workroom.on_run_event_at(&ev, Instant::now());
            }
            break;
        }
    }

    // Propagate the orchestration's result / print final content after restore.
    let outcome = handle.await;
    match outcome {
        Ok(Ok(())) => {
            if let Some(content) = final_content {
                // Printed after the caller restores the terminal (see caller).
                PRINT_AFTER.with(|p| *p.borrow_mut() = Some(content));
            }
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(join_err) if join_err.is_cancelled() => Ok(()), // Ctrl+C abort
        Err(join_err) => Err(anyhow::anyhow!(join_err)),
    }
}
```

Note d'implémentation : le renvoi du `final_content` à imprimer après restauration du terminal peut se faire plus simplement en **retournant `Option<String>`** depuis `run_orchestration_tui` et en laissant `cli/run.rs` l'imprimer (préférer ça au `thread_local PRINT_AFTER` esquissé ci-dessus — le remplacer par un `-> anyhow::Result<Option<String>>`). Garder la restauration terminal AVANT le `println!`.

- [ ] **Step 3 : Brancher dans `src/cli/run.rs::execute`.** Au début d'`execute`, décider le mode :

```rust
    let use_tui = orchestrate.is_some()
        && !json
        && !quiet
        && !no_tui
        && std::io::IsTerminal::is_terminal(&std::io::stdout());

    #[cfg(feature = "tui")]
    if use_tui {
        // Load project orchestration config (for role seeding), best-effort.
        let cfg_yaml = std::fs::read_to_string(".armadai/config.yaml")
            .or_else(|_| std::fs::read_to_string("armadai.yaml"))
            .ok();
        let (agent_name, input, pipe, orchestrate, max_content, route, tags) =
            (agent_name.clone(), input.clone(), pipe.clone(), orchestrate.clone(), max_content, route.clone(), tags.clone());
        let printed = crate::shell::run_view::run_orchestration_tui(
            move |sink| async move {
                run_inner(agent_name, input, pipe, orchestrate, true, false, false, max_content, route, tags, dry_run, &sink).await
            },
            cfg_yaml,
        ).await;
        return match printed {
            Ok(Some(content)) => { println!("{content}"); Ok(()) }
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
    }
    // else: existing headless path unchanged (make_sink, run_inner).
```

Adapter la capture des variables au vrai code d'`execute` (les noms/déplacements exacts). `IsTerminal` vient de `std::io::IsTerminal`.

- [ ] **Step 4 : Compiler + gate 3 modes** (le renderer ne se teste pas unitairement — validation visuelle) :

Run:
```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | tail -5
```
Expected: `No issues found` (3 modes) + suite verte.

- [ ] **Step 5 : Commit**

```bash
git add src/shell/run_view.rs src/cli/mod.rs src/cli/run.rs
git commit -m "feat(run): live workroom TUI for orchestrated runs (default on TTY, --no-tui to disable)"
```

---

### Task 4: e2e systématiques (assertions du flux par pattern + `--no-tui`)

**Contexte technique confirmé** : `armadai` est **binary-only** (pas de `src/lib.rs`) et le harnais e2e (`tests/e2e/`, cible `tests/e2e.rs` → `cargo test --test e2e`) **n'importe AUCUN type `armadai::`** — il lance le vrai binaire avec `fake-claude` et asserte sur la séquence JSONL `--json`. Le rejeu Rust de la projection est donc **non viable** en e2e ; on valide **le flux `RunEvent` que la projection consomme** via les assertions `events` (sous-séquence ordonnée, sous-ensemble de champs — cf. les tests de `evaluate` dans `runner.rs`). La projection elle-même (`on_run_event_at`) est déjà couverte unitairement en Task 1.

**Files:** Modify `tests/e2e/cases/{hierarchical,blackboard,ring}.yaml` ; Create `tests/e2e/cases/no-tui.yaml`.

**Interfaces:**
- Consumes: le format de cas e2e existant (`setup`/`fake`/`expect.events` — cf. `tests/e2e/case.rs`) et le vocabulaire d'événements (`t: run_start | agent_start | agent_end | delegate | vote | board | nested_start | result | …`, clés courtes de `RunEvent` : `agent`, `from`, `to`).

- [ ] **Step 1 : Comprendre le harnais + confirmer l'invocation** — lire `tests/e2e/runner.rs` et `tests/e2e.rs` : comment les cas `cases/*.yaml` sont découverts et exécutés, et avec quelles features le binaire `armadai` est construit pour l'e2e (confirmer la commande exacte à lancer en Step 5). Vérifier le format `expect.events` (sous-séquence, match de sous-ensemble de champs).

- [ ] **Step 2 : Étendre `hierarchical.yaml`** — dans `expect.events`, compléter la sous-séquence pour asservir, pour CHAQUE specialist délégué, la paire `delegate` puis `agent_start`/`agent_end` (c'est l'info exacte que `on_run_event_at` transforme en Working→Done). Respecter le format existant (liste ordonnée, sous-ensemble de champs) et le vocabulaire réel du cas. Exemple d'entrées à ajouter (adapter aux noms d'agents du cas) :

```yaml
  events:
    - { t: run_start }
    - { t: delegate, from: coordinator, to: core-specialist }
    - { t: agent_start, agent: core-specialist }
    - { t: agent_end, agent: core-specialist }
    - { t: result }
```

- [ ] **Step 3 : Étendre `blackboard.yaml` et `ring.yaml`** — même principe, avec le vocabulaire propre à chaque pattern :
  - blackboard : asservir `agent_start`/`agent_end` (et `board`/`agent_select` s'ils sont émis) pour les agents participants.
  - ring : asservir la circulation — `agent_start`/`agent_end` dans l'ordre de l'anneau, et `vote` en phase de vote (le cas `ring.yaml` script déjà les phases propose/vote).
  Ne pas retirer d'assertions existantes ; ajouter les entrées manquantes en gardant la sous-séquence ordonnée cohérente avec la sortie réelle (lancer le cas pour caler l'ordre).

- [ ] **Step 4 : Cas `no-tui.yaml`** — créer `tests/e2e/cases/no-tui.yaml`, aligné sur un cas ring minimal, avec le flag `--no-tui` ajouté ; objectif : prouver que `--no-tui` laisse `exit_code: 0` et la séquence `events` **identiques** au comportement headless (le harnais capture stdout non-TTY → déjà headless, `--no-tui` ne doit rien changer). Réutiliser des `fake.rules` d'un ring minimal (s'aligner sur `ring.yaml` pour les phases propose/vote) :

```yaml
name: no-tui
weight: 1
setup:
  pattern: ring
  agents: [t-a, t-b]
  flags: ["--no-tui"]
  input: "say hi"
fake:
  rules:
    - match: { prompt_contains: "ACTION: <type>" }
      respond: "ACTION: PROPOSE\nCONTENT: hi"
    - match: { prompt_contains: "Synthesize the contributions" }
      respond: "CONFIDENCE: 0.9\nhi is fine."
    - match: {}
      respond: "unexpected"
expect:
  exit_code: 0
  events:
    - { t: run_start }
    - { t: result }
```
(Caler `fake.rules`/`events` sur le comportement réel du ring — lancer le cas et ajuster ; le point est `--no-tui` + exit 0 + séquence inchangée, pas la finesse ring. Confirmer que le harnais passe bien `--no-tui` via `setup.flags`.)

- [ ] **Step 5 : Lancer l'e2e**

Run: `cargo test --test e2e <features confirmées en Step 1> 2>&1 | tail -20`
Expected: `hierarchical`, `blackboard`, `ring`, `no-tui` verts ; aucun cas existant cassé.

- [ ] **Step 6 : Gate complet + commit**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --test e2e   # (avec les features confirmées en Step 1)
git add tests/e2e/
git commit -m "test(e2e): assert per-agent event stream across patterns + --no-tui headless parity"
```

---

## Self-Review
- **Spec coverage** : projection pure `on_run_event_at` (Task 1) ✓ ; `WorkroomSink`/channel + `RunEvent: Clone` (Task 2) ✓ ; renderer live + déclenchement TTY/`--no-tui`/json-quiet-headless-inchangé (Task 3) ✓ ; seed rôles depuis config, dégradation flotte plate (Task 3 Step 2/3) ✓ ; tests unitaires horloge injectée (Task 1), intégration sink→channel→projection (Task 2), e2e par pattern + `--no-tui` (Task 4) ✓ ; note fake-gemini = hors lot (spec) ✓ ; hors périmètre (relais shell, hooks/plugin) non touché ✓.
- **Placeholder scan** : le `PRINT_AFTER thread_local` de Task 3 Step 2 est explicitement remplacé par un retour `Option<String>` dans la note qui suit (pas un placeholder — deux formulations, la note tranche ; l'implémenteur retient le retour `Option<String>`). Task 4 ne dépend plus d'aucun type interne (binary-only confirmé) : assertions `events` YAML uniquement.
- **Type consistency** : `on_run_event_at(&mut self, &RunEvent, Instant)` défini Task 1, consommé Task 2 (test intégration) ; `WorkroomSink::new() -> (Self, UnboundedReceiver<RunEvent>)` défini Task 2, consommé Task 3 ; `RunEvent` variantes/champs repris verbatim de `src/core/events.rs` ; `AgentState`/`TrackedAgent`/`init_from_config`/`on_complete`/`tick`/`render` existants (vérifiés dans workroom.rs). `IsTerminal` via `std::io::IsTerminal`.
- **Risque tranché** : `armadai` binary-only + e2e sans accès aux types `armadai::` → confirmé ; Task 4 valide le flux via assertions `events` (voie B), la projection est couverte unitairement en Task 1. Plus de dépendance à un rejeu Rust en e2e.
- **Risque restant** : la boucle TUI (`run_loop`) mêle `crossterm::event::poll` (bloquant bref) et une tâche tokio — acceptable (poll court) ; l'implémenteur doit garantir la restauration terminal même en cas d'erreur (déjà structuré : restauration après `run_loop`, avant propagation de l'erreur).
