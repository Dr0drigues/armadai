use clap::{Subcommand, ValueEnum};

use crate::core::config::registries_config_path;
use crate::core::project::find_project_config;
use crate::core::registries::{
    RegistriesConfig, RegistryKind, RegistrySource, load_user_registries,
};
use crate::registry::{cache, convert, search, sync};

#[derive(Subcommand)]
pub enum RegistryAction {
    /// Sync (clone or pull) the community registry
    Sync,
    /// Search agents by keyword
    Search {
        /// Search query (keywords, AND logic)
        query: String,
        /// Filter by category
        #[arg(long)]
        category: Option<String>,
    },
    /// List all agents in the registry
    List {
        /// Filter by category
        #[arg(long)]
        category: Option<String>,
    },
    /// Import an agent into the user library
    Add {
        /// Agent path in registry (e.g. "agents/official/security.agent.md")
        agent: String,
        /// Overwrite existing agent
        #[arg(long)]
        force: bool,
    },
    /// Show details of a registry agent
    Info {
        /// Agent name or path in registry
        agent: String,
    },
    /// Manage custom registry sources (URLs)
    Sources {
        #[command(subcommand)]
        action: SourcesAction,
    },
}

/// Custom registry source actions (list/add/remove URLs).
#[derive(Subcommand)]
pub enum SourcesAction {
    /// List all registry sources and their origins
    List,
    /// Add a custom registry source to user config
    Add {
        /// Registry kind (agents/skills/models)
        kind: SourceKind,
        /// Registry source URL
        url: String,
    },
    /// Remove a custom registry source from user config
    Remove {
        /// Registry kind (agents/skills/models)
        kind: SourceKind,
        /// Registry source URL
        url: String,
    },
}

/// Registry kind for custom sources.
#[derive(Clone, Copy, ValueEnum)]
pub enum SourceKind {
    /// Agent registry sources
    Agents,
    /// Skill registry sources
    Skills,
    /// Model catalog sources
    Models,
    /// Starter pack registry sources
    Starters,
}

impl From<SourceKind> for RegistryKind {
    fn from(kind: SourceKind) -> Self {
        match kind {
            SourceKind::Agents => RegistryKind::Agents,
            SourceKind::Skills => RegistryKind::Skills,
            SourceKind::Models => RegistryKind::Models,
            SourceKind::Starters => RegistryKind::Starters,
        }
    }
}

pub async fn execute(action: RegistryAction) -> anyhow::Result<()> {
    match action {
        RegistryAction::Sync => cmd_sync().await,
        RegistryAction::Search { query, category } => cmd_search(&query, category.as_deref()).await,
        RegistryAction::List { category } => cmd_list(category.as_deref()).await,
        RegistryAction::Add { agent, force } => cmd_add(&agent, force).await,
        RegistryAction::Info { agent } => cmd_info(&agent).await,
        RegistryAction::Sources { action } => match action {
            SourcesAction::List => sources_list().await,
            SourcesAction::Add { kind, url } => sources_add(kind, &url).await,
            SourcesAction::Remove { kind, url } => sources_remove(kind, &url).await,
        },
    }
}

async fn cmd_sync() -> anyhow::Result<()> {
    let sources = sync::effective_sources();
    println!("Syncing {} agent registry source(s)...", sources.len());
    sync::registry_sync(&sources)?;
    println!("Building search index...");
    let index = cache::build_index(&sources)?;
    println!("Indexed {} agent(s).", index.entries.len());

    let starter_sources = effective_starter_sources();
    println!("Syncing {} starter source(s)...", starter_sources.len());
    if !starter_sources.is_empty() {
        crate::starters_registry::sync_starters(&starter_sources);
    }

    Ok(())
}

/// Gather typed starter registry sources (user ∪ project), deduplicated by
/// URL. Unlike `resolved_sources` (which flattens to bare URL strings and
/// loses the explicit `kind` override), this keeps the full [`RegistrySource`]
/// so `sync_starters` can dispatch via `resolved_kind()` with an explicit
/// `kind:` honored when set.
pub(crate) fn effective_starter_sources() -> Vec<RegistrySource> {
    let user = load_user_registries();
    let project = find_project_config()
        .map(|(_, cfg)| cfg)
        .and_then(|cfg| cfg.registries);

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let project_starters = project.map(|cfg| cfg.starters).unwrap_or_default();
    for source in user.starters.into_iter().chain(project_starters) {
        if seen.insert(source.url.clone()) {
            out.push(source);
        }
    }
    out
}

