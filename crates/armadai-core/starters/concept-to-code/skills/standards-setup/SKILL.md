---
name: standards-setup
description: "Sets up the standards and the documentation of a repository, like /init does for the project memory: analyses the code, tools, CI, docs and infrastructure, proposes the conventions found, has the user settle the others, writes the project standards skill that fills the code-standards and security slots, creates docs/functional/ and docs/technical/, then runs armadai link. Use when the user types /init-standards, and when a repository has no standards skill yet."
---
# Standards setup

The procedure behind `/init-standards`. It runs in the main conversation: it asks the user questions and runs one command (`armadai link`), so a subagent cannot run it whole. A subagent (`framer`) can do the analysis (step 1) and return the questions to `lead`.

Speak the user's language throughout; the files you write are in that language too, except identifiers and tool names.

## 1. Analyse the repository (read only)

Look for, and note each finding with its file:

- **Languages**: the file extensions present and their weight.
- **Manifests**: `Cargo.toml`, `package.json`, `pyproject.toml`, `requirements*.txt`, `go.mod`, `pom.xml`, `build.gradle*`, `*.csproj`, `Gemfile`, `composer.json`… and the versions they pin.
- **Formatters and linters**: `.editorconfig`, `rustfmt.toml`, `clippy.toml`, `.prettierrc*`, `eslint.config.*`, `.eslintrc*`, `biome.json`, `ruff.toml` or `[tool.ruff]`, `.golangci.yml`, `checkstyle.xml`…
- **Tests**: test folders, frameworks in the manifests, the test command.
- **CI and hooks**: `.github/workflows/`, `.gitlab-ci.yml`, `Jenkinsfile`, `azure-pipelines.yml`, `.githooks/`, `.husky/`, `.pre-commit-config.yaml`.
- **Documentation**: `README*`, `CONTRIBUTING*`, `docs/`, architecture decision records.
- **Infrastructure**: `Dockerfile`, `compose*.yaml`, `k8s/`, `helm/`, `*.tf`, deployment scripts.
- **Security**: `SECURITY.md`, Dependabot or Renovate configuration, `.env.example`, how secrets are kept out of the repository (`.gitignore`).
- **ArmadAI**: `.armadai/docs/cadre.md`, and the skills of `.armadai/skills/` whose frontmatter has `metadata: { armadai-slot: … }`: what they already cover is not asked again.

An empty repository is fine: nothing is detected, everything is to decide, and the defaults you recommend follow the stack the user chooses (ask for it first).

## 2. Propose what was detected

Show a short table: convention, value, source file. Ask the user, with your question tool, to confirm it whole or to say what to correct. A habit seen in the code but enforced by no tool is proposed as "observed", to confirm.

## 3. Have the user settle the rest

For each convention the repository does not fix and the project needs, ask one question with your question tool: 2 to 4 options, your recommendation first and why, and room for another answer. Cover only what applies, most structuring first:

stack and versions; project layout; naming; formatting and linting; tests (tool, location, what must be tested); error handling; logging; dependencies (allowed, how to add one); secrets; input validation; authentication and authorisation; dependency audit; commits and branches; language of the documentation; build and deployment.

Keep it short (eight questions at most in one session). Offer "decide later": the topic stays under "To decide" in the frame.

## 4. Write the project standards skill

Write `.armadai/skills/project-standards/SKILL.md`. If it exists, read it first, show the user what would change, and ask before replacing anything.

```markdown
---
name: project-standards
description: "Code standards and security rules of this project (<stack>). Use before writing, reviewing or explaining code of this repository."
metadata: { armadai-slot: "code-standards, security" }
---
# Project standards

## Stack
- <language, version, framework>. Source: <file | user, YYYY-MM-DD>

## Code standards
- <one rule per line, imperative and checkable>. Source: <file | user, YYYY-MM-DD>

## Tests
- <tool, location, the command to run them, what must be tested>

## Security
- <secrets, input validation, authorisation, dependencies>

## Commands
- format: <command>; lint: <command>; test: <command>
```

- `name` is `project-standards`, the folder name; `description` stays under 1024 characters.
- `metadata: { armadai-slot: "code-standards, security" }` makes the skill fill both slots: every agent that declares them receives it.
- When the infrastructure is described (deployment, environments, services), add an `## Infrastructure` section and the third slot: `metadata: { armadai-slot: "code-standards, security, infra-docs" }`.
- Every rule has its source; nothing the user did not confirm.

## 5. Update the frame

In `.armadai/docs/cadre.md` (see the `cadre` skill): the confirmed and decided conventions under "Decided" with their source, the user's answers under "User decisions" with today's date, the "decide later" topics under "To decide".

## 6. Create the documentation tree

- `docs/functional/`: what the product does, for its users, one sub-folder per domain (`docs/functional/<domain>/…`).
- `docs/technical/`: how it is built, with `architecture/`, `api/`, `infra/` as the project needs them (`docs/technical/<area>/<subject>.md`).

Git keeps no empty folder: give each folder a short `README.md` saying what goes there. Existing documentation is not moved: if some exists, propose where each file would go and move nothing without the user's agreement. The agents' working documents stay in `.armadai/docs/`, never under `docs/`.

## 7. Link

Ask the user's agreement, then run `armadai link`: it regenerates the agents' files so the new skill reaches every agent that declares `code-standards` or `security`. Read its report: the lines `slot code-standards` and `slot security` must no longer say "no skill fills it". If the command fails, show its output; do not retry blindly.

## 8. Sum up

Tell the user: the files written, the conventions recorded (detected, decided by them), what is still to decide, and that `/init-standards` can be run again when the project changes.
