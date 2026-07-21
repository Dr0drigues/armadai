mod audit;
mod cli;
mod core;
mod linker;
mod model_registry;
mod parser;
#[allow(dead_code)]
mod providers;
mod registry;
#[allow(dead_code)]
mod secrets;
#[allow(dead_code)]
mod shell;
mod skills_registry;
// Not yet wired outside tests: `core/starter.rs` integration (B2 Lot B,
// Task 3) is a follow-up task that consumes this module's public API.
#[allow(dead_code)]
mod starters_registry;
#[cfg(feature = "storage")]
mod storage;
#[cfg(feature = "tui")]
mod theme;
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
                .add_directive("armadai=info".parse()?),
        )
        .init();

    core::config::check_migration_hint();

    let args = cli::Cli::parse();
    cli::handle(args).await
}
