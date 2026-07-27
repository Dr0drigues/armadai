# Design System → TUI T3 (Workroom adaptative, drill-down, nettoyage couleurs) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rendre la Workroom du shell adaptative au pattern d'orchestration (arbre hierarchical / colonnes blackboard / anneau ring) avec élargissement au focus, styliser le drill-down au DS, et éliminer tous les `Color::` littéraux non justifiés des vues/widgets TUI et de la Workroom.

**Architecture:** Un champ `pattern: OrchestrationPattern` sur `Workroom`, lu dans `init_from_config`. `render()` dispatche via une fonction pure et testable `layout_mode(inner_width) -> LayoutMode` sur `(pattern, focused, width)`, avec dégradation en liste compacte sous 44 cols. Le shell (`shell/tui.rs`) élargit la zone Workroom de 35 à 60 cols quand elle a le focus. Trois rendus dédiés produisent des `Vec<Line>`. Le popup de détail et les couleurs en dur résiduelles passent par les helpers de `src/theme.rs`.

**Tech Stack:** Rust edition 2024, ratatui, feature `tui`, enum `crate::core::orchestration::OrchestrationPattern`.

## Global Constraints

- Feature `tui`. Gate à chaque tâche : clippy **3 modes** (`--no-default-features --features tui`, `--features tui,providers-api`, `--features tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test --no-default-features --features tui`.
- **Thème accent-only** : accents laiton + signaux via tier ; textes/muted en couleurs NOMMÉES adaptatives (lisibles fond clair ET sombre). N'utiliser QUE les helpers de `src/theme.rs`, jamais de `Color::` littéral nouveau.
- **Objectif ferme T3c** : zéro `Color::Cyan/Magenta/Blue/Yellow/Rgb` littéral dans `src/tui/views/`, `src/tui/widgets/`, `src/shell/workroom.rs`. Les `Color::DarkGray/Gray/Green` nommés déjà cohérents peuvent rester.
- **Tests touchant l'environnement** : aucun test de ce plan ne lit l'env (le pattern vient d'une string passée en argument) → pas de sérialisation env requise ici, mais ne PAS introduire de lecture d'env dans les tests.
- **Composant** : `src/shell/workroom.rs` (feature-gated `#![cfg(feature = "tui")]`) + `src/shell/tui.rs` (largeur, popup) + `src/theme.rs` (extension `Glyphs`). Ne PAS toucher `armadai tui` (Orchestration tab).
- Branche de travail : partir de `release/1.0.0`. Une PR par sous-lot : **T3a = Tasks 1–3**, **T3b = Tasks 4–5**, **T3c = Tasks 6–7**. Revue indépendante + validation visuelle Dimitri (`armadai shell`) par sous-lot.

## File Structure

- `src/theme.rs` — étendre la struct `Glyphs` (connecteurs d'arbre, flèches, board) + ses deux jeux UNICODE/ASCII. (Task 1)
- `src/shell/workroom.rs` — champ `pattern`, parsing, `LayoutMode` + `layout_mode()`, `token_holder_index()`, refactor de `render()` en dispatch, les fns `compact_lines/hierarchical_lines/blackboard_lines/ring_lines`, nettoyage bordure. (Tasks 2–6)
- `src/shell/tui.rs` — largeur conditionnelle de la zone Workroom (Task 3) ; styling DS du popup (Task 6).
- `src/tui/views/palette.rs`, `src/tui/views/agent_detail.rs` — nettoyage couleurs (Task 7).

---

### Task 1: Étendre `Glyphs` (connecteurs d'arbre, flèches, board)

**Files:**
- Modify: `src/theme.rs:188-223` (struct `Glyphs`, consts `UNICODE`/`ASCII`, fn `glyphs`)

**Interfaces:**
- Produces: champs supplémentaires sur `Glyphs` — `tree_branch: &'static str`, `tree_last: &'static str`, `arrow_down: &'static str`, `arrow_up: &'static str`, `board: &'static str`. Accessibles via `theme::glyphs()`.

- [ ] **Step 1: Écrire le test qui échoue** — ajouter dans le module `#[cfg(test)] mod tests` de `src/theme.rs` :

```rust
    #[test]
    fn glyphs_expose_tree_and_flow_symbols() {
        let g = Glyphs::UNICODE;
        assert_eq!(g.tree_branch, "├─");
        assert_eq!(g.tree_last, "└─");
        assert_eq!(g.arrow_down, "↓");
        assert_eq!(g.arrow_up, "↑");
        assert_eq!(g.board, "▤");
        let a = Glyphs::ASCII;
        assert_eq!(a.tree_branch, "+-");
        assert_eq!(a.tree_last, "\\-");
        assert_eq!(a.arrow_down, "v");
        assert_eq!(a.arrow_up, "^");
        assert_eq!(a.board, "#");
    }
```

- [ ] **Step 2: Lancer le test — échoue à la compilation**

Run: `cargo test --no-default-features --features tui theme::tests::glyphs_expose 2>&1 | tail -5`
Expected: erreur `no field 'tree_branch' on type 'Glyphs'`.

- [ ] **Step 3: Étendre la struct et les deux jeux.** Remplacer le bloc `pub struct Glyphs { … }` + `impl Glyphs { const UNICODE … const ASCII … }` (actuellement lignes ~190-213) par :

```rust
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Glyphs {
    pub flag_running: &'static str,
    pub flag_ok: &'static str,
    pub bullet: &'static str,
    pub arrow: &'static str,
    pub tree_branch: &'static str,
    pub tree_last: &'static str,
    pub arrow_down: &'static str,
    pub arrow_up: &'static str,
    pub board: &'static str,
}

impl Glyphs {
    #[allow(dead_code)]
    const UNICODE: Glyphs = Glyphs {
        flag_running: "⚑",
        flag_ok: "◆",
        bullet: "●",
        arrow: "→",
        tree_branch: "├─",
        tree_last: "└─",
        arrow_down: "↓",
        arrow_up: "↑",
        board: "▤",
    };

    #[allow(dead_code)]
    const ASCII: Glyphs = Glyphs {
        flag_running: "*",
        flag_ok: "#",
        bullet: "-",
        arrow: "->",
        tree_branch: "+-",
        tree_last: "\\-",
        arrow_down: "v",
        arrow_up: "^",
        board: "#",
    };
}
```

- [ ] **Step 4: Lancer le test — passe**

Run: `cargo test --no-default-features --features tui theme::tests::glyphs_expose 2>&1 | tail -5`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 5: Gate + commit**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
git add src/theme.rs
git commit -m "feat(theme): extend Glyphs with tree connectors, flow arrows, board symbol"
```

---

### Task 2: Champ `pattern` + parsing dans `init_from_config`

**Files:**
- Modify: `src/shell/workroom.rs` (imports en tête ~ligne 15, struct `Workroom` ~ligne 56, `new()` ~ligne 71, `init_from_config` ~ligne 168)

**Interfaces:**
- Consumes: `crate::core::orchestration::OrchestrationPattern { Hierarchical, Blackboard, Ring }`.
- Produces: champ `Workroom.pattern: OrchestrationPattern` ; fn libre `parse_pattern(config_yaml: &str) -> OrchestrationPattern` (au niveau module, testable).

- [ ] **Step 1: Écrire les tests qui échouent** — ajouter dans `mod tests` de `src/shell/workroom.rs` :

```rust
    #[test]
    fn parse_pattern_reads_known_values() {
        assert_eq!(
            parse_pattern("orchestration:\n  pattern: blackboard\n"),
            OrchestrationPattern::Blackboard
        );
        assert_eq!(
            parse_pattern("orchestration:\n  pattern: \"ring\"\n"),
            OrchestrationPattern::Ring
        );
        assert_eq!(
            parse_pattern("orchestration:\n  pattern: Hierarchical\n"),
            OrchestrationPattern::Hierarchical
        );
    }

    #[test]
    fn parse_pattern_defaults_to_hierarchical() {
        assert_eq!(parse_pattern(""), OrchestrationPattern::Hierarchical);
        assert_eq!(
            parse_pattern("orchestration:\n  pattern: bogus\n"),
            OrchestrationPattern::Hierarchical
        );
    }

    #[test]
    fn init_from_config_sets_pattern() {
        let mut wr = Workroom::new();
        wr.init_from_config("orchestration:\n  pattern: ring\ncoordinator: dev-lead\n");
        assert_eq!(wr.pattern, OrchestrationPattern::Ring);
    }
```

- [ ] **Step 2: Lancer — échoue** (pas de champ `pattern`, pas de `parse_pattern`).

Run: `cargo test --no-default-features --features tui workroom::tests::parse_pattern 2>&1 | tail -5`
Expected: erreur de compilation.

- [ ] **Step 3: Ajouter l'import.** En tête de `src/shell/workroom.rs`, après `use crate::theme;` (ligne ~16) :

```rust
use crate::core::orchestration::OrchestrationPattern;
```

- [ ] **Step 4: Ajouter le champ.** Dans `pub struct Workroom { … }`, après `focused: bool,` :

```rust
    /// Active orchestration pattern (drives the focused layout).
    pattern: OrchestrationPattern,
```

Dans `impl Workroom { pub fn new() -> Self { Self { … } } }`, ajouter à l'initialisation, après `focused: false,` :

```rust
            pattern: OrchestrationPattern::Hierarchical,
```

- [ ] **Step 5: Ajouter la fn libre `parse_pattern`** (niveau module, juste avant `fn keep_tail`) :

```rust
/// Detect the orchestration pattern from a project config YAML string.
/// Tolerant line scan (matches the heuristic style of `init_from_config`):
/// reads the first `pattern:` value and maps it, defaulting to Hierarchical.
fn parse_pattern(config_yaml: &str) -> OrchestrationPattern {
    for line in config_yaml.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("pattern:") {
            let value = rest.trim().trim_matches('"').to_ascii_lowercase();
            return match value.as_str() {
                "blackboard" => OrchestrationPattern::Blackboard,
                "ring" => OrchestrationPattern::Ring,
                _ => OrchestrationPattern::Hierarchical,
            };
        }
    }
    OrchestrationPattern::Hierarchical
}
```

- [ ] **Step 6: Câbler dans `init_from_config`.** Au tout début de `pub fn init_from_config(&mut self, config_yaml: &str)`, juste après `self.agents.clear();` :

```rust
        self.pattern = parse_pattern(config_yaml);
```

- [ ] **Step 7: Lancer — passe**

Run: `cargo test --no-default-features --features tui workroom:: 2>&1 | tail -8`
Expected: les nouveaux tests passent, les existants aussi.

- [ ] **Step 8: Gate + commit**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
git add src/shell/workroom.rs
git commit -m "feat(workroom): parse orchestration pattern from project config"
```

---

### Task 3: `LayoutMode` + `layout_mode()` + dispatch `render` + largeur au focus + layout hierarchical

**Files:**
- Modify: `src/shell/workroom.rs` (nouveau enum `LayoutMode`, méthodes `layout_mode`/`role_rank`/`tree_prefix`, refactor `render` ~ligne 507)
- Modify: `src/shell/tui.rs:791-796` (largeur conditionnelle)

**Interfaces:**
- Consumes: `theme::border_style()`, `theme::heading()`, `theme::glyphs()`, `Glyphs.tree_branch/tree_last` (Task 1), `Workroom.pattern` (Task 2).
- Produces: `pub enum LayoutMode { Compact, Hierarchical, Blackboard, Ring }` ; `pub(crate) fn layout_mode(&self, inner_width: u16) -> LayoutMode` ; `fn compact_lines(&self) -> Vec<Line>` ; `fn hierarchical_lines(&self) -> Vec<Line>`.

- [ ] **Step 1: Écrire les tests qui échouent** — dans `mod tests` de `src/shell/workroom.rs` :

```rust
    #[test]
    fn layout_mode_compact_when_unfocused() {
        let mut wr = Workroom::new();
        wr.pattern = OrchestrationPattern::Ring;
        wr.set_focused(false);
        assert_eq!(wr.layout_mode(60), LayoutMode::Compact);
    }

    #[test]
    fn layout_mode_rich_when_focused_and_wide() {
        let mut wr = Workroom::new();
        wr.pattern = OrchestrationPattern::Ring;
        wr.set_focused(true);
        assert_eq!(wr.layout_mode(60), LayoutMode::Ring);
    }

    #[test]
    fn layout_mode_degrades_when_narrow() {
        let mut wr = Workroom::new();
        wr.pattern = OrchestrationPattern::Blackboard;
        wr.set_focused(true);
        assert_eq!(wr.layout_mode(30), LayoutMode::Compact);
    }
```

- [ ] **Step 2: Lancer — échoue** (pas de `LayoutMode`, pas de `layout_mode`).

Run: `cargo test --no-default-features --features tui workroom::tests::layout_mode 2>&1 | tail -5`
Expected: erreur de compilation.

- [ ] **Step 3: Ajouter l'enum + la logique de sélection.** Juste avant `impl Workroom` (ou en tête du fichier après les `use`), ajouter :

```rust
/// Which layout the workroom renders. Compact is the idle/narrow fallback;
/// the three rich modes only appear when focused and wide enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Compact,
    Hierarchical,
    Blackboard,
    Ring,
}

/// Minimum inner width (columns, borders excluded) to render a rich layout.
const RICH_WIDTH_MIN: u16 = 44;
```

Dans `impl Workroom`, ajouter :

```rust
    /// Decide the layout from pattern, focus, and available inner width.
    pub(crate) fn layout_mode(&self, inner_width: u16) -> LayoutMode {
        if !self.focused || inner_width < RICH_WIDTH_MIN {
            return LayoutMode::Compact;
        }
        match self.pattern {
            OrchestrationPattern::Hierarchical => LayoutMode::Hierarchical,
            OrchestrationPattern::Blackboard => LayoutMode::Blackboard,
            OrchestrationPattern::Ring => LayoutMode::Ring,
        }
    }
```

- [ ] **Step 4: Lancer — les 3 tests passent** (Blackboard/Ring existent déjà comme variantes ; leur rendu arrive en Task 4-5).

Run: `cargo test --no-default-features --features tui workroom::tests::layout_mode 2>&1 | tail -5`
Expected: `test result: ok. 3 passed`.

- [ ] **Step 5: Refactorer `render` en dispatch + extraire `compact_lines`.** Remplacer intégralement le corps de `pub fn render(&self, frame: &mut Frame, area: Rect)` (lignes ~507-588) par :

```rust
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let inner_width = area.width.saturating_sub(2); // exclude borders
        let lines = match self.layout_mode(inner_width) {
            LayoutMode::Compact => self.compact_lines(),
            LayoutMode::Hierarchical => self.hierarchical_lines(),
            // Blackboard/Ring rich layouts land in T3b; degrade until then.
            LayoutMode::Blackboard | LayoutMode::Ring => self.compact_lines(),
        };

        let panel = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border_style())
                .title(format!(" Workroom · {} ", self.pattern))
                .title_style(theme::heading()),
        );
        frame.render_widget(panel, area);
    }

    /// The idle/narrow layout: role-indented flat list (historical rendering).
    fn compact_lines(&self) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();
        for (idx, agent) in self.agents.iter().enumerate() {
            let (icon, state_str, style) = self.state_display(agent);
            let role_style = self.role_style(agent);
            let indent = match agent.role {
                AgentRole::Coordinator => "",
                AgentRole::Lead => "  ",
                AgentRole::Agent => "    ",
            };
            let is_selected = self.focused && idx == self.selected;
            let name_style = if is_selected {
                role_style.add_modifier(Modifier::REVERSED)
            } else {
                role_style
            };
            let marker = if is_selected { "▸ " } else { "" };
            lines.push(Line::from(vec![
                Span::raw(indent),
                Span::raw(marker),
                Span::styled(format!("{icon} "), style),
                Span::styled(&agent.name, name_style),
                Span::styled(format!("  {state_str}"), style),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                theme::muted(),
            )));
        }
        self.push_footer(&mut lines);
        lines
    }

    /// Shared state icon/label/style for an agent (used by every layout).
    fn state_display(&self, agent: &TrackedAgent) -> (String, String, Style) {
        match agent.state {
            AgentState::Working => {
                let spinner = SPINNER[agent.spinner_frame];
                let elapsed = agent
                    .started_at
                    .map(|s| format!(" {:.0}s", s.elapsed().as_secs_f64()))
                    .unwrap_or_default();
                (spinner.to_string(), format!("working{elapsed}"), theme::working())
            }
            AgentState::Delegating => {
                let spinner = SPINNER[agent.spinner_frame];
                (spinner.to_string(), "delegating".to_string(), theme::delegating())
            }
            AgentState::Done => ("✓".to_string(), "done".to_string(), theme::done()),
            AgentState::Idle => ("○".to_string(), "idle".to_string(), theme::muted()),
        }
    }

    /// Role-based name style.
    fn role_style(&self, agent: &TrackedAgent) -> Style {
        match agent.role {
            AgentRole::Coordinator => theme::role_coordinator(),
            AgentRole::Lead => theme::role_lead(),
            AgentRole::Agent => theme::role_agent(),
        }
    }

    /// Append the blank line + Ctrl+W hint footer shared by all layouts.
    fn push_footer(&self, lines: &mut Vec<Line>) {
        lines.push(Line::from(""));
        if self.focused {
            lines.push(Line::from(Span::styled(
                "Ctrl+W exit · j/k select",
                theme::muted(),
            )));
            lines.push(Line::from(Span::styled("Enter detail", theme::muted())));
        } else {
            lines.push(Line::from(Span::styled("Ctrl+W focus", theme::muted())));
        }
    }
```

Note : cette étape supprime le `border_style(Color::Rgb(48,54,61))` en dur (l.582) au profit de `theme::border_style()` — le nettoyage bordure de la Section 4 est donc fait ici. Retirer l'import `Color` s'il devient inutilisé (clippy le signalera).

- [ ] **Step 6: Ajouter `hierarchical_lines` + les helpers d'arbre.** Dans `impl Workroom` :

```rust
    /// Rank for tree nesting: Coordinator=0, Lead=1, Agent=2.
    fn role_rank(role: &AgentRole) -> u8 {
        match role {
            AgentRole::Coordinator => 0,
            AgentRole::Lead => 1,
            AgentRole::Agent => 2,
        }
    }

    /// Box-drawing connector prefix for the agent at `i` in the tree layout.
    /// Coordinators have no connector; a node is "last" when the next node
    /// climbs back to a shallower level (or the list ends).
    fn tree_prefix(&self, i: usize) -> String {
        let agent = &self.agents[i];
        if agent.role == AgentRole::Coordinator {
            return String::new();
        }
        let g = theme::glyphs();
        let rank = Self::role_rank(&agent.role);
        let is_last = i + 1 >= self.agents.len()
            || Self::role_rank(&self.agents[i + 1].role) < rank;
        let connector = if is_last { g.tree_last } else { g.tree_branch };
        let indent = if agent.role == AgentRole::Agent { "  " } else { "" };
        format!("{indent}{connector} ")
    }

    /// The focused hierarchical (pyramid) layout with box-drawing connectors.
    fn hierarchical_lines(&self) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();
        for (idx, agent) in self.agents.iter().enumerate() {
            let (icon, state_str, style) = self.state_display(agent);
            let role_style = self.role_style(agent);
            let is_selected = self.focused && idx == self.selected;
            let name_style = if is_selected {
                role_style.add_modifier(Modifier::REVERSED)
            } else {
                role_style
            };
            lines.push(Line::from(vec![
                Span::styled(self.tree_prefix(idx), theme::muted()),
                Span::styled(format!("{icon} "), style),
                Span::styled(&agent.name, name_style),
                Span::styled(format!("  {state_str}"), style),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                theme::muted(),
            )));
        }
        self.push_footer(&mut lines);
        lines
    }
```

- [ ] **Step 7: Écrire le test de `tree_prefix`** — dans `mod tests` :

```rust
    #[test]
    fn tree_prefix_marks_last_sibling() {
        let mut wr = Workroom::new();
        wr.init_from_config(
            "coordinator: lead\nagents:\n- a\n- b\n",
        );
        // index 0 = coordinator (no connector)
        assert_eq!(wr.tree_prefix(0), "");
        // last agent uses the "last" connector, earlier ones the branch
        let last = wr.agents.len() - 1;
        assert!(wr.tree_prefix(last).contains(theme::glyphs().tree_last));
        assert!(wr.tree_prefix(last - 1).contains(theme::glyphs().tree_branch));
    }
```

- [ ] **Step 8: Câbler la largeur au focus dans `src/shell/tui.rs`.** Remplacer le bloc de contraintes horizontales (lignes ~791-796) :

```rust
            let h_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(0),     // Messages (main)
                    Constraint::Length(35), // Workroom panel
                ])
                .split(chunks[1]);
```

par :

```rust
            let workroom_width = if self.workroom.is_focused() { 60 } else { 35 };
            let h_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(0),                  // Messages (main)
                    Constraint::Length(workroom_width),  // Workroom panel (widens on focus)
                ])
                .split(chunks[1]);
```

- [ ] **Step 9: Lancer la suite + gate**

Run: `cargo test --no-default-features --features tui workroom:: 2>&1 | tail -10`
Expected: tous verts (dont `tree_prefix_marks_last_sibling`, `layout_mode_*`).

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
```
Expected: `No issues found` dans les 3 modes.

- [ ] **Step 10: Commit (fin de T3a)**

```bash
git add src/shell/workroom.rs src/shell/tui.rs
git commit -m "feat(workroom): pattern-aware layout dispatch, focus widening, hierarchical tree"
```

**➡️ Fin du sous-lot T3a (Tasks 1–3). PR + revue indépendante + validation visuelle Dimitri (`armadai shell`, pattern hierarchical, Ctrl+W) avant de continuer.**

---

### Task 4: Layout Blackboard

**Files:**
- Modify: `src/shell/workroom.rs` (nouvelle fn `blackboard_lines`, arm du dispatch `render`)

**Interfaces:**
- Consumes: `state_display`, `role_style`, `push_footer` (Task 3), `theme::glyphs().board`, `theme::heading()`, `theme::muted()`.
- Produces: `fn blackboard_lines(&self) -> Vec<Line>`.

- [ ] **Step 1: Écrire le test qui échoue** — dans `mod tests` :

```rust
    #[test]
    fn blackboard_lines_has_board_header() {
        let mut wr = Workroom::new();
        wr.init_from_config("orchestration:\n  pattern: blackboard\ncoordinator: c\nagents:\n- a\n- b\n");
        let lines = wr.blackboard_lines();
        // First line is the shared-board header carrying the board glyph.
        let first = &lines[0];
        let text: String = first.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains(theme::glyphs().board));
        assert!(text.contains("agents"));
    }
```

- [ ] **Step 2: Lancer — échoue** (`blackboard_lines` n'existe pas).

Run: `cargo test --no-default-features --features tui workroom::tests::blackboard 2>&1 | tail -5`
Expected: erreur de compilation.

- [ ] **Step 3: Implémenter `blackboard_lines`.** Dans `impl Workroom` :

```rust
    /// The focused blackboard layout: a shared-board header, then a flat
    /// list of agents (no hierarchy — all react to shared state).
    fn blackboard_lines(&self) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();
        let g = theme::glyphs();
        lines.push(Line::from(Span::styled(
            format!("{} shared board · {} agents", g.board, self.agents.len()),
            theme::heading(),
        )));
        for (idx, agent) in self.agents.iter().enumerate() {
            let (icon, state_str, style) = self.state_display(agent);
            let role_style = self.role_style(agent);
            let is_selected = self.focused && idx == self.selected;
            let name_style = if is_selected {
                role_style.add_modifier(Modifier::REVERSED)
            } else {
                role_style
            };
            let suffix = if agent.state == AgentState::Idle {
                "  idle (waiting on board)".to_string()
            } else {
                format!("  {state_str}")
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{icon} "), style),
                Span::styled(&agent.name, name_style),
                Span::styled(suffix, style),
            ]));
        }
        if self.agents.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                theme::muted(),
            )));
        }
        self.push_footer(&mut lines);
        lines
    }
