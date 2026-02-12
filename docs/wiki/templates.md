# Templates

Templates are Markdown files in the `templates/` directory that serve as starting points for new agents.

## Available Templates

### basic

General-purpose agent with Anthropic provider.

```bash
armadai new my-agent --template basic --description "what this agent does"
```

### dev-review

Code review specialist. Produces structured reviews with severity levels.

```bash
armadai new rust-reviewer --template dev-review --stack rust
```

### dev-test

Test generation specialist. Writes unit tests for given code.

```bash
armadai new test-gen --template dev-test --stack typescript
```

### cli-generic

Wrapper for any CLI tool (claude, aider, custom scripts).

```bash
armadai new my-tool --template cli-generic
```

Then edit `agents/my-tool.md` to set the `command` and `args` fields.

### planning

Sprint and project planning agent. Produces structured plans.

```bash
armadai new sprint-planner --template planning --stack rust
```

### security-review

Security audit specialist. Identifies vulnerabilities (OWASP, CVE).

```bash
armadai new sec-reviewer --template security-review --stack typescript
```

### debug

Debugging assistant with systematic root cause analysis.

```bash
armadai new my-debugger --template debug --stack rust
```

### tech-debt

Technical debt analyzer. Identifies and prioritizes refactoring opportunities.

```bash
armadai new debt-scanner --template tech-debt --stack java
```

### tdd-red / tdd-green / tdd-refactor

TDD cycle agents — use them in a pipeline:

```bash
armadai new failing-tests --template tdd-red --stack rust
armadai new make-pass --template tdd-green --stack rust
armadai new clean-up --template tdd-refactor --stack rust
```

### tech-writer

Documentation writer. Produces clear, structured docs.

```bash
armadai new doc-writer --template tech-writer --stack rust
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
