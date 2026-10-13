use std::path::Path;

use crate::linker::{LinkTarget, supported_target_names};
use armadai_core::config;
use armadai_core::starter::{StarterPack, find_pack_dir, list_available_packs};

pub async fn execute(force: bool, project: bool, pack: Option<String>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    if let Some(pack_name) = pack {
        // a pack always becomes a project here: the user library is gone (C11)
        return init_project_from_pack(&root, &pack_name, force);
    }
    if project {
        return init_project(&root);
    }
    init_global(force)?;
    super::setup::prompt_shell_setup();
    Ok(())
}

/// Create the global config directory and default files.
fn init_global(force: bool) -> anyhow::Result<()> {
    let dir = config::config_dir();

    // Create directory tree
    config::ensure_config_dirs()?;
    let o = crate::cli::style::ok();
    let m = crate::cli::style::muted();
    anstream::println!(
        "{o}Created config directory:{o:#} {m}{}{m:#}",
        dir.display()
    );

    // Write config.yaml
    let config_path = config::config_file_path();
    write_if_missing_or_force(&config_path, config::DEFAULT_CONFIG_YAML, force)?;

    // Write providers.yaml
    let providers_path = config::providers_file_path();
    write_if_missing_or_force(&providers_path, config::DEFAULT_PROVIDERS_YAML, force)?;

    // Install built-in skills
    let skills_installed = armadai_core::skill::install_embedded_skills(force)?;

    anstream::println!("\n{o}ArmadAI initialized at{o:#} {m}{}{m:#}", dir.display());
    anstream::println!("{m}  config:    {}{m:#}", config_path.display());
    anstream::println!("{m}  providers: {}{m:#}", providers_path.display());
    anstream::println!(
        "{m}  skills:    {}{m:#}",
        config::user_skills_dir().display()
    );
    anstream::println!(
        "{m}  registry:  {} (remote starter-pack cache){m:#}",
        config::registry_cache_dir().display()
    );
    if skills_installed > 0 {
        anstream::println!(
            "{o}  built-in:  {} skill(s) installed{o:#}",
            skills_installed
        );
    }

    Ok(())
}

/// Resolve a pack reference: a local directory containing `pack.yaml`
/// takes precedence, otherwise fall back to the named starter pack lookup.
///
/// If both miss, auto-sync-on-miss kicks in: known starter registry sources
/// (user ∪ project, see `crate::starters_registry::effective_starter_sources`)
/// are synced once, then the lookup is retried. Network is only touched on
/// a miss — a plain `armadai init --pack <known-local-name>` never syncs.
pub(crate) fn resolve_pack_dir(name: &str) -> Option<std::path::PathBuf> {
    let candidate = std::path::Path::new(name);
    if candidate.join("pack.yaml").is_file() {
        return Some(candidate.to_path_buf());
    }
    if let Some(dir) = find_pack_dir(name) {
        return Some(dir);
    }

    // Auto-sync-on-miss: fetch remote starter sources, then retry once.
    let sources = crate::starters_registry::effective_starter_sources();
    if sources.is_empty() {
        return None;
    }
    let r = crate::cli::style::running();
    anstream::eprintln!(
        "{r}Pack '{name}' not found locally — syncing remote starter registries...{r:#}"
    );
    let _ = crate::starters_registry::sync_starters(&sources);
    find_pack_dir(name)
}

/// Refuse to overwrite a project: the new format, or a legacy one to migrate.
fn ensure_no_project(root: &Path) -> anyhow::Result<()> {
    if root.join(".armadai/project.yaml").exists() {
        anyhow::bail!("{} already holds .armadai/project.yaml", root.display());
    }
    if (crate::compile::LEGACY_CONFIGS.iter()).any(|c| root.join(c).is_file()) {
        anyhow::bail!(
            "{} holds a legacy project: run `armadai migrate` instead",
            root.display()
        );
    }
    Ok(())
}