```

- [ ] **Step 4: Brancher le dispatch.** Dans `render`, remplacer l'arm temporaire par :

```rust
            LayoutMode::Blackboard => self.blackboard_lines(),
            LayoutMode::Ring => self.compact_lines(), // Ring lands in Task 5
```

(garder `LayoutMode::Hierarchical => self.hierarchical_lines(),` au-dessus et `LayoutMode::Compact => self.compact_lines(),`).

- [ ] **Step 5: Lancer — passe**

Run: `cargo test --no-default-features --features tui workroom:: 2>&1 | tail -8`
Expected: `blackboard_lines_has_board_header` vert, reste vert.

- [ ] **Step 6: Gate + commit**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
git add src/shell/workroom.rs
git commit -m "feat(workroom): blackboard layout with shared-board header"
```

---

### Task 5: Layout Ring (détenteur de jeton)

**Files:**
- Modify: `src/shell/workroom.rs` (fn `token_holder_index`, fn `ring_lines`, arm du dispatch `render`)

**Interfaces:**
- Consumes: `state_display`, `push_footer` (Task 3), champ privé `current_agent: Option<String>`, `theme::selection()`, `theme::glyphs().arrow_down/arrow_up`.
- Produces: `fn token_holder_index(&self) -> Option<usize>` ; `fn ring_lines(&self) -> Vec<Line>`.