async fn cmd_search(query: &str, category: Option<&str>) -> anyhow::Result<()> {
    check_staleness();
    let sources = sync::effective_sources();
    let index = cache::load_or_build_index(&sources)?;

    let entries = match category {
        Some(cat) => {
            let filtered = search::filter_by_category(&index.entries, cat);
            filtered.into_iter().cloned().collect::<Vec<_>>()
        }
        None => index.entries.clone(),
    };

    let results = search::search(&entries, query);

    if results.is_empty() {
        println!("No agents matching '{query}'.");
        return Ok(());
    }

    // Compute column widths
    let name_w = results
        .iter()
        .map(|r| r.entry.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!("  {:<name_w$}  SCORE  DESCRIPTION", "NAME",);
    println!("  {:<name_w$}  -----  -----------", "-".repeat(name_w),);

    for r in &results {
        let desc = r.entry.description.as_deref().unwrap_or("-");
        println!("  {:<name_w$}  {:>5}  {}", r.entry.name, r.score, desc);
    }

    println!("\n  {} result(s).", results.len());
    Ok(())
}

async fn cmd_list(category: Option<&str>) -> anyhow::Result<()> {
    check_staleness();
    let sources = sync::effective_sources();
    let index = cache::load_or_build_index(&sources)?;

    let entries: Vec<&cache::IndexEntry> = match category {
        Some(cat) => search::filter_by_category(&index.entries, cat),
        None => index.entries.iter().collect(),
    };

    if entries.is_empty() {
        println!("No agents in registry.");
        if !sync::sources_dir().is_dir() {
            println!("Run `armadai registry sync` to fetch the registry.");
        }
        return Ok(());
    }

    // Compute column widths
    let name_w = entries
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let cat_w = entries
        .iter()
        .map(|e| e.category.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(8)
        .max(8);

    println!("  {:<name_w$}  {:<cat_w$}  DESCRIPTION", "NAME", "CATEGORY",);
    println!(
        "  {:<name_w$}  {:<cat_w$}  -----------",
        "-".repeat(name_w),
        "-".repeat(cat_w),
    );

    for entry in &entries {
        let cat = entry.category.as_deref().unwrap_or("-");
        let desc = entry.description.as_deref().unwrap_or("-");
        // Truncate description to 60 chars
        let desc_display = if desc.len() > 60 {
            format!("{}...", &desc[..57])
        } else {
            desc.to_string()
        };
        println!(
            "  {:<name_w$}  {:<cat_w$}  {}",
            entry.name, cat, desc_display
        );
    }

    println!("\n  {} agent(s) in registry.", entries.len());
    Ok(())
}

async fn cmd_add(agent: &str, force: bool) -> anyhow::Result<()> {
    check_staleness();
    let sources = sync::effective_sources();
    let index = cache::load_or_build_index(&sources)?;

    // Find the agent in the index by name or path
    let entry = index
        .entries
        .iter()
        .find(|e| e.name == agent || e.path == agent)
        .ok_or_else(|| not_found_error(agent, &index))?;

    println!("Converting {} ...", entry.name);
    let dst = convert::import_to_library(&entry.source, &entry.path, force)?;
    println!("Installed: {}", dst.display());
    println!("\nAgent '{}' added to your library.", entry.name);
    Ok(())
}

async fn cmd_info(agent: &str) -> anyhow::Result<()> {
    check_staleness();
    let sources = sync::effective_sources();
    let index = cache::load_or_build_index(&sources)?;

    let entry = index
        .entries
        .iter()
        .find(|e| e.name == agent || e.path == agent)
        .ok_or_else(|| not_found_error(agent, &index))?;

    println!("Name:        {}", entry.name);
    println!("Path:        {}", entry.path);
    if !entry.source.is_empty() {
        println!("Source:      {}", entry.source);
    }
    if let Some(ref cat) = entry.category {
        println!("Category:    {cat}");
    }
    if let Some(ref desc) = entry.description {
        println!("Description: {desc}");
    }
    if !entry.tags.is_empty() {
        println!("Tags:        [{}]", entry.tags.join(", "));
    }

    // Show the raw content
    let repo = sync::dir_for_key(&entry.source);
    let src = repo.join(&entry.path);
    if src.is_file() {
        println!("\n--- Content ---");
        let content = std::fs::read_to_string(&src)?;
        // Print first 40 lines max
        for (i, line) in content.lines().enumerate() {
            if i >= 40 {
                println!(
                    "  ... (truncated, {} more lines)",
                    content.lines().count() - 40
                );
                break;
            }
            println!("  {line}");
        }
    }

    Ok(())
}

/// Build an actionable "agent not found" error for `registry add`/`info`.
///
/// When the index is empty — which is also what an ignored legacy cache
/// resolves to (see `cache::load_or_build_index`) — this points the user at
/// `registry sync` instead of the generic "not found" message, which would
/// be confusing when the real cause is "there is no registry data at all
/// yet" rather than "this specific agent doesn't exist".
fn not_found_error(agent: &str, index: &cache::Index) -> anyhow::Error {
    if index.entries.is_empty() {
        anyhow::anyhow!(
            "Registry is empty (no data synced yet, or the cache is from an older ArmadAI \
             version). Run `armadai registry sync`, then retry."
        )
    } else {
        anyhow::anyhow!(
            "Agent '{agent}' not found in registry. Try `armadai registry search {agent}`"
        )
    }
}

/// Print a hint if the registry cache is stale or from an older ArmadAI
/// version.
fn check_staleness() {
    if cache::has_legacy_cache() {
        eprintln!(
            "hint: registry cache is from an older ArmadAI version and will be ignored. Run `armadai registry sync` to refresh."
        );
        return;
    }

    if sync::is_stale(7) && sync::sources_dir().is_dir() {
        eprintln!("hint: registry may be outdated. Run `armadai registry sync` to refresh.");
    }
}

/// List all registry sources with their origins (default/user/project).
async fn sources_list() -> anyhow::Result<()> {
    let user = load_user_registries();
    let project = find_project_config()
        .map(|(_, cfg)| cfg)
        .and_then(|cfg| cfg.registries);

    // Agents
    println!("Agents:");
    println!("  [default] {}", sync::DEFAULT_REGISTRY_URL);
    for source in &user.agents {
        println!("  [user]    {}", source.url);
    }
    if let Some(ref proj) = project {
        for source in &proj.agents {
            println!("  [project] {}", source.url);
        }
    }

    // Skills
    println!("\nSkills:");
    for default_url in crate::skills_registry::sync::default_sources() {
        println!("  [default] {}", default_url);
    }
    for source in &user.skills {
        println!("  [user]    {}", source.url);
    }
    if let Some(ref proj) = project {
        for source in &proj.skills {
            println!("  [project] {}", source.url);
        }
    }

    // Models
    println!("\nModels:");
    println!(
        "  [default] {}",
        crate::model_registry::fetch::MODELS_DEV_URL
    );
    for source in &user.models {
        println!("  [user]    {}", source.url);
    }
    if let Some(ref proj) = project {
        for source in &proj.models {
            println!("  [project] {}", source.url);
        }
    }

    // Starters (no built-in default registry — user/project sources only)
    println!("\nStarters:");
    for source in &user.starters {
        println!("  [user]    {}", source.url);
    }
    if let Some(ref proj) = project {
        for source in &proj.starters {
            println!("  [project] {}", source.url);
        }
    }

    Ok(())
}

/// Add a custom registry source to user config (idempotent).
async fn sources_add(kind: SourceKind, url: &str) -> anyhow::Result<()> {
    let mut config = load_user_registries();
    let registry_kind: RegistryKind = kind.into();

    let sources = match registry_kind {
        RegistryKind::Agents => &mut config.agents,
        RegistryKind::Skills => &mut config.skills,
        RegistryKind::Models => &mut config.models,
        RegistryKind::Starters => &mut config.starters,
    };

    // Check if already present (idempotent)
    if sources.iter().any(|s| s.url == url) {
        println!("Source already registered: {url}");
        return Ok(());
    }

    sources.push(RegistrySource {
        url: url.to_string(),
        kind: None,
    });

    save_registries_config(&config)?;
    println!("Added {} registry source: {url}", kind_name(registry_kind));
    println!("  Saved to {}", registries_config_path().display());

    Ok(())
}

/// Remove a custom registry source from user config.
async fn sources_remove(kind: SourceKind, url: &str) -> anyhow::Result<()> {
    let mut config = load_user_registries();
    let registry_kind: RegistryKind = kind.into();

    let sources = match registry_kind {
        RegistryKind::Agents => &mut config.agents,
        RegistryKind::Skills => &mut config.skills,
        RegistryKind::Models => &mut config.models,
        RegistryKind::Starters => &mut config.starters,
    };

    let before = sources.len();
    sources.retain(|s| s.url != url);

    if sources.len() == before {
        println!("Source not found in config: {url}");
        return Ok(());
    }

    save_registries_config(&config)?;
    println!(
        "Removed {} registry source: {url}",
        kind_name(registry_kind)
    );
    println!("  Saved to {}", registries_config_path().display());

    Ok(())
}

/// Save the registries config to disk, creating parent directory if needed.
fn save_registries_config(config: &RegistriesConfig) -> anyhow::Result<()> {
    let path = registries_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let yaml = serde_yaml_ng::to_string(config)?;
    std::fs::write(&path, yaml)?;
    Ok(())
}

/// Human-readable name for a registry kind.
fn kind_name(kind: RegistryKind) -> &'static str {
    match kind {
        RegistryKind::Agents => "agents",
        RegistryKind::Skills => "skills",
        RegistryKind::Models => "models",
        RegistryKind::Starters => "starters",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_source_kind_starters_maps_to_registry_kind() {
        assert_eq!(
            RegistryKind::from(SourceKind::Starters),
            crate::core::registries::RegistryKind::Starters
        );
    }

    #[test]
    fn not_found_error_on_empty_index_points_to_sync_not_search() {
        let err = not_found_error("some-agent", &cache::Index::default());
        let msg = err.to_string();
        assert!(
            msg.contains("registry sync"),
            "expected an actionable sync hint, got: {msg}"
        );
        assert!(
            !msg.contains("search"),
            "suggesting a search on an empty registry is not actionable, got: {msg}"
        );
    }

    #[test]
    fn not_found_error_on_non_empty_index_suggests_search() {
        let index = cache::Index {
            entries: vec![cache::IndexEntry {
                path: "agents/other.md".to_string(),
                name: "other".to_string(),
                description: None,
                tags: vec![],
                category: None,
                source: "github/awesome-copilot-abc12345".to_string(),
            }],
        };
        let err = not_found_error("missing-agent", &index);
        let msg = err.to_string();
        assert!(msg.contains("missing-agent"));
        assert!(msg.contains("registry search"));
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::await_holding_lock)]
    async fn sources_add_and_load() {
        use tempfile::tempdir;

        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();

        let temp = tempdir().unwrap();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", temp.path());
        }

        let url = "https://custom.example.com/agents.git";
        sources_add(SourceKind::Agents, url).await.unwrap();

        let loaded = load_user_registries();
        assert_eq!(loaded.agents.len(), 1);
        assert_eq!(loaded.agents[0].url, url);

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::await_holding_lock)]
    async fn sources_remove_existing() {
        use tempfile::tempdir;

        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();

        let temp = tempdir().unwrap();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", temp.path());
        }

        let url = "https://custom.example.com/agents.git";
        sources_add(SourceKind::Agents, url).await.unwrap();
        sources_remove(SourceKind::Agents, url).await.unwrap();

        let loaded = load_user_registries();
        assert!(loaded.agents.is_empty());

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::await_holding_lock)]
    async fn sources_add_is_idempotent() {
        use tempfile::tempdir;

        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();

        let temp = tempdir().unwrap();
        // SAFETY: serialised via ENV_MUTEX; restored at end of test.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", temp.path());
        }

        let url = "https://custom.example.com/agents.git";
        sources_add(SourceKind::Agents, url).await.unwrap();
        sources_add(SourceKind::Agents, url).await.unwrap();

        let loaded = load_user_registries();
        assert_eq!(loaded.agents.len(), 1, "duplicate add should be idempotent");

        match orig {
            Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
        }
    }
}
