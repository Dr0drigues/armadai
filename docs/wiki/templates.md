# Templates

Templates are Markdown files that serve as starting points for new agents, used by
`armadai new`.

## Where templates are read from

The templates described below are **embedded in the binary**: `armadai new` works out of the box,
with no setup and no templates directory.

A template file with the same name in a templates directory **overrides** the embedded one, and a
file with a new name adds a template of your own. The directory is `templates/` in the current
directory if it exists, otherwise `~/.config/armadai/templates/`. Resolution order for
`--template <name>`:

1. `<templates dir>/<name>.md` (project, or user directory when the project has none)
2. the template embedded in the binary

To customise a built-in template, copy its source from `crates/armadai/templates/` into one of
those directories and edit it.

## Available Templates

| Template | Purpose |
|---|---|
| `basic` | General-purpose agent |
| `dev-review` | Code review specialist; structured reviews with severity levels |
| `dev-test` | Test generation specialist |
| `cli-generic` | Agent wrapping a command-line tool (`command`/`args` fields — inert since the execution engine was removed) |
| `planning` | Sprint and project planning |
| `security-review` | Security review (OWASP, CVE) |
| `debug` | Debugging assistant with systematic root cause analysis |
| `tech-debt` | Technical debt analyzer |
| `tdd-red` / `tdd-green` / `tdd-refactor` | The three steps of a TDD cycle |
| `tech-writer` | Documentation writer |

```bash
armadai new rust-reviewer --template dev-review --stack rust
armadai new sec-reviewer --template security-review --stack typescript -d "audit OWASP top 10"
armadai new -i                     # guided wizard
```

The new agent is written to `<name>.md` in the first agents directory that exists among
`.armadai/agents/` and `agents/`, otherwise in your user library (`~/.config/armadai/agents/`).
Reference it from the project config to have
`armadai link` compile it (see [Getting Started](getting-started.md#writing-your-own-agent)).

## Placeholders

Templates use `{{placeholder}}` syntax for customizable values:

| Placeholder | Filled by | Description |
|---|---|---|
| `{{name}}` | positional `<name>` | Agent display name (title-cased from the slug) |
| `{{description}}` | `--description` / `-d` | Agent description |
| `{{stack}}` | `--stack` / `-s` | Tech stack (rust, typescript, java, etc.) |
| `{{model}}` | always | Set to `latest:pro` |
| `{{command}}`, `{{args}}`, `{{tags}}` | — | Left for manual editing |

Placeholders not replaced remain in the output file; the command lists them so you know what to
fill in manually.

## Creating Custom Templates

Add a `.md` file to your templates directory following the [agent format](agent-format.md):

```markdown
# {{name}}

## Metadata
- provider: claude
- model: {{model}}
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

Use any `{{placeholder}}` name — only `{{name}}`, `{{description}}`, `{{stack}}` and `{{model}}`
are replaced automatically. Others are left for manual editing.
