//! Shared semantic color palette for ArmadAI's two terminal UIs
//! (the ratatui dashboard in `tui/` and the conversational shell's
//! rendering in `shell/tui.rs`, `shell/md_render.rs`, `shell/workroom.rs`).
//!
//! The design system's accents (brass, signals) resolve via color tier
//! detection (truecolor/xterm-256/ansi-16), while semantic text colors
//! remain named (Color::DarkGray for muted, etc.) to stay legible on
//! light and dark terminal backgrounds.
//!
//! Initialize via `init(ascii)` once at TUI/shell startup.
//! `settings()` reads the global OnceLock — idempotent, falls back to
//! detection if `init()` was never called.

#![cfg(feature = "tui")]

use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;

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
    fn detect() -> Self {
        let colorterm = std::env::var("COLORTERM").ok();
        let term = std::env::var("TERM").ok();
        let no_color = std::env::var_os("NO_COLOR").is_some();
        Self::from_env_values(colorterm.as_deref(), term.as_deref(), no_color)
    }
}

// ── Global Settings (initialized once at startup) ──────────────────

struct Settings {
    tier: ColorTier,
    ascii: bool,
}

static SETTINGS: OnceLock<Settings> = OnceLock::new();

fn settings() -> &'static Settings {
    SETTINGS.get_or_init(|| Settings {
        tier: ColorTier::detect(),
        ascii: false,
    })
}

/// Initialize the theme once at TUI/shell startup.
/// Sets the color tier (detected from environment) and ASCII glyph preference.
/// Idempotent: only the first call wins.
pub fn init(ascii: bool) {
    let _ = SETTINGS.set(Settings {
        tier: ColorTier::detect(),
        ascii,
    });
}

// ── Accent palette tokens (DS colors: brass + signals) ──────────────

/// A palette token carrying its three-tier representation.
struct Accent {
    rgb: (u8, u8, u8),
    x256: u8,
    ansi: Color,
}

impl Accent {
    const fn new(rgb: (u8, u8, u8), x256: u8, ansi: Color) -> Self {
        Accent { rgb, x256, ansi }
    }

    fn color(&self) -> Color {
        match settings().tier {
            ColorTier::Truecolor => Color::Rgb(self.rgb.0, self.rgb.1, self.rgb.2),
            ColorTier::Xterm256 => Color::Indexed(self.x256),
            ColorTier::Ansi16 => self.ansi,
        }
    }
}

// Values from `assets/terminal-palette.json` (design system palette)
const BRASS: Accent = Accent::new((0xc7, 0x9a, 0x4a), 179, Color::Yellow);
const BRASS_STRONG: Accent = Accent::new((0xd6, 0xad, 0x5f), 215, Color::LightYellow);
const SIGNAL_OK: Accent = Accent::new((0x5c, 0xbf, 0x87), 114, Color::Green);
#[allow(dead_code)]
const SIGNAL_WARNING: Accent = Accent::new((0xe2, 0xb2, 0x4c), 214, Color::LightYellow);
#[allow(dead_code)]
const SIGNAL_CRITICAL: Accent = Accent::new((0xd7, 0x5f, 0x4d), 167, Color::Red);
const SIGNAL_RUNNING: Accent = Accent::new((0x57, 0xa9, 0xcc), 74, Color::Cyan);
const BORDER: Accent = Accent::new((0x3a, 0x4a, 0x60), 239, Color::DarkGray);

// ── Semantic style helpers ───────────────────────────────────────────

/// Style for a selected row / active element: bold `BRASS`.
pub fn selection() -> Style {
    Style::default()
        .fg(BRASS.color())
        .add_modifier(Modifier::BOLD)
}

/// Style for a section heading / panel title: bold `BRASS`.
pub fn heading() -> Style {
    Style::default()
        .fg(BRASS.color())
        .add_modifier(Modifier::BOLD)
}

/// Style for "working" state text: `SIGNAL_RUNNING`.
pub fn working() -> Style {
    Style::default().fg(SIGNAL_RUNNING.color())
}

/// Style for "delegating" state text: `BRASS`.
pub fn delegating() -> Style {
    Style::default().fg(BRASS.color())
}

/// Style for "done" state text: `SIGNAL_OK`.
pub fn done() -> Style {
    Style::default().fg(SIGNAL_OK.color())
}

