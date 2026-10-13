//! `armadai link`: compile `.armadai/` (the canonical model) for each target.
//!
//! Pipeline (spec §3): load and validate (any error stops before a write) → one plan
//! per target → `native` files and collisions → preflight of the files shared with
//! the user → writes (whole files through `linker::manifest::write_files`, shared files
//! through `compile::patch`) → conformance report.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_model::Project;

use crate::compile::claude_settings::{self, HookEntry};
use crate::compile::patch::{self, PatchKind, PatchedFile};
use crate::compile::{self, Artifact, ArtifactKind, Report, Status, read_tree};
use crate::linker::manifest::{self, DigestCheck, FileOutcome, Lookup, Outcome, ProducedBy};

type Settings = (PathBuf, Vec<HookEntry>, BTreeMap<String, serde_json::Value>);

/// Everything one target needs, computed before anything is written.
struct Prepared {
    target: String,
    output_dir: PathBuf,
    artifacts: Vec<Artifact>,
    blocks: Vec<(PathBuf, String)>,
    /// `(settings.json path relative to the root, hooks, native keys)`, Claude only.
    settings: Option<Settings>,
    previous: Option<manifest::TargetManifest>,
    /// Everything the project produces for this target, `--agents` or not: what
    /// is stale is decided on this, never on a filtered subset.
    all_produced: BTreeSet<PathBuf>,
    report: Report,
}

impl Prepared {
    /// Where an artifact lands, relative to the project root.
    fn rel(&self, a: &Artifact) -> PathBuf {
        match a.kind {
            ArtifactKind::Native => a.path.clone(),
            _ => self.output_dir.join(&a.path),
        }
    }

    fn previous_patch(&self, rel: &Path) -> Option<&PatchedFile> {
        self.previous
            .as_ref()
            .and_then(|p| p.patched.iter().find(|f| f.path == rel))
    }
}

/// The project at or above the current directory, validated; diagnostics go to stderr.
pub fn load_project() -> anyhow::Result<(PathBuf, Project)> {
    let root = compile::find_project(&std::env::current_dir()?)?;
    match armadai_model::load_checked(&root) {
        Ok((p, _)) => Ok((root, p)),
        Err(ds) => {
            for d in &ds {
                eprintln!("{}", d.render());
            }
            bail!("the project is invalid; nothing was written")
        }
    }
}

/// `--target`, else `targets` of project.yaml, else claude.
pub fn targets_of(flag: &[crate::linker::LinkTarget], project: &Project) -> Vec<String> {
    if !flag.is_empty() {
        flag.iter().map(|t| t.to_string()).collect()
    } else if !project.manifest.targets.is_empty() {
        project.manifest.targets.clone()
    } else {
        vec!["claude".into()]
    }
}

/// `--output`, else `outputs.<target>` of project.yaml, else the target's default.
pub fn output_of(target: &str, output: Option<&Path>, project: &Project) -> PathBuf {
    output
        .map(Path::to_path_buf)
        .or_else(|| project.manifest.outputs.get(target).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(compile::default_output(target)))
}

