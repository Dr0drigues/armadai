# Example: Rust Web Team

A complete example of an ArmadAI project for reviewing, testing, documenting, and improving Rust code.

## Fleet composition

| Agent | Role | Temperature |
|-------|------|-------------|
| `rust-reviewer` | Code review (bugs, safety, idioms) | 0.3 |
| `rust-test-writer` | Generate unit tests | 0.3 |
| `security-auditor` | Security vulnerability scanning | 0.2 |
| `doc-writer` | Generate `///` doc comments | 0.5 |
| `refactor-advisor` | Identify code smells and propose refactoring | 0.5 |

## Setup

```bash
# From the ArmadAI root directory
cd examples/demo-rust-team
```

The `.armadai/config.yaml` file declares the project agents. Agent files live in `.armadai/agents/`.

## Usage

ArmadAI does not run the agents: it compiles them into the native files of a coding assistant,
which then runs them.

```bash
# Check the agent definitions
armadai validate

# Generate the Claude Code subagents (.claude/agents/)
armadai link --target claude
```

Then open the project in Claude Code (`claude`) and ask, for example, for a review of
`sample-code/user_service.rs`: the generated agents (`rust-reviewer`, `security-auditor`, ...)
are available as Claude Code subagents.

### Check for deprecated models

```bash
# Check if any agent uses a deprecated model
armadai models check

# Auto-update deprecated models in-place
armadai models update
```

## Other targets

```bash
# Preview without writing
armadai link --target codex --dry-run
```

The supported targets are `claude`, `codex`, `opencode` and `copilot`.

## Expected output

Each agent produces structured output in French (system prompts request this). For example, `rust-reviewer` will identify issues like:

- `.unwrap()` calls that could panic under contention
- `u.active == true` instead of idiomatic `u.active`
- Missing `Default` impl for `UserService`
- Email validation too simplistic (just checks for `@`)

The `security-auditor` might flag:

- No input sanitization on `bulk_import` (CSV injection potential)
- Mutex poisoning not handled (`.unwrap()` on lock)
- No rate limiting or access control patterns

## Customizing

To add a new agent to the fleet:

```bash
# Interactive creation
armadai new -i

# Then add it to .armadai/config.yaml agents list
```

Or copy an existing agent and modify it:

```bash
cp .armadai/agents/rust-reviewer.md .armadai/agents/perf-analyzer.md
# Edit the new file
```
