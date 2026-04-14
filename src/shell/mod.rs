//! Shell module for ArmadAI interactive mode
//!
//! This module provides the parser and protocol support for the ArmadAI shell,
//! including marker detection for end-of-response, delegation, and metadata extraction.

pub mod config;
pub mod detect;
pub mod json_runner;
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
pub mod workroom;

// Re-exported for external use when shell command is implemented
#[allow(unused_imports)]
pub use parser::{ParsedResponse, parse_response};
#[allow(unused_imports)]
pub use runner::{Message, MessageRole, RunnerConfig, SessionMetrics, ShellRunner, TurnMetrics};
