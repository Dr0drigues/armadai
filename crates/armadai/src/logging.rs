//! Runtime control of the global `tracing` subscriber's filter.
//!
//! `main.rs` installs a `reload`-able `EnvFilter` layer and stores the reload
//! handle here via [`install`]. This lets the live orchestration TUI
//! ([`crate::shell::run_view`]) temporarily silence all `tracing` output
//! while it owns the terminal (alternate screen + raw mode) — logging to
//! stderr during that window corrupts the rendered frame — and restore the
//! original filter on exit.
//!
//! Not feature-gated: this module always compiles (it's `mod`-declared from
//! `main.rs` unconditionally), even though only `tui`-gated code calls
//! [`suppress`]/[`restore`]. Headless commands never call either, so their
//! logging behavior is unchanged.

use std::sync::OnceLock;

use tracing_subscriber::{EnvFilter, Registry, reload};

/// Reload handle for the `EnvFilter` layer installed in `main.rs`.
static RELOAD_HANDLE: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// Store the reload handle created in `main.rs`. Must be called at most once,
/// before any call to [`suppress`]/[`restore`] can have an effect.
pub fn install(handle: reload::Handle<EnvFilter, Registry>) {
    // Ignore a second `install`: only the first handle registered (from
    // `main.rs` startup) is meaningful; there is no legitimate call site that
    // would install twice.
    let _ = RELOAD_HANDLE.set(handle);
}

/// Silence all `tracing` output. No-op if [`install`] was never called (e.g.
/// in unit tests that don't run through `main`).
pub fn suppress() {
    if let Some(handle) = RELOAD_HANDLE.get() {
        let _ = handle.reload(EnvFilter::new("off"));
    }
}

/// Restore the default filter (`armadai=info`, or whatever `RUST_LOG`
/// specifies) after a [`suppress`] call. No-op if [`install`] was never
/// called.
pub fn restore() {
    if let Some(handle) = RELOAD_HANDLE.get() {
        let filter = EnvFilter::from_default_env().add_directive(
            "armadai=info"
                .parse()
                .expect("static directive always parses"),
        );
        let _ = handle.reload(filter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppress_and_restore_are_noop_without_install() {
        // `main()` never runs in the test binary, so `install` was never
        // called here — both must degrade to no-ops rather than panicking,
        // since headless commands and most tests never touch this module at
        // all.
        suppress();
        restore();
    }
}
