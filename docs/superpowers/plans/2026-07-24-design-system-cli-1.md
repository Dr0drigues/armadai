# Design System → CLI, lot CLI-1 (module de style + `armadai run` humain) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Doter la sortie humaine du CLI d'un module de style ANSI cohérent avec l'identité DS (laiton + signaux), et l'appliquer à la sortie humaine de `armadai run`.

**Architecture:** Nouveau module `src/cli/style.rs` (non feature-gated) : helpers sémantiques renvoyant des `anstyle::Style` (valeurs DS en RGB), rendus via `anstream` qui strippe automatiquement les codes ANSI selon `NO_COLOR`/`CLICOLOR`/TTY. Accent-only : le corps de texte reste en couleur par défaut du terminal ; on n'accentue que titres/actif/statuts/secondaire. Application aux `println!`/`eprintln!` humains de `src/cli/run.rs`.

**Tech Stack:** Rust edition 2024, `anstyle` + `anstream` (déjà dans `Cargo.lock`).

## Global Constraints
- Gate à chaque tâche : clippy **3 modes** (`--no-default-features --features tui`, `--features tui,providers-api`, `--features tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test --no-default-features --features tui`.
- `src/cli/style.rs` **NON gated `tui`** (la sortie CLI de base ne dépend pas de ratatui). C'est un module DISTINCT de `src/theme.rs` (ratatui) — ne pas les confondre.
- **Accent-only** : corps de texte non coloré (fg par défaut, lisible fond clair ET sombre) ; accents = laiton (titres/actif), signaux (statuts), bright-black (secondaire).
- **Détection couleur déléguée à `anstream`** — ne PAS réimplémenter NO_COLOR/TTY. Le contenu machine (`--json`) passe par `EventSink`, jamais par les helpers humains → jamais coloré.
- Valeurs DS (de `assets/terminal-palette.json`) : brass `#c79a4a`, signal_ok `#5cbf87`, signal_warning `#e2b24c`, signal_critical `#d75f4d`, signal_running `#57a9cc` ; muted = `AnsiColor::BrightBlack`.
- Branche `feat/cli-ds-1` (déjà créée depuis `release/1.0.0`, contient déjà la spec). Une PR, revue indépendante + validation visuelle Dimitri.
- Hors périmètre : toutes les autres commandes (`list`/`new`/`link`/`init`/`models`/…) = lots CLI-2+.

## File Structure
- `Cargo.toml` — ajouter `anstyle` + `anstream` en `[dependencies]`. (Task 1)
- `src/cli/style.rs` — nouveau module de style CLI (helpers + tests). (Task 1)
- `src/cli/mod.rs` — déclarer `mod style;`. (Task 1)
- `src/cli/run.rs` — styliser la sortie humaine. (Task 2)

---

### Task 1: Module `src/cli/style.rs` (socle ANSI)

**Files:**
- Modify: `Cargo.toml` (`[dependencies]`)
- Create: `src/cli/style.rs`
- Modify: `src/cli/mod.rs` (ajout `mod style;`)

**Interfaces:**
- Produces: `crate::cli::style::{header, accent, ok, warn, err, running, muted, agent} -> anstyle::Style`.

- [ ] **Step 1 : Ajouter les dépendances.**

Run: `cargo add anstyle anstream`
Expected: `Cargo.toml` gagne `anstyle = "1.0"` (ou version résolue) et `anstream = "0.6"` (ou version résolue), cohérentes avec `Cargo.lock`. Vérifier qu'elles apparaissent sous `[dependencies]` (non-optional, non feature-gated).

- [ ] **Step 2 : Écrire le module avec ses tests (le test rend le fichier compilable et vérifie le comportement).** Créer `src/cli/style.rs` :

```rust
//! ANSI styling for human CLI output (design system "pont de commandement").
//!
//! Distinct from `crate::theme` (ratatui `Color`, for the TUI/shell). This is
//! for plain stdout/stderr. **Accent-only**: body text keeps the terminal's
//! default foreground (legible on light AND dark backgrounds); only accents
//! (brass), status signals, and secondary text are coloured. Colour on/off is
//! delegated entirely to `anstream` (respects `NO_COLOR`/`CLICOLOR`/TTY) — the
//! call sites print with `anstream::println!`/`anstream::eprintln!`.

use anstyle::{AnsiColor, Color, RgbColor, Style};

// Design-system accents (assets/terminal-palette.json).
const BRASS: Color = Color::Rgb(RgbColor(0xc7, 0x9a, 0x4a));
const SIGNAL_OK: Color = Color::Rgb(RgbColor(0x5c, 0xbf, 0x87));
const SIGNAL_WARNING: Color = Color::Rgb(RgbColor(0xe2, 0xb2, 0x4c));
const SIGNAL_CRITICAL: Color = Color::Rgb(RgbColor(0xd7, 0x5f, 0x4d));
const SIGNAL_RUNNING: Color = Color::Rgb(RgbColor(0x57, 0xa9, 0xcc));

/// Section heading / active element: bold brass.
pub fn header() -> Style {
    Style::new().bold().fg_color(Some(BRASS))
}
/// Accent (brass) without bold.
pub fn accent() -> Style {
    Style::new().fg_color(Some(BRASS))
}
/// Success / done status.
pub fn ok() -> Style {
    Style::new().fg_color(Some(SIGNAL_OK))
}
/// Warning status.
pub fn warn() -> Style {
    Style::new().fg_color(Some(SIGNAL_WARNING))
}
/// Error / critical status.
pub fn err() -> Style {
    Style::new().fg_color(Some(SIGNAL_CRITICAL))
}
/// In-progress / running status.
pub fn running() -> Style {
    Style::new().fg_color(Some(SIGNAL_RUNNING))
}
/// Secondary / muted text: bright-black (named, adapts to terminal theme).
pub fn muted() -> Style {
    Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)))
}
/// Agent / role name: bold, no colour (avoids clashing with status colours).
pub fn agent() -> Style {
    Style::new().bold()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn accents_carry_expected_style() {
        assert_eq!(header().get_fg_color(), Some(BRASS));
        assert!(header().get_effects().contains(anstyle::Effects::BOLD));
        assert_eq!(ok().get_fg_color(), Some(SIGNAL_OK));
        assert_eq!(err().get_fg_color(), Some(SIGNAL_CRITICAL));
        assert_eq!(
            muted().get_fg_color(),
            Some(Color::Ansi(AnsiColor::BrightBlack))
        );
        // agent(): bold, no colour.
        assert_eq!(agent().get_fg_color(), None);
        assert!(agent().get_effects().contains(anstyle::Effects::BOLD));
    }

    #[test]
    fn anstream_emits_codes_when_forced_on_and_strips_when_off() {
        let h = header();
        let styled = format!("{h}hello{h:#}");
        const ESC: u8 = 0x1b;

        // Forced colour ON → ANSI escape present.
        let mut on = anstream::AutoStream::always(Vec::new());
        write!(on, "{styled}").unwrap();
        assert!(on.into_inner().contains(&ESC), "expected ANSI codes when forced on");

        // Forced colour OFF → escapes stripped.
        let mut off = anstream::AutoStream::never(Vec::new());
        write!(off, "{styled}").unwrap();
        assert!(!off.into_inner().contains(&ESC), "expected no ANSI codes when forced off");
    }
}
```

Note : vérifier au compilateur les noms exacts de l'API `anstyle` de la version résolue — `Style::new()`, `.bold()`, `.fg_color(Some(Color))`, `.get_fg_color()`, `.get_effects()`, `Effects::BOLD`, `Color::Rgb(RgbColor(r,g,b))`, `Color::Ansi(AnsiColor::BrightBlack)` — et `anstream::AutoStream::{always,never}` + `.into_inner()`. Adapter si l'API diffère (ne PAS deviner : lire le crate).

- [ ] **Step 3 : Déclarer le module.** Dans `src/cli/mod.rs`, ajouter (près des autres `mod`) :

```rust
mod style;
```
(non gated ; s'il faut le rendre visible ailleurs plus tard, `pub(crate) mod style;` — pour ce lot `run.rs` est dans le même crate/module, `crate::cli::style` suffit.)

- [ ] **Step 4 : Compiler + tests.**

Run: `cargo test --no-default-features --features tui cli::style 2>&1 | tail -8`
Expected: `accents_carry_expected_style` + `anstream_emits_codes_when_forced_on_and_strips_when_off` verts.

- [ ] **Step 5 : Gate 3 modes + commit** (les deps nouvelles touchent les 3 modes) :

```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
git add Cargo.toml Cargo.lock src/cli/style.rs src/cli/mod.rs
git commit -m "feat(cli): ANSI style module for human output (design-system accents via anstyle/anstream)"
```

---

### Task 2: Appliquer le style à la sortie humaine de `armadai run`

**Files:**
- Modify: `src/cli/run.rs` (sorties humaines `println!`/`eprintln!` — voir ancrages)

**Interfaces:**
- Consumes: `crate::cli::style::{header, accent, ok, warn, err, running, muted, agent}` (Task 1) ; les macros `anstream::eprintln!`/`anstream::println!`.

**Principe d'édition** : remplacer chaque `eprintln!`/`println!` HUMAIN identifié par son équivalent `anstream::eprintln!`/`anstream::println!` en injectant les styles via la syntaxe anstyle `{s}…{s:#}` (le spec `#` réinitialise). NE PAS toucher : les `sink.emit(...)`, le contenu `--json`, ni les `println!("{content}")`/`println!("{outcome_text}")` du RÉSULTAT final (le corps reste non coloré — accent-only ; on ne stylise QUE le chrome autour). NE PAS toucher les `tracing::` logs.

- [ ] **Step 1 : En-tête d'étape pipeline** (~l.329). Remplacer :

```rust
            eprintln!("--- [{}/{} {}] ---", i + 1, chain.len(), name);
```
par :
```rust
            let h = crate::cli::style::header();
            anstream::eprintln!("{h}--- [{}/{} {}] ---{h:#}", i + 1, chain.len(), name);
```

- [ ] **Step 2 : Fallback modèle** (~l.528). Remplacer :

```rust
                eprintln!("[{agent_name}] Model unavailable, falling back to {fallback_model}...");
```
par :
```rust
                let w = crate::cli::style::warn();
                anstream::eprintln!("{w}[{agent_name}] Model unavailable, falling back to {fallback_model}...{w:#}");
```

- [ ] **Step 3 : Résumé (tokens/coût/durée)** (~l.565). Remplacer le bloc :

```rust
    eprintln!(
        "\n[{}] model={} tokens={}/{} cost=${:.6} duration={}ms",
        agent_name,
        response.model,
        response.tokens_in,
        response.tokens_out,
        response.cost,
        duration_ms
    );
```
par (nom d'agent accentué, métriques en muted) :
```rust
    let acc = crate::cli::style::accent();
    let mut_ = crate::cli::style::muted();
    anstream::eprintln!(
        "\n{acc}[{}]{acc:#} {mut_}model={} tokens={}/{} cost=${:.6} duration={}ms{mut_:#}",
        agent_name,
        response.model,
        response.tokens_in,
        response.tokens_out,
        response.cost,
        duration_ms
    );
```

- [ ] **Step 4 : Statuts d'orchestration humains** (branches blackboard/ring/hierarchical, déjà gated `human_output`). Styliser les lignes de statut (PAS le `println!("{outcome_text}")`/`println!("{}", result.content)` du résultat, qui restent bruts). Exemples à appliquer sur le même modèle :

  - `eprintln!("[blackboard] Starting with {} agent(s), max {} rounds", …)` (~l.1352 zone) → préfixe `running()`.
  - `eprintln!("[blackboard] Halted: {:?}", state.status)` (~l.1373) → `ok()` si l'issue est un succès, sinon `warn()`/`err()` selon `state.status` (utiliser une petite closure locale `status_style(&state.status)` si plusieurs branches en ont besoin ; sinon `warn()` par défaut pour un halt).
  - Idem `[ring] status: …` (~l.1469) et l'en-tête de démarrage hierarchical (~l.1525/1552).

  Modèle (Starting) :
```rust
            let r = crate::cli::style::running();
            anstream::eprintln!(
                "{r}[blackboard] Starting with {} agent(s), max {} rounds{r:#}",
                agent_map.len(),
                config.max_rounds
            );
```
  Modèle (Halted) :
```rust
            let s = crate::cli::style::warn();
            anstream::eprintln!("{s}[blackboard] Halted: {:?}{s:#}", state.status);
```
  (Appliquer l'équivalent aux branches ring et hierarchical avec le même vocabulaire de style. Rester factuel : `running` pour le démarrage, `warn`/`ok` pour l'issue.)

- [ ] **Step 5 : Vérifier qu'aucun chemin machine n'est touché.**

Run: `grep -n "anstream::" src/cli/run.rs` — doit n'apparaître que sur des chemins HUMAINS (jamais dans un bloc `if json` / autour d'un `sink.emit`). Vérifier aussi que les `println!` du résultat final (contenu) sont **inchangés**.

- [ ] **Step 6 : Gate 3 modes + tests + e2e.**

Run:
```bash
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | tail -5
cargo test --test e2e --no-default-features --features tui,storage 2>&1 | tail -5
```
Expected : tout vert. L'e2e est **inchangé** : il capture stdout non-TTY → anstream strippe → octets identiques ; et le contenu `--json`/résultat n'est pas touché.

- [ ] **Step 7 : Commit**

```bash
git add src/cli/run.rs
git commit -m "feat(cli): apply design-system accents to armadai run human output"
```

**➡️ Fin du lot CLI-1. PR + revue indépendante + validation visuelle Dimitri : `armadai run` (mode humain) montre les accents ; `armadai run … | cat` et `NO_COLOR=1 armadai run …` → aucune couleur ; `--json` inchangé.**

---

## Self-Review
- **Spec coverage** : module `style.rs` non-gated, anstyle+anstream, API sémantique accent-only, valeurs DS → Task 1 ✓ ; application à `run` humain (pipeline header, fallback, résumé, statuts orchestration) sans toucher RunEvent/--json/résultat brut → Task 2 ✓ ; tests rendu on/off déterministes → Task 1 Step 2 ✓ ; e2e intact → Task 2 Step 6 ✓ ; gate 3 modes → chaque tâche ✓ ; hors-scope (autres commandes) non touché ✓.
- **Placeholder scan** : pas de TODO ; les « ~l.X » sont des repères indicatifs, chaque Step donne le code before/after exact à matcher. Step 4 liste explicitement les lignes ; le `status_style` optionnel est décrit (pas un placeholder — fallback `warn()` par défaut donné).
- **Type consistency** : helpers `style::{header,accent,ok,warn,err,running,muted,agent}` définis Task 1, consommés Task 2 ; API `anstyle`/`anstream` à confirmer au compilateur (noté). `anstream::eprintln!`/`println!` fully-qualified (pas de clash avec les macros std). Le corps résultat reste `println!` std non stylé (accent-only).
