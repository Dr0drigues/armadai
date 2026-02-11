mod cli;
mod core;
mod parser;
#[allow(dead_code)]
mod providers;
#[allow(dead_code)]
mod secrets;
#[cfg(feature = "storage")]
#[allow(dead_code)]
mod storage;
#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "web")]
mod web;

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
