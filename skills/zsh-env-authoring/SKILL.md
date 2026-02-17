---
name: zsh-env-authoring
description: Reference for developing and extending the zsh_env modular shell configuration framework
version: "1.0"
tools:
  - zsh
  - shell
---

# ZSH_ENV Authoring

Complete reference for developing modules, functions, aliases, completions, and themes within the `~/.zsh_env` framework.

ZSH_ENV is a modular Zsh configuration suite focused on productivity. It provides a structured loading order, a UI library for consistent output, lazy-loaded functions, plugin management, and tool-specific completions. This skill covers the architecture, conventions, and extension points.

See the `references/` directory for detailed documentation:

- **architecture.md** — Loading order, module system, and directory layout
- **ui-system.md** — The `_ui_*` function library for consistent terminal output
- **extending.md** — How to add functions, aliases, completions, modules, and themes
