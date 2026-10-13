# Skill examples

## Conventions shared by a team

`.armadai/skills/rust-conventions/SKILL.md`:

```markdown
---
name: rust-conventions
description: Rust 2024 conventions of this project (errors, tests, clippy); use when writing or reviewing Rust
---

# Rust conventions

- Errors: `anyhow` in binaries, `thiserror` in libraries; never `unwrap` outside tests.
- Tests: unit tests next to the code, integration tests under `tests/`, `tempfile` for files.
- `cargo clippy --all-targets -- -D warnings` passes.
```

Each agent of the team lists it: `skills: [rust-conventions]`.

## A procedure with a script

```
.armadai/skills/release-notes/
  SKILL.md
  scripts/changes.sh        # git log --oneline <last-tag>..HEAD
  references/style.md       # sections, tone, what to leave out
```

```markdown
---
name: release-notes
description: Writes release notes from the git log since the last tag; use when preparing a release
metadata:
  version: "1.0"
---

# Release notes

1. Run `scripts/changes.sh` for the commits since the last tag.
2. Group them as `references/style.md` says (Added, Changed, Fixed, Removed).
3. Write the notes; never invent a change the log does not show.
```