/// A path compared as a case-insensitive file system would.
fn folded(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

fn prepare(
    project: &Project,
    root: &Path,
    target: &str,
    output: Option<&Path>,
    agents: Option<&[String]>,
) -> anyhow::Result<Prepared> {
    compile::check_output(root, &output_of(target, output, project), output.is_some())?;
    let plan = compile::compiler_for(target)?.compile(project, root)?;
    let mut p = Prepared {
        target: target.to_string(),
        output_dir: output_of(target, output, project),
        artifacts: plan.artifacts,
        blocks: plan.blocks,
        settings: None,
        previous: match manifest::lookup_target(root, target) {
            Lookup::Found(t) => Some(t),
            Lookup::Fallback => None,
        },
        report: plan.report,
        all_produced: BTreeSet::new(),
    };
    p.all_produced = p.artifacts.iter().map(|a| p.rel(a)).collect();

    // `--agents` narrows the agent files only, never the root file nor the hooks
    if let Some(filter) = agents {
        let unknown: Vec<&str> = (filter.iter())
            .filter(|a| !project.agents.contains_key(*a))
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            bail!(
                "--agents names no agent of the project: {}",
                unknown.join(", ")
            );
        }
        let skipped: BTreeSet<String> = (p.artifacts.iter())
            .filter(|a| a.kind == ArtifactKind::Agent && !filter.contains(&a.produced_by.name))
            .map(|a| a.produced_by.name.clone())
            .collect();
        p.artifacts
            .retain(|a| !(a.kind == ArtifactKind::Agent && skipped.contains(&a.produced_by.name)));
        for id in &skipped {
            let element = format!("agent {id}");
            p.report.lines.retain(|l| l.element != element);
            p.report.push(
                target,
                element,
                Status::NotCompiled,
                Some("skipped (--agents)"),
            );
        }
    }

    if target == "claude" {
        let rel = p.output_dir.join("settings.json");
        let hooks = claude_settings::hook_entries(project)?;
        let keys = claude_settings::native_settings(project)?;
        patch::preflight_settings(
            root,
            &rel,
            &p.output_dir,
            &hooks,
            &keys,
            p.previous_patch(&rel),
        )?;
        p.settings = Some((rel, hooks, keys));
    }
    for (rel, _) in &p.blocks {
        patch::preflight_markdown(root, rel)?;
    }

    // `.armadai/native/<target>/<path>` is copied as is to `<root>/<path>`
    let native_dir = root.join(".armadai/native").join(target);
    if native_dir.is_dir() {
        // everything the project produces, `--agents` or not
        // compared case-insensitively: on macOS `.CLAUDE/agents/DEV.md` is the
        // generated `.claude/agents/dev.md`
        let mut generated: BTreeSet<String> = (p.all_produced.iter().cloned())
            .chain(p.blocks.iter().map(|(r, _)| r.clone()))
            .chain(p.settings.iter().map(|(r, _, _)| r.clone()))
            .map(|r| folded(&r))
            .collect();
        for (path, content) in read_tree(&native_dir, &root.join(".armadai"))? {
            compile::check_destination(root, Path::new("."), &path)
                .map_err(|e| anyhow::anyhow!("native file: {e}"))?;
            if !generated.insert(folded(&path)) {
                bail!(
                    "native file .armadai/native/{target}/{} collides with a file ArmadAI \
                     generates (paths are compared ignoring case, also on a case-sensitive \
                     file system, so the project links the same everywhere); nothing was written",
                    path.display()
                );
            }
            p.report.push(
                target,
                format!("native {}", path.display()),
                Status::Compiled,
                Some("copied as is"),
            );
            p.artifacts.push(Artifact {
                produced_by: ProducedBy::native(path.display().to_string()),
                path,
                content,
                kind: ArtifactKind::Native,
            });
        }
    }

    // every destination, before any write: an output from project.yaml must stay in
    // the project; only `--output` may lead elsewhere, for its own files
    let trust = if output.is_some() {
        p.output_dir.clone()
    } else {
        PathBuf::from(".")
    };
    for a in &p.artifacts {
        let t = if a.kind == ArtifactKind::Native {
            Path::new(".")
        } else {
            trust.as_path()
        };
        compile::check_destination(root, t, &p.rel(a))?;
    }
    for (rel, _) in &p.blocks {
        compile::check_destination(root, Path::new("."), rel)?;
    }
    if let Some((rel, _, _)) = &p.settings {
        compile::check_destination(root, &trust, rel)?;
    }

    // the root file of the former Claude linker, which Claude Code still reads
    if target == "claude" && root.join(".claude/CLAUDE.md").is_file() {
        let recorded = (p.previous.iter().flat_map(|t| &t.entries))
            .any(|e| e.path == Path::new(".claude/CLAUDE.md"));
        if !recorded {
            p.report.push(
                target,
                ".claude/CLAUDE.md",
                Status::Approximated,
                Some("stale file left in place: not recorded by link, remove it by hand"),
            );
        }
    }
    Ok(p)
}

