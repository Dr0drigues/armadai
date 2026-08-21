use std::path::{Path, PathBuf};

use crate::linker::model_resolution::{self, TargetKind};
use crate::linker::{self, LinkAgent};
use armadai_core::project;

/// A file `unlink` might remove, together with what removing it safely
/// requires.
///
/// `unlink` keeps no manifest of what `link` actually wrote (issue #338) —
/// it only knows how to recompute what `link` *would* write today. So for
/// anything the linker itself produces (agent/coordinator instruction
/// files, skill copies, prompt copies), the only safe rule is: **delete a
/// file only if its on-disk content is byte-for-byte identical to what
/// would be generated right now.** A hand-written file at a would-be
/// generated path never matches and is always kept; a file the linker
/// really did write and that hasn't been touched since always matches and
/// is reclaimed.
///
/// Accepted limitation, stated here and echoed in the CLI output: content
/// can differ from what `link` would generate right now for two reasons
/// `unlink` has no way to distinguish — the file was edited since linking,
/// or it was linked with different options (an explicit `--model`, or an
/// interactive prompt answer). Either way the file is kept, becoming a
/// visible orphan instead of a silent deletion. An orphan can be spotted
/// and removed by hand; content deleted by mistake cannot be un-deleted.
/// On `opencode` (an `Orchestrator` target below) in particular, `--model`
/// or an interactive answer at link time makes the generated file
/// permanently un-reclaimable by `unlink` until the write manifest (the
/// other half of #338) lands and makes detection exact.
///
/// `AlwaysDelete` covers the single opt-in case that isn't linker output at
/// all: the project config file removed via `--with-config`. There is
/// nothing generated to diff it against — the flag is the user's own
/// confirmation, so no content guard applies.
enum Candidate {
    Generated { path: PathBuf, expected: Vec<u8> },
    AlwaysDelete { path: PathBuf },
}

impl Candidate {
    fn path(&self) -> &Path {
        match self {
            Candidate::Generated { path, .. } | Candidate::AlwaysDelete { path } => path,
        }
    }
}

