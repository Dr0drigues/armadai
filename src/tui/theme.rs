//! Central theme for the TUI: the design-system "command bridge" palette
//! (brass + chart-blue + signal flags) resolved to ratatui colors, degrading
//! truecolor -> xterm-256 -> ansi-16 by terminal capability, plus a glyph set
//! with an ASCII fallback. Views read colors/glyphs through `App::theme` and
//! never build `Color::` literals directly.

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ColorTier {
    Truecolor,
    Xterm256,
    Ansi16,
}

#[allow(dead_code)]
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
#[allow(dead_code)]
pub enum Signal {
    Ok,
    Warning,
    Critical,
    Running,
    Halted,
}

/// A palette token carrying its three-tier representation.
#[allow(dead_code)]
struct Tok {
    rgb: (u8, u8, u8),
    x256: u8,
    ansi: Color,
}
#[allow(dead_code)]
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
// (These palette tokens are used by Theme methods and will be fully utilized in Task 2)
#[allow(dead_code)]
const BRASS: Tok = Tok::new((0xc7, 0x9a, 0x4a), 179, Color::Yellow);
#[allow(dead_code)]
const BRASS_STRONG: Tok = Tok::new((0xd6, 0xad, 0x5f), 215, Color::LightYellow);
#[allow(dead_code)]
const SIGNAL_OK: Tok = Tok::new((0x5c, 0xbf, 0x87), 114, Color::Green);
#[allow(dead_code)]
const SIGNAL_WARNING: Tok = Tok::new((0xe2, 0xb2, 0x4c), 214, Color::LightYellow);
#[allow(dead_code)]
const SIGNAL_CRITICAL: Tok = Tok::new((0xd7, 0x5f, 0x4d), 167, Color::Red);
#[allow(dead_code)]
const SIGNAL_RUNNING: Tok = Tok::new((0x57, 0xa9, 0xcc), 74, Color::Cyan);
#[allow(dead_code)]
const SIGNAL_HALTED: Tok = Tok::new((0x8a, 0x92, 0x9b), 245, Color::DarkGray);
#[allow(dead_code)]
const TEXT_PRIMARY: Tok = Tok::new((0xf1, 0xf4, 0xf6), 255, Color::White);
#[allow(dead_code)]
const TEXT_SECONDARY: Tok = Tok::new((0xc2, 0xc9, 0xcf), 251, Color::Gray);
#[allow(dead_code)]
const TEXT_MUTED: Tok = Tok::new((0x9a, 0xa4, 0xad), 245, Color::DarkGray);
#[allow(dead_code)]
const SURFACE_BG: Tok = Tok::new((0x0f, 0x1b, 0x2d), 234, Color::Black);
#[allow(dead_code)]
const SURFACE_PANEL: Tok = Tok::new((0x1c, 0x2b, 0x40), 236, Color::Black);
#[allow(dead_code)]
const BORDER: Tok = Tok::new((0x3a, 0x4a, 0x60), 239, Color::DarkGray);

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Glyphs {
    pub flag_running: &'static str,
    pub flag_ok: &'static str,
    pub bullet: &'static str,
    pub arrow: &'static str,
}
impl Glyphs {
    // Glyph sets (will be used by views in Task 2)
    #[allow(dead_code)]
    const UNICODE: Glyphs = Glyphs {
        flag_running: "⚑",
        flag_ok: "◆",
        bullet: "●",
        arrow: "→",
    };
    #[allow(dead_code)]
    const ASCII: Glyphs = Glyphs {
        flag_running: "*",
        flag_ok: "#",
        bullet: "-",
        arrow: "->",
    };
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Theme {
    pub tier: ColorTier,
    pub ascii: bool,
}

#[allow(dead_code)]
impl Theme {
    pub fn detect(ascii: bool) -> Self {
        Theme {
            tier: ColorTier::detect(),
            ascii,
        }
    }
    pub fn brass(&self) -> Color {
        BRASS.resolve(self.tier)
    }
    pub fn brass_strong(&self) -> Color {
        BRASS_STRONG.resolve(self.tier)
    }
    pub fn text_primary(&self) -> Color {
        TEXT_PRIMARY.resolve(self.tier)
    }
    pub fn text_secondary(&self) -> Color {
        TEXT_SECONDARY.resolve(self.tier)
    }
    pub fn text_muted(&self) -> Color {
        TEXT_MUTED.resolve(self.tier)
    }
    pub fn surface_bg(&self) -> Color {
        SURFACE_BG.resolve(self.tier)
    }
    pub fn surface_panel(&self) -> Color {
        SURFACE_PANEL.resolve(self.tier)
    }
    pub fn border(&self) -> Color {
        BORDER.resolve(self.tier)
    }
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
        Style::default()
            .fg(self.brass())
            .add_modifier(Modifier::BOLD)
    }
    /// Style for panel borders.
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border())
    }
    pub fn glyphs(&self) -> Glyphs {
        if self.ascii {
            Glyphs::ASCII
        } else {
            Glyphs::UNICODE
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            tier: ColorTier::detect(),
            ascii: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn tier_from_env_values() {
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
    fn brass_resolves_per_tier() {
        let tc = Theme {
            tier: ColorTier::Truecolor,
            ascii: false,
        };
        assert_eq!(tc.brass(), Color::Rgb(0xc7, 0x9a, 0x4a));
        let x = Theme {
            tier: ColorTier::Xterm256,
            ascii: false,
        };
        assert_eq!(x.brass(), Color::Indexed(179));
        let a = Theme {
            tier: ColorTier::Ansi16,
            ascii: false,
        };
        assert_eq!(a.brass(), Color::Yellow);
    }

    #[test]
    fn glyphs_switch_on_ascii() {
        let uni = Theme {
            tier: ColorTier::Truecolor,
            ascii: false,
        }
        .glyphs();
        let asc = Theme {
            tier: ColorTier::Truecolor,
            ascii: true,
        }
        .glyphs();
        assert_ne!(uni.flag_running, asc.flag_running);
    }
}