- [ ] **Step 1: Écrire les tests qui échouent** — dans `mod tests` :

```rust
    #[test]
    fn token_holder_index_matches_current_agent() {
        let mut wr = Workroom::new();
        wr.init_from_config("orchestration:\n  pattern: ring\ncoordinator: c\nagents:\n- alpha\n- beta\n");
        // No token holder initially.
        assert_eq!(wr.token_holder_index(), None);
        // The token moves to an agent via a DELEGATE marker in the stream.
        wr.parse_streaming_line("<!--ARMADAI_DELEGATE:alpha-->");
        let idx = wr.token_holder_index().expect("alpha holds the token");
        assert_eq!(wr.agents[idx].name, "alpha");
    }

    #[test]
    fn ring_lines_marks_token_holder_bold_brass() {
        let mut wr = Workroom::new();
        wr.init_from_config("orchestration:\n  pattern: ring\ncoordinator: c\nagents:\n- alpha\n- beta\n");
        wr.parse_streaming_line("<!--ARMADAI_DELEGATE:alpha-->");
        let lines = wr.ring_lines();
        // The span carrying the holder name uses the bold selection (brass) style.
        let holder_styled = lines.iter().flat_map(|l| l.spans.iter()).any(|s| {
            s.content.contains("alpha") && s.style.add_modifier.contains(Modifier::BOLD)
        });
        assert!(holder_styled);
    }
```

