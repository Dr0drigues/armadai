# Design System → TUI T3d (theming du chrome du shell) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Router toutes les couleurs codées en dur du chrome du shell (`src/shell/tui.rs`) via le thème central `src/theme.rs` (accent-only), avec un curseur d'input adaptatif (vidéo inverse) visible sur fond clair comme sombre.

**Architecture:** Ajout d'un helper `theme::cursor()` (vidéo inverse). Puis remplacement, dans `src/shell/tui.rs`, de chaque `Color::` littéral du chrome (en-tête, rôles de message, spinner, bordures/titres d'input, curseur) par un helper `theme::*`. La détection de curseur du helper de test (`find_cursor_in_buffer`) est adaptée au nouveau style (modifier REVERSED sur une cellule espace).

**Tech Stack:** Rust edition 2024, ratatui, feature `tui`.

## Global Constraints

- Feature `tui`. Gate à chaque tâche : clippy **3 modes** (`--features tui`, `--features tui,providers-api`, `--features tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test --no-default-features --features tui`.
- **Thème accent-only** : n'utiliser QUE les helpers de `src/theme.rs`. Objectif ferme : plus aucun `Color::Cyan/Green/Yellow/White/Black` littéral dans `src/shell/tui.rs` (les `Color::DarkGray` déjà présents sont routés vers `theme::muted()`).
- **Mapping sémantique validé (Dimitri 2026-07-23)** : en-tête + titres + assistant → `theme::heading()` (laiton gras) ; user → `theme::stack()` + BOLD (vert signal_ok) ; system + spinner + footer popup → `theme::muted()` (+ DIM / ITALIC conservés) ; bordures → `theme::border_style()` ; curseur → `theme::cursor()` (vidéo inverse).
- **rust-analyzer non fiable** dans cet env (faux positifs ABI 1.97.0/1.97.1, unlinked-file, inactive-cfg, snapshots mi-édition) → vérifier TOUJOURS au compilateur.
- Branche : `feat/tui-t3d-shell-chrome` (base release/1.0.0). Une PR, revue indépendante + validation visuelle Dimitri (`armadai shell`, fond clair) avant merge.

## File Structure
- `src/theme.rs` — nouveau helper `cursor()` + test. (Task 1)
- `src/shell/tui.rs` — routage des couleurs du chrome + adaptation de `find_cursor_in_buffer`. (Task 2)

---

### Task 1: Helper `theme::cursor()` (vidéo inverse)

**Files:** Modify `src/theme.rs` (nouveau helper + test).

**Interfaces:**
- Produces: `pub fn cursor() -> Style` — `Style::default().add_modifier(Modifier::REVERSED)`.

- [ ] **Step 1: Test qui échoue** — dans `#[cfg(test)] mod tests` de `src/theme.rs` :

```rust
    #[test]
    fn cursor_is_reversed() {
        assert!(cursor().add_modifier.contains(Modifier::REVERSED));
    }
```

- [ ] **Step 2: Lancer — échoue** (`cannot find function cursor`).

Run: `cargo test --no-default-features --features tui theme::tests::cursor_is_reversed 2>&1 | tail -5`
Expected: erreur de compilation.

- [ ] **Step 3: Implémenter le helper** — dans `src/theme.rs`, à côté des autres helpers (ex. après `border_style()`), ajouter :

```rust
/// Style for the input block cursor: reverse-video (swaps the cell's
/// foreground/background) so the cursor stays visible on both light and
/// dark terminals. Rendered on a single space cell.
pub fn cursor() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}
```

- [ ] **Step 4: Lancer — passe**

Run: `cargo test --no-default-features --features tui theme::tests::cursor_is_reversed 2>&1 | tail -5`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 5: Gate + commit**

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
git add src/theme.rs
git commit -m "feat(theme): add reverse-video cursor style helper"
```

---

### Task 2: Router le chrome du shell + adapter la détection de curseur

**Files:** Modify `src/shell/tui.rs` (en-tête ~782, footer popup ~838, rôles de message ~869-881, spinner ~912-914, curseur ~1071/1158/1165, bordures/titres input ~1079-1081/1192-1194, test helper `find_cursor_in_buffer` ~1414-1417).

**Interfaces:**
- Consumes: `theme::heading()`, `theme::stack()`, `theme::muted()`, `theme::border_style()`, `theme::cursor()` (Task 1). `use crate::theme;` est déjà présent (ajouté en T3c).

- [ ] **Step 1: En-tête** (~782). Remplacer :
```rust
        let header = Paragraph::new(header_text).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
```
par :
```rust
        let header = Paragraph::new(header_text).style(theme::heading());
```

- [ ] **Step 2: Footer du popup** (~838). Remplacer `Style::default().fg(Color::DarkGray)` (la ligne du footer `" Esc to close │ ↑↓ scroll"`) par `theme::muted()`.

- [ ] **Step 3: Rôles de message** (~869-881). Remplacer le bloc :
```rust
            let role_style = if msg.is_system {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM)
            } else if msg.is_user {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            };
