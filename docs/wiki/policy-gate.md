# Policy gate: enforcing a declared topology

A project declares who delegates to whom in its flows (`.armadai/flows/`). Without a gate, that
declaration governs nothing inside a Claude Code session: the model reads it, can quote it back
verbatim, and still routes elsewhere. Measured on ArmadAI itself, the declared coordinator received
9 delegations out of 520.

The policy gate turns the flows into a rule Claude Code has to obey, through a `PreToolUse` hook
on the delegation tool (`Agent`, and its former name `Task`).

## What it enforces

```yaml
# .armadai/project.yaml
schema_version: 1
coordinator: lead
policy:
  delegation: strict        # off | strict — default: off
  free_agents: [Explore, Plan]
```

Under `strict`, a delegation is allowed when:

- from the **main session**: with a coordinator, the main session *is* the coordinator (it lives
  in `CLAUDE.md`) and drives the flows, so it may reach every agent of a flow step; without a
  coordinator, the agents of entry steps only (the flow's `start` when declared, else the validator's definition, see
  [`no-entry-step`](project-format.md)). An agent run with `claude --agent <coordinator>` counts
  as the main session;
- from an **agent X**: the agents of the steps a step of X leads to, by a link or `on_failure`;
- the target is in `free_agents`, from anywhere — project agents, or agents Claude Code provides
  itself (`Explore`, `Plan`, `general-purpose`…: the validator knows the built-in agents of the
  declared targets and warns, `unknown-free-agent`, only for any other name);
- the coordinator itself is never a target, even when it runs a flow step: it lives in
  `CLAUDE.md`, it is no sub-agent, so a refusal never lists it;
- **anything else is refused.**

An `Agent` call without `subagent_type` (or with an empty one) is judged as the implicit
`general-purpose` agent: omitting the target is not a way around the policy.

A refusal names the permitted targets, which is what lets the model rewrite its call rather than
retry blindly:

```
the declared flows allow only [Explore, dev, qa] from the main thread; hand the work to one of
those, or add `doc` to policy.free_agents in .armadai/project.yaml
```

Under `strict`, the generated `CLAUDE.md` marks the agents the gate will refuse from the main
session ("not delegable"), and the conformance report says
`policy delegation=strict: compiled (enforced: delegation topology, by the PreToolUse gate)`.

## The delegation marker

Under `strict`, the `description` of a delegation to an agent that holds a flow step must start with
a marker naming that step:

```
[<flow>/<step>]           [lot/plan] write the plan
[<flow>/<step>#<lot>]     [lot/plan#12] write the plan
```

