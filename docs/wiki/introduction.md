# ArmadAI

> Declarative agent teams — describe them once, compile them for Claude Code, Codex, OpenCode and Copilot.

ArmadAI lets you describe a team of specialized AI agents — each a simple Markdown file or an
entry of `.armadai/agents.yaml` — together with the prompts and skills they share, and compiles
that description with `armadai link` into the native configuration files of Claude Code, Codex,
OpenCode or GitHub Copilot. The assistant runs the agents; ArmadAI only writes their
configuration, and `armadai unlink` removes exactly what it wrote.

## Where to next

- New here? Start with [Getting Started](getting-started.md).
- Writing agents? See the [Agent format](agent-format.md).
- Generating files for your assistant? Read [Link](link.md).
- Enforcing who delegates to whom in Claude Code? See the [Policy gate](policy-gate.md).
