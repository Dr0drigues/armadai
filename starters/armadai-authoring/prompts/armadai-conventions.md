---
name: armadai-conventions
description: Shared naming conventions and quality standards for ArmadAI content authoring
apply_to: [agent-builder, prompt-builder, skill-builder]
---

# ArmadAI Conventions

## Naming

- Use **kebab-case** for all identifiers: agent filenames, prompt names, skill directories, tags
- Agent files: `<name>.md` (e.g., `code-reviewer.md`, `test-writer.md`)
- Prompt files: `<name>.md` (e.g., `rust-conventions.md`, `output-format.md`)
- Skill directories: `<name>/` (e.g., `docker-compose/`, `api-testing/`)

## Tags

Use lowercase, common vocabulary for tags. Standard tag categories:

| Category | Examples |
|----------|---------|
| Role | `coordinator`, `lead`, `reviewer`, `writer`, `analyzer` |
| Domain | `security`, `testing`, `documentation`, `devops`, `authoring` |
| Technology | `rust`, `python`, `typescript`, `docker`, `kubernetes` |
| Activity | `review`, `analysis`, `generation`, `migration` |

## Quality Standards

- Every agent MUST have: H1 heading, `## Metadata` (with provider + model), `## System Prompt`
- Every prompt MUST have: YAML frontmatter with `name` and `description`
- Every skill MUST have: `SKILL.md` with frontmatter (`name`, `description`, `version`)
- System prompts should be specific, actionable, and structured
- Avoid vague instructions like "be helpful" or "do your best"
- Include concrete examples when explaining expected behavior

## File Organization

- Agents go in `~/.config/armadai/agents/` or project-local paths
- Prompts go in `~/.config/armadai/prompts/` or project-local paths
- Skills go in `~/.config/armadai/skills/` or project-local paths
- Group related content in starter packs for reuse
