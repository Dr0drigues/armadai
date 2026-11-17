---
name: rust-conventions
description: Rust code conventions (formatting, lints, errors, types, tests, documentation, dependencies). Load before writing, changing or reviewing Rust code.
metadata:
  armadai-slot: code-standards
---
# Rust conventions

Follow the project's own rules first; these apply where it says nothing.

## Before a commit
- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets -- -D warnings` passes.
- `cargo test` passes.
- Silence a lint only on the item that needs it, with a comment saying why. Prefer `#[expect(lint)]` over `#[allow(lint)]`: it warns once the lint no longer fires.

## Naming
- `snake_case` for functions, methods, variables and modules. `UpperCamelCase` for types, traits and enum variants. `SCREAMING_SNAKE_CASE` for constants and statics.
- Conversions: `as_` is cheap and borrowed, `to_` is costly or owned, `into_` consumes `self`. A getter has no `get_` prefix.

## Errors
- Return `Result` and propagate with `?`. A library defines its error types (an enum, with `thiserror` if the project uses it). An application may use `anyhow::Result` and add `.context(...)` naming what failed and on what.
- No `unwrap()` outside tests. `expect("…")` only for an invariant the code itself guarantees, and the message states that invariant.
- Never `panic!` on bad input: report it.

## Types and APIs
- Make invalid states unrepresentable: an enum instead of several booleans, a newtype instead of a bare `String` or integer for an id or a unit.
- Take borrowed parameters (`&str`, `&[T]`, `&Path`) and return owned values.
- Keep visibility as small as it can be: private by default, then `pub(crate)`, `pub` only for the real API.
- Derive `Debug` on public types. Derive `Clone`, `PartialEq`, `Eq`, `Default` when they have a meaning.
- Prefer iterators to index loops. Do not `clone()` only to satisfy the borrow checker: change the ownership instead.

## Structure
- One responsibility per module. Keep pure logic apart from I/O, so that it is tested without disk, network or clock.
- Short functions. A function that needs a comment per section is several functions.

## Tests
- Unit tests in `#[cfg(test)] mod tests` beside the code, integration tests in `tests/`.
- Assert the whole value (`assert_eq!` on the complete output), not a fragment that another output would also contain.
- A test must fail when the code it protects is wrong: break that code once, see the test fail, restore it.
- Temporary files through `tempfile`. A test never touches the network nor the user's home.

## Documentation
- `///` on every public item: what it does and what it returns. Add `# Errors` and `# Panics` sections where they apply.
- A comment says why, not what the code already says.

## Dependencies
- Before adding a crate, check the standard library and the crates already in `Cargo.lock`.
- Enable only the features you use (`default-features = false` when it helps).
- Commit `Cargo.lock` for a binary.
