use crate::linker::{LinkTarget, supported_target_names};
use armadai_core::config;
use armadai_core::starter::{StarterPack, find_pack_dir, list_available_packs};

pub async fn execute(force: bool, project: bool, pack: Option<String>) -> anyhow::Result<()> {
    if let Some(ref pack_name) = pack
        && project
    {
        // Combined mode: install pack + create project config referencing it
        init_global(force)?;
        let installed_pack = install_pack(pack_name, force)?;
        init_project_with_pack(&installed_pack, pack_name)?;
        return Ok(());
    }

    if project {
        return init_project();
    }

    // Always init global config first
    init_global(force)?;

    // Install starter pack if requested
    if let Some(pack_name) = pack {
        install_pack(&pack_name, force)?;
    }

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
        "{m}  agents:    {}{m:#}",
        config::user_agents_dir().display()
    );
    anstream::println!(
        "{m}  prompts:   {}{m:#}",
        config::user_prompts_dir().display()
    );
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

/// Install a starter pack by name or local path. Returns the loaded pack definition.
fn install_pack(name: &str, force: bool) -> anyhow::Result<StarterPack> {
    let pack_dir = match resolve_pack_dir(name) {
        Some(dir) => dir,
        None => {
            let available = list_available_packs();
            if available.is_empty() {
                anyhow::bail!("Starter pack '{name}' not found. No packs available.");
            } else {
                anyhow::bail!(
                    "Starter pack '{name}' not found. Available packs: {}",
                    available.join(", ")
                );
            }
        }
    };

    let pack = StarterPack::load(&pack_dir)?;
    let r = crate::cli::style::running();
    let a = crate::cli::style::accent();
    anstream::println!(
        "\n{r}Installing starter pack:{r:#} {a}{}{a:#} — {}",
        pack.name,
        pack.description
    );

    let (agents, prompts, skills) = pack.install(&pack_dir, force)?;
    let o = crate::cli::style::ok();
    let m = crate::cli::style::muted();
    anstream::println!(
        "\n{o}Pack{o:#} {a}'{}'{a:#} {o}installed:{o:#} {m}{} agent(s), {} prompt(s), {} skill(s){m:#}",
        pack.name,
        agents,
        prompts,
        skills
    );

    Ok(pack)
}

/// Generate the YAML content for an empty armadai project configuration.
pub fn generate_empty_project_yaml() -> String {
    "\
# ArmadAI project configuration
# See: https://github.com/Dr0drigues/swarm-festai

# Agents used in this project
agents:
  # - name: code-reviewer           # Named agent from user library
  # - registry: official/security   # Agent from the registry
  # - path: .armadai/agents/team.md # Local agent file

# Composable prompt fragments
prompts: []
  # - name: rust-conventions
  # - path: .armadai/prompts/style.md

# Skills (Agent Skills open standard)
skills: []
  # - name: docker-compose
  # - path: .armadai/skills/deploy/

# Context files injected into agent runs
sources: []
  # - docs/architecture.md
  # - CONTRIBUTING.md

# Linker configuration
# link:
#   target: claude
#   overrides:
#     claude:
#       output: .claude/
#     copilot:
#       output: .github/agents/
"
    .to_string()
}

/// Create a `.armadai/` project structure in the current directory.
fn init_project() -> anyhow::Result<()> {
    let dotarmadai = std::path::Path::new(".armadai");
    let dotarmadai_config = dotarmadai.join("config.yaml");
    let legacy_path = std::path::Path::new("armadai.yaml");

    if dotarmadai_config.exists() {
        anyhow::bail!(".armadai/config.yaml already exists in current directory");
    }
    if legacy_path.exists() {
        anyhow::bail!(
            "armadai.yaml already exists in current directory. \
             Remove it first or manually move it to .armadai/config.yaml"
        );
    }

    // Create directory structure
    for subdir in &["agents", "prompts", "skills", "starters"] {
        std::fs::create_dir_all(dotarmadai.join(subdir))?;
    }

    let content = generate_empty_project_yaml();
    std::fs::write(&dotarmadai_config, content)?;
    let o = crate::cli::style::ok();
    let m = crate::cli::style::muted();
    anstream::println!("{o}Created .armadai/ project structure:{o:#}");
    anstream::println!("{m}  .armadai/config.yaml{m:#}");
    anstream::println!("{m}  .armadai/agents/{m:#}");
    anstream::println!("{m}  .armadai/prompts/{m:#}");
    anstream::println!("{m}  .armadai/skills/{m:#}");
    anstream::println!("{m}  .armadai/starters/{m:#}");
    anstream::println!(
        "\n{m}  Edit .armadai/config.yaml to declare agents, prompts, skills and link targets.{m:#}"
    );

    // Check for deprecated models in newly created project
    if let Some((root, _)) = armadai_core::project::find_project_config() {
        armadai_core::model_updater::auto_check_and_prompt(&root, true);
    }

    Ok(())
}

