//! Shell module for ArmadAI interactive mode
//!
//! This module provides the parser and protocol support for the ArmadAI shell,
//! including marker detection for end-of-response, delegation, and metadata extraction.

pub mod parser;

// Re-exported for external use when shell command is implemented
#[allow(unused_imports)]
pub use parser::{ParsedResponse, parse_response};