/// An empty project: `project.yaml`, the four folders (kept by `.gitkeep`) and
/// `.armadai/.gitignore`.
fn init_project(root: &Path) -> anyhow::Result<()> {
    ensure_no_project(root)?;
    let base = root.join(".armadai");
    for dir in ["agents", "skills", "flows", "automations"] {
        std::fs::create_dir_all(base.join(dir))?;
        std::fs::write(base.join(dir).join(".gitkeep"), "")?;
    }
    std::fs::write(base.join("project.yaml"), "schema_version: 1\n")?;
    crate::linker::manifest::ensure_gitignore(root)?;
    let o = crate::cli::style::ok();
    let m = crate::cli::style::muted();
    anstream::println!("{o}Created .armadai/:{o:#}");
    for f in [
        "project.yaml",
        "agents/",
        "skills/",
        "flows/",
        "automations/",
        ".gitignore",
    ] {
        anstream::println!("{m}  .armadai/{f}{m:#}");
    }
    anstream::println!("\n{m}  Add agents under .armadai/agents/, then run `armadai link`.{m:#}");
    Ok(())
}

fn md_name(name: &str) -> String {
    if name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{name}.md")
    }
}

/// The pack, materialised as a legacy project under `tmp`: its files copied
/// under `tmp/.armadai/`, and a synthetic config naming them by path (never by
/// name, which would also search the user's library).
fn materialize_pack(
    pack_dir: &Path,
    pack: &StarterPack,
    pack_name: &str,
    tmp: &Path,
) -> anyhow::Result<()> {
    let base = tmp.join(".armadai");
    let unsafe_name = |n: &str| n.contains('/') || n.contains('\\') || n.contains("..");
    let mut config = String::from("agents:\n");
    std::fs::create_dir_all(base.join("agents"))?;
    for name in pack.agents.iter().filter(|n| !unsafe_name(n)) {
        let file = md_name(name);
        let src = pack_dir.join("agents").join(&file);
        if !src.is_file() {
            eprintln!(
                "  warn: agent '{name}' not found in pack at {}",
                src.display()
            );
            continue;
        }
        std::fs::copy(&src, base.join("agents").join(&file))?;
        config.push_str(&format!("  - path: .armadai/agents/{file}\n"));
    }
    config.push_str("prompts:\n");
    std::fs::create_dir_all(base.join("prompts"))?;
    for name in pack.prompts.iter().filter(|n| !unsafe_name(n)) {
        let file = md_name(name);
        let src = pack_dir.join("prompts").join(&file);
        if src.is_file() {
            std::fs::copy(&src, base.join("prompts").join(&file))?;
            config.push_str(&format!("  - path: .armadai/prompts/{file}\n"));
        }
    }
    config.push_str("skills:\n");
    for name in pack.skills.iter().filter(|n| !unsafe_name(n)) {
        let src = pack_dir.join("skills").join(name);
        if !src.is_dir() {
            eprintln!("  warn: skill '{name}' is not bundled in the pack: not added");
            continue;
        }
        copy_tree(&src, &base.join("skills").join(name))?;
        config.push_str(&format!("  - path: .armadai/skills/{name}\n"));
    }
    let coordinator = detect_pack_coordinator(pack, pack_name);
    let target = detect_pack_link_target(pack_name);
    if coordinator.is_some() || target.is_some() {
        config.push_str("link:\n");
        if let Some(t) = &target {
            config.push_str(&format!("  target: {t}\n"));
        }
        if let Some(c) = &coordinator {
            config.push_str(&format!("  coordinator: {c}\n"));
        }
    }
    if let Some(c) = &coordinator {
        config.push_str(&format!(
            "orchestration:\n  enabled: true\n  pattern: hierarchical\n  coordinator: {c}\n  teams:\n    - agents:\n"
        ));
        for a in pack.agents.iter().filter(|a| *a != c) {
            config.push_str(&format!("        - {a}\n"));
        }
    }
    std::fs::write(base.join("config.yaml"), config)?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &dest)?;
        } else if e.file_type()?.is_file() {
            std::fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// A canonical project from a starter pack. A native pack (`format: 1`) is
/// copied (`native_pack`); a legacy one is materialised as a legacy project in a
/// scratch directory and converted by `migrate`, then its `.armadai/` is copied
/// here, backup excluded.
fn init_project_from_pack(root: &Path, pack_name: &str, force: bool) -> anyhow::Result<()> {
    if !force {
        ensure_no_project(root)?;
    }
    let Some(pack_dir) = resolve_pack_dir(pack_name) else {
        let available = list_available_packs();
        anyhow::bail!(
            "Starter pack '{pack_name}' not found. Available packs: {}",
            if available.is_empty() {
                "none".into()
            } else {
                available.join(", ")
            }
        );
    };
    let pack = StarterPack::load(&pack_dir)?;
    no_link_at(&root.join(".armadai"))?;
    match pack.format {
        Some(1) => return super::native_pack::install(root, &pack_dir, &pack.name),
        Some(n) => anyhow::bail!(
            "pack '{}' uses format {n}; this armadai reads format 1: update armadai",
            pack.name
        ),
        None => {}
    }
    let tmp = tempfile::tempdir()?;
    materialize_pack(&pack_dir, &pack, pack_name, tmp.path())?;
    let report = crate::migrate::run(tmp.path(), false, false)?;
    let built = tmp.path().join(".armadai");
    // every destination, before the first write: never through a symbolic link
    let mut todo = vec![built.clone()];
    while let Some(dir) = todo.pop() {
        for e in std::fs::read_dir(&dir)? {
            let path = e?.path();
            let rel = path.strip_prefix(&built)?;
            if rel.starts_with(".migrated") {
                continue;
            }
            let mut at = root.join(".armadai");
            for c in rel.components() {
                at.push(c);
                no_link_at(&at)?;
            }
            if path.is_dir() {
                todo.push(path);
            }
        }
    }
    for e in std::fs::read_dir(&built)? {
        let e = e?;
        if e.file_name() == ".migrated" {
            continue;
        }
        let dest = root.join(".armadai").join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &dest)?;
        } else {
            std::fs::create_dir_all(root.join(".armadai"))?;
            std::fs::copy(e.path(), dest)?;
        }
    }
    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!(
        "{o}Created .armadai/ from pack{o:#} {a}'{}'{a:#}: {m}{} agent(s), {} skill(s){m:#}",
        pack.name,
        report
            .written
            .iter()
            .filter(|p| p.starts_with(".armadai/agents"))
            .count(),
        report
            .written
            .iter()
            .filter(|p| p.starts_with(".armadai/skills") && p.ends_with("SKILL.md"))
            .count(),
    );
    anstream::println!("{m}  legacy pack converted (no `format: 1` in its pack.yaml){m:#}");
    for w in &report.warnings {
        let s = crate::cli::style::warn();
        anstream::eprintln!("{s}  warn: {w}{s:#}");
    }
    anstream::println!("{m}  Run `armadai link` to generate the target files.{m:#}");
    Ok(())
}

