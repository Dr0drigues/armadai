# Audit

`armadai audit` scans a project's *native* Claude Code configuration — no ArmadAI setup required — and reports on it as an adoption funnel: is `.claude/agents/`, `.claude/skills/` and `CLAUDE.md` internally consistent, and does what they *declare* match what Claude Code actually *ran*.

## Usage

```bash
armadai audit [path]                 # defaults to the current directory
armadai audit --report report.md     # write the report to a file (markdown)
armadai audit --report report.html   # ...or HTML, by extension
armadai audit --min-severity warn    # only show findings at or above this severity
armadai audit --quiet                # shortcut for --min-severity warn
armadai audit --propose              # generate an installable ArmadAI pack
armadai audit --deep                 # add an optional LLM-driven pass
```

The command exits non-zero when critical findings exist, regardless of `--min-severity` (which only filters what is displayed). See `armadai audit --help` for the exact option reference.

## What gets checked

Most of the rule surface predates this page's focus and is only summarized here — see `armadai audit --help` and the rule codes printed in the report for the exhaustive list:

- **`A0x` — static asset rules.** Checks over the declared `.claude/agents/`, `.claude/skills/` and `CLAUDE.md` in isolation: unparsable files, missing descriptive fields, deprecated or unknown models, oversized prompts, duplicated content, permissive tool access, malformed skills, broken `@agent` references, plaintext secrets.
- **`C0x` — collision rules.** Checks across declared assets: name collisions, overlapping scopes, overlapping activation surfaces, double ownership of the same module, inconsistent tool restrictions.
- **`D0x` — optional deep pass (`--deep`).** Sends secret-redacted prompt excerpts to an installed CLI (`claude` or `gemini`) for an LLM-driven review, layered on top of the static findings.
- **`--propose`.** Generates an installable ArmadAI pack (`.armadai-proposal/`) from the audited native configuration.

The rest of this page covers the newer half: **observed usage**, rules `U01`–`U04`.

## Observed usage

Beyond what a project declares, `armadai audit` also measures what it actually *ran*, by scanning the project's Claude Code transcripts. A project with no transcripts at all is not an error: the audit still runs to completion, simply without an "Observed usage" section and without any `U0x` finding — the same report you'd get before this feature existed.

### Discovery

Claude Code writes one JSONL transcript per session under `~/.claude/projects/<slug>/`, where `<slug>` is the project's absolute path with every path separator replaced by `-` (e.g. `/Users/x/work/proj` → `-Users-x-work-proj`).

Resolution is two-tier:

1. try the slug directly;
2. if that directory doesn't exist, scan every directory under the projects root and keep the ones whose transcript entries declare the audited project as their `cwd`.

The `cwd` field recorded in each transcript is authoritative; the slug is only an access shortcut, because its exact encoding of dots, underscores and spaces isn't publicly documented.

Internally, only absolute path forms are ever matched against — but that's transparent to you: a relative path you pass (e.g. `armadai audit .`) is canonicalized to an absolute form before matching, so it resolves exactly as expected. Relative forms are never matched *as given*, because that used to actively misfire: a bare `.` made the slug lookup collapse onto the projects root directory itself, before the canonical absolute form — the only one that could ever legitimately match — got a chance to try.

Set `ARMADAI_CLAUDE_PROJECTS_DIR` to point at a different projects root. It exists for the test suite, and for auditing a corpus of transcripts stored elsewhere.

### Opting out

This pass reads this project's own transcript history under `~/.claude/projects/`; that data is only ever read from and aggregated on this machine, never sent anywhere (the *contents* of `--deep`-selected finding messages are the exception — see below). To skip it entirely: pass `--no-usage` on the command line, or set `audit.usage: false` in `armadai.yaml` / `.armadai/config.yaml`. The flag always wins when both are set. Either way, `usage` stays unset and the report looks exactly like it did before this feature existed.

### Scan

The scan streams every transcript line by line rather than loading files into memory — a full scan of a project with a few hundred megabytes of transcripts across dozens of files completes in about a second or two, which is why there is no `--since` flag to restrict the window.

Each line is parsed once. The scan never fails on bad data: an unreadable file is skipped, a malformed line is skipped, and a missing field only degrades the metric it feeds — never the rest of the scan.

### Two metrics, not one

- **Skills** are measured in **attributed turns** (the transcript's `attributionSkill` field) — how many turns a skill actually governed, not how many times it was invoked. A single invocation can end up governing many subsequent turns, so the two numbers can diverge sharply; turns governed is the more honest measure of how much a skill actually shaped a session.
- **Agents** are measured in both **invocations** and **turns**. An invocation is one
  delegation; a turn is one assistant step the agent actually took. Claude Code records each
  sub-agent under `<session-id>/subagents/agent-<id>.jsonl`, with a `.meta.json` alongside it
  carrying `agentType`, `parentAgentId` and `spawnDepth`. The two numbers diverge sharply —
  on this project one agent shows 286 invocations for over 16,000 turns — so reading
  invocations as a measure of work done would badly mislead.
- Because that metadata states the parent and the depth outright, the observed **delegation
  depth** is reported as a fact rather than inferred from a chain of message identifiers.
- Only sub-agents that actually ran leave a `.meta.json`. A delegation refused before it
  started leaves none, so these counts are executions, not attempts.

### Rules

| Rule | Severity | Detects |
|---|---|---|
| `U01` | Warning | A declared agent or skill that never ran across the observed sessions. |
| `U02` | Info | A sub-agent that ran but is not declared in this project's `.claude/agents/` — including Claude Code's own built-ins (`general-purpose`, `Explore`, `Plan`). A non-built-in name may instead come from a plugin, which is outside what this check can see. ArmadAI has no implicit equivalent for the built-ins, which makes this the rule that matters most for migration: ignoring it loses the actual workers. |
| `U03` | Warning | The root instructions name a coordinator that delegations bypass in practice (its observed share of delegations falls below half). |
| `U04` | Info | Activity of a declared skill: how many turns it governed across the scanned sessions, reported without judgement. |

All four rules are silent when nothing was observed for a project: absence of measurement is never treated as evidence of absence of use.

`U03` only fires when exactly one declared agent unambiguously qualifies as "the coordinator" — its `@name` mention must sit on a line that also carries delegation or coordination wording, with a word boundary around the name. If more than one declared agent qualifies, the rule stays silent, because accusing the wrong agent of being bypassed is worse than saying nothing.

### Report output

When transcripts are found, the terminal output and both report formats (`--report out.md` and `--report out.html`) gain an "Observed usage" section — sessions scanned, the observed time window, agents by invocation and skills by attributed turns — printed before any finding. Each of the two invocation/turn lists is truncated to the top 10; an "(+N others)" line says when more were hidden.

`--deep` additionally sends every finding's message — including any U01-U04 message produced by this pass — to the selected LLM CLI as part of its analysis payload.

### Assumed limits

- **Ring and Blackboard orchestration are not inferable from a transcript, and are never proposed.** Claude Code's delegation model is a tree-shaped call/return: there is no cycle to observe for a Ring, and no shared, concurrently-written blackboard to observe either. Guessing either pattern from the shape of an observed tree would be invention dressed up as measurement.
- **A parallel fan-out can't always be attributed precisely.** When one assistant message opens several agents at once, a later delegation nested inside one of them cannot be traced back to that specific sibling — the transcript carries no link from a sub-agent's entry back to the `tool_use` that spawned it. When that happens, the delegation is attributed to the root rather than guessing which sibling it belongs to.

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Migration v0 → v1](migration-v0-to-v1.md) — mentions `armadai audit` as a complement once migration off the legacy format is done
