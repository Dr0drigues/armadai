//! Shell module for ArmadAI interactive mode
//!
//! This module provides the parser and protocol support for the ArmadAI shell,
//! including marker detection for end-of-response, delegation, and metadata extraction.

pub mod detect;
pub mod parser;
pub mod runner;

#[cfg(feature = "tui")]
pub mod tui;

#[cfg(feature = "tui")]
pub mod app;

// Re-exported for external use when shell command is implemented
#[allow(unused_imports)]
pub use parser::{ParsedResponse, parse_response};
#[allow(unused_imports)]
pub use runner::{Message, MessageRole, RunnerConfig, SessionMetrics, ShellRunner, TurnMetrics};
