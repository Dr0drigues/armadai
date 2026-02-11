mod cli;
mod core;
mod parser;
mod providers;
mod secrets;
mod storage;
mod tui;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("swarm_festai=info".parse()?),
        )
        .init();

    let args = cli::Cli::parse();
    cli::handle(args).await
}
