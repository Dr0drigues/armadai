//! FROZEN copy of `crates/armadai/src/cli/style.rs` at commit c486959,
//! reduced to the styles the audit engine prints with. Not kept in sync.
//!
//! ANSI styling for human CLI output (design system "pont de commandement").
//! **Accent-only**: body text keeps the terminal's default foreground; only
//! accents, status signals and secondary text are coloured. Colour on/off is
//! delegated entirely to `anstream` (respects `NO_COLOR`/`CLICOLOR`/TTY).

use anstyle::{AnsiColor, Color, RgbColor, Style};

// Design-system accents (assets/terminal-palette.json).
const BRASS: Color = Color::Rgb(RgbColor(0xc7, 0x9a, 0x4a));
const SIGNAL_OK: Color = Color::Rgb(RgbColor(0x5c, 0xbf, 0x87));
const SIGNAL_WARNING: Color = Color::Rgb(RgbColor(0xe2, 0xb2, 0x4c));
const SIGNAL_CRITICAL: Color = Color::Rgb(RgbColor(0xd7, 0x5f, 0x4d));

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
/// Secondary / muted text: bright-black (named, adapts to terminal theme).
pub fn muted() -> Style {
    Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)))
}
