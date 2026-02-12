mod config;
mod costs;
mod fleet;
mod history;
mod init;
mod inspect;
mod list;
pub(crate) mod new;
mod run;
mod up;
mod update;
mod validate;

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "armadai",
    about = "AI agent fleet orchestrator",
    long_about = "AI agent fleet orchestrator — define, manage and run specialized agents from Markdown files.\n\n\
        Each agent is a .md file in agents/ with metadata, system prompt, and optional instructions.\n\
        Supports any LLM provider (Claude, GPT, Gemini) via CLI tools or API.",
    version,
    after_help = "Examples:\n  \
        armadai new my-agent --template dev-review --stack rust\n  \
        armadai run my-agent \"Review this code for bugs\"\n  \
        armadai run --pipe reviewer writer src/main.rs\n  \
        armadai list --tags dev --stack rust\n  \
        armadai tui\n  \
        armadai web --port 8080\n\n\
        Documentation: https://github.com/Dr0drigues/swarm-festai"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run an agent with the given input
    #[command(
        long_about = "Run an agent with the given input.\n\n\
            Loads the agent definition from agents/<name>.md, sends the input to the \
            configured provider, and prints the response. Use --pipe to chain multiple \
            agents sequentially (output of one becomes input of the next).",
        after_help = "Examples:\n  \
            armadai run code-reviewer \"Review this function\"\n  \
            armadai run summarizer @long-document.txt\n  \
            armadai run --pipe reviewer writer src/main.rs"
    )]
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
    #[command(
        long_about = "Create a new agent from a template.\n\n\
            Available templates: basic, dev-review, dev-test, cli-generic, planning, \
            security-review, debug, tech-debt, tdd-red, tdd-green, tdd-refactor, tech-writer.\n\
            The new agent is created at agents/<name>.md.\n\n\
            Use --interactive (-i) for a guided step-by-step creation wizard.",
        after_help = "Examples:\n  \
            armadai new my-assistant\n  \
            armadai new reviewer --template dev-review --stack rust\n  \
            armadai new scanner --template security-review -d \"audit OWASP top 10\"\n  \
            armadai new -i"
    )]
    New {
        /// Agent name (optional in interactive mode)
        name: Option<String>,
        /// Template to use
        #[arg(long, short, default_value = "basic")]
        template: String,
        /// Tech stack (replaces {{stack}} placeholder)
        #[arg(long, short)]
        stack: Option<String>,
        /// Agent description (replaces {{description}} placeholder)
        #[arg(long, short)]
        description: Option<String>,
        /// Interactive creation wizard
        #[arg(long, short = 'i')]
        interactive: bool,
    },
    /// List available agents
    #[command(after_help = "Examples:\n  \
        armadai list\n  \
        armadai list --tags dev review\n  \
        armadai list --stack rust")]
    List {
        /// Filter by tags
        #[arg(long)]
        tags: Option<Vec<String>>,
        /// Filter by stack
        #[arg(long)]
        stack: Option<String>,
    },
    /// Inspect an agent's parsed configuration
    #[command(long_about = "Inspect an agent's parsed configuration.\n\n\
            Displays the fully parsed agent definition: metadata, system prompt, \
            instructions, output format, and pipeline configuration.")]
    Inspect {
        /// Agent name
        agent: String,
    },
    /// Validate agent config without making API calls (dry-run)
    #[command(
        long_about = "Validate agent config without making API calls (dry-run).\n\n\
            Checks that the Markdown file parses correctly and all required fields are present. \
            If no agent name is given, validates all agents in the agents/ directory."
    )]
    Validate {
        /// Agent name (validates all if omitted)
        agent: Option<String>,
    },
    /// View execution history
    #[command(after_help = "Examples:\n  \
        armadai history\n  \
        armadai history --agent code-reviewer\n  \
        armadai history --replay abc123")]
    History {
        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
        /// Replay a specific execution by ID
        #[arg(long)]
        replay: Option<String>,
    },
    /// View cost tracking
    #[command(after_help = "Examples:\n  \
        armadai costs\n  \
        armadai costs --agent code-reviewer\n  \
        armadai costs --from 2025-01-01")]
    Costs {
        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        from: Option<String>,
    },
    /// Manage providers and secrets
    #[command(
        long_about = "Manage providers and secrets.\n\n\
            Configure API keys, view provider status, and manage SOPS + age encryption.",
        after_help = "Examples:\n  \
            armadai config providers\n  \
            armadai config secrets init\n  \
            armadai config secrets rotate"
    )]
    Config {
        #[command(subcommand)]
        action: config::ConfigAction,
    },
    /// Launch the TUI dashboard
    #[cfg(feature = "tui")]
    #[command(long_about = "Launch the TUI dashboard.\n\n\
            Interactive terminal interface for browsing agents, viewing history and costs. \
            Use Tab/Shift+Tab or 1-4 to switch views, j/k to navigate, Enter for details, \
            : or Ctrl+P for command palette, q to quit.")]
    Tui,
    /// Launch the web UI
    #[cfg(feature = "web")]
    #[command(
        long_about = "Launch the web UI.\n\n\
            Starts an HTTP server with a browser-based dashboard for browsing agents, \
            viewing execution history, and tracking costs.",
        after_help = "Examples:\n  \
            armadai web\n  \
            armadai web --port 8080"
    )]
    Web {
        /// Port to listen on
        #[arg(long, short, default_value = "3000")]
        port: u16,
    },
    /// Start infrastructure services (Docker Compose)
    #[command(long_about = "Start infrastructure services (Docker Compose).\n\n\
        Starts SurrealDB and LiteLLM proxy containers defined in docker-compose.yml.")]
    Up,
    /// Stop infrastructure services (Docker Compose)
    #[command(long_about = "Stop infrastructure services (Docker Compose).\n\n\
        Stops and removes the containers started by 'armadai up'.")]
    Down,
    /// Manage agent fleets
    #[command(
        subcommand,
        long_about = "Manage agent fleets.\n\n\
            Create named groups of agents and link them to project directories via armadai.yaml.",
        after_help = "Examples:\n  \
            armadai fleet create my-fleet --all\n  \
            armadai fleet link my-fleet\n  \
            armadai fleet list\n  \
            armadai fleet show my-fleet"
    )]
    Fleet(fleet::FleetAction),
    /// Initialize ArmadAI configuration
    #[command(
        long_about = "Initialize ArmadAI configuration.\n\n\
            Creates ~/.config/armadai/ with default config.yaml, providers.yaml, \
            and subdirectories (agents/, prompts/, skills/, fleets/, registry/).\n\n\
            Use --project to create a minimal armadai.yaml in the current directory.",
        after_help = "Examples:\n  \
            armadai init\n  \
            armadai init --force\n  \
            armadai init --project"
    )]
    Init {
        /// Overwrite existing config files
        #[arg(long)]
        force: bool,
        /// Create a project-local armadai.yaml instead
        #[arg(long)]
        project: bool,
    },
    /// Self-update to the latest release
    #[command(long_about = "Self-update to the latest release.\n\n\
            Downloads the latest binary from GitHub Releases and replaces the current one.")]
    Update,
    /// Generate shell completion scripts
    #[command(after_help = "Examples:\n  \
        armadai completion bash > ~/.local/share/bash-completion/completions/armadai\n  \
        armadai completion zsh > ~/.zfunc/_armadai\n  \
        armadai completion fish > ~/.config/fish/completions/armadai.fish")]
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

pub async fn handle(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Run { agent, input, pipe } => run::execute(agent, input, pipe).await,
        Command::New {
            name,
            template,
            stack,
            description,
            interactive,
        } => new::execute(name, template, stack, description, interactive).await,
        Command::List { tags, stack } => list::execute(tags, stack).await,
        Command::Inspect { agent } => inspect::execute(agent).await,
        Command::Validate { agent } => validate::execute(agent).await,
        Command::History { agent, replay } => history::execute(agent, replay).await,
        Command::Costs { agent, from } => costs::execute(agent, from).await,
        Command::Config { action } => config::execute(action).await,
        #[cfg(feature = "tui")]
        Command::Tui => crate::tui::run().await,
        #[cfg(feature = "web")]
        Command::Web { port } => crate::web::serve(port).await,
        Command::Fleet(action) => fleet::execute(action).await,
        Command::Init { force, project } => init::execute(force, project).await,
        Command::Update => update::execute().await,
        Command::Up => up::start().await,
        Command::Down => up::stop().await,
        Command::Completion { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "armadai",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}
