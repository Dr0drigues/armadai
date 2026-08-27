use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::linker::model_resolution::{self, TargetKind};
use crate::linker::{self, LinkAgent};
use armadai_core::project;

pub async fn execute(
    target: Option<crate::linker::LinkTarget>,
    model_flag: Option<String>,
    coordinator_flag: Option<String>,
    dry_run: bool,
    force: bool,
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

    if let Err(e) = armadai_core::project_registry::register_project(&root) {
        tracing::warn!("Failed to register project in registry: {:?}", e);
    }
    armadai_core::model_updater::auto_check_and_prompt(&root, std::io::stdin().is_terminal());

    // Every agent in `.armadai/agents.yaml` is included automatically (it
    // does not need to be relisted in `agents:` — that would duplicate the
    // declaration this format exists to remove), so an otherwise-empty
    // `agents:` list is only a real error when there is no declarations
    // file either.
    if !armadai_core::agent_source::project_declares_agents(&root, &config) {
        anyhow::bail!("No agents declared in project config.");
    }

    // 1b. Validate orchestration config if enabled
    if let Some(ref orch) = config.orchestration
        && orch.enabled
        && let Err(errors) = armadai_core::orchestration::validate_config(orch)
    {
        let e = crate::cli::style::err();
        anstream::eprintln!("{e}Orchestration validation failed:{e:#}\n");
        for error in &errors {
            anstream::eprintln!("{e}  - {}{e:#}", error);
        }
        anyhow::bail!(
            "Cannot link with invalid orchestration config. {} error(s) found.",
            errors.len()
        );
    }

    // 2. Resolve and load agents — file-backed and declared alike.
    let fragments = armadai_core::agent_source::project_fragments(&root);
    let (agents, warnings) =
        armadai_core::agent_source::load_all_agents(&config, &root, &fragments);
    for w in &warnings {
        let s = crate::cli::style::warn();
        anstream::eprintln!("{s}  warn: {}{s:#}", w.message());
    }

    let mut link_agents: Vec<LinkAgent> = agents.iter().map(LinkAgent::from).collect();

    if link_agents.is_empty() {
        anyhow::bail!("No agents could be resolved. Check your project config.");
    }

    // 2b. Resolve deprecated model aliases before remapping
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

    // 3a. `link` writes config that other tools then trust — unlike `list`
    // (read-only), it must never silently ship a fleet smaller than the one
    // declared. But only when THIS chantier's format is the reason (a
    // dropped declaration, or a shadowing collision): a pre-existing
    // failure (an unparseable `.md`, an unresolvable ref) keeps its exact
    // old behaviour — warn above, link what did load, exit 0 — since that
    // behaviour predates this format and was never wrong.
    //
    // Checked AFTER `--agents` filtering, and scoped to it: a loss outside
    // what was actually requested (`--agents good` when only `bad` was
    // dropped) must not refuse a link that never intended to write `bad` in
    // the first place. `--dry-run` reaches this exact same check with no
    // special-casing either way, further down — a preview must refuse
    // exactly when the real link would.
    if armadai_core::agent_source::blocks_a_write(&warnings, agents_filter.as_deref()) {
        anyhow::bail!(
            "one or more agents could not be loaded (see warning(s) above) — refusing to link a smaller fleet than declared. Fix the issue(s), or rerun once resolved."
        );
    }

    // 3b. Extract coordinator if configured (CLI flag takes priority over config)
    let coordinator_name =
        coordinator_flag.or_else(|| config.link.as_ref().and_then(|l| l.coordinator.clone()));
    let mut coordinator = coordinator_name.and_then(|name| {
        let idx = link_agents
            .iter()
            .position(|a| crate::linker::name_matches_reference(&a.name, &name))?;
        Some(link_agents.remove(idx))
    });

    // 4. Determine target
    let target_name = target
        .map(|t| t.to_string())
        .or_else(|| config.link.as_ref().and_then(|l| l.target.clone()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No link target specified. Use --target or set link.target in armadai.yaml.\n{}",
                crate::linker::supported_targets_sentence()
            )
        })?;

    // 4b. Model resolution: remap agent models based on target kind
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
            if let Some(ref model) = model_flag {
                model_resolution::remap_models_for_orchestrator(&mut link_agents, model);
                if let Some(ref mut coord) = coordinator {
                    model_resolution::remap_models_for_orchestrator(
                        std::slice::from_mut(coord),
                        model,
                    );
                }
            } else if std::io::stdin().is_terminal() {
                #[cfg(feature = "providers-api")]
                let model = model_resolution::prompt_model_interactive().await?;
                #[cfg(not(feature = "providers-api"))]
                let model = model_resolution::prompt_model_interactive()?;
                model_resolution::remap_models_for_orchestrator(&mut link_agents, &model);
                if let Some(ref mut coord) = coordinator {
                    model_resolution::remap_models_for_orchestrator(
                        std::slice::from_mut(coord),
                        &model,
                    );
                }
            } else {
                // Non-interactive without --model: resolve latest:* placeholders
                // using each agent's own provider
                model_resolution::resolve_latest_placeholders(&mut link_agents);
                if let Some(ref mut coord) = coordinator {
                    model_resolution::resolve_latest_placeholders(std::slice::from_mut(coord));
                }
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
    // The target's own root — never itself recorded as a `created_dirs`
    // entry below, so `unlink` never removes it even when everything
    // inside it is reclaimed (issue #338 case 1's second half: `.claude/`
    // itself must survive, and the same protection extends to a custom
    // `--output` directory).
    let target_root = root.join(&output_dir);

    // 7. Generate files
    let sources = &config.sources;
    let files = linker.generate(&link_agents, coordinator.as_ref(), sources);

    if files.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No files to generate.{m:#}");
        return Ok(());
    }

    // 8. Resolve output paths relative to project root, tagging each with
    // what produced it for the link manifest (issue #338). Every `Linker`
    // implementation emits exactly one file per agent, in the same order as
    // `link_agents`, before any aggregate/context file — verified by
    // reading each of claude/codex/copilot/gemini/opencode's own
    // `generate()`. Anything past that prefix is the target's
    // coordinator/context document (`CLAUDE.md`, `GEMINI.md`, `AGENTS.md`,
    // `copilot-instructions.md`, `instructions.md`), attributed to the
    // configured coordinator, or — for the handful of targets that emit a
    // team-roster document even with no coordinator set (codex, gemini) —
    // to the target itself, since no single agent owns it.
    let agent_count = link_agents.len();
    let output_files: Vec<(PathBuf, String, linker::manifest::ProducedBy)> = files
        .into_iter()
        .enumerate()
        .map(|(idx, f)| {
            // Replace the default output dir prefix with the custom output dir
            let default_dir = PathBuf::from(linker.default_output_dir());
            let relative = f.path.strip_prefix(&default_dir).unwrap_or(&f.path);
            let final_path = root.join(&output_dir).join(relative);
            let produced_by = if idx < agent_count {
                linker::manifest::ProducedBy::agent(link_agents[idx].name.clone())
            } else {
                linker::manifest::ProducedBy::coordinator(
                    coordinator
                        .as_ref()
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| target_name.clone()),
                )
            };
            (final_path, f.content, produced_by)
        })
        .collect();

    // 8b. Resolve and collect skill files
    let (skill_dirs, skill_errors) = project::resolve_all_skills(&config, &root);
    for err in &skill_errors {
        let w = crate::cli::style::warn();
        anstream::eprintln!("{w}  warn: {err}{w:#}");
    }

    let mut extra_files: Vec<(PathBuf, String, linker::manifest::ProducedBy)> = Vec::new();
    let mut skill_count = 0;
    for skill_dir in &skill_dirs {
        if let Ok(entries) = collect_dir_files(skill_dir) {
            let skill_name = skill_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            for (relative, content) in entries {
                let final_path = root
                    .join(&output_dir)
                    .join("skills")
                    .join(skill_name)
                    .join(&relative);
                extra_files.push((
                    final_path,
                    content,
                    linker::manifest::ProducedBy::skill(skill_name),
                ));
            }
            skill_count += 1;
        }
    }

    // 8c. Resolve and collect prompt files
    let (prompt_paths, prompt_errors) = project::resolve_all_prompts(&config, &root);
    for err in &prompt_errors {
        let w = crate::cli::style::warn();
        anstream::eprintln!("{w}  warn: {err}{w:#}");
    }

    let mut prompt_count = 0;
    for prompt_path in &prompt_paths {
        if let Ok(content) = std::fs::read_to_string(prompt_path) {
            let filename = prompt_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.md");
            let prompt_name = prompt_path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or(filename);
            let final_path = root.join(&output_dir).join("prompts").join(filename);
            extra_files.push((
                final_path,
                content,
                linker::manifest::ProducedBy::prompt(prompt_name),
            ));
            prompt_count += 1;
        }
    }

    // 9. Dry run or write
    if dry_run {
        let h = crate::cli::style::header();
        let a = crate::cli::style::accent();
        let m = crate::cli::style::muted();
        anstream::println!(
            "{h}Dry run{h:#} — files that would be generated for {a}'{}'{a:#}:\n",
            target_name
        );
        for (path, _, _) in &output_files {
            anstream::println!("{m}  {}{m:#}", path.display());
        }
        for (path, _, _) in &extra_files {
            anstream::println!("{m}  {}{m:#}", path.display());
        }
        anstream::println!(
            "\n{m}  {} file(s) total.{m:#}",
            output_files.len() + extra_files.len()
        );
        return Ok(());
    }

    // The actual write — exists-guard and manifest write both live in
    // `linker::manifest::write_files` now (issue #347): this used to be an
    // inline loop here, and the shell wizard's own independent copy of it
    // (with neither guarantee) is exactly the defect that extraction
    // fixed. Every caller that writes linker output goes through this one
    // function.
    let outcomes = linker::manifest::write_files(
        &root,
        &target_name,
        &output_dir,
        &target_root,
        output_files.into_iter().chain(extra_files).collect(),
        force,
    )?;

    let mut written = 0;
    let mut skipped = 0;
    let mut unchanged = 0;
    for outcome in &outcomes {
        match outcome {
            linker::manifest::FileOutcome::Wrote(path) => {
                let m = crate::cli::style::muted();
                anstream::println!("{m}  wrote {}{m:#}", path.display());
                written += 1;
            }
            linker::manifest::FileOutcome::UpToDate(path) => {
                let m = crate::cli::style::muted();
                anstream::println!("{m}  up-to-date {}{m:#}", path.display());
                unchanged += 1;
            }
            linker::manifest::FileOutcome::SkippedExisting(path) => {
                let w = crate::cli::style::warn();
                anstream::eprintln!(
                    "{w}  skip: {} already exists (use --force to overwrite){w:#}",
                    path.display()
                );
                skipped += 1;
            }
        }
    }

    let mut summary = format!("Linked {} agent(s)", link_agents.len());
    if skill_count > 0 {
        summary.push_str(&format!(", {} skill(s)", skill_count));
    }
    if prompt_count > 0 {
        summary.push_str(&format!(", {} prompt(s)", prompt_count));
    }
    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!(
        "\n{o}{}{o:#} to {a}'{}'{a:#}: {m}{} written, {} skipped, {} unchanged.{m:#}",
        summary,
        target_name,
        written,
        skipped,
        unchanged
    );

    Ok(())
}

/// Collect all files from a directory recursively as (relative_path, content) pairs.
/// Only includes text files (valid UTF-8).
fn collect_dir_files(dir: &Path) -> anyhow::Result<Vec<(PathBuf, String)>> {
    let mut files = Vec::new();
    collect_dir_files_recursive(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn collect_dir_files_recursive(
    base: &Path,
    current: &Path,
    files: &mut Vec<(PathBuf, String)>,
) -> anyhow::Result<()> {
    let entries = std::fs::read_dir(current)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir_files_recursive(base, &path, files)?;
        } else if path.is_file()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let relative = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
            files.push((relative, content));
        }
    }
    Ok(())
}