pub async fn execute(
    target: Option<crate::linker::LinkTarget>,
    coordinator_flag: Option<String>,
    dry_run: bool,
    with_config: bool,
    output: Option<PathBuf>,
    agents_filter: Option<Vec<String>>,
) -> anyhow::Result<()> {
    // 1. Find project config
    let (root, config) = project::find_project_config().ok_or_else(|| {
        anyhow::anyhow!(
            "No project config found (.armadai/config.yaml or armadai.yaml). \
             Run `armadai init --project` to create one."
        )
    })?;

    // Every agent in `.armadai/agents.yaml` is included automatically (it
    // does not need to be relisted in `agents:`), the same gate `link`,
    // `list` and `run` all widened for this format: an otherwise-empty
    // `agents:` list is only a real error when there is no declarations file
    // either. Without this, `unlink` reports the false "No agents declared in
    // project config." for exactly the project `link` just wrote three files
    // for, and removes nothing.
    if !armadai_core::agent_source::project_declares_agents(&root, &config) {
        anyhow::bail!("No agents declared in project config.");
    }

    // 2. Resolve and load agents — file-backed and declared alike.
    let fragments = armadai_core::agent_source::project_fragments(&root);
    let (agents, warnings) =
        armadai_core::agent_source::load_all_agents(&config, &root, &fragments);
    // `unlink` writes no config — it only removes files `link` would have
    // written — so unlike `link` it never needs to refuse over a drop: warn
    // and remove whatever can still be resolved, same policy as `list`.
    for w in &warnings {
        let s = crate::cli::style::warn();
        anstream::eprintln!("{s}  warn: {}{s:#}", w.message());
    }

    let mut link_agents: Vec<LinkAgent> = agents.iter().map(LinkAgent::from).collect();

    if link_agents.is_empty() {
        anyhow::bail!("No agents could be resolved. Check your project config.");
    }

    // 2b. Resolve deprecated model aliases — `link` does this before
    // generating, so the content guard below must reproduce it too, or
    // every agent still using a since-renamed model would never match.
    for agent in &mut link_agents {
        armadai_core::model_aliases::resolve_model_deprecations(
            &mut agent.model,
            &mut agent.model_fallback,
        );
    }

    // 3. Filter by --agents if provided
    if let Some(ref filter) = agents_filter {
        let filter_lower: Vec<String> = filter.iter().map(|s| s.to_lowercase()).collect();
        link_agents.retain(|a| filter_lower.contains(&a.name.to_lowercase()));
        if link_agents.is_empty() {
            anyhow::bail!("No agents match the given filter: {}", filter.join(", "));
        }
    }

    // 3b. Extract coordinator if configured (CLI flag takes priority over config)
    let coordinator_name =
        coordinator_flag.or_else(|| config.link.as_ref().and_then(|l| l.coordinator.clone()));
    let mut coordinator = coordinator_name.and_then(|name| {
        let idx = link_agents
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(&name))?;
        Some(link_agents.remove(idx))
    });

    // 4. Determine target
    let target_name = target
        .map(|t| t.to_string())
        .or_else(|| config.link.as_ref().and_then(|l| l.target.clone()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No link target specified. Use --target or set link.target in armadai.yaml.\n\
                 Supported targets: claude, copilot, gemini, opencode"
            )
        })?;

    // 4b. Model resolution — mirror what `link` computes for this target,
    // so the regenerated content used by the guard below matches what
    // `link` actually wrote. For `LlmEditor` targets (claude, gemini,
    // codex) this is a pure function of the current config, exactly like
    // `link`'s own step 4b, so it reproduces byte-for-byte. For
    // `Orchestrator` targets (copilot, opencode), `link` may additionally
    // honour an explicit `--model` flag or an interactive prompt at link
    // time — `unlink` takes neither, so it can only reproduce the
    // no-flag/non-interactive default (`latest:*` resolution per agent). A
    // link that used an explicit model there produces content `unlink`
    // cannot recompute; the guard then correctly keeps those files rather
    // than guessing why they differ. On `opencode` specifically, this makes
    // a `--model`-linked (or interactively-answered) file permanently
    // un-reclaimable by `unlink` until the write manifest lands.
    let target_kind = model_resolution::classify_target(&target_name);
    match target_kind {
        TargetKind::LlmEditor { provider } => {
            #[cfg(feature = "providers-api")]
            {
                model_resolution::remap_models_for_llm_editor(&mut link_agents, provider).await;
                if let Some(ref mut coord) = coordinator {
                    model_resolution::remap_models_for_llm_editor(
                        std::slice::from_mut(coord),
                        provider,
                    )
                    .await;
                }
            }
            #[cfg(not(feature = "providers-api"))]
            {
                model_resolution::remap_models_for_llm_editor(&mut link_agents, provider);
                if let Some(ref mut coord) = coordinator {
                    model_resolution::remap_models_for_llm_editor(
                        std::slice::from_mut(coord),
                        provider,
                    );
                }
            }
        }
        TargetKind::Orchestrator => {
            model_resolution::resolve_latest_placeholders(&mut link_agents);
            if let Some(ref mut coord) = coordinator {
                model_resolution::resolve_latest_placeholders(std::slice::from_mut(coord));
            }
        }
    }

    // 5. Create linker
    let linker = linker::create_linker(&target_name)?;

    // 6. Determine output directory
    let output_dir = output
        .or_else(|| {
            config
                .link
                .as_ref()
                .and_then(|l| l.overrides.get(&target_name))
                .and_then(|o| o.output.as_ref())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from(linker.default_output_dir()));
    // The target's own root directory (`.claude/`, `.github/`, ...) —
    // `remove_empty_ancestors` must never delete this, however empty it
    // ends up (issue #338 case 1's second half).
    let target_root = root.join(&output_dir);

    // 7. Regenerate the expected file list — same content `link` would
    // write today — so deletions can be gated on a content match instead
    // of trusting paths alone.
    let sources = &config.sources;
    let files = linker.generate(&link_agents, coordinator.as_ref(), sources);

    if files.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No files to remove.{m:#}");
        return Ok(());
    }

    // 8. Resolve output paths relative to project root, keeping the
    // generated content alongside each path for the guard below.
    let mut candidates: Vec<Candidate> = files
        .into_iter()
        .map(|f| {
            let default_dir = PathBuf::from(linker.default_output_dir());
            let relative = f
                .path
                .strip_prefix(&default_dir)
                .unwrap_or(&f.path)
                .to_path_buf();
            Candidate::Generated {
                path: root.join(&output_dir).join(relative),
                expected: f.content.into_bytes(),
            }
        })
        .collect();

    // 8b. Include skill files — but only the ones the skill's *source*
    // directory still names. `link` copies exactly those paths into
    // `<output_dir>/skills/<name>/`; anything else found there afterwards
    // (issue #338 case 3 — the worst measured outcome) was placed by the
    // user and has no source-side counterpart, so it is never even
    // considered, let alone swept recursively.
    let (skill_dirs, _) = project::resolve_all_skills(&config, &root);
    for skill_dir in &skill_dirs {
        let skill_name = skill_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let dest_dir = root.join(&output_dir).join("skills").join(skill_name);
        if !dest_dir.exists() {
            continue;
        }
        for (relative, expected) in collect_source_files(skill_dir) {
            candidates.push(Candidate::Generated {
                path: dest_dir.join(&relative),
                expected,
            });
        }
    }

    // 8c. Include prompt files, gated the same way: the expected content is
    // whatever the source prompt file currently holds.
    let (prompt_paths, _) = project::resolve_all_prompts(&config, &root);
    for prompt_path in &prompt_paths {
        let filename = prompt_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.md");
        if let Ok(expected) = std::fs::read(prompt_path) {
            candidates.push(Candidate::Generated {
                path: root.join(&output_dir).join("prompts").join(filename),
                expected,
            });
        }
    }

    // 9. Optionally include the project config file itself. This is the
    // user's own config, not something the linker generates, so there is
    // nothing to diff it against — `--with-config` is opt-in and is itself
    // the confirmation.
    if with_config {
        // Detect which config file is active
        let dotarmadai_config = root.join(".armadai").join("config.yaml");
        let legacy_yaml = root.join("armadai.yaml");
        let legacy_yml = root.join("armadai.yml");

        if dotarmadai_config.exists() {
            candidates.push(Candidate::AlwaysDelete {
                path: dotarmadai_config,
            });
        } else if legacy_yaml.exists() {
            candidates.push(Candidate::AlwaysDelete { path: legacy_yaml });
        } else if legacy_yml.exists() {
            candidates.push(Candidate::AlwaysDelete { path: legacy_yml });
        }
    }

    // 10. Dry run
    if dry_run {
        let h = crate::cli::style::header();
        let a = crate::cli::style::accent();
        let m = crate::cli::style::muted();
        let w = crate::cli::style::warn();
        anstream::println!(
            "{h}Dry run{h:#} — files that would be removed for {a}'{}'{a:#}:\n",
            target_name
        );
        let mut would_remove = 0;
        let mut would_keep = 0;
        let mut absent = 0;
        for candidate in &candidates {
            let path = candidate.path();
            if !path.exists() {
                anstream::println!("{m}  {} (already absent){m:#}", path.display());
                absent += 1;
                continue;
            }
            match candidate {
                Candidate::AlwaysDelete { .. } => {
                    anstream::println!("{m}  {}{m:#}", path.display());
                    would_remove += 1;
                }
                Candidate::Generated { expected, .. } => {
                    if content_matches(path, expected) {
                        anstream::println!("{m}  {}{m:#}", path.display());
                        would_remove += 1;
                    } else {
                        anstream::println!(
                            "{w}  {} (would keep — content differs from what link \
                             would generate now; possibly edited since linking, or \
                             linked with different options such as --model or an \
                             interactive prompt answer){w:#}",
                            path.display()
                        );
                        would_keep += 1;
                    }
                }
            }
        }
        anstream::println!(
            "\n{m}  {} would be removed, {} would be kept (content differs), \
             {} already absent.{m:#}",
            would_remove,
            would_keep,
            absent
        );
        if would_keep > 0 {
            anstream::println!(
                "{m}  Kept files differ from what link would generate now — possibly \
                 edited since linking, or linked with different options (e.g. \
                 --model, or an interactive prompt answer); unlink cannot tell \
                 which. Remove them by hand if you no longer want them.{m:#}"
            );
        }
        return Ok(());
    }

    // 11. Delete existing files whose content still matches what the linker
    // would generate today.
    let mut deleted = 0;
    let mut kept = 0;
    let mut absent = 0;
    let mut deleted_generated: Vec<PathBuf> = Vec::new();
    let mut deleted_config: Vec<PathBuf> = Vec::new();

    for candidate in &candidates {
        let path = candidate.path();
        if !path.exists() {
            absent += 1;
            continue;
        }

        match candidate {
            Candidate::AlwaysDelete { .. } => {
                std::fs::remove_file(path)?;
                let m = crate::cli::style::muted();
                anstream::println!("{m}  deleted {}{m:#}", path.display());
                deleted += 1;
                deleted_config.push(path.to_path_buf());
            }
            Candidate::Generated { expected, .. } => {
                if content_matches(path, expected) {
                    std::fs::remove_file(path)?;
                    let m = crate::cli::style::muted();
                    anstream::println!("{m}  deleted {}{m:#}", path.display());
                    deleted += 1;
                    deleted_generated.push(path.to_path_buf());
                } else {
                    let w = crate::cli::style::warn();
                    anstream::println!(
                        "{w}  kept {} (content differs from what link would generate \
                         now; possibly edited since linking, or linked with different \
                         options such as --model or an interactive prompt answer){w:#}",
                        path.display()
                    );
                    kept += 1;
                }
            }
        }
    }

    // 12. Clean up empty ancestor directories left behind — bounded so the
    // cascade can never remove the target's own root directory (issue
    // #338 case 1's second half). Linker-generated paths are bounded by
    // `target_root` (e.g. `.claude/`); the project config file (if
    // removed) is bounded by the project root instead, since it lives
    // outside the target's tree entirely.
    for path in &deleted_generated {
        if let Some(parent) = path.parent() {
            remove_empty_ancestors(parent, &target_root);
        }
    }
    for path in &deleted_config {
        if let Some(parent) = path.parent() {
            remove_empty_ancestors(parent, &root);
        }
    }

    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!(
        "\n{o}Unlinked{o:#} {a}'{}'{a:#}: {m}{} deleted, {} kept (content differs), \
         {} already absent.{m:#}",
        target_name,
        deleted,
        kept,
        absent
    );
    if kept > 0 {
        anstream::println!(
            "{m}  Kept files differ from what link would generate now — possibly edited \
             since linking, or linked with different options (e.g. --model, or an \
             interactive prompt answer); unlink cannot tell which. Remove them by hand \
             if you no longer want them.{m:#}"
        );
    }

    Ok(())
}

