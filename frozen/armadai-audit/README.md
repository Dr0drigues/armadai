# armadai-audit (frozen)

The engine behind the former `armadai audit` command: static analysis of
native agentic configs (`.claude/agents/`, `.claude/skills/`, `CLAUDE.md`),
observed-usage rules U01-U04 read from Claude Code transcripts, the
`--global` scope, Markdown/HTML reports and the `--propose` pack generator.

## Why it is frozen

Step 0 of the v1 cleanup removes from the `armadai` binary every surface that
is not part of the declarative core. `audit` is kept rather than deleted, but
outside the product: this crate is **not a member of the root workspace**
(it carries its own `[workspace]` table and is listed in the root `exclude`),
so neither CI nor `cargo test` at the repository root compiles it.

It still depends on `armadai-core` and `armadai-providers` (model catalogue
only, without the `api` feature) by path. The handful of items it used from
the `armadai` binary crate are frozen copies under `src/vendored/`, each
naming its origin (commit `c486959`).

Known gap: `--deep` is still parsed, but the crate has no execution provider,
so it fails with
`--deep needs an execution provider and is unavailable in the frozen build`.

## Build and test

```bash
cd frozen/armadai-audit
cargo test
cargo run -- --help          # armadai-audit [PATH] [--global] [--report FILE] ...
```

Because nothing builds it automatically, a change to `armadai-core` or
`armadai-providers` can break this crate silently. That is accepted: it is
frozen, not maintained.

## Future

The audit is to be rebuilt on the declarative model after v1 (see
`docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md`,
section 2.3). Until then this crate is the reference implementation to start
from.
