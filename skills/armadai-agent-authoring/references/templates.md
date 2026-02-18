# Built-in Agent Templates

ArmadAI ships with 12 templates used by `armadai new` to scaffold new agents. Each template uses placeholders: `{{name}}`, `{{description}}`, `{{stack}}`, `{{tags}}`.

## Template Catalog

### `basic`

General-purpose agent template. Good starting point for any agent.

- Provider: `anthropic`
- Temperature: `0.7`
- Sections: Metadata, System Prompt, Instructions, Output Format

### `dev-review`

Code review specialist. Analyzes code for bugs, security issues, and style.

- Provider: `anthropic`
- Temperature: `0.3`
- Tags: `[review, quality]`
- Sections: Metadata, System Prompt, Instructions, Output Format

### `dev-test`

Test writing agent. Generates unit tests for provided code.

- Provider: `anthropic`
- Temperature: `0.5`
- Tags: `[test, quality]`
- Sections: Metadata, System Prompt, Instructions, Output Format

### `debug`

Debugging specialist. Analyzes error traces and proposes fixes.

- Provider: `anthropic`
- Temperature: `0.3`
- Tags: `[debug, fix]`
- Sections: Metadata, System Prompt, Instructions, Output Format

### `planning`

Architecture and planning agent. Produces structured plans and design documents.

- Provider: `anthropic`
- Temperature: `0.5`
- Tags: `[planning, architecture]`
- Sections: Metadata, System Prompt, Instructions, Output Format

### `security-review`

Security audit specialist. Scans code for OWASP Top 10 and common vulnerabilities.

- Provider: `anthropic`
- Temperature: `0.2`
- Tags: `[security, review]`
- Sections: Metadata, System Prompt, Instructions, Output Format

### `tdd-red`

TDD Red phase — writes failing tests for a feature specification.

- Provider: `anthropic`
- Temperature: `0.4`
- Tags: `[tdd, test]`
- Pipeline: chains to `tdd-green`

### `tdd-green`

TDD Green phase — writes minimal implementation to pass the failing tests.

- Provider: `anthropic`
- Temperature: `0.4`
- Tags: `[tdd, implementation]`
- Pipeline: chains to `tdd-refactor`

### `tdd-refactor`

TDD Refactor phase — refactors the green implementation for quality and maintainability.

- Provider: `anthropic`
- Temperature: `0.5`
- Tags: `[tdd, refactor]`

### `tech-debt`

Technical debt analysis agent. Identifies and prioritizes tech debt items.

- Provider: `anthropic`
- Temperature: `0.4`
- Tags: `[tech-debt, analysis]`
- Sections: Metadata, System Prompt, Instructions, Output Format

### `tech-writer`

Technical documentation writer. Produces clear, structured documentation.

- Provider: `anthropic`
- Temperature: `0.6`
- Tags: `[docs, writing]`
- Sections: Metadata, System Prompt, Instructions, Output Format

### `cli-generic`

CLI tool wrapper template. For agents that execute external commands.

- Provider: `cli`
- Tags: `[cli, tool]`
- Fields: `command`, `args`, `timeout`

## Usage

Create a new agent from a template:

```bash
# Interactive mode (prompts for all fields)
armadai new -i

# Quick mode with template
armadai new --name my-reviewer --template dev-review

# With stack specification
armadai new --name rust-reviewer --template dev-review --stack rust
```

## Placeholder Reference

| Placeholder | Description | Example |
|---|---|---|
| `{{name}}` | Agent name (kebab-case) | `code-reviewer` |
| `{{description}}` | One-line description | `Reviews Rust code for quality` |
| `{{stack}}` | Technology stack | `rust` |
| `{{tags}}` | Comma-separated tags | `review, quality` |
