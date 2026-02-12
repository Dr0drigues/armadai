# Example: Rust Web Team

A complete example of an ArmadAI fleet for reviewing, testing, documenting, and improving Rust code.

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
cd example
```

The `armadai.yaml` file links this directory to the fleet. The `source: ..` path points back to the ArmadAI root where agents are loaded from `example/agents/`.

## Usage

### Single agent

```bash
# Code review
armadai run rust-reviewer @sample-code/user_service.rs

# Generate tests
armadai run rust-test-writer @sample-code/user_service.rs

# Security audit
armadai run security-auditor @sample-code/user_service.rs

# Generate documentation
armadai run doc-writer @sample-code/user_service.rs

# Refactoring suggestions
armadai run refactor-advisor @sample-code/user_service.rs
```

### Pipeline (chained agents)

```bash
# Review then generate tests for the findings
armadai run --pipe rust-reviewer rust-test-writer sample-code/user_service.rs

# Full quality pass: review → security → refactor
armadai run --pipe rust-reviewer security-auditor refactor-advisor @sample-code/user_service.rs
```

### Piping with stdin

```bash
# Pipe code from another command
cat sample-code/user_service.rs | armadai run rust-reviewer

# Pipe a git diff
git diff HEAD~1 | armadai run rust-reviewer
```

## Fleet management

```bash
# List the fleet linked to this directory
armadai fleet list

# Show fleet details with agent status
armadai fleet show rust-web-team

# Validate all agents in the fleet
armadai validate
```

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

# Then add it to armadai.yaml agents list
```

Or copy an existing agent and modify it:

```bash
cp agents/rust-reviewer.md agents/perf-analyzer.md
# Edit the new file
```