```
par :
```rust
            let role_style = if msg.is_system {
                theme::muted().add_modifier(Modifier::DIM)
            } else if msg.is_user {
                theme::stack().add_modifier(Modifier::BOLD)
            } else {
                theme::heading()
            };
```
(`theme::heading()` porte déjà BOLD ; l'assistant garde donc gras + laiton.)

- [ ] **Step 4: Spinner de chargement** (~912-914). Remplacer :
```rust
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
```
par :
```rust
                theme::muted().add_modifier(Modifier::ITALIC),
```

- [ ] **Step 5: Curseur bloc** — les TROIS occurrences (~1071, ~1158, ~1165) de :
```rust
                    Style::default().bg(Color::White).fg(Color::Black),
```
(et sa variante indentée) par :
```rust
                    theme::cursor(),
```
Attention : garder le `Span::styled(" ", …)` — le curseur reste une cellule espace.

- [ ] **Step 6: Bordures + titres d'input** — les DEUX blocs (~1077-1082 et ~1190-1195). Remplacer dans chacun :
```rust
                        .border_style(Style::default().fg(Color::DarkGray))
                        .title(" Input ")
                        .title_style(Style::default().fg(Color::Cyan)),
```
par :
```rust
                        .border_style(theme::border_style())
                        .title(" Input ")
                        .title_style(theme::heading()),
```

- [ ] **Step 7: Adapter la détection de curseur du test** (`find_cursor_in_buffer`, ~1406-1424). Remplacer la condition :
```rust
                    // Cursor is styled with bg=White, fg=Black
                    if cell.bg == ratatui::prelude::Color::White
                        && cell.fg == ratatui::prelude::Color::Black
                    {
                        return Some((x, y));
                    }
```
par (détecter le modifier REVERSED sur une cellule espace — le nom d'agent sélectionné de la Workroom utilise aussi REVERSED mais jamais sur un espace) :
```rust
                    // Cursor is styled reverse-video on a single space cell.
                    if cell.symbol() == " "
                        && cell.modifier.contains(ratatui::style::Modifier::REVERSED)
                    {
                        return Some((x, y));
                    }
```

- [ ] **Step 8: Vérifier l'absence de résidus + imports.**

Run: `grep -n "Color::Cyan\|Color::Green\|Color::Yellow\|Color::White\|Color::Black" src/shell/tui.rs`
Expected: aucune ligne (les seules `Color::` restantes acceptables seraient `Color::DarkGray` si une occurrence hors scope subsiste — vérifier qu'il n'en reste pas d'inattendue). Si `Color` devient inutilisé, clippy le signalera → retirer l'import ; s'il reste utilisé, le garder.

- [ ] **Step 9: Gate 3 modes + suite** (les tests de curseur `find_cursor_in_buffer` valident la nouvelle détection).

Run:
```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | tail -8
```
Expected: `No issues found` (3 modes) + suite verte (dont les tests d'input/curseur).

- [ ] **Step 10: Commit**

```bash
git add src/shell/tui.rs
git commit -m "style(shell): route chrome colors through the theme, adaptive reverse-video cursor"
```

**➡️ Fin de T3d. PR + revue indépendante + validation visuelle Dimitri (`armadai shell` fond clair : en-tête, messages user/assistant/system, curseur visible, input) avant merge.**

---

## Self-Review
- **Spec coverage** : header→heading (T2.1) ; footer popup→muted (T2.2) ; rôles system/user/assistant→muted+DIM / stack+BOLD / heading (T2.3, mapping validé) ; spinner→muted+ITALIC (T2.4) ; curseur→cursor() (T2.5, helper T1) ; bordures/titres input→border_style/heading (T2.6) ; détection test adaptée (T2.7) ; grep de non-régression (T2.8). ✓
- **Placeholder scan** : aucun.
- **Type consistency** : `theme::cursor()` défini T1, consommé T2.5 ; helpers `heading/stack/muted/border_style` existants ; `cell.symbol()`/`cell.modifier` sont l'API ratatui de `Cell` (les tests existants lisent déjà `cell.bg`/`cell.fg`). Le modifier REVERSED sur espace évite la collision avec les noms d'agents REVERSED de la Workroom.
- **Risque** : si un test existant s'appuyait sur `bg=White` du curseur autrement que via `find_cursor_in_buffer`, il casserait — vérifier via la suite complète (Step 9). Le curseur REVERSED sur cellule vide reste détectable et visible.