Note : `current_agent` est positionné par le marqueur `ARMADAI_DELEGATE:` (via `parse_streaming_line` → `apply_marker`), PAS par `on_delegate`. Les noms d'agents doivent préexister (créés ici par `init_from_config`).

- [ ] **Step 2: Lancer — échoue**

Run: `cargo test --no-default-features --features tui workroom::tests::token_holder 2>&1 | tail -5`
Expected: erreur de compilation (`token_holder_index` / `ring_lines` absents).

- [ ] **Step 3: Implémenter `token_holder_index` + `ring_lines`.** Dans `impl Workroom` :

```rust
    /// Index of the agent currently holding the ring token, if any.
    fn token_holder_index(&self) -> Option<usize> {
        let current = self.current_agent.as_deref()?;
        self.agents.iter().position(|a| a.name == current)
    }

    /// The focused ring layout: sequential agents with flow arrows; the
    /// token holder is highlighted (bold brass) with a "holds token" suffix.
    fn ring_lines(&self) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();
        let g = theme::glyphs();
        let holder = self.token_holder_index();
        let last = self.agents.len().saturating_sub(1);
        for (idx, agent) in self.agents.iter().enumerate() {
            let (icon, state_str, style) = self.state_display(agent);
            let is_holder = holder == Some(idx);
            let is_selected = self.focused && idx == self.selected;
            let name_style = if is_holder {
                theme::selection()
            } else if is_selected {
                self.role_style(agent).add_modifier(Modifier::REVERSED)
            } else {
                self.role_style(agent)
            };
            let marker = if is_holder { "▸ " } else { "  " };
            let mut spans = vec![
                Span::raw(marker),
                Span::styled(&agent.name, name_style),
                Span::styled(format!(" {icon} "), style),
                Span::styled(state_str, style),
            ];
            if is_holder {
                spans.push(Span::styled("   ← holds token", theme::selection()));
            }
            lines.push(Line::from(spans));
            if idx != last {
                lines.push(Line::from(Span::styled(
                    format!("  {}", g.arrow_down),
                    theme::muted(),
                )));
            }
        }
        if self.agents.is_empty() {
            lines.push(Line::from(Span::styled(
                "No agents configured",
                theme::muted(),
            )));
        } else {
            // Loop-back arrow closing the ring.
            lines.push(Line::from(Span::styled(
                format!("  {} (loops to top)", g.arrow_up),
                theme::muted(),
            )));
        }
        self.push_footer(&mut lines);
        lines
    }
```

