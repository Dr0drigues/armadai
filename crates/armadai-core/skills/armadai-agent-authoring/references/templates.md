# Built-in agent templates

`armadai new agent <id> --template <name>` writes `.armadai/agents/<id>.md` from a template, then
validates the project (a creation that would introduce an error is undone).

| Template | Default description | Tier |
|---|---|---|
| `basic` (default) | General-purpose assistant | pro |
| `dev-review` | Reviews code changes for bugs, security, performance and style | pro |
| `dev-test` | Writes thorough, maintainable tests for existing code | pro |
| `planning` | Breaks a goal down into an implementation plan with tasks and risks | pro |
| `security-review` | Audits code for security vulnerabilities and proposes fixes | pro |
| `debug` | Finds the root cause of a bug and proposes a minimal fix | pro |
| `tech-debt` | Finds tech debt and plans its remediation | pro |
| `tdd-red` | TDD red phase: writes failing tests that specify the behaviour | pro |
| `tdd-green` | TDD green phase: writes the minimal code that makes the failing tests pass | pro |
| `tdd-refactor` | TDD refactor phase: improves the code while keeping the tests green | pro |
| `tech-writer` | Writes and improves technical documentation | pro |

## Options

```bash
armadai new agent rust-reviewer --template dev-review --stack rust
armadai new agent auditor --template security-review -d "Audits the payment code" --tier max
armadai new agent helper --skills release-notes
armadai new agent dev --force              # replace an existing file
```

- `--description` replaces the template's description (set on the frontmatter, never
  substituted into the YAML) and fills `{{description}}` in the body.
- `--stack` fills `{{stack}}` (default: "this project's stack").
- `{{name}}` is the id, title-cased.

## The TDD cycle as a flow

```bash
armadai new agent red --template tdd-red --stack rust
armadai new agent green --template tdd-green --stack rust
armadai new agent refactor --template tdd-refactor --stack rust
armadai new flow tdd --agents red,green,refactor
```

## Your own templates

A file `<name>.md` in `templates/` (current directory) or `~/.config/armadai/templates/` adds a
template, or overrides the built-in one of the same name. It is an agent file (frontmatter and
body) with a real default `description`; `{{name}}`, `{{description}}` and `{{stack}}` are
replaced in the body only. A template in the legacy format is refused with a clear error.
