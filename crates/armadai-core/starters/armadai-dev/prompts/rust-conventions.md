---
name: rust-conventions
description: Rust 2024 edition coding conventions, clippy strict mode, and feature flag best practices
apply_to:
  - dev-lead
  - core-specialist
  - provider-specialist
  - cli-specialist
  - ui-specialist
  - qa-specialist
  - release-manager
---
# Rust Conventions

## Edition and Toolchain
- Use Rust 2024 edition
- Run `cargo clippy -- -D warnings` to enforce strict lints
- Run `cargo fmt -- --check` in CI to ensure consistent formatting
- Test with both stable and MSRV (minimum supported Rust version)

## Style
- Use `snake_case` for functions, methods, variables, and modules
- Use `CamelCase` for types, traits, and enum variants
- Use `SCREAMING_SNAKE_CASE` for constants and statics
- Prefer `impl Trait` over `dyn Trait` when possible
- Derive `Debug` and `Clone` on public types

## Error Handling
- Use `anyhow::Result` for application code, `thiserror` for library errors
- Avoid `.unwrap()` in production code — use `?` or explicit error handling
- Provide context with `.context()` or `.with_context()`
- Propagate errors instead of panicking
- Document error conditions in public APIs

## Feature Flags
- Gate heavy optional dependencies behind feature flags
- Use `#[cfg(feature = "...")]` for conditional compilation
- Ensure clippy and tests pass with all feature flag combinations
- Document feature flags in README and Cargo.toml
- Provide graceful fallbacks when optional features are disabled

## Structure
- Keep functions short and focused (< 50 lines)
- Prefer iterators over manual loops
- Use `#[must_use]` on functions that return values that should not be ignored
- Organize code into modules with clear boundaries
- Separate domain logic from I/O and infrastructure

## Async
- Use `tokio::spawn` for concurrent tasks
- Use `tokio::select!` for multiplexing async operations
- Avoid blocking calls in async contexts — use `spawn_blocking` if needed
- Test async code with `#[tokio::test]`

## Testing
- Place unit tests in a `#[cfg(test)] mod tests` block in the same file
- Use `tempfile::tempdir()` for filesystem tests
- Name tests descriptively: `test_<function>_<scenario>`
- Test both happy paths and error conditions
- Use property-based testing with `proptest` for complex logic
- Ensure tests pass in all feature flag combinations

## Conventional Commits
- Follow conventional commit format: `type(scope): subject`
- Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`
- Keep subject line under 72 characters
- Use imperative mood ("add feature" not "added feature")
- Reference issues/PRs in the body when relevant
