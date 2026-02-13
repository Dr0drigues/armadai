# Skill Examples

## Example 1: Framework Reference Skill

A comprehensive reference skill for a framework (like Platodin):

```
platodin-reference/
├── SKILL.md
└── references/
    ├── overview.md
    ├── web.md
    ├── security.md
    ├── mongo.md
    ├── kafka.md
    ├── webclient.md
    ├── testing.md
    └── configuration.md
```

**SKILL.md**:

```markdown
---
name: platodin-reference
description: Platodin framework reference (Spring Boot overlay v4.x)
version: "4.0"
tools: []
---

# Platodin Reference

Complete reference documentation for the Platodin framework — a Spring Boot overlay for building APIs and event-driven applications.

See the `references/` directory for module-specific documentation.
```

Each reference file covers one module with its API, configuration, and patterns.

## Example 2: CI Templates Skill

A skill providing CI/CD pipeline templates and scripts:

```
ci-templates-reference/
├── SKILL.md
├── references/
│   ├── github-actions.md
│   ├── gitlab-ci.md
│   └── conventions.md
├── scripts/
│   ├── lint.sh
│   └── deploy.sh
└── assets/
    ├── github-workflow.yaml
    └── gitlab-ci.yaml
```

**SKILL.md**:

```markdown
---
name: ci-templates-reference
description: CI/CD pipeline templates and conventions
version: "2.0"
tools:
  - docker
  - gh
---

# CI Templates Reference

Standardized CI/CD pipeline templates for GitHub Actions and GitLab CI.

Includes ready-to-use workflow files in `assets/` and helper scripts in `scripts/`.

See `references/` for detailed documentation of each CI platform and conventions.
```

## Example 3: Minimal Skill

A skill with just a SKILL.md and one reference file:

```
code-standards/
├── SKILL.md
└── references/
    └── standards.md
```

**SKILL.md**:

```markdown
---
name: code-standards
description: Team coding standards and conventions
version: "1.0"
tools: []
---

# Code Standards

Team-wide coding standards. See `references/standards.md` for the full specification.
```

This pattern works well for single-topic knowledge packs that are too complex for a simple prompt file.
