//! Shared semantic color palette for ArmadAI's two terminal UIs
//! (the ratatui dashboard in `tui/` and the conversational shell's
//! rendering in `shell/tui.rs`, `shell/md_render.rs`, `shell/workroom.rs`).
//!
//! Every color here is a ratatui **named** color, never `Color::Rgb(...)`.
//! Named colors are resolved against the terminal's own ANSI palette, so
//! they stay legible whether the user runs a dark or a light terminal
//! theme. `Rgb(...)` bypasses that palette and can produce (near-)invisible
//! text on a terminal theme the author didn't test against — see
//! `docs/proposals/ux-audit-tui-2026-07-20.md` (P0-1, P0-2, P1-8).
//!
//! Keep this module the single source of truth for "what color means what"
//! across both UIs: selection, state, role, and a couple of shared accents.
//! Anything not covered here (borders, one-off accents) is left to local
//! judgement — see the audit's P1-8 for the deliberately-out-of-scope list.

#![cfg(feature = "tui")]

use ratatui::style::{Color, Modifier, Style};

// ── Semantic colors ──────────────────────────────────────────────

/// Selected row / active element (dashboard lists, palette, workroom focus).
pub const SELECTION: Color = Color::Cyan;
/// Section headings / titles (dashboard detail titles, panel titles).
pub const HEADING: Color = Color::Cyan;

/// An agent or turn is actively generating.
pub const WORKING: Color = Color::Green;
/// A coordinator/lead is waiting on a delegated sub-agent.
pub const DELEGATING: Color = Color::Yellow;
/// An agent has finished its work. Distinct from `WORKING` (Blue vs Green)
/// so "still going" and "finished" never look the same at a glance.
pub const DONE: Color = Color::Blue;
/// Idle / not-yet-involved / de-emphasized text.
pub const MUTED: Color = Color::DarkGray;

/// Errors. Not yet wired to a call site in either UI (neither currently
/// renders an error/failure banner) — reserved so the next consumer (e.g.
/// a shell error banner, a failed-run indicator) has a semantic slot to
/// reach for instead of inventing another ad-hoc red.
#[allow(dead_code)]
pub const ERROR: Color = Color::Red;
/// Warnings (distinct semantic slot from `DELEGATING`, same color today).
/// Reserved for the same reason as `ERROR`.
#[allow(dead_code)]
pub const WARNING: Color = Color::Yellow;

/// Orchestration coordinator role.
pub const ROLE_COORDINATOR: Color = Color::Magenta;
/// Orchestration lead role.
pub const ROLE_LEAD: Color = Color::Yellow;
// Plain agent role deliberately has NO color constant: it renders in the
// terminal's default foreground (bold only) so it never collides with
// `DONE` (Blue) when an agent's row also shows its finished state.

/// Agent tag accent (metadata tags in the dashboard).
pub const TAG: Color = Color::Yellow;
/// Agent stack accent (metadata stacks in the dashboard).
pub const STACK: Color = Color::Green;

// ── Style helpers ─────────────────────────────────────────────────

/// Style for a selected row / active element: bold `SELECTION`.
pub fn selection() -> Style {
    Style::default().fg(SELECTION).add_modifier(Modifier::BOLD)
}

/// Style for a section heading / panel title: bold `HEADING`.
pub fn heading() -> Style {
    Style::default().fg(HEADING).add_modifier(Modifier::BOLD)
}

/// Style for "working" state text.
pub fn working() -> Style {
    Style::default().fg(WORKING)
}

/// Style for "delegating" state text.
pub fn delegating() -> Style {
    Style::default().fg(DELEGATING)
}

/// Style for "done" state text.
pub fn done() -> Style {
    Style::default().fg(DONE)
}

/// Style for muted / idle text.
pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

/// Style for error text. See `ERROR` for why this has no call site yet.
#[allow(dead_code)]
pub fn error() -> Style {
    Style::default().fg(ERROR)
}

/// Style for warning text. See `WARNING` for why this has no call site yet.
#[allow(dead_code)]
pub fn warning() -> Style {
    Style::default().fg(WARNING)
}

/// Style for a coordinator role label: bold `ROLE_COORDINATOR`.
pub fn role_coordinator() -> Style {
    Style::default()
        .fg(ROLE_COORDINATOR)
        .add_modifier(Modifier::BOLD)
}

/// Style for a lead role label: bold `ROLE_LEAD`.
pub fn role_lead() -> Style {
    Style::default().fg(ROLE_LEAD).add_modifier(Modifier::BOLD)
}

/// Style for a plain agent role label: bold, no color (see `ROLE_AGENT` note
/// above — avoids colliding with `DONE`).
pub fn role_agent() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// Style for a tag accent.
pub fn tag() -> Style {
    Style::default().fg(TAG)
}

/// Style for a stack accent.
pub fn stack() -> Style {
    Style::default().fg(STACK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_colors_are_named_not_rgb() {
        // Sanity: these consts are the exact named colors the palette
        // sign-off specified. A future accidental edit to `Color::Rgb(...)`
        // would break this and be caught immediately.
        assert_eq!(SELECTION, Color::Cyan);
        assert_eq!(HEADING, Color::Cyan);
        assert_eq!(WORKING, Color::Green);
        assert_eq!(DELEGATING, Color::Yellow);
        assert_eq!(DONE, Color::Blue);
        assert_eq!(MUTED, Color::DarkGray);
        assert_eq!(ERROR, Color::Red);
        assert_eq!(WARNING, Color::Yellow);
        assert_eq!(ROLE_COORDINATOR, Color::Magenta);
        assert_eq!(ROLE_LEAD, Color::Yellow);
        assert_eq!(TAG, Color::Yellow);
        assert_eq!(STACK, Color::Green);
    }

    #[test]
    fn done_is_distinct_from_working() {
        // The whole point of P0-1: idle/done/working must be visually
        // distinguishable, not all shades of gray/green.
        assert_ne!(DONE, WORKING);
    }

    #[test]
    fn role_agent_style_has_no_explicit_color() {
        assert_eq!(role_agent().fg, None);
    }

    #[test]
    fn selection_and_heading_styles_are_bold() {
        assert!(selection().add_modifier.contains(Modifier::BOLD));
        assert!(heading().add_modifier.contains(Modifier::BOLD));
    }
}
