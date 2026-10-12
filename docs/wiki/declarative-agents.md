# Declarative agents

> **Legacy format.** This page describes the format of projects before v1 (`.armadai/config.yaml`,
> `armadai.yaml`). It is still read by `armadai migrate`, which converts such a project, and
> internally by `armadai new` (templates) and `armadai init --pack` (starter packs). Projects use
> the [Project format](project-format.md).

A project can declare its agents in `.armadai/agents.yaml` instead of writing each one as a
Markdown file. The YAML is parsed into the same `Agent` struct the `.md` parser produces — same
linker, same everything downstream. Nothing changes for a project that never creates the file.

## What this does not buy

Measured on the 77 agents in `~/.config/armadai/agents/` (2026-08-20), before writing a line of
this format:

- **No token saving.** `## Metadata` is configuration, not prompt text. The 5,152 tokens of
  `## Metadata` blocks across that library cost disk space and eyestrain, not context. What the
  assistant actually reads is the ~52,460 tokens of instruction prose, and moving it into YAML
  changes nothing about that: the prose still has to exist somewhere, in full.
- **No reduction in context saturation**, for the same reason — and composition can make it worse.
  Fragments in this library average ~66 lines and a composed prompt averages ~680 tokens; an agent
  that composes four of them ends up with a longer prompt than one written to fit, not a shorter
  one. You are trading duplication on disk for inclusion at link time, not eliminating either.
- **No guaranteed improvement in agent quality.** A prompt assembled from generic, reusable
  fragments is less specific than one written for exactly one agent, and an agent left with vague
  instructions fills in the gaps itself. Whatever consistency you gain this way may cost you
  precision.

The one real gain is structural: a fact that used to live in one `.md` file now has exactly one
place to be wrong in, instead of a `.md` file and whatever someone remembers about it.

## What a declaration looks like

```yaml
# .armadai/agents.yaml
defaults:
  provider: claude
  model: latest:pro
  temperature: 0.3

agents:
  - name: code-reviewer
    description: Reviews Rust changes for correctness and style
    scope: [src/**, tests/**]
    tags: [rust, review]
    prompt:
      - specialist-base
      - { project-architecture: { module: core } }

  - name: doc-writer
    description: Writes and updates the user documentation
    temperature: 0.4          # the only deviation from defaults
    prompt: [specialist-base]
```

`defaults` supplies values every agent inherits unless it overrides them. `provider` is required —
at agent or `defaults` level — exactly like the `.md` format: an agent with no provider anywhere
fails to load, on both sides. `defaults` has no `scope` field — `scope` is per-agent only, and an
agent that omits it simply has none.

The format also accepts `command`, `args`, `max_tokens` and `timeout`, like the `.md` metadata;
they configured the removed execution engine; `migrate` drops them and says so (see
[Agent format](agent-format.md#metadata-required)).

## The `.md` format is not going away

This is an additional input, not a replacement. `.md` agents keep working exactly as before, and a
project can mix both: some agents declared, some written by hand, loaded together by every command
that resolves a fleet (see the [Agent format](agent-format.md) page for the `.md` side).

## Defaults merge; lists replace, never merge

Every field a declared agent omits falls back to `defaults`. Scalars (`temperature`, `model`, …)
behave the way you'd expect. **Lists do not.** An agent that declares `tags: [own]` has exactly
`[own]` — the default's tags are not appended. An agent that declares `tags: []` has no tags at
all, not "inherit whatever `defaults` set." This is the one rule most likely to surprise a reader
coming from a format where lists usually merge — they don't, here, ever (`tags`, `stacks`,
`model_fallback`, `args` all follow the same rule).

## Composing a prompt from fragments

`prompt:` is a list of steps. A step is either a bare fragment name:

```yaml
prompt: [specialist-base]
```

or a single-key map naming a fragment and the variables to substitute into it:

```yaml
prompt:
  - { project-architecture: { module: core } }
```

Each fragment body goes through the same `{{var}}` substitution `armadai new` uses
(`crates/armadai-core/src/template.rs`), then the rendered fragments are joined with a blank line,
in the order declared. Every fragment body is trimmed first, so a fragment file's own trailing
newline doesn't pile up into two or three blank lines at the join. Two variables are always
available implicitly: `name` (the agent's own name) and `description` (the agent's own
`description:`, when it declares one) — a step's own variables win if it supplies the same name.
`{{ name }}` (inner whitespace, either side or both) is the same placeholder as `{{name}}`.

## Failure is the default, not degradation

A missing fragment, or a fragment left with an unsubstituted `{{variable}}`, **fails agent loading**
— it does not ship a shorter or blanker prompt. The same applies to `description`: if a fragment
references `{{description}}` and the agent declares none, loading fails, naming both the agent and
the fragment. An agent shipped with amputated instructions doesn't complain about it, it fills the
gaps itself, and a hard failure is the only way to make that visible before the files are written.
If you need a fragment's `{{description}}` filled without the agent declaring one, a prompt step
can supply it directly as one of its own variables.

`migrate` turns a `latest:*` model into a tier and keeps any other model id as
`native.<target>.model`, which `armadai models check` then checks for deprecation.

## A name both declared and written as a file: no precedence

If a name in `agents.yaml` collides with a `.md` file — compared as a slug, so `Code Reviewer`,
`code-reviewer` and `code_reviewer` all collide with each other — **the declaration is refused**,
and neither side is given precedence.

This check runs against **all three** directories ArmadAI searches for agent files:
`.armadai/agents/` (project-local, preferred), `agents/` (project-local, legacy), and
**`~/.config/armadai/agents/` — your personal library**, which the project doesn't own and may not
even know about. On a machine with a large personal library, this is the first thing a new user is
likely to hit: you write `name: code-reviewer` in a fresh project's `agents.yaml`, and loading
refuses because you happen to have a `code-reviewer.md` in your global agents directory.

A silent "the local file wins" (or the reverse) would recreate exactly the duplicated truth this
format exists to remove. Refusing outright tells you immediately and by name:

```
agent 'code-reviewer' is declared in .armadai/agents.yaml and also written as
/Users/you/.config/armadai/agents/code-reviewer.md — remove one; there is
deliberately no precedence between them
```

What to do: rename or delete one side — whichever is actually stale. There is no flag to suppress
the check or to force one side to win.

## Who reads this format

Since step 2 of v1, only `armadai migrate` reads it (and, internally, `armadai new` and
`armadai init --pack`): it converts every declared agent into `.armadai/agents/<id>.md` with its
composed prompt, exactly like a file-backed one, and its report says what was dropped. `list`,
`inspect`, `link`, `validate` and `models` read the [Project format](project-format.md) only.

## See also

[Agent format](agent-format.md) documents the `.md` side. Instructions, Output Format and Context
are `.md`-only for now: a declared agent's prompt is its composed fragments, and `to_agent()`
always resolves `context` to `None`.
