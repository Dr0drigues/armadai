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
// `#[allow(dead_code)]`: this lot only lands the module; call sites in `cli/*`
// wire these up in a follow-up lot (same convention as `crate::theme`).
#[allow(dead_code)]
const BRASS: Color = Color::Rgb(RgbColor(0xc7, 0x9a, 0x4a));
#[allow(dead_code)]
const SIGNAL_OK: Color = Color::Rgb(RgbColor(0x5c, 0xbf, 0x87));
#[allow(dead_code)]
const SIGNAL_WARNING: Color = Color::Rgb(RgbColor(0xe2, 0xb2, 0x4c));
#[allow(dead_code)]
const SIGNAL_CRITICAL: Color = Color::Rgb(RgbColor(0xd7, 0x5f, 0x4d));
#[allow(dead_code)]
const SIGNAL_RUNNING: Color = Color::Rgb(RgbColor(0x57, 0xa9, 0xcc));

/// Section heading / active element: bold brass.
#[allow(dead_code)]
pub fn header() -> Style {
    Style::new().bold().fg_color(Some(BRASS))
}
/// Accent (brass) without bold.
#[allow(dead_code)]
pub fn accent() -> Style {
    Style::new().fg_color(Some(BRASS))
}
/// Success / done status.
#[allow(dead_code)]
pub fn ok() -> Style {
    Style::new().fg_color(Some(SIGNAL_OK))
}
/// Warning status.
#[allow(dead_code)]
pub fn warn() -> Style {
    Style::new().fg_color(Some(SIGNAL_WARNING))
}
/// Error / critical status.
#[allow(dead_code)]
pub fn err() -> Style {
    Style::new().fg_color(Some(SIGNAL_CRITICAL))
}
/// In-progress / running status.
#[allow(dead_code)]
pub fn running() -> Style {
    Style::new().fg_color(Some(SIGNAL_RUNNING))
}
/// Secondary / muted text: bright-black (named, adapts to terminal theme).
#[allow(dead_code)]
pub fn muted() -> Style {
    Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)))
}
/// Agent / role name: bold, no colour (avoids clashing with status colours).
#[allow(dead_code)]
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
        assert!(
            on.into_inner().contains(&ESC),
            "expected ANSI codes when forced on"
        );

        // Forced colour OFF → escapes stripped.
        let mut off = anstream::AutoStream::never(Vec::new());
        write!(off, "{styled}").unwrap();
        assert!(
            !off.into_inner().contains(&ESC),
            "expected no ANSI codes when forced off"
        );
    }
}
