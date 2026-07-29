mod audit;
// `watch` (the only consumer of `drive_session`/`Mapper` etc.) is gated behind
// `tui`; without it, most of `claude_adapter` would be flagged dead code even
// though `register_from_stdin` (used unconditionally by
// `__claude-register-session`) stays live. Only suppress the lint when `tui`
// is off, so dead-code detection stays active in `tui` builds (all 3 CI
// clippy combos include `tui`).
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
mod claude_adapter;
mod cli;
#[cfg(feature = "storage")]
mod db;
#[cfg(feature = "storage")]
mod es_log;
mod linker;
mod logging;
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
            // Logs go to stderr, never stdout: stdout is reserved for program
            // output (`--json` RunEvents, human-readable results, and the
            // Claude Code hook contract's "nothing on stdout" requirement for
            // `__claude-register-session`). The default fmt layer writes to
            // stdout, so this must be explicit.
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .init();
        logging::install(reload_handle);
    }

    armadai_core::config::check_migration_hint();

    let args = cli::Cli::parse();
    cli::handle(args).await
}
