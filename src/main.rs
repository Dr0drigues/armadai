mod audit;
mod cli;
mod core;
#[cfg(feature = "storage")]
mod db;
#[cfg(feature = "storage")]
mod es_log;
mod linker;
mod logging;
mod model_registry;
#[allow(dead_code)]
mod providers;
mod registry;
#[allow(dead_code)]
mod shell;
mod skills_registry;
mod starters_registry;
#[cfg(feature = "tui")]
mod theme;
#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "web")]
mod web;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    {
        use tracing_subscriber::prelude::*;

        let filter = tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("armadai=info".parse()?);
        let (filter_layer, reload_handle) = tracing_subscriber::reload::Layer::new(filter);
        tracing_subscriber::registry()
            .with(filter_layer)
            .with(tracing_subscriber::fmt::layer())
            .init();
        logging::install(reload_handle);
    }

    core::config::check_migration_hint();

    let args = cli::Cli::parse();
    cli::handle(args).await
}