- [ ] **Step 4: Brancher le dispatch.** Dans `render`, remplacer l'arm Ring temporaire :

```rust
            LayoutMode::Blackboard => self.blackboard_lines(),
            LayoutMode::Ring => self.ring_lines(),
```

- [ ] **Step 5: Lancer — passe**

Run: `cargo test --no-default-features --features tui workroom:: 2>&1 | tail -10`
Expected: `token_holder_index_matches_current_agent` + `ring_lines_marks_token_holder_bold_brass` verts, reste vert.

- [ ] **Step 6: Gate + commit (fin de T3b)**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
git add src/shell/workroom.rs
git commit -m "feat(workroom): ring layout with token-holder highlight and flow arrows"
```

**➡️ Fin du sous-lot T3b (Tasks 4–5). PR + revue indépendante + validation visuelle Dimitri (patterns blackboard & ring) avant de continuer.**

---

### Task 6: Styling DS du popup de détail

**Files:**
- Modify: `src/shell/tui.rs:840-851` (`render_popup`)

**Interfaces:**
- Consumes: `theme::border_style()`, `theme::heading()`.

- [ ] **Step 1: Vérifier l'import du thème.** `grep -n "use crate::theme" src/shell/tui.rs` — s'il est absent, l'ajouter en tête du fichier :

```rust
use crate::theme;
```

- [ ] **Step 2: Remplacer le styling en dur du popup.** Dans `fn render_popup`, remplacer le bloc `.block( … )` (lignes ~840-848) :

```rust
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" ArmadAI ")
                    .title_style(Style::default().fg(Color::Cyan).bold()),
            )
