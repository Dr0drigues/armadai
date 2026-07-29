//! Module tree for the e2e harness: case file format, the generic runner that
//! executes cases against the `fake-claude` stub, and declarative expect evaluation.

pub mod case;
pub mod harness;
mod hook_stdout;
pub mod report;
pub mod runner;
