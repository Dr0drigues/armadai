use clap::Subcommand;

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
}

pub async fn execute(action: RegistryAction) -> anyhow::Result<()> {
    match action {
        RegistryAction::Sync => cmd_sync().await,
        RegistryAction::Search { query, category } => cmd_search(&query, category.as_deref()).await,
        RegistryAction::List { category } => cmd_list(category.as_deref()).await,
        RegistryAction::Add { agent, force } => cmd_add(&agent, force).await,
        RegistryAction::Info { agent } => cmd_info(&agent).await,
    }
}

async fn cmd_sync() -> anyhow::Result<()> {
    println!("Syncing community registry...");
    sync::registry_sync(None)?;
    println!("Building search index...");
    let index = cache::build_index()?;
    println!("Indexed {} agent(s).", index.entries.len());
    Ok(())
}

async fn cmd_search(query: &str, category: Option<&str>) -> anyhow::Result<()> {
    check_staleness();
    let index = cache::load_or_build_index()?;

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
    let index = cache::load_or_build_index()?;

    let entries: Vec<&cache::IndexEntry> = match category {
        Some(cat) => search::filter_by_category(&index.entries, cat),
        None => index.entries.iter().collect(),
    };

    if entries.is_empty() {
        println!("No agents in registry.");
        if !sync::repo_dir().join(".git").is_dir() {
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
    let index = cache::load_or_build_index()?;

    // Find the agent in the index by name or path
    let entry = index
        .entries
        .iter()
        .find(|e| e.name == agent || e.path == agent)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Agent '{agent}' not found in registry. Try `armadai registry search {agent}`"
            )
        })?;

    println!("Converting {} ...", entry.name);
    let dst = convert::import_to_library(&entry.path, force)?;
    println!("Installed: {}", dst.display());
    println!("\nAgent '{}' added to your library.", entry.name);
    Ok(())
}

async fn cmd_info(agent: &str) -> anyhow::Result<()> {
    check_staleness();
    let index = cache::load_or_build_index()?;

    let entry = index
        .entries
        .iter()
        .find(|e| e.name == agent || e.path == agent)
        .ok_or_else(|| anyhow::anyhow!("Agent '{agent}' not found in registry."))?;

    println!("Name:        {}", entry.name);
    println!("Path:        {}", entry.path);
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
    let repo = sync::repo_dir();
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

/// Print a hint if the registry is stale (> 7 days old).
fn check_staleness() {
    if sync::is_stale(7) && sync::repo_dir().join(".git").is_dir() {
        eprintln!("hint: registry may be outdated. Run `armadai registry sync` to refresh.");
    }
}
