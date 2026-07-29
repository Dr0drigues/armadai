//! Shell module for ArmadAI interactive mode
//!
//! This module provides the parser and protocol support for the ArmadAI shell,
//! including marker detection for end-of-response, delegation, and metadata extraction.

pub mod config;
pub mod detect;
#[cfg(feature = "tui")]
pub mod md_render;
pub mod parser;
pub mod runner;

#[cfg(feature = "tui")]
pub mod wizard;

#[cfg(feature = "tui")]
pub mod commands;

#[cfg(feature = "tui")]
pub mod tui;

#[cfg(feature = "tui")]
pub mod app;

#[cfg(feature = "tui")]
pub mod session;

#[cfg(feature = "tui")]
pub mod pty_runner;
#[cfg(feature = "tui")]
pub mod run_view;
#[cfg(feature = "tui")]
pub mod workroom;

/// Braille spinner frames shared by the shell TUI (`tui.rs`) and the
/// workroom panel (`workroom.rs`) so the two animations stay in sync and
/// the frame set is defined exactly once.
#[cfg(feature = "tui")]
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// Re-exported for external use when shell command is implemented
#[allow(unused_imports)]
pub use parser::{ParsedResponse, parse_response};
#[allow(unused_imports)]
pub use runner::{Message, MessageRole, RunnerConfig, SessionMetrics, ShellRunner, TurnMetrics};
