# Design System → TUI, T1 (fondation thème + dashboard) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Créer le module thème central `src/tui/theme.rs` (palette DS + dégradation truecolor→256→16 + glyphes/`--ascii`) et re-skiner la vue dashboard pour l'utiliser, sans changer les autres vues.

**Architecture:** `theme.rs` expose `Theme` (tier couleur détecté + drapeau ascii) résolvant chaque token DS en `ratatui::style::Color` selon la capacité du terminal, plus des helpers de style et un jeu de glyphes. `App` porte un `theme: Theme` ; `tui::run(ascii)` le construit ; les vues lisent `app.theme`. T1 re-skine seulement le dashboard (pilote).

**Tech Stack:** Rust edition 2024, ratatui/crossterm (feature `tui`), clap.

## Global Constraints
- Feature `tui`. Gate CI : clippy **3 modes** (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`.
- Valeurs de palette = `assets/terminal-palette.json` du DS (exactes, ci-dessous).
- Tests touchant l'environnement (`COLORTERM`/`TERM`/`NO_COLOR`) : **sérialiser via `crate::core::config::ENV_MUTEX`** (cf. fix flaky récent) ou injecter le tier explicitement plutôt que lire l'env — préférer une fonction testable `ColorTier::from_env_values(colorterm, term, no_color)` pure + un `detect()` qui lit l'env.
- `--ascii` = flag CLI uniquement (décision Dimitri), sur `armadai tui` en T1.
- Ne re-skiner que le **dashboard** en T1 ; les autres vues restent inchangées (T2). Validation : `armadai tui`.

---

### Task 1: Module `theme.rs`

**Files:**
- Create: `src/tui/theme.rs`
- Modify: `src/tui/mod.rs` (déclarer `pub mod theme;`)

**Interfaces:**
- Produces : `ColorTier`, `Signal`, `Theme` (`detect(ascii)`, `brass()`, `brass_strong()`, `signal(Signal)`, `text_primary()/secondary()/muted()`, `surface_bg()/panel()`, `border()`, helpers de style, `glyphs()`), `Glyphs`.

- [ ] **Step 1 : Écrire les tests (TDD)** — dans `src/tui/theme.rs` `#[cfg(test)] mod tests` :
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn tier_from_env_values() {
        assert_eq!(ColorTier::from_env_values(Some("truecolor"), None, false), ColorTier::Truecolor);
        assert_eq!(ColorTier::from_env_values(Some("24bit"), None, false), ColorTier::Truecolor);
        assert_eq!(ColorTier::from_env_values(None, Some("xterm-256color"), false), ColorTier::Xterm256);
        assert_eq!(ColorTier::from_env_values(None, Some("xterm"), false), ColorTier::Ansi16);
        assert_eq!(ColorTier::from_env_values(Some("truecolor"), None, true), ColorTier::Ansi16); // NO_COLOR wins
    }

    #[test]
    fn brass_resolves_per_tier() {
        let tc = Theme { tier: ColorTier::Truecolor, ascii: false };
        assert_eq!(tc.brass(), Color::Rgb(0xc7, 0x9a, 0x4a));
        let x = Theme { tier: ColorTier::Xterm256, ascii: false };
        assert_eq!(x.brass(), Color::Indexed(179));
        let a = Theme { tier: ColorTier::Ansi16, ascii: false };
        assert_eq!(a.brass(), Color::Yellow);
    }

    #[test]
    fn glyphs_switch_on_ascii() {
        let uni = Theme { tier: ColorTier::Truecolor, ascii: false }.glyphs();
        let asc = Theme { tier: ColorTier::Truecolor, ascii: true }.glyphs();
        assert_ne!(uni.flag_running, asc.flag_running);
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — `cargo test --no-default-features --features tui theme::` → FAIL (types absents).

- [ ] **Step 3 : Implémenter** `src/tui/theme.rs` :
```rust
//! Central theme for the TUI: the design-system "command bridge" palette
//! (brass + chart-blue + signal flags) resolved to ratatui colors, degrading
//! truecolor -> xterm-256 -> ansi-16 by terminal capability, plus a glyph set
//! with an ASCII fallback. Views read colors/glyphs through `App::theme` and
//! never build `Color::` literals directly.

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorTier {
    Truecolor,
    Xterm256,
    Ansi16,
}

impl ColorTier {
    /// Pure decision from raw env values (testable without touching the env).
    pub fn from_env_values(colorterm: Option<&str>, term: Option<&str>, no_color: bool) -> Self {
        if no_color {
            return ColorTier::Ansi16;
        }
        if matches!(colorterm, Some("truecolor") | Some("24bit")) {
            return ColorTier::Truecolor;
        }
        match term {
            Some(t) if t.contains("256color") => ColorTier::Xterm256,
            _ => ColorTier::Ansi16,
        }
    }

    /// Detect from the process environment.
    pub fn detect() -> Self {
        let colorterm = std::env::var("COLORTERM").ok();
        let term = std::env::var("TERM").ok();
        let no_color = std::env::var_os("NO_COLOR").is_some();
        Self::from_env_values(colorterm.as_deref(), term.as_deref(), no_color)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Signal {
    Ok,
    Warning,
    Critical,
    Running,
    Halted,
}

/// A palette token carrying its three-tier representation.
struct Tok {
    rgb: (u8, u8, u8),
    x256: u8,
    ansi: Color,
}
impl Tok {
    const fn new(rgb: (u8, u8, u8), x256: u8, ansi: Color) -> Self {
        Tok { rgb, x256, ansi }
    }
    fn resolve(&self, tier: ColorTier) -> Color {
        match tier {
            ColorTier::Truecolor => Color::Rgb(self.rgb.0, self.rgb.1, self.rgb.2),
            ColorTier::Xterm256 => Color::Indexed(self.x256),
            ColorTier::Ansi16 => self.ansi,
        }
    }
}

// Values from assets/terminal-palette.json.
const BRASS: Tok = Tok::new((0xc7, 0x9a, 0x4a), 179, Color::Yellow);
const BRASS_STRONG: Tok = Tok::new((0xd6, 0xad, 0x5f), 215, Color::LightYellow);
const SIGNAL_OK: Tok = Tok::new((0x5c, 0xbf, 0x87), 114, Color::Green);
const SIGNAL_WARNING: Tok = Tok::new((0xe2, 0xb2, 0x4c), 214, Color::LightYellow);
const SIGNAL_CRITICAL: Tok = Tok::new((0xd7, 0x5f, 0x4d), 167, Color::Red);
const SIGNAL_RUNNING: Tok = Tok::new((0x57, 0xa9, 0xcc), 74, Color::Cyan);
const SIGNAL_HALTED: Tok = Tok::new((0x8a, 0x92, 0x9b), 245, Color::DarkGray);
const TEXT_PRIMARY: Tok = Tok::new((0xf1, 0xf4, 0xf6), 255, Color::White);
const TEXT_SECONDARY: Tok = Tok::new((0xc2, 0xc9, 0xcf), 251, Color::Gray);
const TEXT_MUTED: Tok = Tok::new((0x9a, 0xa4, 0xad), 245, Color::DarkGray);
const SURFACE_BG: Tok = Tok::new((0x0f, 0x1b, 0x2d), 234, Color::Black);
const SURFACE_PANEL: Tok = Tok::new((0x1c, 0x2b, 0x40), 236, Color::Black);
const BORDER: Tok = Tok::new((0x3a, 0x4a, 0x60), 239, Color::DarkGray);

#[derive(Clone, Copy)]
pub struct Glyphs {
    pub flag_running: &'static str,
    pub flag_ok: &'static str,
    pub bullet: &'static str,
    pub arrow: &'static str,
}
impl Glyphs {
    const UNICODE: Glyphs = Glyphs { flag_running: "⚑", flag_ok: "◆", bullet: "●", arrow: "→" };
    const ASCII: Glyphs = Glyphs { flag_running: "*", flag_ok: "#", bullet: "-", arrow: "->" };
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub tier: ColorTier,
    pub ascii: bool,
}

impl Theme {
    pub fn detect(ascii: bool) -> Self {
        Theme { tier: ColorTier::detect(), ascii }
    }
    pub fn brass(&self) -> Color { BRASS.resolve(self.tier) }
    pub fn brass_strong(&self) -> Color { BRASS_STRONG.resolve(self.tier) }
    pub fn text_primary(&self) -> Color { TEXT_PRIMARY.resolve(self.tier) }
    pub fn text_secondary(&self) -> Color { TEXT_SECONDARY.resolve(self.tier) }
    pub fn text_muted(&self) -> Color { TEXT_MUTED.resolve(self.tier) }
    pub fn surface_bg(&self) -> Color { SURFACE_BG.resolve(self.tier) }
    pub fn surface_panel(&self) -> Color { SURFACE_PANEL.resolve(self.tier) }
    pub fn border(&self) -> Color { BORDER.resolve(self.tier) }
    pub fn signal(&self, s: Signal) -> Color {
        match s {
            Signal::Ok => SIGNAL_OK,
            Signal::Warning => SIGNAL_WARNING,
            Signal::Critical => SIGNAL_CRITICAL,
            Signal::Running => SIGNAL_RUNNING,
            Signal::Halted => SIGNAL_HALTED,
        }
        .resolve(self.tier)
    }
    /// Accent style for active/selected items (brass, bold).
    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.brass()).add_modifier(Modifier::BOLD)
    }
    /// Style for panel borders.
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border())
    }
    pub fn glyphs(&self) -> Glyphs {
        if self.ascii { Glyphs::ASCII } else { Glyphs::UNICODE }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme { tier: ColorTier::detect(), ascii: false }
    }
}
```
Déclarer `pub mod theme;` dans `src/tui/mod.rs`.

- [ ] **Step 4 : Vérifier vert** — `cargo test --no-default-features --features tui theme::` → PASS (3 tests).

- [ ] **Step 5 : clippy 3 modes + fmt.** Expected 0 warning, fmt propre.

- [ ] **Step 6 : Commit**
```bash
git add src/tui/theme.rs src/tui/mod.rs
git commit -m "feat(tui): central theme module (DS palette, tier fallback, glyphs)"
```

---

### Task 2: `--ascii` flag, thème dans `App`, re-skin dashboard

**Files:**
- Modify: `src/cli/mod.rs` (`Tui { global }` → ajouter `ascii: bool` ; handler passe `ascii`)
- Modify: `src/tui/mod.rs` (`run()` → `run(ascii: bool)`, régler `app.theme`)
- Modify: `src/tui/app.rs` (`App` : champ `pub theme: theme::Theme` ; `App::new()` l'initialise à `Theme::default()`)
- Modify: `src/tui/views/dashboard.rs` (remplacer les `Color::` par `app.theme.*`)

**Interfaces:**
- Consumes : `Theme` (Task 1).

- [ ] **Step 1 : Flag `--ascii` sur `Tui`** — dans `src/cli/mod.rs`, ajouter au variant `Tui` :
```rust
    Tui {
        /// Show agents from the global library (~/.config/armadai/) only
        #[arg(long)]
        global: bool,
        /// Use ASCII glyphs instead of Unicode (for limited terminals)
        #[arg(long)]
        ascii: bool,
    },
```
Et le handler (`Command::Tui { global, ascii } => { … crate::tui::run(ascii).await }`).

- [ ] **Step 2 : `App` porte le thème** — `src/tui/app.rs` : ajouter `pub theme: crate::tui::theme::Theme` au `struct App` ; dans `App::new()`, `theme: crate::tui::theme::Theme::default()`.

- [ ] **Step 3 : `tui::run(ascii)`** — `src/tui/mod.rs` : `pub async fn run(ascii: bool) -> Result<()>` ; après `let mut app = app::App::new();`, `app.theme = app::App::theme_for(ascii);` — plus simple : `app.theme = crate::tui::theme::Theme::detect(ascii);`.

- [ ] **Step 4 : Re-skin `dashboard.rs`** — remplacer les `Color::` (header, onglets, liste d'agents) par les helpers du thème via `app.theme` : onglet actif = `app.theme.accent_style()` ; bordures de bloc = `app.theme.border_style()` ; texte principal/secondaire = `text_primary()/secondary()` ; statuts éventuels = `signal(...)`. Reprendre la disposition master-detail existante (ne pas la réécrire). S'appuyer sur l'écran DS `ui_kits/tui/dashboard-ux.html` pour l'esprit (accent laiton, surfaces sobres).

- [ ] **Step 5 : build + gate** — `cargo build --no-default-features --features tui` + `cargo test --no-default-features --features tui` + clippy 3 modes + fmt. Lancer `cargo run --no-default-features --features tui -- tui` (+ `--ascii`, + `COLORTERM= cargo run … tui` pour tester le fallback) et vérifier visuellement le dashboard (accent laiton, lisible en truecolor / 256 / 16 / ascii).

- [ ] **Step 6 : Commit**
```bash
git add src/cli/mod.rs src/tui/mod.rs src/tui/app.rs src/tui/views/dashboard.rs
git commit -m "feat(tui): --ascii flag, thread theme through App, reskin the dashboard"
```

---

## Self-Review
- **Couverture** : module thème (Task 1) ✓ ; détection tier testable (`from_env_values`) sans race env ✓ ; `--ascii` CLI + thème dans App + dashboard re-skiné (Task 2) ✓ ; autres vues intouchées (T2) ✓.
- **Placeholders** : code theme.rs complet ; les remplacements dashboard référencent les helpers (`accent_style`/`border_style`/`text_*`/`signal`) — concret.
- **Cohérence types** : `Theme`/`ColorTier`/`Signal`/`Glyphs` cohérents Task 1↔2 ; `App.theme: Theme` ; `run(ascii: bool)`.
- **Env/tests** : détection via `from_env_values` pur (testé) ; `detect()` lit l'env seulement à l'init (pas dans les tests) → pas de race `set_var`.
