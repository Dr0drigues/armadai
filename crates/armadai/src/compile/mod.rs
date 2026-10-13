//! Compilation of the canonical model (`armadai-model`) into per-target artifacts.
//!
//! A compiler only plans: it returns the files to write and the conformance report.
//! Writing goes through `linker::manifest` (whole files) and `compile::patch`
//! (files shared with the user, such as `.claude/settings.json`).

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
    /// Copied from `.armadai/native/<target>/`; its path is relative to the project root.
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

/// Every file under `dir` as (path relative to `dir`, bytes), sorted. Refused, with
/// an error naming it: a symbolic link leaving `dir` or pointing to a directory
/// (never followed: no loop, no duplicate), anything that is not a regular file.
///
/// `dir` itself must resolve under `within` (the project's `.armadai/`): a skill or
/// `native/<target>` folder that is a link leaving the sources is refused too.
pub fn read_tree(dir: &Path, within: &Path) -> anyhow::Result<Vec<(PathBuf, Vec<u8>)>> {
    read_tree_skipping(dir, within).map(|(files, _)| files)
}

/// Files of a folder as (path relative to it, bytes).
pub type Files = Vec<(PathBuf, Vec<u8>)>;

/// A backup `new --force` moves aside (`<file>.armadai-backup`): never a source.
pub fn is_backup(p: &Path) -> bool {
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(".armadai-backup"))
}

/// [`read_tree`], plus the stray backups it left out (relative to `dir`).
pub fn read_tree_skipping(dir: &Path, within: &Path) -> anyhow::Result<(Files, Vec<PathBuf>)> {
    let mut skipped = Vec::new();
    let base = dir
        .canonicalize()
        .with_context(|| format!("cannot read {}", dir.display()))?;
    let allowed = within
        .canonicalize()
        .with_context(|| format!("cannot read {}", within.display()))?;
    if !base.starts_with(&allowed) {
        bail!("{} points outside {}", dir.display(), within.display());
    }
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in
            std::fs::read_dir(&d).with_context(|| format!("cannot read {}", d.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                todo.push(path);
                continue;
            }
            if kind.is_symlink() {
                let real = path
                    .canonicalize()
                    .with_context(|| format!("cannot resolve {}", path.display()))?;
                if !real.starts_with(&base) {
                    bail!("{} points outside {}", path.display(), dir.display());
                }
                if !real.is_file() {
                    bail!(
                        "{} is a link to a directory or a special file",
                        path.display()
                    );
                }
            } else if !kind.is_file() {
                bail!("{} is not a regular file", path.display());
            }
            if is_backup(&path) {
                skipped.push(path.strip_prefix(dir)?.to_path_buf());
                continue;
            }
            let bytes =
                std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
            out.push((path.strip_prefix(dir)?.to_path_buf(), bytes));
        }
    }
    out.sort();
    skipped.sort();
    Ok((out, skipped))
}

/// The first component of `rel` names `.git` or `.armadai`, in any case (on a
/// case-insensitive file system `.GIT` is `.git`).
fn into_sources(rel: &Path) -> bool {
    rel.components().next().is_some_and(|c| {
        let c = c.as_os_str().to_string_lossy();
        c.eq_ignore_ascii_case(".git") || c.eq_ignore_ascii_case(".armadai")
    })
}

/// Every file link writes: `root/rel` must stay under `root/trust` once resolved
/// (symbolic links on the way followed, a link at the destination itself refused),
/// and never reach `.git/` nor `.armadai/`, lexically or really, in any case.
pub fn check_destination(root: &Path, trust: &Path, rel: &Path) -> anyhow::Result<()> {
    use crate::linker::manifest::{lexically_normalize, resolve_real};
    let path = root.join(rel);
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symbolic link; link does not write through it",
            rel.display()
        );
    }
    let real = resolve_real(root, rel);
    let real_root = resolve_real(root, Path::new("."));
    let lexical = lexically_normalize(&path);
    let inside_lexically = lexical
        .strip_prefix(lexically_normalize(root))
        .ok()
        .map(Path::to_path_buf);
    if inside_lexically.as_deref().is_some_and(into_sources)
        || real.strip_prefix(&real_root).is_ok_and(into_sources)
    {
        bail!("{} would write into .git/ or .armadai/", rel.display());
    }
    if !real.starts_with(resolve_real(root, trust)) {
        bail!(
            "{} resolves outside {}; nothing was written",
            rel.display(),
            if trust == Path::new(".") {
                "the project".to_string()
            } else {
                trust.display().to_string()
            }
        );
    }
    Ok(())
}

/// An output directory link may write and unlink may clean: anywhere (a `--output`
/// may leave the project for one run) except the project root itself, its sources
/// (`.armadai/`) and `.git/`, lexically and through symbolic links.
/// `from_flag`: the folder came from `--output` (else from `outputs` or the
/// target's default), so the message names where it comes from.
pub fn check_output(root: &Path, output: &Path, from_flag: bool) -> anyhow::Result<()> {
    use crate::linker::manifest::resolve_real;
    let base = resolve_real(root, Path::new("."));
    let out = resolve_real(root, output);
    if out == base || out.strip_prefix(&base).is_ok_and(into_sources) || into_sources(output) {
        let what = if from_flag {
            "--output"
        } else {
            "output folder (from `outputs` of project.yaml or the target's default)"
        };
        bail!(
            "{what} {} resolves to the project root, `.armadai/` or `.git/`",
            output.display()
        );
    }
    Ok(())
}

/// The compiler of a link target.
pub fn compiler_for(target: &str) -> anyhow::Result<Box<dyn Compiler>> {
    Ok(match target {
        "claude" => Box::new(claude::ClaudeCompiler),
        "codex" => Box::new(legacy_targets::LegacyCompiler { target: "codex" }),
        "copilot" => Box::new(legacy_targets::LegacyCompiler { target: "copilot" }),
        "opencode" => Box::new(legacy_targets::LegacyCompiler { target: "opencode" }),
        _ => bail!(
            "Unknown link target: '{target}'. {}",
            crate::linker::supported_targets_sentence()
        ),
    })
}

/// Where a target writes when `--output` and `outputs` say nothing.
pub fn default_output(target: &str) -> &'static str {
    match target {
        "codex" => ".codex",
        "copilot" => ".github",
        "opencode" => ".opencode",
        _ => ".claude",
    }
}

/// Files of the legacy format: a project holding one of them and no
/// `.armadai/project.yaml` must be migrated first.
pub const LEGACY_CONFIGS: [&str; 3] = [".armadai/config.yaml", "armadai.yaml", "armadai.yml"];

/// The nearest ancestor of `cwd` holding `.armadai/project.yaml`, without crossing a
/// `.git` boundary. A legacy project found first is an error asking for `migrate`.
pub fn find_project(cwd: &Path) -> anyhow::Result<PathBuf> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        if d.join(".armadai/project.yaml").is_file() {
            return Ok(d.to_path_buf());
        }
        if LEGACY_CONFIGS.iter().any(|c| d.join(c).is_file()) {
            bail!(
                "{} uses the legacy format, run `armadai migrate`",
                d.display()
            );
        }
        if d.join(".git").exists() {
            break;
        }
        dir = d.parent();
    }
    bail!(
        "no ArmadAI project found (.armadai/project.yaml); run `armadai init --project`, \
         or `armadai migrate` for a legacy project"
    )
}

pub trait Compiler {
    /// `root` is the project root (the directory holding `.armadai/`).
    fn compile(&self, project: &Project, root: &Path) -> anyhow::Result<Plan>;
}