`flow` and `step` are ids (`[a-z0-9][a-z0-9-]{0,63}`); the lot is a free name of 1 to 40 ASCII letters (either case), digits, `.` or `-` (`Y2`, `GA1`, `v1.8.0`; `Y2` and `y2` are two lots, `_` and `+` are not allowed); and the
marker is the very first thing of the description. `flow status` and `flow stats` read it to place
the sub-agent that follows (see [Flow status](flow-status.md#the-delegation-marker)); the gate
refuses a delegation without a good one, at once, with a reason that tells the model how to rewrite
its call:

```
the description of a delegation to `reviewer` must start with [flow/step] or [flow/step#lot] (a lot is up to 40 letters, digits, `.` or `-`), with flow/step among: evolution/review, mr/revue-mr
```

```
the marker [evolution/build#menu] does not name a step `reviewer` holds; use one of: evolution/review, mr/revue-mr
```

The first covers a missing or malformed marker, the second a well-formed marker that names a flow
or step the target does not hold. The list is every `<flow>/<step>` the target holds, sorted.

Spared, with no marker asked:

- an agent of `policy.free_agents` (it wins over a step the same agent holds);
- an agent that holds no step in any flow;
- a **resumption** of a sub-agent by `SendMessage`: the gate's matcher is `Agent|Task`, so it never
  sees it (the sub-agent keeps the marker of its first delegation in the log);
- every delegation under `policy.delegation: off`, and a project the gate cannot judge (invalid,
  legacy, no project).

The topology check comes first: a target the flows do not permit is refused with the list of
permitted targets, as before, and the marker is judged only for a permitted one. A refused
delegation records no marker in the flow log.

**Effect on an existing project.** The gate reads `.armadai/project.yaml` at runtime, so a project
under `strict` is refused from the moment a binary with this check is installed, before any new
`link`: its coordinator must mark its delegations at once. Run `armadai link --target claude`
again to get the `### Marking delegations` section in the root `CLAUDE.md` block, which tells the
coordinator how (and, under `strict`, that the gate refuses a delegation without it).

## Installing the hook

`armadai link --target claude` installs it in `<output>/settings.json` (`.claude/settings.json` by
default) when `policy.delegation` is `strict`, and removes it once the policy is `off` again:

```json
{ "hooks": { "PreToolUse": [ { "matcher": "Agent|Task",
  "hooks": [ { "type": "command", "command": "armadai __claude-policy-gate" } ] } ] } }
```

The hook runs `armadai` from your `PATH`.

## Migrating a project

Turning on `strict` before declaring what you actually use will refuse most of your delegations.
Start from an inventory of the sub-agents your sessions actually call — the agents in
`.armadai/agents/`, plus the Claude Code built-ins you rely on — put them in flows or in
`free_agents`, and only then switch `policy.delegation` to `strict`. `armadai migrate` turns a
legacy `orchestration:` section (teams, `policy`, `free_agents`) into a flow and this policy.

## Two things to know before relying on it

**An unreachable gate allows everything, silently.** If the command cannot run — binary not
installed or not on the `PATH` — Claude Code receives no opinion and goes on with its normal
permission flow. The failure direction is the safe one, but it is invisible: if refusals stop
appearing when you expect them, check that the command still resolves.

**An invalid or legacy project disables the gate, with a warning on stderr.** A project that does
not validate ("… is invalid, the delegation gate is inactive") or a legacy one ("legacy format,
run armadai migrate; gate inactive") yields no verdict; stdout stays empty.

**A project whose pack cannot be read is refused, not waved through.** With `pack:`, the gate
reads the project as `link` compiled it: the locked snapshot of the pack, overlaid by your files
(variables stay as written). If that view cannot be opened (the cache was removed, the lock was
advanced by a `git pull`, the pack is missing), the delegation is refused with

```
the project's pack cannot be read, so the delegation policy cannot be checked: <reason>
```

and stderr says `armadai policy gate: <dir> could not be opened, delegation refused: <reason>`.
The reason names the cure when there is one (cache missing: `armadai pack sync`; lock missing or
for another pack: `armadai pack update`). A pack that holds a `pack:` of its own, or a `project.yaml`
the gate cannot read, is refused too, with no cure to name. The policy itself lives
in that view, so the gate cannot tell whether it was `strict`: a project that was linked under
`strict` and then lost its snapshot would otherwise silently lose its gate. A view that opens but
whose merged project has errors is still "no opinion" (the invalid-project case above).

The gate finds the project from the session's directory upwards (without crossing a `.git`
boundary), and from a git worktree falls back to the main checkout when the worktree has no
`.armadai/` of its own. Inside a git submodule there is no such fallback to the superproject.

The gate and the `grant` hook decide on the lock of the checkout they resolve the project in, so a checkout that has
[pinned](starter-packs.md#pinning-a-pack-per-checkout) its pack is gated by the pinned version of
the pack, not by the one `.armadai/pack.lock` now says. When the worktree has no `.armadai/` of
its own and the gate falls back to the main checkout, it reads that checkout's lock: its pin if it
has one, else its `.armadai/pack.lock`, never the worktree's pin. A pinned snapshot that is not in the cache
is refused like any missing snapshot, naming `armadai pack sync`; the `grant` hook, on a pin it
cannot read, gives no opinion.

## Messages between sessions

The gate judges `Agent` and `Task` calls only. A `SendMessage` to another session (the
[session link](link.md#the-lot-journal-and-the-session-link)) is given no opinion, under `strict` too, and no hook can
observe that message: it is not recorded by `flow status` either. A message is not a delegation, and the
topology says nothing about it.

## Limits

The gate judges whether a delegation is *permitted*, never whether it is *apt*. It cannot see
which files a sub-agent will touch, nor track a budget or a depth across calls — those need other
interception points and persistent state. Flow states and `on_failure` are guidance in
`CLAUDE.md`; the gate applies the links only.

It is specific to Claude Code. OpenCode gets the same rule as native permissions
(`permission.task`, see [Link](link.md#opencode)); Codex and Copilot receive no equivalent (their
report says `not-compiled`).

Uncertainty resolves to *no opinion*: an unreadable payload, an absent, legacy or invalid project, a
tool other than `Agent`/`Task`, `policy.delegation: off`. The one exception is a pack the gate
cannot read (above), which is refused, with the cure in the message when there is one. A gate that refuses because it
failed to understand is a gate you uninstall the same day, so the refusal is kept to the case where
the gate knows the project cannot be read.