/// Files the previous link wrote that this one no longer produces: removed while
/// still exactly what link wrote, otherwise left in place and reported.
fn remove_stale(root: &Path, p: &mut Prepared, produced: &BTreeSet<PathBuf>, dry_run: bool) {
    let Some(prev) = &p.previous else { return };
    let mut lines = Vec::new();
    for e in &prev.entries {
        if e.outcome != Outcome::Created || produced.contains(&e.path) {
            continue;
        }
        let trust_root = match e.produced_by.kind {
            manifest::ProducedByKind::Native => Path::new("."),
            _ => prev.root.as_path(),
        };
        let path = manifest::resolve_real(root, &e.path);
        if !path.exists() {
            continue;
        }
        let own = manifest::is_trusted(root, trust_root, &e.path)
            && (e.digest.as_deref())
                .is_some_and(|d| manifest::check_digest(d, &path) == DigestCheck::Matches);
        let element = format!("stale {}", e.path.display());
        if own && dry_run {
            lines.push((
                element,
                Status::Compiled,
                "would be removed: no longer produced",
            ));
        } else if own && std::fs::remove_file(&path).is_ok() {
            lines.push((element, Status::Compiled, "removed: no longer produced"));
        } else {
            lines.push((
                element,
                Status::Approximated,
                "stale file left in place: edited since link wrote it",
            ));
        }
    }
    for (element, status, reason) in lines {
        p.report.push(&p.target, element, status, Some(reason));
    }
}

/// Skill and native files keep the executable bit of their source (scripts).
#[cfg(unix)]
fn copy_executable_bits(root: &Path, p: &Prepared) {
    use std::os::unix::fs::PermissionsExt;
    for a in &p.artifacts {
        let source = match a.kind {
            ArtifactKind::Skill => a
                .path
                .strip_prefix("skills")
                .ok()
                .map(|rest| root.join(".armadai/skills").join(rest)),
            ArtifactKind::Native => {
                Some(root.join(".armadai/native").join(&p.target).join(&a.path))
            }
            _ => None,
        };
        let exec = source
            .and_then(|s| std::fs::metadata(s).ok())
            .is_some_and(|m| m.permissions().mode() & 0o111 != 0);
        let dest = root.join(p.rel(a));
        if let (true, Ok(m)) = (exec, std::fs::metadata(&dest)) {
            let mode = m.permissions().mode() | ((m.permissions().mode() & 0o444) >> 2);
            let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode));
        }
    }
}

#[cfg(not(unix))]
fn copy_executable_bits(_: &Path, _: &Prepared) {}

/// A file link left in place is not compiled: its element's report line says so
/// (never `compiled` for a file link did not write).
fn not_written(p: &mut Prepared, root: &Path, path: &Path) {
    let Some(a) = p.artifacts.iter().find(|a| root.join(p.rel(a)) == path) else {
        return;
    };
    let element = match a.produced_by.kind {
        manifest::ProducedByKind::Skill => format!("skill {}", a.produced_by.name),
        manifest::ProducedByKind::Native => format!("native {}", a.produced_by.name),
        _ => format!("agent {}", a.produced_by.name),
    };
    let rel = p.rel(a);
    let ours_before = (p.previous.iter().flat_map(|t| &t.entries))
        .any(|e| e.path == rel && e.outcome == Outcome::Created);
    let reason = if ours_before {
        format!(
            "edited since the last link (use --force): {}",
            rel.display()
        )
    } else {
        format!(
            "file exists and was not written by link (use --force): {}",
            rel.display()
        )
    };
    p.report.lines.retain(|l| l.element != element);
    let target = p.target.clone();
    p.report
        .push(&target, element, Status::NotCompiled, Some(&reason));
}

