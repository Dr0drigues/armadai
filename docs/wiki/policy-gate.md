# Policy gate: enforcing a declared topology

A project can declare who delegates to whom in `.armadai/config.yaml`. Without a gate, that
declaration governs nothing inside a Claude Code session: the model reads it, can quote it back
verbatim, and still routes elsewhere. Measured on ArmadAI itself, the declared coordinator received
9 delegations out of 520.

The policy gate turns that declaration into a rule Claude Code has to obey, through a `PreToolUse`
hook on the delegation tool.

## What it enforces

```yaml
orchestration:
  policy: strict            # off | strict — default: off
  coordinator: dev-lead
  teams:
    - agents: [core-specialist, cli-specialist, qa-specialist]
  free_agents: [Explore, Plan]
```

- the main thread may reach **only** `coordinator`;
- the coordinator may reach team leads, and the agents of lead-less teams;
- a lead may reach **its own** team;
- a plain specialist does not sub-delegate;
- `free_agents` are reachable from anywhere — they are assistance agents, outside the topology;
- **anything not declared is refused.**

That last line is the whole design. There is no built-in exemption list: if you use an agent, you
declare it, and declaring it is what makes the config an honest description of your fleet. Claude
Code's own built-ins (`general-purpose`, `Explore`, `Plan`) are not special — declare the ones you
want.

A refusal names the permitted target, which is what lets the model rewrite its call rather than
retry blindly:

```
the declared topology allows only [dev-lead] from the main thread; hand the work to one of
those, or declare 'qa-specialist' in orchestration.teams or orchestration.free_agents
```

`policy` is independent of `enabled`, which governs the `run --orchestrate` engine. Two neighbouring
keys, two different jobs.

## Migrating a project

Turning on `strict` before declaring what you actually use will refuse most of your delegations.
`armadai audit` gives you the inventory: rule `U02` lists every sub-agent that ran but is not
declared in this project's `.claude/agents/`. Read it, then declare each of those agents in
`teams` or `free_agents`, and switch `policy` to `strict`.

One thing that trips people up: `U02` and the gate look at **different declarations**. The audit
reads `.claude/agents/`; the gate reads `orchestration.teams` and `orchestration.free_agents`.
Declaring an agent for the gate therefore does **not** silence `U02` — the rule is an inventory of
what runs, not a checklist that empties as you configure. Treat it as the starting list, not as
progress.

## Installing the hook

Not yet automated. Add it to `.claude/settings.json` (shared) or `.claude/settings.local.json`
(yours only):

```json
{ "hooks": { "PreToolUse": [ { "matcher": "Agent|Task",
  "hooks": [ { "type": "command", "command": "armadai __claude-policy-gate" } ] } ] } }
```

Prefer `armadai` resolved from your `PATH` over an absolute path into a build directory.

## Two things to know before relying on it

**An unreachable gate allows everything, silently.** If the command cannot run — binary not
installed, repository moved, `target/` cleaned — Claude Code receives no opinion and lets the call
through. The failure direction is the safe one, but it is invisible: nothing distinguishes "no
violation" from "no gate". If refusals stop appearing when you expect them, check that the command
still resolves.

**A worktree needs the main checkout.** `.armadai/` is gitignored, so it only exists in your main
checkout. A session opened in a git worktree resolves its config by following the worktree's
`gitdir:` back to that checkout — if you keep your config somewhere unusual, the gate goes quiet
there.

**The policy is local, not shared.** The hook can be committed, but the topology it enforces lives
in `.armadai/config.yaml`, which this project does not version. Two developers can therefore run
the same gate under different rules, or none.

## Limits

The gate judges whether a delegation is *permitted*, never whether it is *apt*: sending a UI task to
the QA specialist is allowed if the topology allows it. It cannot see which files a sub-agent will
touch, nor track a budget or a depth across calls — those need other interception points and
persistent state.

It is specific to Claude Code. The other link targets (codex, copilot, gemini, opencode) have no
sub-agent notion and no equivalent hook, so nothing portable is offered here.

Every uncertainty resolves to *allowed*: unreadable payload or config, no declared coordinator,
`policy: off`. A gate that refuses because it failed to understand is a gate you uninstall the same
day.
