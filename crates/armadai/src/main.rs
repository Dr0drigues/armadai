mod claude_adapter;
mod cli;
mod linker;
mod registry;
mod skills_registry;
mod starters_registry;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    {
        use tracing_subscriber::prelude::*;

        let filter = tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("armadai=info".parse()?);
        tracing_subscriber::registry()
            .with(filter)
            // Logs go to stderr, never stdout: stdout is reserved for program
            // output (`--json` RunEvents, human-readable results, and the
            // Claude Code hook contract's "nothing on stdout" requirement for
            // `__claude-policy-gate`). The default fmt layer writes to
            // stdout, so this must be explicit.
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .init();
    }

    armadai_core::config::check_migration_hint();

    let args = cli::Cli::parse();
    cli::handle(args).await
}
