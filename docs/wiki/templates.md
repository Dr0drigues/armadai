# Templates

Templates are Markdown files in the `templates/` directory that serve as starting points for new agents.

## Available Templates

### basic

General-purpose agent with Anthropic provider.

```bash
swarm new my-agent --template basic --description "what this agent does"
```

### dev-review

Code review specialist. Produces structured reviews with severity levels.

```bash
swarm new rust-reviewer --template dev-review --stack rust
```

### dev-test

Test generation specialist. Writes unit tests for given code.

```bash
swarm new test-gen --template dev-test --stack typescript
```

### cli-generic

Wrapper for any CLI tool (claude, aider, custom scripts).

```bash
swarm new my-tool --template cli-generic
```

Then edit `agents/my-tool.md` to set the `command` and `args` fields.

### planning

Sprint and project planning agent. Produces structured plans.

```bash
swarm new sprint-planner --template planning --stack rust
```

### security-review

Security audit specialist. Identifies vulnerabilities (OWASP, CVE).

```bash
swarm new sec-reviewer --template security-review --stack typescript
```

### debug

Debugging assistant with systematic root cause analysis.

```bash
swarm new my-debugger --template debug --stack rust
```

### tech-debt

Technical debt analyzer. Identifies and prioritizes refactoring opportunities.

```bash
swarm new debt-scanner --template tech-debt --stack java
```

### tdd-red / tdd-green / tdd-refactor

TDD cycle agents — use them in a pipeline:

```bash
swarm new failing-tests --template tdd-red --stack rust
swarm new make-pass --template tdd-green --stack rust
swarm new clean-up --template tdd-refactor --stack rust
```

### tech-writer

Documentation writer. Produces clear, structured docs.

```bash
swarm new doc-writer --template tech-writer --stack rust
```

## Placeholders

Templates use `{{placeholder}}` syntax for customizable values:

| Placeholder | CLI Flag | Description |
|---|---|---|
| `{{name}}` | positional `<name>` | Agent display name (auto title-cased from the slug) |
| `{{description}}` | `--description` / `-d` | Agent description for the system prompt |
| `{{stack}}` | `--stack` / `-s` | Tech stack (rust, typescript, java, etc.) |
| `{{command}}` | — | CLI command (edit manually) |
| `{{args}}` | — | CLI arguments (edit manually) |
| `{{tags}}` | — | Agent tags (edit manually) |

Placeholders not replaced by CLI flags remain in the output file. The command lists them so you know what to fill in manually.

## Creating Custom Templates

Add a `.md` file to the `templates/` directory following the [agent format](agent-format.md):

```markdown
# {{name}}

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.5
- tags: [{{tags}}]
- stacks: [{{stack}}]

## System Prompt

You are an expert at {{description}} for {{stack}} projects.

## Instructions

1. Analyze the input
2. Apply domain expertise
3. Produce structured output

## Output Format

Clear, actionable response with examples.
```

Use any `{{placeholder}}` name — only `{{name}}`, `{{description}}`, and `{{stack}}` are auto-replaced by CLI flags. Others are left for manual editing.
