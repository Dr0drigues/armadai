//! Compilation of the canonical model (`armadai-model`) into per-target artifacts.
//!
//! A compiler only plans: it returns the files to write and the conformance report.
//! Writing goes through `linker::manifest` (whole files) and `compile::patch`
//! (files shared with the user, such as `.claude/settings.json`).

pub mod claude;
pub mod claude_settings;
pub mod json_keys;
pub mod legacy_targets;
pub mod opencode;
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
    /// A command the user types (`commands/<id>.md`): depends on no agent.
    Command,
    Instructions,
    /// Copied from `.armadai/native/<target>/`; its path is relative to the project root.
    Native,
    /// A file of the `armadai-ui` plugin; its path is relative to the project root.
    Plugin,
}

/// Where link writes the `armadai-ui` plugin (`ui.subagent_status_line`).
pub const PLUGIN_DIR: &str = ".armadai/plugins/armadai-ui";

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
    /// Owned keys of a JSON file shared with the user (path relative to the
    /// project root), such as `opencode.json`.
    pub json: Option<(PathBuf, json_keys::JsonKeys)>,
    pub report: Report,
}

/// Merges `native.<target>` into the frontmatter `m` of `what` (`agent `dev``), the
/// keys in `skip` aside (the caller resolved them); a collision with a generated key
/// is an error. True when there was something to merge.
pub fn merge_native(
    m: &mut serde_json::Map<String, serde_json::Value>,
    native: &armadai_model::Native,
    target: &str,
    what: &str,
    skip: &[&str],
) -> anyhow::Result<bool> {
    let Some(native) = native.get(target) else {
        return Ok(false);
    };
    let Some(native) = native.as_object() else {
        bail!("{what}: native.{target} must be a mapping");
    };
    for (k, v) in native.iter().filter(|(k, _)| !skip.contains(&k.as_str())) {
        if m.contains_key(k) {
            bail!("{what}: native.{target} key `{k}` collides with a generated key");
        }
        m.insert(k.clone(), v.clone());
    }
    Ok(true)
}

/// One line per slot of agent `id` on target `t`: a filled slot is `compiled` with
/// its skills, or `approximated` with the given reason; an empty one `not-compiled`.
pub(crate) fn slot_lines(
    r: &mut Report,
    t: &str,
    project: &Project,
    id: &str,
    f: &armadai_model::AgentFront,
    approximated: Option<&str>,
) {
    for s in &f.slots {
        let fills = armadai_model::slot_fills(project, s);
        let element = format!("agent {id} slot {s}");
        if fills.is_empty() {
            r.push(t, element, Status::NotCompiled, Some("no skill fills it"));
        } else if let Some(why) = approximated {
            r.push(t, element, Status::Approximated, Some(why));
        } else {
            r.push(t, element, Status::Compiled, Some(&fills.join(", ")));
        }
    }
}

/// The journal lines of a target that does not compile the journal instructions
/// (`sessions.journal`, and each agent that declares `journal`): Claude Code only.
pub(super) fn journal_not_compiled(r: &mut Report, t: &str, project: &Project) {
    let lines = (project.manifest.sessions.journal).then(|| "journal".to_string());
    let agents = (project.agents.iter())
        .filter(|(_, a)| a.front.journal.is_some())
        .map(|(id, _)| format!("agent {id} journal"));
    for element in lines.into_iter().chain(agents) {
        r.push(
            t,
            element,
            Status::NotCompiled,
            Some(report::UI_CLAUDE_ONLY),
        );
    }
}

