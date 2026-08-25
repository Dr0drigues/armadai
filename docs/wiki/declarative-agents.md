# Declarative agents

A project can declare its agents in `.armadai/agents.yaml` instead of writing each one as a
Markdown file. The YAML is parsed into the same `Agent` struct the `.md` parser produces — same
provider factory, same linker, same everything downstream. Nothing changes for a project that
never creates the file.

## What this does not buy

Measured on the 77 agents in `~/.config/armadai/agents/` (2026-08-20), before writing a line of
this format:

- **No token saving.** Metadata never reaches the model — ArmadAI parses it locally to pick a
  provider, model and temperature. The 5,152 tokens of `## Metadata` blocks across that library
  cost disk space and eyestrain, not API spend. What actually costs money is the ~52,460 tokens of
  instruction prose, and moving it into YAML changes nothing about that: the prose still has to
  exist somewhere, in full, for the model to read it.
- **No reduction in context saturation**, for the same reason — and composition can make it worse.
  Fragments in this library average ~66 lines and a composed prompt averages ~680 tokens; an agent
  that composes four of them ends up with a longer system prompt than one written to fit, not a
  shorter one. You are trading duplication on disk for inclusion at runtime, not eliminating either.
- **No guaranteed improvement in agent quality.** A prompt assembled from generic, reusable
  fragments is less specific than one written for exactly one agent, and an agent left with vague
  instructions fills in the gaps itself. Whatever consistency you gain this way may cost you
  precision. Only a real task comparison would say — this chantier didn't run one.

The one real gain is structural, not computational: a fact that used to live in one `.md` file now
has exactly one place to be wrong in, instead of a `.md` file and whatever someone remembers about
it. If you're adopting this expecting a smaller or cheaper context, don't — that expectation is
what this section exists to correct before you build on it.

## What a declaration looks like

```yaml
# .armadai/agents.yaml
defaults:
  provider: claude
  model: latest:pro
  temperature: 0.3
  max_tokens: 8192

agents:
  - name: core-specialist
    description: Core domain and orchestration engine
    scope: [src/core/**, src/parser/**]
    tags: [rust, domain]
    prompt:
      - specialist-base
      - { armadai-architecture: { module: core } }

  - name: ui-specialist
    description: TUI and Web dashboards
    temperature: 0.4          # the only deviation from defaults
    prompt: [specialist-base]
```

`defaults` supplies values every agent inherits unless it overrides them. `provider` is required —
at agent or `defaults` level — exactly like the `.md` format: an agent with no provider anywhere
fails to load, on both sides. `command`/`args` are also expressible, so a `provider: cli` agent
(wrapping `claude`, `gemini`, or any other command-line tool) can be declared, not just an API one.
`defaults` has no `scope` field — there is no fleet-wide default scope; `scope` is per-agent only,
and an agent that omits it simply has none.

## The `.md` format is not going away

This is an additional input, not a replacement. `.md` agents keep working exactly as before, and a
project can mix both: some agents declared, some written by hand, loaded together by every command
that resolves a fleet (see the [Agent format](agent-format.md) page for the `.md` side).

## Defaults merge; lists replace, never merge

Every field a declared agent omits falls back to `defaults`. Scalars (`temperature`, `model`,
`max_tokens`, …) behave the way you'd expect. **Lists do not.** An agent that declares `tags: [own]`
has exactly `[own]` — the default's tags are not appended. An agent that declares `tags: []` has no
tags at all, not "inherit whatever `defaults` set." This is the one rule most likely to surprise a
reader coming from a format where lists usually merge — say it plainly: they don't, here, ever
(`tags`, `stacks`, `model_fallback`, `args` all follow the same rule).

## Composing a prompt from fragments

`prompt:` is a list of steps. A step is either a bare fragment name:

```yaml
prompt: [specialist-base]
```

or a single-key map naming a fragment and the variables to substitute into it:

```yaml
prompt:
  - { armadai-architecture: { module: core } }
```