```

par :

```rust
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border_style())
                    .title(" ArmadAI ")
                    .title_style(theme::heading()),
            )
```

(La ligne de footer `Esc to close │ ↑↓ scroll` en `Color::DarkGray` reste — couleur nommée adaptative.)

- [ ] **Step 3: Compiler + gate**

Run: `cargo build --no-default-features --features tui 2>&1 | tail -5`
Expected: build OK (si `Color` devient inutilisé ailleurs, clippy le dira — ne pas retirer d'import encore utilisé par le header/messages).

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
```

- [ ] **Step 4: Commit**

```bash
git add src/shell/tui.rs
git commit -m "style(shell): theme the drill-down popup border and title"
```

---

### Task 7: Nettoyage des couleurs en dur (palette + agent_detail)

**Files:**
- Modify: `src/tui/views/palette.rs:30,39,70`
- Modify: `src/tui/views/agent_detail.rs:109,145,166,180`

**Interfaces:**
- Consumes: `theme::heading()`, `theme::tag()`, `theme::stack()`, `theme::working()`, `theme::border_style()`.

- [ ] **Step 1: `palette.rs` — prompt `:`.** Remplacer (l.30) :

```rust
        Span::styled(": ", Style::default().fg(Color::Cyan)),
```
par :
```rust
        Span::styled(": ", theme::heading()),
```

