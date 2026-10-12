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
  coordinator, the agents of entry steps only (no incoming link nor `on_failure`). An agent run
  with `claude --agent <coordinator>` counts as the main session;
- from an **agent X**: the agents of the steps a step of X leads to, by a link or `on_failure`;
- the target is in `free_agents`, from anywhere — project agents, or agents Claude Code provides
  itself (`Explore`, `Plan`, `general-purpose`: the validator only warns, `unknown-free-agent`);
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

The gate finds the project from the session's directory upwards (without crossing a `.git`
boundary), and from a git worktree falls back to the main checkout when the worktree has no
`.armadai/` of its own.

## Limits

The gate judges whether a delegation is *permitted*, never whether it is *apt*. It cannot see
which files a sub-agent will touch, nor track a budget or a depth across calls — those need other
interception points and persistent state. Flow states and `on_failure` are guidance in
`CLAUDE.md`; the gate applies the links only.

It is specific to Claude Code: Codex, Copilot and OpenCode receive no equivalent (their report
says `not-compiled`).

Every uncertainty resolves to *no opinion*: unreadable payload or project, a tool other than
`Agent`/`Task`, `policy.delegation: off`. A gate that refuses because it failed to understand is a
gate you uninstall the same day.
