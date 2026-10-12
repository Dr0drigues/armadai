# Declarative agents

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
they configured the removed execution engine and nothing executes them; `armadai inspect`
still displays them (see
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

Deprecated model names are checked and fixed in `agents.yaml` too — `defaults.model` and every
deviating agent's own `model`/`model_fallback`, one finding per occurrence — the same textual
in-place rewrite `armadai models update` does for `.md` files, chosen so comments and key order
survive. Two forms are refused rather than half-fixed: a quoted `"model":` key, and any case where
the structured YAML parse and the textual rewrite disagree about what needs changing.

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

A collision costs only the colliding declaration, never the rest of the fleet, but the commands
react differently:

- `armadai list` is read-only over the whole fleet: it prints a warning for the colliding name and
  carries on, showing every other agent that did load.
- `armadai inspect <name>` is about one specific agent and refuses to guess: naming a colliding
  agent **hard-fails** (exit 1), naming both `agents.yaml` and the colliding `.md` file. It checks
  only the name you asked for, so it says nothing about a collision elsewhere in the fleet; only
  `armadai list` (or `armadai link`) looks at the whole fleet at once.
- `armadai link` writes files other tools then trust, so it refuses to write **anything** when
  the collision falls inside what you asked it to link (the whole fleet by default, or the names
  passed to `--agents`).

## What's wired

Every command that resolves a project's agents sees declared agents alongside file-backed ones:

| Command | Sees declared agents? |
|---|---|
| `armadai link` | Yes (and refuses to write on an unresolved collision, see above) |
| `armadai unlink` | Yes |
| `armadai list` | Yes |
| `armadai inspect` | Yes |
| `armadai models check` / `armadai models update` | Yes — including the deprecated-model rewrite described above |
| `armadai validate` | Yes — a declared agent named as an `orchestration.coordinator`/`teams[].lead`/`teams[].agents` entry resolves, even when never relisted in the config's `agents:` |

## Parity with the `.md` format — and its limits

A declared agent and its hand-written `.md` twin are proven, by test
(`crates/armadai/src/linker/mod.rs`), to project **byte-identical native output** on all four link
targets (claude, codex, copilot, opencode) — same file paths, same content — and to carry identical
values for every field the declarative format is able to express.

It is narrower than "the two formats are interchangeable," in two ways:

- **The linkers don't emit every field.** No target's generated file includes `command`, `args`,
  `tags`, `scope` or `stacks`, so the projection comparison cannot fail on a divergence in those
  fields. They are checked separately, at the `Agent`/`AgentMetadata` level, by a second test.
- **Seven `AgentMetadata` fields have no declarative equivalent at all**: `cost_limit`,
  `rate_limit`, `context_window`, `mode`, `orchestration`, `triggers`, `ring_config`. A declared
  agent always resolves these to `None`/default. None of the kept commands uses them.

So: identical projection, for what both formats can actually say.

## See also

[Agent format](agent-format.md) documents the `.md` side. Instructions, Output Format and Context
are `.md`-only for now: a declared agent's prompt is its composed fragments, and `to_agent()`
always resolves `context` to `None`.