/// Refuse a destination that is a symbolic link (dangling or not): a pack never
/// writes through one.
fn no_link_at(path: &Path) -> anyhow::Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        if !path.exists() {
            anyhow::bail!(
                "{} is a dangling symbolic link: remove it, or create what it points to",
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned()
                )
            );
        }
        anyhow::bail!(
            "{} is a symbolic link: refusing to write a pack through it",
            path.display()
        );
    }
    Ok(())
}

/// Detect a coordinator agent from a pack by scanning agent files for a
/// `tags: [... coordinator ...]` metadata entry.
fn detect_pack_coordinator(pack: &StarterPack, pack_name: &str) -> Option<String> {
    let agents_dir = resolve_pack_dir(pack_name)?.join("agents");
    if !agents_dir.is_dir() {
        return None;
    }

    for agent_name in &pack.agents {
        let filename = if agent_name.ends_with(".md") {
            agent_name.clone()
        } else {
            format!("{agent_name}.md")
        };
        let path = agents_dir.join(&filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let trimmed = line.trim().trim_start_matches("- ");
                if let Some(tags_value) = trimmed.strip_prefix("tags:") {
                    let tags_lower = tags_value.to_lowercase();
                    if tags_lower.contains("coordinator") {
                        return Some(agent_name.clone());
                    }
                }
            }
        }
    }

    None
}

/// Map an agent `provider:` value to a supported link target, if it names one.
///
/// Only a supported target name is kept as is; `anthropic` (the vendor of
/// Claude Code) maps to `claude`. Anything else (`google`, `gemini`, `cli`,
/// `openai`, ...) says nothing about which tool to link and yields `None`.
fn link_target_for_provider(provider: &str) -> Option<String> {
    let provider = provider.trim();
    if provider == "anthropic" {
        return Some(LinkTarget::Claude.as_str().to_string());
    }
    supported_target_names()
        .into_iter()
        .find(|name| *name == provider)
        .map(str::to_string)
}

/// Pick the link target from the providers of a pack's agents, given in
/// agent-file order: the first provider that maps to a supported target wins.
fn pick_link_target(providers: &[String]) -> Option<String> {
    providers.iter().find_map(|p| link_target_for_provider(p))
}

/// Order agent files by file name.
fn sort_agent_files(files: &mut [std::path::PathBuf]) {
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
}

