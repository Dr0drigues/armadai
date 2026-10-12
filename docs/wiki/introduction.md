# ArmadAI

> Declarative agent teams — describe them once, compile them for Claude Code, Codex, OpenCode and Copilot.

ArmadAI lets you describe a team of specialized AI agents in a `.armadai/` folder — one Markdown
file per agent, with the skills they share, the flows that say who hands work to whom, and
automations — and compiles that description with `armadai link` into the native configuration
files of Claude Code, Codex, OpenCode or GitHub Copilot, with a conformance report of what each
target received. The assistant runs the agents; ArmadAI only writes their configuration, and
`armadai unlink` removes exactly what it wrote.

## Where to next

- New here? Start with [Getting Started](getting-started.md).
- Writing agents? See the [Project format](project-format.md).
- Generating files for your assistant? Read [Link](link.md).
- Enforcing who delegates to whom in Claude Code? See the [Policy gate](policy-gate.md).