- [ ] **Step 2: `palette.rs` — bordures.** Sur les deux blocs (l.39 et l.70), retirer l'appel `.border_style(Style::default().fg(Color::Cyan))` — le `.style(theme::border_style())` déjà présent juste en dessous suffit. Le bloc passe de :

```rust
            .title_style(theme::heading())
            .border_style(Style::default().fg(Color::Cyan))
            .style(theme::border_style()),
```
à :
```rust
            .title_style(theme::heading())
            .style(theme::border_style()),
```

(idem pour le second bloc l.70 : supprimer sa ligne `.border_style(Style::default().fg(Color::Cyan))`.)

- [ ] **Step 3: `agent_detail.rs` — les quatre spans.** Appliquer :

```rust
// l.109  scope
Span::styled(meta.scope.join(", "), theme::tag()),
// l.145  pattern
Span::styled(pattern.to_string(), theme::heading()),
// l.166  parts (était Yellow)
Span::styled(parts.join(", "), theme::stack()),
// l.180  parts (était Blue)
Span::styled(parts.join(", "), theme::working()),
```

- [ ] **Step 4: Vérifier l'absence de résidus + imports inutilisés.**

Run: `grep -n "Color::Cyan\|Color::Magenta\|Color::Blue\|Color::Yellow\|Color::Rgb" src/tui/views/palette.rs src/tui/views/agent_detail.rs src/shell/workroom.rs`
Expected: aucune ligne renvoyée.

