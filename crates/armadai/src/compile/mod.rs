//! Compilation of the canonical model (`armadai-model`) into per-target artifacts.
//!
//! A compiler only plans: it returns the files to write and the conformance report.
//! Writing goes through `linker::manifest` (whole files) and `compile::patch`
//! (files shared with the user, such as `.claude/settings.json`).
// Temporary: not wired into `link` until step 2's Task 6, which removes this allow.
#![allow(dead_code)]

pub mod claude;
pub mod claude_settings;
pub mod legacy_targets;
pub mod patch;
pub mod report;

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_model::Project;

use crate::linker::manifest::ProducedBy;
pub use report::{Report, Status};

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

/// Markers around the ArmadAI block of a markdown file the user may also edit.
pub const BLOCK_BEGIN: &str = "<!-- armadai:begin -->";
pub const BLOCK_END: &str = "<!-- armadai:end -->";

/// Every file under `dir` as (path relative to `dir`, bytes), sorted. A file whose
/// real path leaves `dir` (a symlink pointing out) is an error naming it.
pub fn read_tree(dir: &Path) -> anyhow::Result<Vec<(PathBuf, Vec<u8>)>> {
    let base = dir
        .canonicalize()
        .with_context(|| format!("cannot read {}", dir.display()))?;
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in
            std::fs::read_dir(&d).with_context(|| format!("cannot read {}", d.display()))?
        {
            let path = entry?.path();
            let real = path
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", path.display()))?;
            if !real.starts_with(&base) {
                bail!("{} points outside {}", path.display(), dir.display());
            }
            if real.is_dir() {
                todo.push(path);
            } else {
                let bytes = std::fs::read(&path)
                    .with_context(|| format!("cannot read {}", path.display()))?;
                out.push((path.strip_prefix(dir)?.to_path_buf(), bytes));
            }
        }
    }
    out.sort();
    Ok(out)
}

pub trait Compiler {
    fn target(&self) -> &'static str;
    /// `root` is the project root (the directory holding `.armadai/`).
    fn compile(&self, project: &Project, root: &Path) -> anyhow::Result<Plan>;
}
