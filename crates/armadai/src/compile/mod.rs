//! Compilation of the canonical model (`armadai-model`) into per-target artifacts.
//!
//! A compiler only plans: it returns the files to write and the conformance report.
//! Writing goes through `linker::manifest` (whole files) and `compile::patch`
//! (files shared with the user, such as `.claude/settings.json`).
// Temporary: not wired into `link` until step 2's Task 6, which removes this allow.
#![allow(dead_code)]

pub mod report;

use std::path::{Path, PathBuf};

use armadai_model::Project;

use crate::linker::manifest::ProducedBy;
pub use report::Report;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Agent,
    Skill,
    Instructions,
    Settings,
    Native,
}

/// One whole file a target needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// Relative to the target's output directory (`.claude` for Claude Code).
    pub path: PathBuf,
    pub content: Vec<u8>,
    pub kind: ArtifactKind,
    pub produced_by: ProducedBy,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub artifacts: Vec<Artifact>,
    /// Markdown blocks patched into files the user may own: (path relative to
    /// the project root, block content without the ArmadAI markers).
    pub blocks: Vec<(PathBuf, String)>,
    pub report: Report,
}

pub trait Compiler {
    fn target(&self) -> &'static str;
    /// `root` is the project root (the directory holding `.armadai/`).
    fn compile(&self, project: &Project, root: &Path) -> anyhow::Result<Plan>;
}