- [ ] **Step 5: Compiler + gate 3 modes** (clippy signalera tout `use … Color` devenu inutilisé — retirer alors l'import ou la variante `Color` non utilisée dans le `use ratatui::style::{…}` du fichier concerné) :

Run:
```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | tail -5
```
Expected: `No issues found` (3 modes) + suite verte.

- [ ] **Step 6: Commit (fin de T3c)**

```bash
git add src/tui/views/palette.rs src/tui/views/agent_detail.rs
git commit -m "style(tui): route remaining hardcoded colors through the theme"
```

---

### Task 8: Cohérence `--ascii` — glyphes `pointer`/`arrow_back` + flag shell + `theme::init`

**Contexte (ajout T3c, décidé 2026-07-23) :** la revue de T3b a relevé que le marqueur `▸` (compact/ring) et le `←` de « holds token » sont des littéraux Unicode hors `glyphs()`, donc **non dégradés en `--ascii`**. De plus `armadai shell` — surface qui HÉBERGE la Workroom (seul vrai consommateur des glyphes) — n'a ni flag `--ascii` ni appel à `theme::init`, donc le fallback ASCII de la Workroom est intestable. Cette tâche route ces deux glyphes et câble `--ascii` sur le shell.

**Files:**
- Modify: `src/theme.rs` (struct `Glyphs` + `UNICODE`/`ASCII` + test)
- Modify: `src/shell/workroom.rs` (routage `▸`/`←` dans `compact_lines`, `ring_lines`)
- Modify: `src/cli/mod.rs` (variante `Shell` → `Shell { ascii: bool }`, handler)
- Modify: `src/shell/app.rs` (`run_shell(ascii: bool)` + `theme::init(ascii)`)

**Interfaces:**
- Produces: champs `Glyphs.pointer: &'static str`, `Glyphs.arrow_back: &'static str`.

- [ ] **Step 1: Étendre le test des glyphes** — dans `#[cfg(test)] mod tests` de `src/theme.rs`, ajouter à `glyphs_expose_tree_and_flow_symbols` (ou un nouveau test) :

```rust
    #[test]
    fn glyphs_expose_pointer_and_back_arrow() {
        assert_eq!(Glyphs::UNICODE.pointer, "▸");
        assert_eq!(Glyphs::UNICODE.arrow_back, "←");
        assert_eq!(Glyphs::ASCII.pointer, ">");
        assert_eq!(Glyphs::ASCII.arrow_back, "<-");
    }
```

- [ ] **Step 2: Lancer — échoue** (`no field pointer`).

Run: `cargo test --no-default-features --features tui theme::tests::glyphs_expose_pointer 2>&1 | tail -5`
Expected: erreur de compilation.

- [ ] **Step 3: Ajouter les deux champs** à `pub struct Glyphs` (après `board`) :

```rust
    pub pointer: &'static str,
    pub arrow_back: &'static str,
```
Dans `const UNICODE` (après `board: "▤",`) : `pointer: "▸",` et `arrow_back: "←",`.
Dans `const ASCII` (après `board: "#",`) : `pointer: ">",` et `arrow_back: "<-",`.

- [ ] **Step 4: Router les littéraux dans `src/shell/workroom.rs`.**
Dans `compact_lines`, ajouter en tête `let g = theme::glyphs();` (s'il n'y est pas déjà) et remplacer :
```rust
            let marker = if is_selected { "▸ " } else { "" };
```
par :
```rust
            let marker = if is_selected {
                format!("{} ", g.pointer)
            } else {
                String::new()
            };
```
(adapter le `Span::raw(marker)` en `Span::raw(marker)` — `String` accepté par `Span::raw`.)

Dans `ring_lines` (qui a déjà `let g = theme::glyphs();`), remplacer :
```rust
            let marker = if is_holder { "▸ " } else { "  " };
```
par :
```rust
            let marker = if is_holder {
                format!("{} ", g.pointer)
            } else {
                "  ".to_string()
            };
```
et remplacer :
```rust
                spans.push(Span::styled("   ← holds token", theme::selection()));
```
par :
```rust
                spans.push(Span::styled(
                    format!("   {} holds token", g.arrow_back),
                    theme::selection(),
                ));
```

- [ ] **Step 5: Flag `--ascii` sur `armadai shell`.** Dans `src/cli/mod.rs`, remplacer la variante unit `Shell,` (~l.254, garder le `#[command(long_about = …)]` au-dessus) par :

```rust
    Shell {
        /// Use ASCII glyphs instead of Unicode (for limited terminals)
        #[arg(long)]
        ascii: bool,
    },
```
et le handler (~l.532) :
```rust
        Command::Shell => crate::shell::app::run_shell().await,
```
par :
```rust
        Command::Shell { ascii } => crate::shell::app::run_shell(ascii).await,
```

- [ ] **Step 6: `run_shell(ascii)` + `theme::init`.** Dans `src/shell/app.rs`, changer la signature :
```rust
pub async fn run_shell() -> Result<()> {
```
en :
```rust
pub async fn run_shell(ascii: bool) -> Result<()> {
    crate::theme::init(ascii);
```
(l'appel `theme::init(ascii)` en toute première ligne du corps, avant tout rendu ; idempotent — cf. `src/theme.rs`.)

- [ ] **Step 7: Gate 3 modes + suite.**

Run:
```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | tail -5
grep -n '▸\|←' src/shell/workroom.rs
```
Expected: `No issues found` (3 modes) + suite verte + le `grep` ne renvoie que le commentaire doc (« holds token » en toutes lettres reste dans un commentaire, pas de littéral `▸`/`←` dans le code de rendu).

- [ ] **Step 8: Commit (fin de T3c)**

```bash
git add src/theme.rs src/shell/workroom.rs src/cli/mod.rs src/shell/app.rs
git commit -m "feat(shell): wire --ascii flag and route pointer/back-arrow glyphs for ASCII fallback"
```

**➡️ Fin du sous-lot T3c (Tasks 6–8). PR + revue indépendante + validation visuelle Dimitri (palette ⌃P, agent detail, popup, `armadai shell --ascii`) avant merge.**

---

## Self-Review

**Spec coverage :**
- Champ `pattern` + parsing `init_from_config` → Task 2 ✓
- Dispatch `(pattern, focused, width)` + dégradation seuil 44 → Task 3 (`layout_mode`) ✓
- Élargissement 35→60 au focus → Task 3 Step 8 ✓
- Layout hierarchical box-drawing → Task 3 ✓ ; blackboard → Task 4 ✓ ; ring + détenteur de jeton → Task 5 ✓
- Extension `Glyphs` (board, arrows, connecteurs) → Task 1 ✓
- Drill-down popup stylé → Task 6 ✓
- Nettoyage couleurs (palette + agent_detail + bordure workroom l.582) → Task 7 + Task 3 Step 5 ✓ (la bordure workroom est traitée dans le refactor `render`)
- Tests feature `tui`, pas d'accès env → tous les tests passent une string au parsing ✓
- Gate clippy 3 modes + fmt + test → présent en fin de chaque sous-lot ✓
- Hors périmètre (armadai tui Orchestration) non touché ✓

**Placeholder scan :** aucun TODO/TBD ; l'arm `Ring => compact_lines()` en Task 4 Step 4 est une dégradation fonctionnelle explicite remplacée en Task 5, pas un placeholder mort.

**Type consistency :** `layout_mode(&self, inner_width: u16) -> LayoutMode` (Task 3) utilisé par `render` (Task 3) ; `LayoutMode` variantes Compact/Hierarchical/Blackboard/Ring cohérentes Tasks 3-5 ; `state_display`/`role_style`/`push_footer` définies en Task 3, consommées Tasks 4-5 ; `token_holder_index` défini/consommé en Task 5 ; `parse_pattern` défini/testé Task 2 ; helpers `theme::*` existants (vérifiés dans `src/theme.rs`). `OrchestrationPattern` implémente `Display` (`src/core/orchestration/mod.rs:66`) → `format!(" Workroom · {} ", self.pattern)` valide.