Each fragment body goes through the same `{{var}}` substitution `armadai new` uses
(`crates/armadai-core/src/template.rs`), then the rendered fragments are joined with a blank line,
in the order declared. Every fragment body is trimmed first, so a fragment file's own trailing
newline doesn't pile up into two or three blank lines at the join. Two variables are always
available implicitly: `name` (the agent's own name) and `description` (the agent's own
`description:`, when it declares one) — a step's own variables win if it supplies the same name.
`{{ name }}` (inner whitespace, either side or both) is the same placeholder as `{{name}}` — the
missing-value check and the substitution agree on that by construction, so a spaced placeholder is
never left in the rendered prompt unsubstituted.

## Failure is the default, not degradation

A missing fragment, or a fragment left with an unsubstituted `{{variable}}`, **fails agent loading**
— it does not ship a shorter or blanker prompt. The same applies to `description`: if a fragment
references `{{description}}` and the agent declares none, loading fails, naming both the agent and
the fragment. This is deliberate: an agent shipped with amputated instructions doesn't complain
about it, it fills the gaps itself, and a hard failure is the only way to make that visible before a
model ever sees the result. If you genuinely need a fragment's `{{description}}` filled without the
agent declaring one, a prompt step can supply it directly as one of its own variables — that's the
escape hatch, not a default.

Deprecated model names are checked and fixed in `agents.yaml` too — `defaults.model` and every
deviating agent's own `model`/`model_fallback`, one finding per occurrence — the same textual
in-place rewrite `armadai models update` already does for `.md` files, chosen so comments and key
order survive. Two forms are refused rather than half-fixed: a quoted `"model":` key, and any case
where the structured YAML parse and the textual rewrite disagree about what needs changing. The
tool would rather stop and tell you than guess at a partial fix.

## A name both declared and written as a file: no precedence

If a name in `agents.yaml` collides with a `.md` file — compared as a slug, so `Core Specialist`,
`core-specialist` and `core_specialist` all collide with each other — **the declaration is
refused**, and neither side is given precedence.

This check runs against **all three** directories ArmadAI already searches for agent files:
`.armadai/agents/` (project-local, preferred), `agents/` (project-local, legacy), and
**`~/.config/armadai/agents/` — your personal library**, which the project doesn't own and may not
even know about. On a machine with a large accumulated personal library, this is the first thing a
new user is likely to hit: you write `name: code-reviewer` in a fresh project's `agents.yaml`, and
loading refuses because you happen to have a `code-reviewer.md` sitting in your global agents
directory from an unrelated project.

The reasoning: a silent "the local file wins" (or the reverse) would recreate exactly the duplicated
truth this format exists to remove. You'd edit the `.md`, see no effect because the YAML quietly
took precedence, and have nothing on screen telling you why — the same failure mode as two
`CLAUDE.md` files disagreeing and the model citing one while acting on the other. Refusing outright,
by contrast, tells you immediately and by name. The message looks like this:

```
agent 'code-reviewer' is declared in .armadai/agents.yaml and also written as
/Users/you/.config/armadai/agents/code-reviewer.md — remove one; there is
deliberately no precedence between them
```

What to do: rename or delete one side — whichever is actually stale. There is no flag to suppress
the check or to force one side to win; a suppression flag would just be the silent precedence rule
in disguise.

The blast radius is scoped, though: a collision costs only the colliding declaration, never the
rest of the fleet — every other declared agent, and the colliding `.md` file itself, still load
fine elsewhere. But the three commands that can hit one don't all react the same way:

- `armadai list` is read-only over the whole fleet: it prints a warning for the colliding name and
  carries on, showing every other agent that did load.
- `armadai run <name>` and `armadai inspect <name>` are about one specific agent, and refuse to
  guess which side of a collision you meant: naming a colliding agent **hard-fails** (exit 1),
  naming both `agents.yaml` and the colliding `.md` file. A command that is about to execute or
  describe one agent must not silently pick a winner between a declaration and a file.
