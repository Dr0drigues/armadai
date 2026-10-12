# TUI thème — consolidation dans src/theme.rs (correctif) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Corriger l'erreur d'architecture de T1 : consolider l'identité DS dans le thème central **existant** `src/theme.rs` (partagé TUI + shell), en mode **accent-only** — accents (laiton) + signaux au DS avec dégradation tier, textes/fonds en couleurs nommées adaptatives (lisibles fond clair ET sombre). Supprimer le module parallèle `src/tui/theme.rs` et revert le wiring T1.

**Architecture:** `src/theme.rs` garde son API sémantique (`selection/heading/working/delegating/done/muted/error/warning/roles/tag/stack`) que TUI+shell appellent déjà — donc **aucune vue à modifier**. En interne : les **accents** (SELECTION/HEADING/DELEGATING/roles/tag/…) et **signaux** (working→running, done→ok, error→critical, warning) sont résolus depuis la palette DS (`terminal-palette.json`) selon un `ColorTier` détecté une fois (global `OnceLock`, réglé par `theme::init(ascii)` au démarrage). MUTED + textes restent en couleurs nommées adaptatives. Glyphes unicode/ASCII via le même global.

**Tech Stack:** Rust edition 2024, ratatui, feature `tui`.

## Global Constraints
- Feature `tui`. Gate : clippy **3 modes** (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`.
- **Principe de lisibilité conservé** (cf. `docs/proposals/ux-audit-tui-2026-07-20.md`) : textes/muted en couleurs NOMMÉES (adaptatives). SEULS les accents (tons moyens : laiton `#c79a4a`, signaux) passent en Rgb/256/16 tier — ils restent lisibles sur fond clair ET sombre.
- **Aucune vue TUI ni le shell ne changent d'API** : ils appellent déjà `theme::…()`. Ne PAS réintroduire `app.theme`.
- `--ascii` = flag CLI (déjà sur `tui`), recâblé pour appeler `theme::init(ascii)`.

## Palette DS (accents, de terminal-palette.json)
brass #c79a4a/179/Yellow · brass_strong #d6ad5f/215/LightYellow · signal_ok #5cbf87/114/Green · signal_warning #e2b24c/214/LightYellow · signal_critical #d75f4d/167/Red · signal_running #57a9cc/74/Cyan.

---

### Task 1: Rework `src/theme.rs` (accents DS + tier + init + glyphs)

**Files:** Modify `src/theme.rs`.

- [ ] **Step 1 : Ajouter `ColorTier` + résolution accent + global.**
```rust
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorTier { Truecolor, Xterm256, Ansi16 }
impl ColorTier {
    pub fn from_env_values(colorterm: Option<&str>, term: Option<&str>, no_color: bool) -> Self {
        if no_color { return ColorTier::Ansi16; }
        if matches!(colorterm, Some("truecolor") | Some("24bit")) { return ColorTier::Truecolor; }
        match term { Some(t) if t.contains("256color") => ColorTier::Xterm256, _ => ColorTier::Ansi16 }
    }
    fn detect() -> Self {
        Self::from_env_values(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
            std::env::var_os("NO_COLOR").is_some(),
        )
    }
}

struct Settings { tier: ColorTier, ascii: bool }
static SETTINGS: OnceLock<Settings> = OnceLock::new();
fn settings() -> &'static Settings {
    SETTINGS.get_or_init(|| Settings { tier: ColorTier::detect(), ascii: false })
}
/// Initialise the theme once at TUI/shell startup (color tier + ascii glyphs).
/// Idempotent: only the first call wins.
pub fn init(ascii: bool) {
    let _ = SETTINGS.set(Settings { tier: ColorTier::detect(), ascii });
}

// Accent token: DS three-tier. Text/muted stay named (not here).
struct Accent { rgb: (u8, u8, u8), x256: u8, ansi: Color }
impl Accent {
    const fn new(rgb: (u8, u8, u8), x256: u8, ansi: Color) -> Self { Accent { rgb, x256, ansi } }
    fn color(&self) -> Color {
        match settings().tier {
            ColorTier::Truecolor => Color::Rgb(self.rgb.0, self.rgb.1, self.rgb.2),
            ColorTier::Xterm256 => Color::Indexed(self.x256),
            ColorTier::Ansi16 => self.ansi,
        }
    }
}
const BRASS: Accent = Accent::new((0xc7,0x9a,0x4a), 179, Color::Yellow);
const BRASS_STRONG: Accent = Accent::new((0xd6,0xad,0x5f), 215, Color::LightYellow);
const SIG_OK: Accent = Accent::new((0x5c,0xbf,0x87), 114, Color::Green);
const SIG_WARNING: Accent = Accent::new((0xe2,0xb2,0x4c), 214, Color::LightYellow);
const SIG_CRITICAL: Accent = Accent::new((0xd7,0x5f,0x4d), 167, Color::Red);
const SIG_RUNNING: Accent = Accent::new((0x57,0xa9,0xcc), 74, Color::Cyan);
```

- [ ] **Step 2 : Ré-exprimer les helpers via les accents DS** (garder MUTED nommé). Remplacer les `const … : Color` + fns par des fns qui résolvent l'accent :
  - `selection()` = bold `BRASS.color()` ; `heading()` = bold `BRASS.color()`.
  - `working()` = `SIG_RUNNING.color()` ; `delegating()` = `BRASS.color()` ; `done()` = `SIG_OK.color()` (distinct de working : running/blue vs ok/green).
  - `muted()` = `Color::DarkGray` (INCHANGÉ, nommé, adaptatif).
  - `error()` = `SIG_CRITICAL.color()` ; `warning()` = `SIG_WARNING.color()`.
  - `role_coordinator()` = bold `BRASS_STRONG.color()` ; `role_lead()` = bold `BRASS.color()` ; `role_agent()` = bold sans couleur (INCHANGÉ).
  - `tag()` = `BRASS.color()` ; `stack()` = `SIG_OK.color()`.
  Supprimer les anciens `const SELECTION/HEADING/...` publics (ou les garder privés si référencés ailleurs — vérifier `grep -rn "theme::SELECTION\|theme::HEADING\|theme::MUTED\|theme::WORKING\|theme::DONE\|theme::DELEGATING\|theme::ERROR\|theme::WARNING\|theme::ROLE_\|theme::TAG\|theme::STACK" src/` ; s'ils sont utilisés hors des fns, adapter les appelants ou garder des consts pour les seuls non-accent comme MUTED).

- [ ] **Step 3 : Glyphes** — ajouter un `Glyphs` (unicode/ascii) + `pub fn glyphs() -> Glyphs { if settings().ascii { ASCII } else { UNICODE } }`, à la disposition des vues/shell qui en veulent (optionnel pour ce lot si aucun appelant ; sinon exposer et utiliser là où des glyphes en dur existent — hors scope si aucun).

- [ ] **Step 4 : Mettre à jour les tests** de `src/theme.rs`. L'ancien `semantic_colors_are_named_not_rgb` devient faux (les accents sont DS). Le remplacer par :
```rust
#[test]
fn tier_detection() {
    assert_eq!(ColorTier::from_env_values(Some("truecolor"), None, false), ColorTier::Truecolor);
    assert_eq!(ColorTier::from_env_values(None, Some("xterm-256color"), false), ColorTier::Xterm256);
    assert_eq!(ColorTier::from_env_values(Some("truecolor"), None, true), ColorTier::Ansi16);
    assert_eq!(ColorTier::from_env_values(None, Some("xterm"), false), ColorTier::Ansi16);
}
#[test]
fn muted_stays_named_adaptive() {
    // Text/idle must stay a named color (legible on light & dark terminals).
    assert_eq!(muted().fg, Some(Color::DarkGray));
}
#[test]
fn done_distinct_from_working() { assert_ne!(done().fg, working().fg); }
#[test]
fn role_agent_has_no_color() { assert_eq!(role_agent().fg, None); }
#[test]
fn selection_is_bold() { assert!(selection().add_modifier.contains(Modifier::BOLD)); }
```
(Ces tests n'appellent pas `init()` → ils lisent le tier par défaut détecté ; ils asservissent `muted`/`role_agent`/bold/`done≠working` qui sont stables quel que soit le tier. Ne PAS asserter la valeur exacte d'un accent — elle dépend du tier de la machine de test.)

- [ ] **Step 5 : Gate** — `cargo test --no-default-features --features tui theme::` + suite + clippy 3 modes + fmt.

- [ ] **Step 6 : Commit** `git commit -m "feat(theme): DS accents (brass + signals) with tier fallback in the shared theme"`

---

### Task 2: Supprimer le module parallèle + revert le wiring T1

**Files:** Delete `src/tui/theme.rs` ; Modify `src/tui/mod.rs`, `src/tui/app.rs`, `src/tui/views/dashboard.rs`, `src/cli/mod.rs`.

- [ ] **Step 1 : Supprimer `src/tui/theme.rs`** (`git rm`) + retirer `pub mod theme;` de `src/tui/mod.rs`.
- [ ] **Step 2 : `App`** (`src/tui/app.rs`) : retirer le champ `pub theme: …` + son init dans `App::new()`.
- [ ] **Step 3 : `tui::run(ascii)`** (`src/tui/mod.rs`) : garder la signature `run(ascii: bool)` ; remplacer `app.theme = …` par un appel unique `crate::theme::init(ascii);` (avant la boucle). Idem pour le point d'entrée du **shell** (`armadai shell`) si on veut `--ascii` — sinon `theme::init(false)` implicite via le défaut ; pour ce lot, appeler `crate::theme::init(ascii)` dans `tui::run` suffit (le shell hérite du défaut détecté).
- [ ] **Step 4 : `dashboard.rs`** : revert les `app.theme.*` → les fns `crate::theme::…()` (selection/heading/muted/…), comme les autres vues. Réintroduire `use crate::theme;` si besoin. Le search_bar : si T1 lui a passé le thème, revenir à la signature d'origine (il lit `crate::theme::…` directement).
- [ ] **Step 5 : `cli/mod.rs`** : garder `--ascii` sur `Tui`, handler `run(ascii)` (inchangé — c'est `run` qui appelle `theme::init`).
- [ ] **Step 6 : Gate** — `cargo build`/`test`/clippy 3 modes/fmt tous verts ; `grep -rn "app.theme\|tui::theme\|crate::tui::theme" src/` → 0 ; `grep -rn "Color::Rgb" src/theme.rs` autorisé (accents), ailleurs inchangé.
- [ ] **Step 7 : Commit** `git commit -m "refactor(tui): drop parallel theme module, route through the shared src/theme.rs"`

---

## Self-Review
- **Architecture** : un seul thème (`src/theme.rs`, tui+shell) ✓ ; identité DS sur accents+signaux avec tier ✓ ; textes/muted nommés adaptatifs (lisible fond clair — le bug de Dimitri) ✓ ; module parallèle supprimé ✓ ; vues/shell inchangés (API `theme::`) ✓.
- **Placeholders** : code theme.rs concret ; les remaps de sémantique listés.
- **Cohérence** : `theme::init(ascii)` appelé dans `tui::run` ; accents résolus via `settings().tier` ; tests n'asservissent pas les valeurs tier-dépendantes.
- **Risque** : consts publiques `SELECTION`/etc. si référencées hors les fns → grep + adapter (Step 2). L'ordre d'`init` : `OnceLock` idempotent, `settings()` fallback détecte si `init` pas appelé (tests/shell).
