mod config;
mod costs;
mod history;
mod inspect;
mod list;
mod new;
mod run;
mod up;
mod validate;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "swarm", about = "AI agent fleet orchestrator", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run an agent with the given input
    Run {
        /// Agent name (filename without .md)
        agent: String,
        /// Input text or file path (use @file.txt for files)
        input: Option<String>,
        /// Pipeline mode: chain agents sequentially
        #[arg(long, num_args = 1..)]
        pipe: Option<Vec<String>>,
    },
    /// Create a new agent from a template
    New {
        /// Agent name
        name: String,
        /// Template to use
        #[arg(long, short, default_value = "basic")]
        template: String,
        /// Tech stack (replaces {{stack}} placeholder)
        #[arg(long, short)]
        stack: Option<String>,
        /// Agent description (replaces {{description}} placeholder)
        #[arg(long, short)]
        description: Option<String>,
    },
    /// List available agents
    List {
        /// Filter by tags
        #[arg(long)]
        tags: Option<Vec<String>>,
        /// Filter by stack
        #[arg(long)]
        stack: Option<String>,
    },
    /// Inspect an agent's parsed configuration
    Inspect {
        /// Agent name
        agent: String,
    },
    /// Validate agent config without making API calls (dry-run)
    Validate {
        /// Agent name (validates all if omitted)
        agent: Option<String>,
    },
    /// View execution history
    History {
        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
        /// Replay a specific execution by ID
        #[arg(long)]
        replay: Option<String>,
    },
    /// View cost tracking
    Costs {
        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        from: Option<String>,
    },
    /// Manage providers and secrets
    Config {
        #[command(subcommand)]
        action: config::ConfigAction,
    },
    /// Launch the TUI dashboard
    #[cfg(feature = "tui")]
    Tui,
    /// Start infrastructure services (Docker Compose)
    Up,
    /// Stop infrastructure services (Docker Compose)
    Down,
}

pub async fn handle(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Run { agent, input, pipe } => run::execute(agent, input, pipe).await,
        Command::New {
            name,
            template,
            stack,
            description,
        } => new::execute(name, template, stack, description).await,
        Command::List { tags, stack } => list::execute(tags, stack).await,
        Command::Inspect { agent } => inspect::execute(agent).await,
        Command::Validate { agent } => validate::execute(agent).await,
        Command::History { agent, replay } => history::execute(agent, replay).await,
        Command::Costs { agent, from } => costs::execute(agent, from).await,
        Command::Config { action } => config::execute(action).await,
        #[cfg(feature = "tui")]
        Command::Tui => crate::tui::run().await,
        Command::Up => up::start().await,
        Command::Down => up::stop().await,
    }
}