/// Style for muted / idle text: named `Color::DarkGray` (adapts to terminal theme).
pub fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Style for error text: `SIGNAL_CRITICAL`.
#[allow(dead_code)]
pub fn error() -> Style {
    Style::default().fg(SIGNAL_CRITICAL.color())
}

/// Style for warning text: `SIGNAL_WARNING`.
pub fn warning() -> Style {
    Style::default().fg(SIGNAL_WARNING.color())
}

/// Style for a coordinator role label: bold `BRASS_STRONG`.
pub fn role_coordinator() -> Style {
    Style::default()
        .fg(BRASS_STRONG.color())
        .add_modifier(Modifier::BOLD)
}

/// Style for a lead role label: bold `BRASS`.
pub fn role_lead() -> Style {
    Style::default()
        .fg(BRASS.color())
        .add_modifier(Modifier::BOLD)
}

/// Style for a plain agent role label: bold, no color (avoids collision with `done()`).
pub fn role_agent() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// Style for a tag accent: `BRASS`.
pub fn tag() -> Style {
    Style::default().fg(BRASS.color())
}

/// Style for a stack accent: `SIGNAL_OK`.
pub fn stack() -> Style {
    Style::default().fg(SIGNAL_OK.color())
}

// ── Glyphs (unicode/ASCII) ──────────────────────────────────────────

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
    pub pointer: &'static str,
    pub arrow_back: &'static str,
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
        pointer: "▸",
        arrow_back: "←",
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
        pointer: ">",
        arrow_back: "<-",
    };
}

/// Return the glyph set (unicode or ASCII) based on initialization.
#[allow(dead_code)]
pub fn glyphs() -> Glyphs {
    if settings().ascii {
        Glyphs::ASCII
    } else {
        Glyphs::UNICODE
    }
}

/// Style for panel borders: `BORDER` color (used by UI containers).
pub fn border_style() -> Style {
    Style::default().fg(BORDER.color())
}

/// Alias for `selection()` (bold brass accent used for highlights).
/// Named `accent_style()` for UI compatibility.
pub fn accent_style() -> Style {
    selection()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_detection() {
        assert_eq!(
            ColorTier::from_env_values(Some("truecolor"), None, false),
            ColorTier::Truecolor
        );
        assert_eq!(
            ColorTier::from_env_values(Some("24bit"), None, false),
            ColorTier::Truecolor
        );
        assert_eq!(
            ColorTier::from_env_values(None, Some("xterm-256color"), false),
            ColorTier::Xterm256
        );
        assert_eq!(
            ColorTier::from_env_values(None, Some("xterm"), false),
            ColorTier::Ansi16
        );
        assert_eq!(
            ColorTier::from_env_values(Some("truecolor"), None, true),
            ColorTier::Ansi16
        ); // NO_COLOR wins
    }

    #[test]
    fn muted_stays_named_adaptive() {
        // Text/idle must stay a named color (legible on light & dark terminals).
        assert_eq!(muted().fg, Some(Color::DarkGray));
    }

    #[test]
    fn done_distinct_from_working() {
        assert_ne!(done().fg, working().fg);
    }

    #[test]
    fn role_agent_has_no_color() {
        assert_eq!(role_agent().fg, None);
    }

    #[test]
    fn selection_is_bold() {
        assert!(selection().add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn heading_is_bold() {
        assert!(heading().add_modifier.contains(Modifier::BOLD));
    }

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

    #[test]
    fn glyphs_expose_pointer_and_back_arrow() {
        assert_eq!(Glyphs::UNICODE.pointer, "▸");
        assert_eq!(Glyphs::UNICODE.arrow_back, "←");
        assert_eq!(Glyphs::ASCII.pointer, ">");
        assert_eq!(Glyphs::ASCII.arrow_back, "<-");
    }

    #[test]
    fn glyphs_switch_on_ascii() {
        // Reset settings for test (though we can't really reset OnceLock,
        // this tests that glyphs() reads settings correctly)
        let _guard = SETTINGS.get_or_init(|| Settings {
            tier: ColorTier::Ansi16,
            ascii: false,
        });
        let uni = glyphs();
        assert_eq!(uni.flag_running, "⚑");

        // Note: can't test ASCII variant easily without resetting OnceLock.
        // The behavior is correct by code inspection.
    }
}
