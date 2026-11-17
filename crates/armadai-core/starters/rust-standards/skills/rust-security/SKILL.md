---
name: rust-security
description: Security rules for Rust code (unsafe, untrusted input, files and paths, processes, secrets, cryptography, dependencies). Load before writing or reviewing Rust code that reads input, touches files, runs processes or uses the network.
metadata:
  armadai-slot: security
---
# Rust security

## unsafe
- A crate that needs no `unsafe` declares `#![forbid(unsafe_code)]`.
- Otherwise keep `unsafe` in one module per feature. Each block carries a `// SAFETY:` comment stating the invariant it relies on. Enable `clippy::undocumented_unsafe_blocks`.
- A signal handler only does async-signal-safe work.

## Untrusted input
- Validate at the boundary: parse into typed values, refuse instead of guessing. With serde, use `#[serde(deny_unknown_fields)]` for files you own.
- Bound everything an input controls: lengths, counts, nesting depth, bytes read (`Read::take`).
- Arithmetic on input uses `checked_*` or `saturating_*`. Convert integers with `try_from`, never with an `as` that truncates.
- No indexing or slicing with an unchecked value: use `get`.

## Files and paths
- A path built from input must not be absolute nor contain `..`. Check it before joining it to a base directory.
- Do not follow symbolic links by accident: inspect with `symlink_metadata` before writing.
- Write a file atomically: a temporary file in the same directory, then `rename`. Use `create_new(true)` when a file must not be overwritten.
- Temporary files and folders through `tempfile`, never a fixed name in a shared directory.

## Processes
- Run programs with `std::process::Command` and separate arguments, never a shell string built from input (`sh -c`).
- Put `--` (or the tool's own end-of-options marker) before arguments that come from input.
- Pass a child process only the environment it needs.

## Secrets
- Never log, print or put in an error message a secret, a token or a URL carrying credentials. Mask them.
- A type holding a secret does not derive `Debug`: write one that masks it, or use a crate such as `secrecy`.
- Compare secrets in constant time (`subtle`).

## Cryptography and randomness
- No home-made cryptography: use maintained, audited crates.
- Secrets and tokens come from the operating system's generator (`getrandom`, or `rand`'s OS generator), never a seeded or fast generator.

## Network
- Keep TLS certificate verification on.
- Every request has a timeout.

## Dependencies
- Run `cargo audit` (RustSec advisories) in CI. `cargo deny` also checks licenses and sources when the project uses it.
- Before adding a crate, look at its maintenance, its `unsafe` and its `build.rs`.

## Concurrency
- Never hold a lock across an `.await`.
- No `static mut`: use atomics, `OnceLock` or a `Mutex`.