/// The `## Verdict` section appended to the prompt of an agent holding at least one
/// flow step with `states` or a failure path (`on_failure`, a `when: failed` link):
/// the union of those states, plus `failed` for a failure path (and `done` when the
/// step declares no state), sorted. `None` otherwise.
pub fn verdict_instruction(project: &Project, agent: &str) -> Option<String> {
    let states: std::collections::BTreeSet<&str> = (project.flows.values())
        .flat_map(|f| f.steps.iter().map(move |(id, s)| (f, id, s)))
        .filter(|(_, _, s)| s.agent == agent)
        .flat_map(|(f, id, s)| {
            let fails = s.on_failure.is_some()
                || (f.links.iter()).any(|l| &l.from == id && l.when.as_deref() == Some("failed"));
            let implied = match (fails, s.states.is_empty()) {
                (false, _) => &[][..],
                (true, true) => &["done", "failed"][..],
                (true, false) => &["failed"][..],
            };
            s.states
                .iter()
                .map(String::as_str)
                .chain(implied.iter().copied())
        })
        .collect();
    (!states.is_empty()).then(|| {
        format!(
            "\n\n## Verdict\n\nEnd your reply with exactly one line `STATE: <state>`, \
             where <state> is one of: {}.\n",
            states.into_iter().collect::<Vec<_>>().join(", ")
        )
    })
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

/// The destination of a file of the `armadai-ui` plugin: the one place under `.armadai/`
/// link may write. `rel` must sit lexically under [`PLUGIN_DIR`] and no component on
/// the way (`.armadai` included) may be a symbolic link, so the path resolves to itself.
pub fn check_plugin_destination(root: &Path, rel: &Path) -> anyhow::Result<()> {
    let plain = rel
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !plain || !rel.starts_with(PLUGIN_DIR) {
        bail!("{} is not inside {PLUGIN_DIR}", rel.display());
    }
    let mut at = PathBuf::new();
    for c in rel.components() {
        at.push(c);
        if std::fs::symlink_metadata(root.join(&at)).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(
                "{} is a symbolic link; link does not write through it",
                at.display()
            );
        }
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

/// The compiler of a link target writing into `output` (relative to the root).
pub fn compiler_for(target: &str, output: &Path) -> anyhow::Result<Box<dyn Compiler>> {
    Ok(match target {
        "claude" => Box::new(claude::ClaudeCompiler),
        "codex" => Box::new(legacy_targets::LegacyCompiler { target: "codex" }),
        "copilot" => Box::new(legacy_targets::LegacyCompiler { target: "copilot" }),
        "opencode" => Box::new(opencode::OpencodeCompiler::new(output)),
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
    /// `root`: the project root, where files are written and the user's files read
    /// (the root `CLAUDE.md`; `opencode.json` is read by `patch.rs`, not here). `sources`: the directory holding the
    /// `.armadai/` to compile — `root` itself, or the effective view of task 9.
    fn compile(&self, project: &Project, root: &Path, sources: &Path) -> anyhow::Result<Plan>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    /// A project with a coordinator and one skill `s` (with a script), in `dir`.
    fn sources_with_skill(dir: &Path) -> Project {
        write(
            dir,
            ".armadai/project.yaml",
            "schema_version: 1\ncoordinator: lead\n",
        );
        write(
            dir,
            ".armadai/agents/lead.md",
            "---\ndescription: L\n---\nB\n",
        );
        write(
            dir,
            ".armadai/skills/s/SKILL.md",
            "---\nname: s\ndescription: S\n---\nB\n",
        );
        write(dir, ".armadai/skills/s/scripts/run.sh", "#!/bin/sh\n");
        armadai_model::load_checked(dir).unwrap().0
    }

    #[test]
    fn skills_are_read_from_sources() {
        // `opencode` reads the user config: hold the environment lock and an empty
        // config dir, or the broken-config test of `opencode` is read from here
        let _iso = armadai_core::test_support::IsolatedConfigDir::enter();
        let sources = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let p = sources_with_skill(sources.path());
        for t in ["claude", "opencode", "codex", "copilot"] {
            let plan = compiler_for(t, Path::new(default_output(t)))
                .unwrap()
                .compile(&p, root.path(), sources.path())
                .unwrap();
            for want in ["skills/s/SKILL.md", "skills/s/scripts/run.sh"] {
                assert!(
                    plan.artifacts.iter().any(|a| a.path == Path::new(want)),
                    "{t}: no {want}"
                );
            }
        }
    }

    #[test]
    fn the_root_claude_md_is_read_under_root() {
        let sources = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let p = sources_with_skill(sources.path());
        let kept = |root: &Path| {
            let plan = compiler_for("claude", Path::new(".claude"))
                .unwrap()
                .compile(&p, root, sources.path())
                .unwrap();
            plan.report
                .lines
                .iter()
                .any(|l| l.element == "CLAUDE.md" && l.status == Status::Approximated)
        };
        assert!(!kept(root.path()));
        write(root.path(), "CLAUDE.md", "mine\n");
        write(sources.path(), "CLAUDE.md", "theirs\n");
        assert!(kept(root.path()));
        std::fs::remove_file(root.path().join("CLAUDE.md")).unwrap();
        assert!(!kept(root.path()), "sources' CLAUDE.md must not count");
    }

    #[test]
    fn the_plugin_guard_accepts_only_files_of_the_plugin_folder() {
        let root = tempfile::tempdir().unwrap();
        let ok = |rel: &str| check_plugin_destination(root.path(), Path::new(rel));
        for good in [
            ".armadai/plugins/armadai-ui/settings.json",
            ".armadai/plugins/armadai-ui/.claude-plugin/plugin.json",
        ] {
            assert!(ok(good).is_ok(), "{good}");
        }
        for bad in [
            ".armadai/plugins/armadai-ui-x/a",
            ".armadai/plugins/other/a",
            ".armadai/project.yaml",
            ".armadai/plugins/armadai-ui/../../project.yaml",
            ".git/config",
            "src/a",
        ] {
            assert!(ok(bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_plugin_guard_refuses_a_link_on_the_way() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        let rel = ".armadai/plugins/armadai-ui/settings.json";
        // each link in turn: to a folder outside the project, to one inside it
        for (at, to) in [
            (".armadai/plugins", "outside"),
            (".armadai/plugins", "p/src"),
            (".armadai/plugins/armadai-ui", "outside"),
            (".armadai/plugins/armadai-ui", "p/src"),
        ] {
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join(".armadai/plugins")).unwrap();
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::create_dir_all(dir.path().join("outside")).unwrap();
            if at.ends_with("armadai-ui") {
                // `plugins` stays a real folder: the link is the plugin folder itself
            } else {
                std::fs::remove_dir(root.join(at)).unwrap();
            }
            std::os::unix::fs::symlink(dir.path().join(to), root.join(at)).unwrap();
            let e = check_plugin_destination(&root, Path::new(rel)).unwrap_err();
            assert!(e.to_string().contains("symbolic link"), "{at} -> {to}: {e}");
        }
    }
}