- `armadai run --pipe` refuses a colliding name in **any** position of the chain, and refuses it
  before running the first link — so a collision costs no provider call at all. It used to let the
  chain through while the single-agent path refused the very same name, because the chain loop
  resolved agents by file path and so never consulted the declarations; routing it through the same
  by-name loader removed that inconsistency.
- Because that check is scoped to just the one name you asked for, `armadai inspect
  <some-other-name>` says **nothing** about an unrelated collision sitting elsewhere in the fleet —
  it doesn't scan the whole project the way `list` does. Don't rely on `inspect` to surface
  collisions in general; only `armadai list` (or `armadai link`) looks at the whole fleet at once.
- `armadai link` is different again: because it writes files other tools then trust, it refuses to
  write **anything** when the collision falls inside what you asked it to link (the whole fleet by
  default, or the names passed to `--agents`) — a command that writes config must not silently ship
  a smaller fleet than the one you declared.

## Long compositions still get audited

Composing several fragments is easy to do without noticing how long the result got. `armadai
audit`'s existing rule `A05` (system prompt exceeds a configurable token estimate, default 4000,
`audit.prompt_token_threshold` in project config) doesn't care where a prompt came from — once a
declared agent is linked into a native config, `armadai audit` reads that generated file exactly
like a hand-written one. A four-fragment composition is exactly as likely to trip it as an
over-written `.md`.

## What's wired, and what isn't

Every surface that resolves a project's agent fleet sees declared agents alongside file-backed
ones:

| Surface | Sees declared agents? |
|---|---|
| `armadai run <name>` (single agent) | Yes |
| `armadai run --orchestrate` | Yes |
| `armadai run --resume` | Yes |
| `armadai run --pipe` (multi-agent chain) | Yes — any position of the chain, mixed freely with file-backed agents |
| `armadai link` | Yes (and refuses to write on an unresolved collision, see above) |
| `armadai list` | Yes |
| `armadai inspect` | Yes |
| `armadai models check` / `armadai models update` | Yes — including the deprecated-model rewrite described above |
| `armadai unlink` | Yes |
| `armadai validate` | Yes — a declared agent named as an `orchestration.coordinator`/`teams[].lead`/`teams[].agents` entry resolves, even when never relisted in `armadai.yaml`'s `agents:` |
| TUI dashboard | Yes |
| Web API | Yes |
| `armadai shell`'s setup wizard (its own `link`, run once before entering the shell) | Yes |
| `armadai shell`'s in-session pipeline steps (an `agent:` entry relayed from inside the shell) | Yes — declared or file-backed alike; an API-only provider is skipped with an explanation, see below |

There is no longer a "No" row. If one ever reappears, read it as a real capability gap rather than
a wording problem: a surface that resolves agents by *file path* cannot see a declared agent at all,
because a declared agent has no file.

The TUI dashboard and Web API used to be worse than an omission: both resolved a project's agents
via `project::resolve_all_agents`, which only ever returns file paths, so a declared agent (which
has no file) was silently dropped — and if a same-named agent happened to exist in the global
library (`~/.config/armadai/agents/`), *that* agent was shown in its place, with no warning. Both
now go through the same `agent_source::load_all_agents` every other wired surface uses, so a
declared agent is loaded on its own terms rather than shadowed by an unrelated global homonym.