/// Whether undoing the previous settings patch removes the settings.json link
/// created (its record says created, and nothing else would be left in it).
fn settings_file_goes(root: &Path, prev: &PatchedFile) -> bool {
    let PatchKind::SettingsHooks(owned) = &prev.kind else {
        return false;
    };
    prev.created
        && std::fs::read_to_string(root.join(&prev.path))
            .ok()
            .and_then(|t| claude_settings::remove_entries(&t, owned).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .is_some_and(|v| v.as_object().is_some_and(|m| m.is_empty()))
}

const SETTINGS_GONE: &str = "removed: link created it and no hook is left";

/// The `--dry-run` side of `write_files`' decision: the files a real run would
/// leave in place (they exist with other bytes, are not link's previous output,
/// and `--force` is not given), reported exactly as the real run reports them.
/// Returns them, so the dry run never lists them among the files it would generate.
fn simulate_skips(root: &Path, p: &mut Prepared, force: bool) -> BTreeSet<PathBuf> {
    let previous: std::collections::BTreeMap<PathBuf, String> =
        (p.previous.iter().flat_map(|t| &t.entries))
            .filter(|e| e.outcome == Outcome::Created)
            .filter_map(|e| e.digest.clone().map(|d| (e.path.clone(), d)))
            .collect();
    let skipped: Vec<PathBuf> = (p.artifacts.iter())
        .map(|a| (root.join(p.rel(a)), p.rel(a), &a.content))
        .filter(|(path, rel, content)| {
            manifest::left_in_place(path, content, force, previous.get(rel))
        })
        .map(|(path, _, _)| path)
        .collect();
    for path in &skipped {
        not_written(p, root, path);
    }
    skipped.into_iter().collect()
}

/// The bytes of `path`, `None` when absent: to tell a real patch from a no-op.
fn bytes(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

fn write(root: &Path, p: &mut Prepared, force: bool, out: &mut Vec<String>) -> anyhow::Result<()> {
    let mut patched = Vec::new();
    let mut changed: BTreeSet<PathBuf> = BTreeSet::new();

    // the root CLAUDE.md block: written, or removed once no longer produced
    let blocks = std::mem::take(&mut p.blocks);
    for (rel, block) in &blocks {
        let before = bytes(&root.join(rel));
        patched.push(patch::apply_markdown_block(
            root,
            rel,
            block,
            p.previous_patch(rel),
        )?);
        if bytes(&root.join(rel)) != before {
            changed.insert(rel.clone());
        }
    }
    let stale_blocks: Vec<PatchedFile> = (p.previous.iter().flat_map(|t| &t.patched))
        .filter(|f| matches!(f.kind, PatchKind::MarkdownBlock))
        .filter(|f| !blocks.iter().any(|(r, _)| *r == f.path))
        .cloned()
        .collect();
    for f in stale_blocks
        .iter()
        .filter(|f| f.path == Path::new("CLAUDE.md"))
    {
        patch::remove_patch(root, &p.output_dir, f)?;
    }

    if let Some((rel, hooks, keys)) = p.settings.take() {
        let prev = p.previous_patch(&rel).cloned();
        if hooks.is_empty() && keys.is_empty() {
            if let Some(prev) = &prev {
                let goes = settings_file_goes(root, prev);
                patch::remove_patch(root, &p.output_dir, prev)?;
                if goes {
                    p.report.push(
                        &p.target,
                        prev.path.display().to_string(),
                        Status::Compiled,
                        Some(SETTINGS_GONE),
                    );
                }
            }
        } else {
            let before = bytes(&root.join(&rel));
            let (done, reformatted) =
                patch::apply_settings(root, &rel, &p.output_dir, &hooks, &keys, prev.as_ref())?;
            if bytes(&root.join(&rel)) != before {
                changed.insert(rel.clone());
            }
            if reformatted {
                p.report.push(
                    &p.target,
                    rel.display().to_string(),
                    Status::Approximated,
                    Some("settings.json reformatted (values and key order kept)"),
                );
            }
            if let PatchKind::SettingsHooks(owned) = &done.kind {
                for h in hooks.iter().filter(|h| !owned.entries.contains(h)) {
                    p.report.push(
                        &p.target,
                        format!("hook {} {}", h.event, h.command),
                        Status::Approximated,
                        Some("existing identical hook kept, not owned"),
                    );
                }
            }
            patched.push(done);
        }
    }

    let files: Vec<(PathBuf, Vec<u8>, ProducedBy)> = (p.artifacts.iter())
        .map(|a| {
            (
                root.join(p.rel(a)),
                a.content.clone(),
                a.produced_by.clone(),
            )
        })
        .collect();
    let produced = p.all_produced.clone();
    remove_stale(root, p, &produced, false);

    let target_root = root.join(&p.output_dir);
    let outcomes = manifest::write_files(
        root,
        &p.target,
        &p.output_dir,
        &target_root,
        files,
        force,
        &patched,
    )?;
    copy_executable_bits(root, p);

    // files `--agents` left out stay recorded as they were
    let written: BTreeSet<PathBuf> = p.artifacts.iter().map(|a| p.rel(a)).collect();
    let kept: Vec<manifest::ManifestEntry> = (p.previous.iter().flat_map(|t| &t.entries))
        .filter(|e| p.all_produced.contains(&e.path) && !written.contains(&e.path))
        .cloned()
        .collect();
    manifest::append_entries(root, &p.target, kept)?;
    for o in outcomes {
        out.push(match o {
            FileOutcome::Wrote(path) => format!("  wrote {}", path.display()),
            FileOutcome::UpToDate(path) => format!("  up-to-date {}", path.display()),
            FileOutcome::SkippedExisting(path) => {
                eprintln!(
                    "  skip: {} already exists (use --force to overwrite)",
                    path.display()
                );
                not_written(p, root, &path);
                continue;
            }
        });
    }
    for f in patched.iter().filter(|f| changed.contains(&f.path)) {
        out.push(format!("  patched {}", root.join(&f.path).display()));
    }
    Ok(())
}

pub async fn execute(
    targets: Vec<crate::linker::LinkTarget>,
    dry_run: bool,
    force: bool,
    output: Option<PathBuf>,
    agents: Option<Vec<String>>,
    json: bool,
) -> anyhow::Result<()> {
    let (root, project) = load_project()?;
    let targets = targets_of(&targets, &project);
    if output.is_some() && targets.len() > 1 {
        bail!("--output names one directory: link one target at a time with it");
    }

    // every target is planned and checked before the first write
    let mut prepared = (targets.iter())
        .map(|t| prepare(&project, &root, t, output.as_deref(), agents.as_deref()))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut lines = Vec::new();
    if dry_run {
        let mut skips = Vec::new();
        for p in &mut prepared {
            let produced = p.all_produced.clone();
            remove_stale(&root, p, &produced, true);
            skips.push(simulate_skips(&root, p, force));
            if let Some((rel, hooks, keys)) = &p.settings
                && hooks.is_empty()
                && keys.is_empty()
                && let Some(prev) = p.previous_patch(rel)
                && settings_file_goes(&root, prev)
            {
                let element = prev.path.display().to_string();
                let target = p.target.clone();
                p.report
                    .push(&target, element, Status::Compiled, Some(SETTINGS_GONE));
            }
        }
        for (p, skipped) in prepared.iter().zip(&skips) {
            lines.push(format!(
                "Dry run — files that would be generated for '{}':",
                p.target
            ));
            let (kept, written): (Vec<PathBuf>, Vec<PathBuf>) = (p.artifacts.iter())
                .map(|a| root.join(p.rel(a)))
                .partition(|path| skipped.contains(path));
            // what the real run reports `up-to-date`: the same bytes are already there
            let (same, written): (Vec<PathBuf>, Vec<PathBuf>) =
                written.into_iter().partition(|path| {
                    (p.artifacts.iter()).any(|a| {
                        root.join(p.rel(a)) == *path
                            && std::fs::read(path).is_ok_and(|b| b == a.content)
                    })
                });
            for path in &written {
                lines.push(format!("  {}", path.display()));
            }
            for (rel, _) in &p.blocks {
                lines.push(format!("  {} (ArmadAI block)", root.join(rel).display()));
            }
            if let Some((rel, hooks, keys)) = &p.settings
                && (!hooks.is_empty() || !keys.is_empty())
            {
                lines.push(format!("  {} (hooks merged)", root.join(rel).display()));
            }
            if !same.is_empty() {
                lines.push(format!(
                    "Files unchanged for '{}' (already up to date):",
                    p.target
                ));
                for path in &same {
                    lines.push(format!("  {}", path.display()));
                }
            }
            if !kept.is_empty() {
                lines.push(format!(
                    "Files that would be skipped for '{}' (left in place, see the report; \
                     use --force):",
                    p.target
                ));
                for path in &kept {
                    lines.push(format!("  {}", path.display()));
                }
            }
        }
    } else {
        if let Err(e) = armadai_core::project_registry::register_project(&root) {
            tracing::warn!("Failed to register project in registry: {:?}", e);
        }
        manifest::ensure_gitignore(&root).context("cannot write .armadai/.gitignore")?;
        for p in &mut prepared {
            write(&root, p, force, &mut lines)?;
        }
    }

    let mut report = Report::default();
    for p in prepared {
        report.lines.extend(p.report.lines);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report.to_json())?);
    } else {
        for l in &lines {
            println!("{l}");
        }
        println!("\nConformance report:");
        print!("{}", report.render_text());
    }
    Ok(())
}