/// Read the `provider:` of every agent file in `agents_dir`, ordered by file
/// name so the result never depends on `read_dir` order.
fn read_agent_providers(agents_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(agents_dir) else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    sort_agent_files(&mut files);

    files
        .iter()
        .filter_map(|path| {
            let content = std::fs::read_to_string(path).ok()?;
            content.lines().find_map(|line| {
                let provider = line
                    .trim()
                    .trim_start_matches("- ")
                    .strip_prefix("provider:")?
                    .trim();
                (!provider.is_empty()).then(|| provider.to_string())
            })
        })
        .collect()
}

/// Infer the link target of a pack from its agents, or `None` when no agent
/// names a supported target.
fn detect_pack_link_target(pack_name: &str) -> Option<String> {
    let agents_dir = resolve_pack_dir(pack_name)?.join("agents");
    pick_link_target(&read_agent_providers(&agents_dir))
}

fn write_if_missing_or_force(
    path: &std::path::Path,
    content: &str,
    force: bool,
) -> anyhow::Result<()> {
    if path.exists() && !force {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  skip (exists): {}{m:#}", path.display());
        return Ok(());
    }
    std::fs::write(path, content)?;
    if force && path.exists() {
        let w = crate::cli::style::warn();
        anstream::println!("{w}  overwritten:   {}{w:#}", path.display());
    } else {
        let o = crate::cli::style::ok();
        anstream::println!("{o}  created:       {}{o:#}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn link_target_for_provider_maps_only_supported_targets_and_anthropic() {
        assert_eq!(
            link_target_for_provider("anthropic").as_deref(),
            Some("claude")
        );
        for t in ["claude", "codex", "copilot", "opencode"] {
            assert_eq!(link_target_for_provider(t).as_deref(), Some(t));
        }
        for p in ["google", "gemini", "cli", "openai", "gpt", ""] {
            assert_eq!(link_target_for_provider(p), None, "{p:?}");
        }
    }

    #[test]
    fn pick_link_target_first_valid_in_order_wins() {
        assert_eq!(pick_link_target(&[]), None);
        assert_eq!(pick_link_target(&strs(&["google", "cli", "gemini"])), None);
        assert_eq!(
            pick_link_target(&strs(&["cli", "anthropic", "codex"])).as_deref(),
            Some("claude")
        );
        assert_eq!(
            pick_link_target(&strs(&["google", "codex", "claude"])).as_deref(),
            Some("codex")
        );
    }

    #[test]
    fn sort_agent_files_orders_by_file_name() {
        use std::path::PathBuf;
        let mut files: Vec<PathBuf> = ["d/c.md", "d/a.md", "d/b.md"]
            .iter()
            .map(PathBuf::from)
            .collect();
        sort_agent_files(&mut files);
        assert_eq!(
            files,
            ["d/a.md", "d/b.md", "d/c.md"]
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn read_agent_providers_follows_sorted_file_names() {
        let dir = tempfile::tempdir().unwrap();
        // Created in the reverse of the sorted order.
        for (file, provider) in [("c.md", "codex"), ("b.md", "google"), ("a.md", "copilot")] {
            std::fs::write(
                dir.path().join(file),
                format!("# x\n\n## Metadata\n- provider: {provider}\n"),
            )
            .unwrap();
        }
        std::fs::write(dir.path().join("0.txt"), "provider: opencode\n").unwrap();
        let providers = read_agent_providers(dir.path());
        assert_eq!(providers, strs(&["copilot", "google", "codex"]));
        assert_eq!(pick_link_target(&providers).as_deref(), Some("copilot"));
    }

    use super::*;

    #[test]
    fn resolve_pack_dir_accepts_local_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pack.yaml"),
            "name: local-pack\ndescription: d\n",
        )
        .unwrap();
        let resolved = resolve_pack_dir(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(resolved, dir.path());
    }

    #[test]
    fn resolve_pack_dir_miss_without_starter_sources_touches_no_network() {
        // Backward compat: with no starter registry sources configured (the
        // default — no `registries.yaml`), a miss on an unknown pack name
        // must return None without attempting any sync/network call.
        let _guard = armadai_core::test_support::env_lock();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        let config_dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised via `env_lock()`; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", config_dir.path());
        }

        let resolved = resolve_pack_dir("definitely-nonexistent-pack-xyz");
        assert!(resolved.is_none());

        // SAFETY: restoring original env state at end of test scope.
        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }
}
