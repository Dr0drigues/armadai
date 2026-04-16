---
name: armadai-shell-config
description: Reference for the shell section in armadai.yaml — model tiers, tandem, pipeline
version: 1.0.0
tools: []
---

# ArmadAI Shell Configuration

The `shell:` section in `armadai.yaml` configures the interactive shell (`armadai shell`).

## Full schema

```yaml
shell:
  default_provider: gemini        # CLI to use (gemini, claude, aider, codex, copilot, opencode)
  default_model: latest:pro       # tier or concrete model
  timeout: 120                    # CLI timeout in seconds (default 120)
  max_history: 10                 # conversation turns kept for context (default 5)
  auto_save: true                 # auto-save sessions after each turn (default true)

  tandem:
    - provider: gemini
      model: latest:fast
    - provider: claude
      model: latest:pro

  pipeline:
    steps:
      - name: analyze
        prompt: "Analyze this request and identify key components"
        providers:
          - provider: gemini
            model: latest:fast
      - name: review
        prompt: "Review the analysis above and suggest improvements"
        providers:
          - provider: claude
            model: latest:pro
```

## Model tiers

Avoid hardcoding model names — use tiers that resolve per provider.

| Tier | Aliases | Resolves to (Claude) | Resolves to (Gemini) |
|------|---------|----------------------|----------------------|
| `latest:fast` | `latest:low` | claude-haiku-4-5 | gemini-2.5-flash |
| `latest:pro` | `latest:medium`, `latest` | claude-sonnet-4-5 | gemini-2.5-pro |
| `latest:max` | `latest:high` | claude-opus-4-6 | gemini-2.5-pro |

Tier picking:
- `fast` — simple lookups, transformations, high-volume, cost-sensitive
- `pro` — default for analysis, generation, most coordinator roles
- `max` — complex reasoning, architecture, critical decisions, final reviews

## Tandem mode

Sends the same prompt to N providers in parallel, shows all responses labeled.

```yaml
shell:
  tandem:
    - provider: gemini
      model: latest:fast
    - provider: claude
      model: latest:pro
```

In the shell, type your message and both providers receive it simultaneously. Useful for:
- Comparing responses across models
- Cost/quality benchmarking
- Getting diverse perspectives

Trigger explicitly: `/tandem gemini,claude`.
With `tandem:` configured, `/tandem` with no args uses the YAML list.

## Pipeline mode

Sequential chain: provider A's output becomes provider B's input. Named steps with custom prompts.

```yaml
shell:
  pipeline:
    steps:
      - name: analyze
        prompt: "Analyze in detail, identify issues and components"
        providers:
          - provider: gemini
            model: latest:fast
      - name: review
        prompt: "Review above and suggest improvements"
        providers:
          - provider: claude
            model: latest:pro
```

Auto-pipeline: **when `pipeline.steps` is configured, every message automatically goes through the pipeline**. No need to type `/pipeline` each time.

Trigger explicitly: `/pipeline gemini,claude`.

### Step prompt template
The `prompt` field is prepended to the user input at each step. For later steps, the previous stage's output is injected as context:
```
Review and improve the following response:
---
<previous output>
---
Original request: <user input>
```

### Agent-based steps (v0.12+)

A step can reference a project agent instead of a raw provider. The agent's `## System Prompt` and metadata `provider`/`model` drive the step.

```yaml
shell:
  pipeline:
    steps:
      - name: plan
        prompt: "Focus on the user request below"
        providers:
          - agent: architect          # uses architect.md's system prompt + provider
      - name: review
        providers:
          - agent: reviewer
```

Rules:
- `agent:` takes precedence over `provider:`/`model:` (the agent's metadata wins)
- The agent must exist in the project's `agents:` list
- The step's `prompt:` (optional) is appended as extra context after the agent's system prompt
- Mix freely: one step can be `agent:`, another `provider:`

## Provider capabilities matrix

| Provider | JSON output | Stream-JSON | Metrics | `-p` flag type | Notes |
|----------|-------------|-------------|---------|----------------|-------|
| Claude | ✅ | ✅ | cost, tokens, duration, model | flag (no value) | Best integration |
| Gemini | ✅ | ✅ | tokens, latency, model | takes value | |
| Codex | ✅ | ✅ (JSONL) | tokens | positional | |
| Copilot | ✅ | ✅ (JSONL) | usage | takes value | |
| OpenCode | ✅ | ✅ | varies | positional | Requires paid plan |
| Aider | ❌ | ❌ | none | takes value | Text fallback only |

## Example use cases

### Analyze-then-improve (default pipeline)
```yaml
shell:
  pipeline:
    steps:
      - name: analyze
        prompt: "Analyze this request"
        providers: [{provider: gemini, model: latest:fast}]
      - name: improve
        prompt: "Improve based on the analysis"
        providers: [{provider: claude, model: latest:pro}]
```

### Comparative tandem
```yaml
shell:
  tandem:
    - provider: gemini
      model: latest:pro
    - provider: claude
      model: latest:pro
```

### Single provider (no orchestration)
```yaml
shell:
  default_provider: claude
  default_model: latest:pro
```

## See also

- `docs/wiki/orchestration.md` — orchestration patterns reference
- `armadai-orchestration-patterns` skill — decision matrix for patterns
