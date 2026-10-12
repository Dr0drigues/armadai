---
name: rust-conventions
description: Rust coding conventions and best practices
apply_to:
  - code-reviewer
  - test-writer
---
# Rust Conventions

## Style
- Use `snake_case` for functions, methods, variables, and modules
- Use `CamelCase` for types, traits, and enum variants
- Use `SCREAMING_SNAKE_CASE` for constants and statics
- Prefer `impl Trait` over `dyn Trait` when possible

## Error Handling
- Use `anyhow::Result` for application code, `thiserror` for library errors
- Avoid `.unwrap()` in production code — use `?` or explicit error handling
- Provide context with `.context()` or `.with_context()`

## Structure
- Keep functions short and focused (< 50 lines)
- Prefer iterators over manual loops
- Use `#[must_use]` on functions that return values that should not be ignored
- Derive `Debug` and `Clone` on public types

## Testing
- Place unit tests in a `#[cfg(test)] mod tests` block in the same file
- Use `tempfile::tempdir()` for filesystem tests
- Name tests descriptively: `test_<function>_<scenario>`
- Test both happy paths and error conditions