/// Whether `path`'s on-disk bytes are identical to `expected`. Exact byte
/// comparison on purpose — no whitespace or line-ending normalisation. A
/// read failure (permissions, race with an external delete, ...) is treated
/// as "does not match": erring toward keeping a file is the whole point of
/// this guard.
fn content_matches(path: &Path, expected: &[u8]) -> bool {
    std::fs::read(path)
        .map(|actual| actual == expected)
        .unwrap_or(false)
}

/// Collect every file under a skill's *source* directory, keyed by its path
/// relative to that directory, together with its bytes. This mirrors
/// exactly what `link` copies into `<output_dir>/skills/<name>/` (see
/// `cli::link::collect_dir_files`) — including its valid-UTF-8-only gate: a
/// binary asset (e.g. `logo.png`) that `link` silently skips must be
/// skipped here too, or it would surface as a destination candidate that
/// was never actually written, inflating the "already absent" count with a
/// path that was never a real deletion candidate. The relative paths
/// returned here — and only those — are eligible for `unlink` to reclaim. A
/// file in the destination whose relative path isn't in this list was
/// placed there by the user after linking and must never be touched.
fn collect_source_files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    collect_source_files_recursive(dir, dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn collect_source_files_recursive(
    base: &Path,
    current: &Path,
    files: &mut Vec<(PathBuf, Vec<u8>)>,
) {
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_source_files_recursive(base, &path, files);
        } else if path.is_file()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let relative = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
            files.push((relative, content.into_bytes()));
        }
    }
}

/// Walk up from `path` removing empty directories, stopping at `stop_at`
/// (exclusive: `stop_at` itself is never removed, no matter how empty it
/// is). Callers pass the boundary that must survive — the target's root
/// directory for linker-generated paths, the project root for the config
/// file — so this function has no target-specific knowledge of its own.
fn remove_empty_ancestors(path: &Path, stop_at: &Path) {
    let mut current = path.to_path_buf();
    while current.starts_with(stop_at) && current != stop_at {
        if std::fs::read_dir(&current)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
        {
            if std::fs::remove_dir(&current).is_err() {
                break;
            }
        } else {
            break;
        }
        match current.parent() {
            Some(p) => current = p.to_path_buf(),
            None => break,
        }
    }
}
