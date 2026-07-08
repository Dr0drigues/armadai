//! Audit of native agentic configurations (adoption funnel).
//!
//! Reads native CLI configs (Claude Code first) through `ReverseLinker`s,
//! runs static rules over the imported assets and produces an `AuditReport`.
pub mod reverse;
pub mod rules;
