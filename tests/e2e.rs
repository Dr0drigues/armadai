//! Entry point for the `e2e` integration test binary.
//!
//! Declares the `e2e` module tree rooted at `tests/e2e/mod.rs`, so that individual
//! sub-modules (e.g. `e2e::case`) live under `tests/e2e/` while still being reachable
//! as a single test target discovered by Cargo (`tests/*.rs`).
#[path = "e2e/mod.rs"]
mod e2e;