`armadai shell`'s wizard used to be a fifth, independent copy of the project-detection gate — a
plain `config.agents.is_empty()` check with no notion of `.armadai/agents.yaml` at all, so a
declarations-only project failed the wizard's own `link` step with a false "No agents declared"
(issue #339's second half). Its write side had also drifted from `link`'s in a second, unrelated
way: the wizard hand-rolled its own file-write loop, with no link manifest entry and no
exists-guard, so `unlink` could never account for what it wrote and a hand-written file in its way
could be silently overwritten (issue #347). Both were fixed together, since both were the wizard
using its own variant instead of the primitive the rest of the fleet already shares: the wizard's
`run_link` now calls `agent_source::project_declares_agents`/`load_all_agents` for detection and
resolution, and `linker::manifest::write_files` — the same function `cli::link::execute` itself
calls — for the write, so the manifest entry and the exists-guard come from the same place `link`
gets them rather than a third copy that could drift from both.

The last two path-shaped resolvers were `armadai run --pipe`'s chain loop and the shell's
in-session pipeline lookup (`shell::app::resolve_project_agent`). Both resolved a *path* per agent
and handed it down to be parsed, which no declared agent can satisfy — the chain loop failed with a
message naming the three library directories and never the `agents.yaml` that declares the agent,
and the shell answered a declarations-only project with a false "not found in project config".
Both now call the same by-name loaders every other surface uses (`load_agent_for_run`, itself
`agent_source::load_agent_by_name`), and the path-returning helper the chain loop used
(`cli::run::resolve_agent_path`) was **deleted** rather than left unused, so a future call site
cannot pick it up and reintroduce the same blind spot. The shell's honest-but-limiting
"declarative agents can't be run from the shell yet" message is gone because the capability it
described as missing now exists: a pipeline step's `agent:` entry loads a declared agent and relays
it exactly as it relays a file-backed one — same composed system prompt, and the same command
resolution described below.

### What the in-session relay can and cannot run

A pipeline step spawns a CLI and hands it the prompt on argv, so the step's command comes from the
agent's metadata the way `providers::factory` reads it: `provider: cli` spawns whatever `command:`
names, any other provider spawns `command:` if set and the provider name otherwise, and an explicit
`args:` list is passed through verbatim (otherwise the canonical JSON-mode argv for that command is
used). Reading `provider:` alone — which this relay did until it was fixed — turned every
`provider: cli` agent into an attempt to spawn a binary literally named `cli`.

An agent whose provider is an HTTP API (`anthropic`, `openai`, `google`, `proxy`) has no command to
spawn at all. Such a step is **skipped with an explanation** and the rest of the chain runs on: the
step is lost, not the pipeline. Run those agents with `armadai run <name>`, which builds the API
client instead of relaying a CLI.

## Parity with the `.md` format — and its limits

A declared agent and its hand-written `.md` twin are proven, by test
(`crates/armadai/src/linker/mod.rs`), to project **byte-identical native output** on all five link
targets (claude, codex, copilot, gemini, opencode) — same file paths, same content — and to carry
identical values for every field the declarative format is able to express. That's a real, tested
guarantee, covering a plain API agent, a `provider: cli` agent with `command`/`args`, and an agent
composed from two fragments.

It is narrower than "the two formats are interchangeable," though, in two ways worth knowing before
you rely on it:

- **The linker doesn't emit every field it merges.** No target's generated file includes
  `command`, `args`, `tags`, `scope` or `stacks` in its content — the five-target comparison is
  real, but it cannot fail on a divergence in a field none of the five linkers ever write out. Those
  fields are checked separately, at the `Agent`/`AgentMetadata` level, by a second test — necessary
  precisely because the projection check can't see them.
- **Seven `AgentMetadata` fields have no declarative equivalent at all**: `cost_limit`,
  `rate_limit`, `context_window`, `mode`, `orchestration`, `triggers`, `ring_config`. A declared
  agent always resolves these to `None`/default, whatever it says — there is no key for them in the
  format. The parity test's assertions on these seven are true by construction, not by
  demonstration: both sides are always `None`, so they can never fail while the format stays as it
  is. If your `.md` agent uses any of these seven, declaring it means losing that capability, not
  preserving it under a different spelling.

So: identical projection, for what both formats can actually say. Not identical expressiveness.

## See also

[Agent format](agent-format.md) documents the `.md` side — required sections, the full metadata
table, provider types. Anything not covered here (Instructions, Output Format, Pipeline, Context,
Triggers, Ring Config) is `.md`-only for now; a declared agent has no way to set any of them —
`to_agent()` always resolves `context` to `None`, exactly like the other five.