/// Generate YAML content for an armadai project pre-configured with a starter pack.
pub fn generate_project_yaml(pack: &StarterPack, pack_name: &str) -> String {
    // Detect provider and coordinator from the pack's agent files
    let link_target = detect_pack_link_target(pack_name);
    let coordinator_name = detect_pack_coordinator(pack, pack_name);

    let mut content = String::from(
        "# ArmadAI project configuration\n\
         # See: https://github.com/Dr0drigues/swarm-festai\n\n\
         # Agents used in this project\n\
         agents:\n",
    );

    for agent in &pack.agents {
        content.push_str(&format!("  - name: {agent}\n"));
    }

    // Prompts
    if !pack.prompts.is_empty() {
        content.push_str("\n# Composable prompt fragments\nprompts:\n");
        for prompt in &pack.prompts {
            content.push_str(&format!("  - name: {prompt}\n"));
        }
    } else {
        content.push_str("\nprompts: []\n");
    }

    // Skills
    if !pack.skills.is_empty() {
        content.push_str("\n# Skills (Agent Skills open standard)\nskills:\n");
        for skill in &pack.skills {
            content.push_str(&format!("  - name: {skill}\n"));
        }
    } else {
        content.push_str("\nskills: []\n");
    }

    content.push_str("\nsources: []\n");

    // Linker configuration
    if let Some(target) = link_target {
        content.push_str(&format!(
            "\n# Linker configuration\nlink:\n  target: {target}\n"
        ));
        if let Some(ref coord) = coordinator_name {
            content.push_str(&format!("  coordinator: {coord}\n"));
        }
    } else if let Some(ref coord) = coordinator_name {
        content.push_str(&format!(
            "\n# Linker configuration\nlink:\n  coordinator: {coord}\n"
        ));
    }

    // Orchestration configuration (when a coordinator is detected)
    if let Some(ref coord) = coordinator_name {
        let non_coord_agents: Vec<&String> = pack
            .agents
            .iter()
            .filter(|a| Some(*a) != coordinator_name.as_ref())
            .collect();

        content.push_str(&format!(
            "\n# Orchestration — auto-detected coordinator '{coord}'\n\
             orchestration:\n\
             \x20 enabled: true\n\
             \x20 pattern: hierarchical\n\
             \x20 coordinator: {coord}\n\
             \x20 teams:\n\
             \x20   - agents:\n"
        ));
        for agent in &non_coord_agents {
            content.push_str(&format!("        - {agent}\n"));
        }
    }

    content
}

/// Create a `.armadai/` project structure pre-configured with a starter pack.
fn init_project_with_pack(pack: &StarterPack, pack_name: &str) -> anyhow::Result<()> {
    let dotarmadai = std::path::Path::new(".armadai");
    let dotarmadai_config = dotarmadai.join("config.yaml");
    let legacy_path = std::path::Path::new("armadai.yaml");

    if dotarmadai_config.exists() {
        anyhow::bail!(".armadai/config.yaml already exists in current directory");
    }
    if legacy_path.exists() {
        anyhow::bail!(
            "armadai.yaml already exists in current directory. \
             Remove it first or manually move it to .armadai/config.yaml"
        );
    }

    // Create directory structure
    for subdir in &["agents", "prompts", "skills", "starters"] {
        std::fs::create_dir_all(dotarmadai.join(subdir))?;
    }

    let content = generate_project_yaml(pack, pack_name);
    std::fs::write(&dotarmadai_config, &content)?;
    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!(
        "\n{o}Created .armadai/config.yaml with pack{o:#} {a}'{}'{a:#} {o}agents{o:#}",
        pack.name
    );
    if pack_link_target_missing(&content) {
        let w = crate::cli::style::warn();
        anstream::println!(
            "{w}  No link target could be inferred from the pack's agents.{w:#}\n\
             {m}  Run `armadai link --target <{}>` to generate target config files.{m:#}",
            supported_target_names().join("|")
        );
    } else {
        anstream::println!("{m}  Run `armadai link` to generate target config files.{m:#}");
    }

    // Check for deprecated models in newly created project
    if let Some((root, _)) = armadai_core::project::find_project_config() {
        armadai_core::model_updater::auto_check_and_prompt(&root, true);
    }

    Ok(())
}

/// True when the generated project config carries no `link.target`.
fn pack_link_target_missing(yaml: &str) -> bool {
    !yaml.lines().any(|l| l.starts_with("  target: "))
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
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

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

    #[test]
    fn embedded_packs_never_yield_an_invalid_link_target() {
        let starters =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../armadai-core/starters");
        let supported = supported_target_names();
        let mut seen = 0;
        for entry in std::fs::read_dir(&starters).unwrap().flatten() {
            let dir = entry.path();
            if !dir.join("pack.yaml").is_file() {
                continue;
            }
            seen += 1;
            let pack = StarterPack::load(&dir).unwrap();
            let yaml = generate_project_yaml(&pack, dir.to_str().unwrap());
            let targets: Vec<&str> = yaml
                .lines()
                .filter_map(|l| l.strip_prefix("  target: "))
                .collect();
            assert!(targets.len() <= 1, "{}: {targets:?}", pack.name);
            for t in targets {
                assert!(
                    supported.contains(&t.trim()),
                    "{}: invalid link.target {t:?}",
                    pack.name
                );
            }
        }
        assert!(seen >= 7, "expected the embedded packs, saw {seen}");
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
